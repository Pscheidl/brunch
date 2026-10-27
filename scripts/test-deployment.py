#!/usr/bin/env python3
"""Test deployment safeguards without a Docker daemon.

Compose isolation tests use PyYAML when available, otherwise the Compose CLI.
"""

from copy import deepcopy
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import stat
import subprocess
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parent.parent
PREFLIGHT = ROOT / "deploy/preflight.py"
WRAPPER = ROOT / "deploy/brunch.sh"
spec = importlib.util.spec_from_file_location("preflight", PREFLIGHT)
preflight = importlib.util.module_from_spec(spec)
spec.loader.exec_module(preflight)


def load_compose(filename):
    compose_file = ROOT / "deploy" / filename
    try:
        import yaml
    except ImportError:
        if not shutil.which("docker"):
            raise unittest.SkipTest("Compose isolation checks need PyYAML or the Docker Compose CLI")
        result = subprocess.run(
            ["docker", "compose", "-f", str(compose_file), "config", "--no-interpolate", "--format", "json"],
            text=True, capture_output=True, timeout=30, check=True)
        return json.loads(result.stdout)
    return yaml.safe_load(compose_file.read_text())


class ComposeIsolationTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.compose = load_compose("compose.yaml")

    def test_database_has_no_published_port_or_external_route(self):
        db = self.compose["services"]["db"]
        self.assertEqual(set(db["networks"]), {"database"})
        self.assertFalse(db.get("ports"))
        self.assertTrue(self.compose["networks"]["database"]["internal"])
        self.assertFalse(self.compose["networks"]["database"].get("external", False))
        self.assertNotIn("network_mode", db)

    def test_proxy_cannot_join_database_network_but_app_can_reach_both(self):
        services = self.compose["services"]
        self.assertEqual(set(services["app"]["networks"]), {"application", "database"})
        self.assertEqual(set(services["caddy"]["networks"]), {"application"})
        self.assertFalse(services["app"].get("ports"))
        self.assertFalse(self.compose["networks"]["application"].get("internal", False))
        for service in ("app", "caddy"):
            self.assertNotIn("network_mode", services[service])

    def test_database_has_process_and_privilege_limits(self):
        db = self.compose["services"]["db"]
        self.assertGreater(db["pids_limit"], 0)
        self.assertLessEqual(db["pids_limit"], 128)
        self.assertIn("no-new-privileges:true", db["security_opt"])
        self.assertFalse(db.get("privileged", False))


class PreviewIsolationTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.compose = load_compose("compose.preview.yaml")

    def test_preview_exposes_only_loopback_storefront(self):
        services = self.compose["services"]
        self.assertEqual(set(services), {"app", "db", "mailpit"})
        self.assertEqual(self.compose["name"], "brunch-preview")
        self.assertFalse(services["db"].get("ports"))
        self.assertFalse(services["mailpit"].get("ports"))
        ports = services["app"]["ports"]
        self.assertEqual(len(ports), 1)
        if isinstance(ports[0], str):
            self.assertEqual(ports, ["127.0.0.1:3100:3100"])
        else:
            self.assertEqual(ports[0]["host_ip"], "127.0.0.1")
            self.assertEqual(int(ports[0]["published"]), 3100)
            self.assertEqual(int(ports[0]["target"]), 3100)
        for service in services.values():
            self.assertNotIn("network_mode", service)
            self.assertFalse(service.get("privileged", False))

    def test_preview_captures_mail_without_an_external_route_or_payment_configuration(self):
        services = self.compose["services"]
        self.assertEqual(set(services["mailpit"]["networks"]), {"mail_capture"})
        self.assertEqual(set(services["db"]["networks"]), {"database"})
        for network in ("database", "mail_capture"):
            self.assertTrue(self.compose["networks"][network]["internal"])
        self.assertEqual(set(services["app"]["networks"]), {"application", "database", "mail_capture"})
        env = services["app"]["environment"]
        self.assertEqual(env["APP_ENV"], "preview")
        self.assertEqual(env["PUBLIC_URL"], "http://127.0.0.1:3100")
        self.assertEqual(env["SMTP_HOST"], "mailpit")
        self.assertEqual(str(env["SMTP_TLS"]).lower(), "false")
        for key in ("SMTP_USERNAME", "SMTP_PASSWORD", "SMTP_PASSWORD_FILE", "TEYA_ENVIRONMENT"):
            self.assertEqual(env[key], "")
        mail = services["mailpit"]["environment"]
        self.assertEqual(str(mail["MP_SMTP_RELAY_ALL"]).lower(), "false")
        self.assertFalse(any(key.startswith("MP_SMTP_RELAY_") and key != "MP_SMTP_RELAY_ALL" for key in mail))
        self.assertRegex(services["mailpit"]["image"], r"^axllent/mailpit:v[0-9]+\.[0-9]+\.[0-9]+$")


