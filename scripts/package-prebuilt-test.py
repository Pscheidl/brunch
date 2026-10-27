#!/usr/bin/env python3
"""Check prebuilt release isolation, provenance and runtime compatibility guards."""

from contextlib import redirect_stdout
import hashlib
import importlib.util
import io
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch


def load_module(name, filename):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(filename))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


prebuilt = load_module("prebuilt", "package-prebuilt.py")
existing_tests = load_module("release_tests", "test-release-package.py")


class PrebuiltTests(unittest.TestCase):
    def setUp(self):
        self.fixture = existing_tests.PackagingTests()
        self.fixture.setUp()
        self.addCleanup(self.fixture.doCleanups)
        for name in prebuilt.EXTRA_INPUTS:
            self.fixture.write(name, "committed prebuilt input\n")
        self.fixture.commit()
        self.compiled_source = None

        def compile_runtime(source, runtime, root):
            self.compiled_source = (source / "src/lib.rs").read_bytes()
            (runtime / "brunch-server").write_bytes(b"fake compiled binary")
            (runtime / "dist").mkdir()
            (runtime / "dist/index.html").write_text("<title>Brunch Era</title>")
            return {"platform": "linux/amd64", "tools": {}, "binary": {"glibc_max": "2.34"}}

        self.compiler = patch.object(prebuilt, "compile_runtime", side_effect=compile_runtime)
        self.compile_mock = self.compiler.start()
        self.addCleanup(self.compiler.stop)

    def package(self):
        with redirect_stdout(io.StringIO()):
            return prebuilt.package(self.fixture.root, self.fixture.output)

    def test_bundle_omits_credentials_and_checksums_every_artifact(self):
        for path in [".env", "deploy/.env", "deploy/secrets/teya", "public/private.pem"]:
            self.fixture.write(path, "DO_NOT_PACKAGE_REAL_CREDENTIALS\n")
        archive = self.package()
        contents = self.fixture.content(archive)
        self.assertEqual(set(contents), set(prebuilt.release.RELEASE_FILES) | {
            "runtime/Dockerfile", "runtime/.dockerignore", "runtime/brunch-server",
            "runtime/dist/index.html", "runtime/assets.sha256", "release.env",
            "build-provenance.json", "build-inputs.sha256", "SHA256SUMS",
        })
        for name, data in contents.items():
            self.assertNotIn(b"DO_NOT_PACKAGE_REAL_CREDENTIALS", data, name)
        checksums = dict(line.split("  ", 1)[::-1] for line in contents["SHA256SUMS"].decode().splitlines())
        self.assertEqual(set(checksums), set(contents) - {"SHA256SUMS"})
        for name, expected in checksums.items():
            self.assertEqual(hashlib.sha256(contents[name]).hexdigest(), expected)
        asset_checksum = contents["runtime/assets.sha256"].decode().split("  ", 1)[0]
        self.assertEqual(asset_checksum, hashlib.sha256(contents["runtime/dist/index.html"]).hexdigest())
        self.assertIn(b"SOURCE_DIRTY=false", contents["release.env"])
        self.assertNotIn(b"IMAGE_ID", contents["release.env"])

    def test_dirty_tree_is_rejected_before_compilation(self):
        self.fixture.write("src/lib.rs", "uncommitted change")
        with self.assertRaisesRegex(ValueError, "not clean"):
            self.package()
        self.compile_mock.assert_not_called()

    def test_concurrent_edit_does_not_enter_release(self):
        original = (self.fixture.root / "src/lib.rs").read_bytes()
        original_run = prebuilt.release.run

        def edit_after_clean_status(*args, **kwargs):
            result = original_run(*args, **kwargs)
            if args[:2] == ("git", "status"):
                self.fixture.write("src/lib.rs", "changed after snapshot selection")
            return result

        with patch.object(prebuilt.release, "run", edit_after_clean_status):
            self.package()
        self.assertEqual(self.compiled_source, original)

    def test_committed_symlink_cannot_import_private_file(self):
        outside = self.fixture.base / "private-key"
        outside.write_text("private key")
        (self.fixture.root / "public/leak.txt").symlink_to(outside)
        self.fixture.commit()
        with self.assertRaisesRegex(ValueError, "regular release input"):
            self.package()
        self.compile_mock.assert_not_called()

    def test_existing_release_is_not_overwritten(self):
        archive = self.package()
        original = archive.read_bytes()
        with self.assertRaisesRegex(ValueError, "already exists"):
            self.package()
        self.assertEqual(archive.read_bytes(), original)


class RuntimeCompatibilityTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="brunch-elf-test-")
        self.addCleanup(self.temporary.cleanup)
        self.binary = Path(self.temporary.name) / "brunch-server"
        self.binary.write_bytes(b"\x7fELF\x02\x01" + b"\0" * 12 + (62).to_bytes(2, "little"))

    def test_bookworm_compatible_binary_is_accepted(self):
        with patch.object(prebuilt, "command", side_effect=["GLIBC_2.34 GLIBC_2.2.5",
                          "Shared library: [libc.so.6]\nShared library: [libgcc_s.so.1]"]):
            self.assertEqual(prebuilt.validate_binary(self.binary)["glibc_max"], "2.34")

    def test_newer_glibc_is_rejected(self):
        with patch.object(prebuilt, "command", return_value="GLIBC_2.38"):
            with self.assertRaisesRegex(ValueError, "newer glibc"):
                prebuilt.validate_binary(self.binary)

    def test_unavailable_library_is_rejected(self):
        with patch.object(prebuilt, "command", side_effect=["GLIBC_2.34", "Shared library: [libssl.so.3]"]):
            with self.assertRaisesRegex(ValueError, "Unsupported runtime libraries"):
                prebuilt.validate_binary(self.binary)

    def test_other_architecture_is_rejected(self):
        self.binary.write_bytes(b"\x7fELF\x02\x01" + b"\0" * 12 + (183).to_bytes(2, "little"))
        with self.assertRaisesRegex(ValueError, "x86_64"):
            prebuilt.validate_binary(self.binary)

    def test_target_override_cannot_select_a_stale_native_binary(self):
        root = Path(self.temporary.name) / "repo"
        source = root / "snapshot"
        runtime = root / "runtime"
        source.mkdir(parents=True)
        runtime.mkdir()
        (source / "rust-toolchain.toml").write_text('[toolchain]\nchannel="1.98.1"\n')
        (source / "dist").mkdir()
        (source / "dist/index.html").write_text("current frontend")
        stale_binary = root / "server/target/release/brunch-server"
        stale_binary.parent.mkdir(parents=True)
        stale_binary.write_bytes(b"STALE BINARY")
        current_binary = root / "server/target" / prebuilt.RUNTIME_TARGET / "release/brunch-server"

        def fake_run(args, **kwargs):
            if args[0] == "cargo":
                self.assertEqual(args[args.index("--target") + 1], prebuilt.RUNTIME_TARGET)
                current_binary.parent.mkdir(parents=True)
                current_binary.write_bytes(b"CURRENT BINARY")

        versions = ["rustc 1.98.1 (test)", "cargo 1.98.1 (test)", "trunk 0.21.14", "wasm-bindgen 0.2.129"]
        with patch.dict(os.environ, {"CARGO_BUILD_TARGET": "aarch64-unknown-linux-gnu"}), \
                patch.object(prebuilt, "command", side_effect=versions), \
                patch.object(prebuilt, "find_tool", side_effect=lambda name, _: f"/tools/{name}"), \
                patch.object(prebuilt.subprocess, "run", side_effect=fake_run), \
                patch.object(prebuilt, "validate_binary", return_value={}) as validate:
            prebuilt.compile_runtime(source, runtime, root)
        validate.assert_called_once_with(current_binary)
        self.assertEqual((runtime / "brunch-server").read_bytes(), b"CURRENT BINARY")


if __name__ == "__main__":
    unittest.main()
