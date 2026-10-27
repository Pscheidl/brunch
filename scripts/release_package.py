#!/usr/bin/env python3
"""Build an isolated, secret-free Docker release for offline transfer to a VPS."""

import argparse
from datetime import datetime, timezone
import gzip
import hashlib
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile


BUILD_FILES = {
    "Dockerfile", ".dockerignore", "Cargo.toml", "Cargo.lock", "Trunk.toml",
    "rust-toolchain.toml", "index.html", "server/Cargo.toml", "server/Cargo.lock",
    "scripts/build.sh", "scripts/trunk.sh", "scripts/externalize-bootstrap.py",
    "scripts/check-pages.py",
}
BUILD_DIRECTORIES = ("src/", "style/", "public/", "server/src/", "server/migrations/")
# This is deliberately an allowlist, not a recursive copy of deploy/ or docs/.
RELEASE_FILES = (
    "deploy/compose.yaml", "deploy/Caddyfile", "deploy/.env.example",
    "deploy/compose.preview.yaml", "deploy/.env.preview.example", "docs/private-preview.md",
    "deploy/brunch.sh", "deploy/preflight.py", "deploy/init-db.sh",
    "deploy/brunch-backup.service", "deploy/brunch-backup.timer",
    "deploy/brunch-health.service", "deploy/brunch-health.timer", "docs/zoner-cloud.md",
    "docs/teya-payments.md",
)


def run(*command, cwd=None, capture=False):
    return subprocess.run(command, cwd=cwd, check=True, text=True,
                          stdout=subprocess.PIPE if capture else None)


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def build_input(name):
    parts = Path(name).parts
    if any(part == "secrets" or part == ".git" or part.startswith(".env") for part in parts):
        return False
    if name.endswith((".pem", ".key")):
        return False
    return name in BUILD_FILES or name.startswith(BUILD_DIRECTORIES)


def copy_regular(root, relative, target):
    source = root / relative
    # Do not dereference links that could pull credentials from outside the tree.
    if source.is_symlink() or not source.is_file():
        raise ValueError(f"Expected a regular release input: {relative}")
    if not source.resolve().is_relative_to(root.resolve()):
        raise ValueError(f"Release input escapes the repository: {relative}")
    destination = target / relative
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(source, destination)


def committed_tree(root, revision):
    entries = run("git", "ls-tree", "--full-tree", "-r", "-z", revision,
                  cwd=root, capture=True).stdout.split("\0")
    result = {}
    for entry in entries:
        if entry:
            metadata, name = entry.split("\t", 1)
            mode, kind, object_id = metadata.split()
            result[name] = (mode, kind, object_id)
    return result


def copy_committed(root, tree, relative, target):
    mode, kind, object_id = tree.get(relative, (None, None, None))
    if kind != "blob" or mode not in ("100644", "100755"):
        raise ValueError(f"Expected a regular release input in the selected commit: {relative}")
    destination = target / relative
    destination.parent.mkdir(parents=True, exist_ok=True)
    with destination.open("wb") as stream:
        subprocess.run(["git", "cat-file", "blob", object_id], cwd=root, stdout=stream, check=True)
    destination.chmod(0o755 if mode == "100755" else 0o644)


def make_archive(source, target):
    # Drop the temporary path and current gzip timestamp from the archive header.
    with target.open("xb") as raw:
        with gzip.GzipFile(filename="", mode="wb", fileobj=raw, mtime=0) as compressed:
            with tarfile.open(fileobj=compressed, mode="w|") as archive:
                archive.add(source, arcname=source.name)