def configuration(secrets):
    return {
        "name": "brunch-production",
        "services": {
            "db": {"environment": {"POSTGRES_PASSWORD": "a" * 64, "BRUNCH_PASSWORD": "b" * 64}},
            "app": {
                "image": "brunch:0123456789ab",
                "environment": {
                    "APP_ENV": "production", "PUBLIC_URL": "https://snackshop.cz",
                    "DATABASE_URL": "postgres://brunch:" + "b" * 64 + "@db:5432/brunch",
                    "ADMIN_EMAIL": "admin@snackshop.cz", "ORDER_EMAIL": "orders@snackshop.cz",
                    "SMTP_FROM": "Brunch <shop@snackshop.cz>", "SMTP_HOST": "smtp.snackshop.cz",
                    "SMTP_PORT": "587", "SMTP_TLS": "true", "SMTP_USERNAME": "shop@snackshop.cz",
                    "SMTP_PASSWORD": "THIS_PASSWORD_MUST_NOT_APPEAR_IN_OUTPUT",
                    "TEYA_ENVIRONMENT": "",
                },
                "volumes": [{"source": str(secrets), "target": "/run/brunch-secrets"}],
            },
            "caddy": {"environment": {"DOMAIN": "snackshop.cz", "ACME_EMAIL": "admin@snackshop.cz"}},
        },
    }


class PreflightTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="brunch-preflight-test-")
        self.addCleanup(self.temporary.cleanup)
        self.base = Path(self.temporary.name)
        self.secrets = self.base / "secrets"
        self.secrets.mkdir()
        self.config = configuration(self.secrets)

    def run_preflight(self, config=None, raw=None):
        return subprocess.run(["python3", str(PREFLIGHT)],
                              input=raw if raw is not None else json.dumps(config or self.config),
                              text=True, capture_output=True, timeout=10)

    def assert_no_credentials(self, result):
        for secret in ("a" * 64, "b" * 64, "THIS_PASSWORD_MUST_NOT_APPEAR_IN_OUTPUT"):
            self.assertNotIn(secret, result.stdout + result.stderr)

    def test_valid_configuration_prints_only_public_metadata(self):
        result = self.run_preflight()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, "https://snackshop.cz\nbrunch-production\n")
        self.assert_no_credentials(result)

    def test_example_template_is_rejected_without_echoing_its_values(self):
        values = dict(line.split("=", 1) for line in (ROOT / "deploy/.env.example").read_text().splitlines()
                      if line and not line.startswith("#") and "=" in line)
        self.config["services"]["caddy"]["environment"]["DOMAIN"] = values["DOMAIN"]
        result = self.run_preflight()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("DOMAIN", result.stderr)
        self.assertNotIn(values["DOMAIN"], result.stderr)
        self.assert_no_credentials(result)

    def test_production_rejects_staging_teya_and_dirty_images(self):
        for field, value in (("TEYA_ENVIRONMENT", "staging"), ("IMAGE", "brunch:0123456789ab-dirty-20261027T120000Z")):
            with self.subTest(field=field):
                changed = deepcopy(self.config)
                if field == "IMAGE":
                    changed["services"]["app"]["image"] = value
                else:
                    changed["services"]["app"]["environment"][field] = value
                result = self.run_preflight(changed)
                self.assertNotEqual(result.returncode, 0)
                self.assert_no_credentials(result)

    def test_malformed_json_never_echoes_credentials(self):
        result = self.run_preflight(raw='{"secret":"THIS_PASSWORD_MUST_NOT_APPEAR_IN_OUTPUT"')
        self.assertNotEqual(result.returncode, 0)
        self.assert_no_credentials(result)

    def test_invalid_field_types_fail_without_tracebacks_or_credentials(self):
        for invalid in (None, 123, ["THIS_PASSWORD_MUST_NOT_APPEAR_IN_OUTPUT"]):
            with self.subTest(value=invalid):
                changed = deepcopy(self.config)
                changed["services"]["caddy"]["environment"]["DOMAIN"] = invalid
                result = self.run_preflight(changed)
                self.assertNotEqual(result.returncode, 0)
                self.assertNotIn("Traceback", result.stderr)
                self.assert_no_credentials(result)

    def test_database_password_validation_does_not_disclose_values(self):
        changed = deepcopy(self.config)
        changed["services"]["db"]["environment"]["POSTGRES_PASSWORD"] = "THIS_PASSWORD_MUST_NOT_APPEAR_IN_OUTPUT"
        result = self.run_preflight(changed)
        self.assertNotEqual(result.returncode, 0)
        self.assert_no_credentials(result)

    def test_secret_paths_cannot_escape_mount_or_follow_symlinks(self):
        env = self.config["services"]["app"]["environment"]
        secret = self.secrets / "teya-client-secret"
        secret.write_text("private")
        secret.chmod(0o600)
        (self.secrets / "linked").symlink_to(secret)
        for value in ("/etc/private", "/run/brunch-secrets/../private", "/run/brunch-secrets/linked"):
            with self.subTest(value=value):
                env["TEYA_CLIENT_SECRET_FILE"] = value
                with self.assertRaises(ValueError):
                    preflight.validate(self.config)

    def test_secret_requires_private_mode_and_container_owner(self):
        secret = self.secrets / "teya-client-secret"
        secret.write_text("private")
        env = self.config["services"]["app"]["environment"]
        env["TEYA_CLIENT_SECRET_FILE"] = "/run/brunch-secrets/teya-client-secret"
        real_stat = Path.stat

        def metadata_with(uid, mode):
            def metadata(path, *args, **kwargs):
                result = real_stat(path, *args, **kwargs)
                if path == secret:
                    items = list(result)
                    items[0] = stat.S_IFREG | mode
                    items[4] = uid
                    return os.stat_result(items)
                return result
            return metadata

        for uid, mode, accepted in ((10001, 0o600, True), (10001, 0o400, True), (10001, 0o644, False), (10002, 0o600, False)):
            with self.subTest(uid=uid, mode=oct(mode)), patch.object(Path, "stat", metadata_with(uid, mode)):
                if accepted:
                    self.assertEqual(preflight.validate(self.config)[1], "brunch-production")
                else:
                    with self.assertRaises(ValueError):
                        preflight.validate(self.config)

    def test_environment_file_permissions_are_checked(self):
        env_file = self.base / "brunch.env"
        env_file.write_text("SMTP_PASSWORD=THIS_PASSWORD_MUST_NOT_APPEAR_IN_OUTPUT\n")
        for mode, accepted in ((0o600, True), (0o400, True), (0o644, False)):
            with self.subTest(mode=oct(mode)):
                env_file.chmod(mode)
                result = subprocess.run(["python3", str(PREFLIGHT), "--env-file", str(env_file)],
                                        text=True, capture_output=True, timeout=10)
                self.assertEqual(result.returncode == 0, accepted)
                self.assert_no_credentials(result)


class BackupWrapperTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="brunch-backup-test-")
        self.addCleanup(self.temporary.cleanup)
        self.base = Path(self.temporary.name)
        self.secrets = self.base / "secrets"
        self.secrets.mkdir()
        self.backups = self.base / "backups"
        self.config = self.base / "compose.json"
        self.config.write_text(json.dumps(configuration(self.secrets)))
        self.env_file = self.base / "brunch.env"
        self.env_file.write_text("SMTP_PASSWORD=THIS_PASSWORD_MUST_NOT_APPEAR_IN_OUTPUT\n")
        self.env_file.chmod(0o600)
        self.log = self.base / "docker-calls.jsonl"
        self.curl_log = self.base / "curl-calls.jsonl"
        self.assets = self.base / "assets"
        self.assets.mkdir()
        (self.assets / "index.html").write_bytes(b"<!doctype html><title>Release</title>")
        (self.assets / "app.wasm").write_bytes(b"\x00asm\x01\x00\x00\x00")
        self.manifest = self.base / "assets.sha256"
        self.manifest.write_text("".join(
            hashlib.sha256(path.read_bytes()).hexdigest() + "  dist/" + path.name + "\n"
            for path in sorted(self.assets.iterdir())))
        self.bin = self.base / "bin"
        self.bin.mkdir()
        docker = self.bin / "docker"
        docker.write_text("""#!/usr/bin/env python3
import json, os, pathlib, sys
args = sys.argv[sys.argv.index('-f') + 2:]
with pathlib.Path(os.environ['FAKE_DOCKER_LOG']).open('a') as log:
    log.write(json.dumps(args) + '\\n')
scenario = os.environ.get('FAKE_DOCKER_SCENARIO', '')
if args[:1] == ['config']:
    print(pathlib.Path(os.environ['FAKE_DOCKER_CONFIG']).read_text())
elif args == ['exec', '-T', 'app', 'cat', '/app/assets.sha256']:
    if scenario == 'manifest-missing':
        sys.exit(21)
    if scenario != 'manifest-empty':
        print(pathlib.Path(os.environ['FAKE_ASSET_MANIFEST']).read_text(), end='')
elif args[:3] == ['exec', '-T', 'db']:
    command = args[3]
    if command == 'pg_dump':
        sys.stdout.buffer.write(b'PGDMP fake backup bytes with customer data')
        if scenario == 'dump-fail':
            sys.exit(17)
    elif command == 'pg_restore':
        sys.stdin.buffer.read()
        if '--list' in args and scenario == 'list-fail':
            sys.exit(18)
        if '--dbname' in args and scenario == 'restore-fail':
            sys.exit(19)
    elif command == 'psql':
        print('0')
    elif command == 'createdb':
        if scenario == 'create-fail':
            sys.exit(20)
    elif command != 'dropdb':
        sys.exit('Unexpected database command')
else:
    sys.exit('Unexpected Docker command')
""")
        docker.chmod(0o755)
        curl = self.bin / "curl"
        curl.write_text("""#!/usr/bin/env python3
import json, os, pathlib, sys, urllib.parse
with pathlib.Path(os.environ['FAKE_CURL_LOG']).open('a') as log:
    log.write(json.dumps(sys.argv[1:]) + '\\n')
url = urllib.parse.urlsplit(sys.argv[-1])
if url.scheme != 'https' or url.netloc != 'snackshop.cz':
    sys.exit('Unexpected public asset URL')
if os.environ.get('FAKE_DOCKER_SCENARIO') == 'http-fail':
    sys.stdout.buffer.write(b'incomplete response')
    sys.exit(22)
if os.environ.get('FAKE_DOCKER_SCENARIO') == 'asset-tampered':
    sys.stdout.buffer.write(b'stale or tampered public response')
else:
    sys.stdout.buffer.write((pathlib.Path(os.environ['FAKE_CURL_ASSETS']) / url.path.lstrip('/')).read_bytes())
""")
        curl.chmod(0o755)
        self.environment = dict(os.environ, PATH=str(self.bin) + os.pathsep + os.environ["PATH"],
                                BRUNCH_ENV_FILE=str(self.env_file), BRUNCH_BACKUP_DIR=str(self.backups),
                                FAKE_DOCKER_LOG=str(self.log), FAKE_DOCKER_CONFIG=str(self.config),
                                FAKE_ASSET_MANIFEST=str(self.manifest), FAKE_CURL_LOG=str(self.curl_log),
                                FAKE_CURL_ASSETS=str(self.assets))

    def run_wrapper(self, *args, scenario=""):
        result = subprocess.run(["bash", str(WRAPPER), *map(str, args)],
                                env=dict(self.environment, FAKE_DOCKER_SCENARIO=scenario),
                                text=True, capture_output=True, timeout=10)
        for secret in ("a" * 64, "b" * 64, "THIS_PASSWORD_MUST_NOT_APPEAR_IN_OUTPUT", "customer data"):
            self.assertNotIn(secret, result.stdout + result.stderr)
        return result

    def calls(self):
        return [json.loads(line) for line in self.log.read_text().splitlines()]

    def create_dump(self):
        dump = self.base / "valid.dump"
        dump.write_bytes(b"PGDMP fixture")
        Path(str(dump) + ".sha256").write_text(hashlib.sha256(dump.read_bytes()).hexdigest() + "  valid.dump\n")
        return dump

    def test_successful_backup_is_private_and_has_matching_checksum(self):
        result = self.run_wrapper("backup")
        self.assertEqual(result.returncode, 0, result.stderr)
        dumps = list(self.backups.glob("*.dump"))
        self.assertEqual(len(dumps), 1)
        self.assertEqual(stat.S_IMODE(dumps[0].stat().st_mode), 0o600)
        self.assertEqual(stat.S_IMODE(self.backups.stat().st_mode), 0o700)
        expected = Path(str(dumps[0]) + ".sha256").read_text().split()[0]
        self.assertEqual(hashlib.sha256(dumps[0].read_bytes()).hexdigest(), expected)

    def test_dump_or_validation_failure_never_publishes_partial_backup(self):
        for scenario in ("dump-fail", "list-fail"):
            with self.subTest(scenario=scenario):
                result = self.run_wrapper("backup", scenario=scenario)
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(list(self.backups.glob("*.dump*")), [])
                self.assertEqual(list(self.backups.glob(".partial.*")), [])

    def test_missing_payment_secret_does_not_block_database_backup(self):
        config = json.loads(self.config.read_text())
        config["services"]["app"]["environment"].update({
            "TEYA_ENVIRONMENT": "production",
            "TEYA_CLIENT_SECRET_FILE": "/run/brunch-secrets/missing-teya-secret",
        })
        self.config.write_text(json.dumps(config))
        result = self.run_wrapper("backup")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(len(list(self.backups.glob("*.dump"))), 1)

    def test_restore_targets_only_temporary_database_and_cleans_up(self):
        dump = self.create_dump()
        result = self.run_wrapper("restore-check", dump)
        self.assertEqual(result.returncode, 0, result.stderr)
        calls = self.calls()
        created = next(call for call in calls if "createdb" in call)[-1]
        self.assertRegex(created, r"^brunch_verify_[0-9a-f]{16}$")
        for call in calls:
            if "--dbname" in call:
                self.assertEqual(call[call.index("--dbname") + 1], created)
            if "psql" in call:
                self.assertEqual(call[call.index("-d") + 1], created)
            if "dropdb" in call:
                self.assertEqual(call[-1], created)
        self.assertEqual(sum("dropdb" in call for call in calls), 1)

    def test_restore_failure_drops_only_created_temporary_database(self):
        result = self.run_wrapper("restore-check", self.create_dump(), scenario="restore-fail")
        self.assertNotEqual(result.returncode, 0)
        calls = self.calls()
        created = next(call for call in calls if "createdb" in call)[-1]
        drop = next(call for call in calls if "dropdb" in call)
        self.assertEqual(drop[-1], created)
        self.assertIn("--if-exists", drop)

    def test_checksum_failure_does_not_create_or_restore_a_database(self):
        dump = self.create_dump()
        dump.write_bytes(b"tampered")
        result = self.run_wrapper("restore-check", dump)
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(any("createdb" in call or "pg_restore" in call for call in self.calls()))

    def test_failed_database_creation_does_not_drop_an_existing_database(self):
        result = self.run_wrapper("restore-check", self.create_dump(), scenario="create-fail")
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(any("dropdb" in call or "pg_restore" in call for call in self.calls()))

    def test_verify_release_compares_every_public_asset_byte_for_byte(self):
        result = self.run_wrapper("verify-release")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("Every public release asset matches", result.stdout)
        requests = [json.loads(line) for line in self.curl_log.read_text().splitlines()]
        self.assertEqual({args[-1] for args in requests}, {
            "https://snackshop.cz/index.html", "https://snackshop.cz/app.wasm",
        })
        for args in requests:
            self.assertIn("Cache-Control: no-cache", args)
            self.assertEqual(args[args.index("--proto") + 1], "=https")
            self.assertIn("--fail", args)

    def test_verify_release_fails_on_tamper_missing_manifest_or_http_error(self):
        for scenario in ("asset-tampered", "manifest-missing", "manifest-empty", "http-fail"):
            with self.subTest(scenario=scenario):
                self.curl_log.unlink(missing_ok=True)
                result = self.run_wrapper("verify-release", scenario=scenario)
                self.assertNotEqual(result.returncode, 0)
                self.assertNotIn("Every public release asset matches", result.stdout)
                if scenario.startswith("manifest-"):
                    self.assertFalse(self.curl_log.exists())


if __name__ == "__main__":
    unittest.main()
