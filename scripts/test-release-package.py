#!/usr/bin/env python3
"""Exercise release packaging with a fake Docker CLI, without a daemon or network."""

from contextlib import redirect_stdout
import gzip
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import subprocess
import tarfile
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("release_package", Path(__file__).with_name("release_package.py"))
release = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release)


class PackagingTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="brunch-package-test-")
        self.addCleanup(self.temporary.cleanup)
        self.base = Path(self.temporary.name)
        self.root = self.base / "repo"
        self.root.mkdir()
        self.output = self.base / "releases"
        self.bin = self.base / "bin"
        self.bin.mkdir()
        self.log = self.base / "docker.json"
        docker = self.bin / "docker"
        docker.write_text("""#!/usr/bin/env python3
import json, os, pathlib, sys
if sys.argv[1] == 'build':
    paths = sorted(str(p.relative_to(sys.argv[-1])) for p in pathlib.Path(sys.argv[-1]).rglob('*') if p.is_file())
    pathlib.Path(os.environ['FAKE_DOCKER_LOG']).write_text(json.dumps(paths))
    if os.environ.get('FAKE_DOCKER_FAIL') == '1':
        sys.exit(9)
elif sys.argv[1:3] == ['image', 'inspect']:
    print('sha256:' + 'a' * 64)
elif sys.argv[1:3] == ['image', 'save']:
    sys.stdout.buffer.write(b'fake image archive for packaging tests')
else:
    sys.exit('Unexpected fake Docker command')
""")
        docker.chmod(0o755)
        self.environment = patch.dict(os.environ, {
            "PATH": str(self.bin) + os.pathsep + os.environ["PATH"],
            "FAKE_DOCKER_LOG": str(self.log),
        })
        self.environment.start()
        self.addCleanup(self.environment.stop)
        for relative in release.BUILD_FILES | set(release.RELEASE_FILES) | {"src/lib.rs", "server/src/main.rs", "public/favicon.svg"}:
            self.write(relative, "placeholder build or deployment input\n")
        self.write(".gitignore", ".env\nsecrets/\n*.pem\n")
        self.git("init", "--quiet")
        self.git("config", "user.email", "packaging@example.test")
        self.git("config", "user.name", "Packaging Test")
        self.git("config", "commit.gpgsign", "false")
        self.git("config", "core.hooksPath", "/dev/null")
        self.commit()

    def write(self, relative, content):
        path = self.root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content)

    def git(self, *arguments):
        return subprocess.run(["git", *arguments], cwd=self.root, check=True,
                              stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)

    def commit(self):
        self.git("add", ".")
        self.git("commit", "--quiet", "-m", "Packaging fixture")

    def package(self, allow_dirty=False):
        with redirect_stdout(io.StringIO()):
            return release.package(self.root, self.output, allow_dirty)

    def content(self, archive):
        with tarfile.open(archive, "r:gz") as tar:
            return {"/".join(Path(member.name).parts[1:]): tar.extractfile(member).read()
                    for member in tar.getmembers() if member.isfile()}

    def test_release_contains_only_allowed_inputs_and_checksums(self):
        for path in [".env", "deploy/.env", "deploy/secrets/teya", "public/private.pem"]:
            self.write(path, "DO_NOT_PACKAGE_REAL_CREDENTIALS\n")
        archive = self.package()
        contents = self.content(archive)
        self.assertEqual(set(contents), set(release.RELEASE_FILES) | {
            "release.env", "image.tar.gz", "SHA256SUMS", "build-inputs.sha256",
        })
        for name, data in contents.items():
            self.assertNotIn(b"DO_NOT_PACKAGE_REAL_CREDENTIALS", data, name)
        self.assertEqual(gzip.decompress(contents["image.tar.gz"]), b"fake image archive for packaging tests")
        checksums = dict(line.split("  ", 1)[::-1] for line in contents["SHA256SUMS"].decode().splitlines())
        self.assertEqual(set(checksums), set(contents) - {"SHA256SUMS"})
        for name, expected in checksums.items():
            self.assertEqual(hashlib.sha256(contents[name]).hexdigest(), expected)
        outer_checksum = Path(str(archive) + ".sha256").read_text().split("  ", 1)[0]
        self.assertEqual(release.digest(archive), outer_checksum)
        context = json.loads(self.log.read_text())
        self.assertNotIn(".env", context)
        self.assertNotIn("public/private.pem", context)
        self.assertFalse(any(path.startswith("deploy/") for path in context))
        self.assertIn("SOURCE_DIRTY=false", contents["release.env"].decode())

    def test_default_refuses_uncommitted_changes_before_docker(self):
        self.write("src/lib.rs", "uncommitted change")
        with self.assertRaisesRegex(ValueError, "not clean"):
            self.package()
        self.assertFalse(self.log.exists())

    def test_explicit_dirty_build_is_marked(self):
        self.write("src/lib.rs", "uncommitted change")
        archive = self.package(allow_dirty=True)
        self.assertIn("-dirty-", archive.name)
        self.assertIn("SOURCE_DIRTY=true", self.content(archive)["release.env"].decode())

    def test_concurrent_edit_cannot_enter_clean_release(self):
        original_source = (self.root / "src/lib.rs").read_bytes()
        original_document = (self.root / "docs/teya-payments.md").read_bytes()
        original_run = release.run

        def edit_after_clean_status(*command, **kwargs):
            result = original_run(*command, **kwargs)
            if command[:2] == ("git", "status"):
                self.write("src/lib.rs", "changed while packaging")
                self.write("docs/teya-payments.md", "changed while packaging")
            return result

        with patch.object(release, "run", edit_after_clean_status):
            archive = self.package()
        contents = self.content(archive)
        source_hashes = dict(line.split("  ", 1)[::-1] for line in contents["build-inputs.sha256"].decode().splitlines())
        self.assertEqual(source_hashes["src/lib.rs"], hashlib.sha256(original_source).hexdigest())
        self.assertEqual(contents["docs/teya-payments.md"], original_document)
        self.assertIn("SOURCE_DIRTY=false", contents["release.env"].decode())

    def test_symlink_cannot_pull_a_secret_into_build_context(self):
        outside = self.base / "private-secret"
        outside.write_text("secret")
        (self.root / "public/leak.txt").symlink_to(outside)
        self.commit()
        with self.assertRaisesRegex(ValueError, "regular release input"):
            self.package()
        self.assertFalse(self.log.exists())

    def test_failed_build_leaves_no_partial_release(self):
        with patch.dict(os.environ, {"FAKE_DOCKER_FAIL": "1"}):
            with self.assertRaises(subprocess.CalledProcessError):
                self.package()
        self.assertEqual(list(self.output.iterdir()), [])

    def test_existing_release_is_never_overwritten(self):
        archive = self.package()
        original = archive.read_bytes()
        with self.assertRaisesRegex(ValueError, "already exists"):
            self.package()
        self.assertEqual(archive.read_bytes(), original)


if __name__ == "__main__":
    unittest.main()