def package(root, output, allow_dirty=False):
    revision = run("git", "rev-parse", "HEAD", cwd=root, capture=True).stdout.strip()
    dirty = bool(run("git", "status", "--porcelain", "--untracked-files=normal",
                     cwd=root, capture=True).stdout)
    if dirty and not allow_dirty:
        raise ValueError("Working tree is not clean. Commit the release first, or use --allow-dirty for local verification only.")
    version = revision[:12]
    if dirty:
        version += "-dirty-" + datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%S%fZ")
    image = f"brunch:{version}"
    output.mkdir(parents=True, exist_ok=True)
    final_archive = output / f"brunch-{version}.tar.gz"
    final_checksum = output / f"{final_archive.name}.sha256"
    if final_archive.exists() or final_checksum.exists():
        raise ValueError(f"Release already exists: {final_archive.name}")
    # Read a clean release directly from pinned Git objects. A concurrent edit,
    # checkout or commit after the status check cannot change a clean-tag build.
    tree = committed_tree(root, revision) if not dirty else None
    names = tree.keys() if tree is not None else run(
        "git", "ls-files", "--cached", "--others", "--exclude-standard", "-z",
        cwd=root, capture=True).stdout.split("\0")
    inputs = sorted({name for name in names if name and build_input(name)})
    missing = BUILD_FILES - set(inputs)
    if missing:
        raise ValueError("Missing build inputs: " + ", ".join(sorted(missing)))
    with tempfile.TemporaryDirectory(prefix=".brunch-package-", dir=output) as temporary:
        temporary = Path(temporary)
        context = temporary / "context"
        release = temporary / f"brunch-{version}"
        context.mkdir()
        release.mkdir()
        def snapshot(name, target):
            if tree is not None:
                copy_committed(root, tree, name, target)
            else:
                copy_regular(root, name, target)
        for name in inputs:
            snapshot(name, context)
        for name in RELEASE_FILES:
            snapshot(name, release)
        # Keep a provenance record of precisely the snapshot sent to Docker.
        source_manifest = "".join(f"{digest(context / name)}  {name}\n" for name in inputs)
        (release / "build-inputs.sha256").write_text(source_manifest, encoding="utf-8")
        run("docker", "build", "--platform", "linux/amd64", "--build-arg",
            f"SOURCE_REVISION={revision}{'-dirty' if dirty else ''}", "--tag", image, str(context))
        image_id = run("docker", "image", "inspect", "--format", "{{.Id}}", image,
                       capture=True).stdout.strip()
        if not re.fullmatch(r"sha256:[0-9a-f]{64}", image_id):
            raise ValueError("Docker did not return a valid image digest")
        with (release / "image.tar.gz").open("wb") as raw:
            with gzip.GzipFile(filename="", mode="wb", fileobj=raw, mtime=0) as compressed:
                process = subprocess.Popen(["docker", "image", "save", image], stdout=subprocess.PIPE)
                try:
                    shutil.copyfileobj(process.stdout, compressed)
                finally:
                    process.stdout.close()
                    result = process.wait()
                if result:
                    raise subprocess.CalledProcessError(result, ["docker", "image", "save", image])
        (release / "release.env").write_text(
            f"BRUNCH_IMAGE={image}\nSOURCE_REVISION={revision}\n"
            f"SOURCE_DIRTY={'true' if dirty else 'false'}\nIMAGE_ID={image_id}\n", encoding="utf-8")
        checksums = "".join(f"{digest(path)}  {path.relative_to(release).as_posix()}\n"
                            for path in sorted(release.rglob("*")) if path.is_file())
        (release / "SHA256SUMS").write_text(checksums, encoding="utf-8")
        archive = temporary / final_archive.name
        make_archive(release, archive)
        checksum = f"{digest(archive)}  {archive.name}\n"
        # Publish only a completed archive. Failed builds leave no partial release.
        os.replace(archive, final_archive)
        final_checksum.write_text(checksum, encoding="utf-8")
    print(f"Release: {final_archive}")
    print(f"Checksum: {final_checksum}")
    print(f"Image: {image}")
    if dirty:
        print("Local verification build with uncommitted changes, not a production release.")
    return final_archive


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--allow-dirty", action="store_true",
                        help="Allow uncommitted changes for a visibly marked local verification build")
    parser.add_argument("output", nargs="?", type=Path, default=Path("output/releases"))
    args = parser.parse_args()
    root = Path(__file__).resolve().parent.parent
    try:
        package(root, args.output.resolve(), args.allow_dirty)
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        print(f"Release failed: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
