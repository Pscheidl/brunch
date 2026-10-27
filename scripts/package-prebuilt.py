#!/usr/bin/env python3
"""Compile a pinned clean commit locally and package a narrow runtime build context."""

import argparse
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import tomllib

import release_package as release


EXTRA_INPUTS = {"Dockerfile.prebuilt", "scripts/package-prebuilt.py", "scripts/release_package.py"}
RUNTIME_LIBRARIES = {"libgcc_s.so.1", "libm.so.6", "libc.so.6", "ld-linux-x86-64.so.2"}
RUNTIME_TARGET = "x86_64-unknown-linux-gnu"


def command(*args, cwd=None, env=None):
    return subprocess.check_output(args, cwd=cwd, env=env, text=True).strip()


def validate_binary(path):
    with path.open("rb") as stream:
        header = stream.read(20)
    if (len(header) != 20 or header[:6] != b"\x7fELF\x02\x01"
            or int.from_bytes(header[18:20], "little") != 62):
        raise ValueError("The runtime requires a Linux x86_64 ELF binary")
    versions = command("readelf", "--version-info", str(path))
    required = {tuple(map(int, version.split(".")))
                for version in re.findall(r"GLIBC_([0-9.]+)", versions)}
    if not required or max(required) > (2, 36):
        raise ValueError("Backend requires a newer glibc than the Debian bookworm runtime")
    dynamic = command("readelf", "--dynamic", str(path))
    libraries = set(re.findall(r"Shared library: \[([^\]]+)\]", dynamic))
    if not libraries or libraries - RUNTIME_LIBRARIES:
        raise ValueError("Unsupported runtime libraries: " + ", ".join(sorted(libraries)))
    return {"glibc_max": ".".join(map(str, max(required))), "libraries": sorted(libraries)}


def find_tool(name, candidates):
    executable = shutil.which(name)
    if executable:
        return str(Path(executable).absolute())
    for candidate in candidates:
        if candidate and Path(candidate).is_file() and os.access(candidate, os.X_OK):
            return str(Path(candidate).absolute())
    raise ValueError(f"Missing local build tool: {name}")


def compile_runtime(source, runtime, root):
    toolchain = tomllib.loads((source / "rust-toolchain.toml").read_text())["toolchain"]["channel"]
    env = os.environ.copy()
    env["RUSTUP_TOOLCHAIN"] = toolchain
    rustc = command("rustc", "--version", cwd=source, env=env)
    if not rustc.startswith(f"rustc {toolchain} "):
        raise ValueError(f"Expected Rust {toolchain}, got {rustc}")
    trunk = env.get("BRUNCH_TRUNK_BIN") or find_tool(
        "trunk", [root / "target/tools/trunk/trunk", root.parent / "obecni-web/target/tools/trunk/trunk"])
    trunk = str(Path(trunk).absolute())
    cache = Path(env.get("XDG_CACHE_HOME", str(Path.home() / ".cache")))
    bindgen = find_tool("wasm-bindgen", [cache / "trunk/wasm-bindgen-0.2.129/wasm-bindgen"])
    tools = {"rustc": rustc, "cargo": command("cargo", "--version", cwd=source, env=env),
             "trunk": command(trunk, "--version"), "wasm-bindgen": command(bindgen, "--version")}
    if tools["trunk"] != "trunk 0.21.14" or tools["wasm-bindgen"] != "wasm-bindgen 0.2.129":
        raise ValueError("Expected Trunk 0.21.14 and wasm-bindgen 0.2.129")
    env["PATH"] = str(Path(bindgen).parent) + os.pathsep + env["PATH"]
    env["BRUNCH_TRUNK_BIN"] = trunk
    env["CARGO_TARGET_DIR"] = str(root / "target")
    subprocess.run(["./scripts/build.sh", "--public-url", "/"], cwd=source, env=env, check=True)
    subprocess.run([sys.executable, "scripts/check-pages.py", "--dist", "dist", "--public-url", "/"],
                   cwd=source, env=env, check=True)
    env["CARGO_TARGET_DIR"] = str(root / "server/target")
    subprocess.run(["cargo", "build", "--locked", "--release", "--target", RUNTIME_TARGET,
                    "--manifest-path", "server/Cargo.toml"],
                   cwd=source, env=env, check=True)
    binary = root / "server/target" / RUNTIME_TARGET / "release/brunch-server"
    binary_info = validate_binary(binary)
    shutil.copy2(binary, runtime / "brunch-server")
    for path in (source / "dist").rglob("*"):
        if path.is_symlink():
            raise ValueError("Frontend artifacts must not contain symbolic links")
    shutil.copytree(source / "dist", runtime / "dist")
    return {"tools": tools, "binary": binary_info, "platform": "linux/amd64"}


def package(root, output):
    revision = release.run("git", "rev-parse", "HEAD", cwd=root, capture=True).stdout.strip()
    if release.run("git", "status", "--porcelain", "--untracked-files=normal",
                   cwd=root, capture=True).stdout:
        raise ValueError("Working tree is not clean. Commit the release first.")
    version = revision[:12]
    output.mkdir(parents=True, exist_ok=True)
    final_archive = output / f"brunch-prebuilt-{version}.tar.gz"
    final_checksum = output / f"{final_archive.name}.sha256"
    if final_archive.exists() or final_checksum.exists():
        raise ValueError(f"Release already exists: {final_archive.name}")
    tree = release.committed_tree(root, revision)
    inputs = sorted({name for name in tree if release.build_input(name)} | EXTRA_INPUTS)
    missing = (release.BUILD_FILES | EXTRA_INPUTS) - tree.keys()
    if missing:
        raise ValueError("Missing committed build inputs: " + ", ".join(sorted(missing)))
    with tempfile.TemporaryDirectory(prefix=".brunch-prebuilt-", dir=output) as temporary:
        temporary = Path(temporary)
        source = temporary / "source"
        bundle = temporary / f"brunch-prebuilt-{version}"
        runtime = bundle / "runtime"
        source.mkdir()
        runtime.mkdir(parents=True)
        for name in inputs:
            release.copy_committed(root, tree, name, source)
        for name in release.RELEASE_FILES:
            release.copy_committed(root, tree, name, bundle)
        shutil.copy2(source / "Dockerfile.prebuilt", runtime / "Dockerfile")
        (runtime / ".dockerignore").write_text(
            "**\n!Dockerfile\n!brunch-server\n!assets.sha256\n!dist/\n!dist/**\n", encoding="utf-8")
        provenance = compile_runtime(source, runtime, root)
        provenance.update({"source_revision": revision, "source_dirty": False,
                           "build_mode": "locally compiled, runtime assembly required"})
        (bundle / "build-provenance.json").write_text(json.dumps(provenance, indent=2) + "\n")
        (bundle / "build-inputs.sha256").write_text(
            "".join(f"{release.digest(source / name)}  {name}\n" for name in inputs))
        (runtime / "assets.sha256").write_text(
            "".join(f"{release.digest(path)}  {path.relative_to(runtime).as_posix()}\n"
                    for path in sorted((runtime / "dist").rglob("*")) if path.is_file()))
        (bundle / "release.env").write_text(
            f"BRUNCH_IMAGE=brunch:{version}\nSOURCE_REVISION={revision}\n"
            "SOURCE_DIRTY=false\nBUILD_MODE=prebuilt\n")
        (bundle / "SHA256SUMS").write_text(
            "".join(f"{release.digest(path)}  {path.relative_to(bundle).as_posix()}\n"
                    for path in sorted(bundle.rglob("*")) if path.is_file()))
        archive = temporary / final_archive.name
        release.make_archive(bundle, archive)
        checksum = f"{release.digest(archive)}  {archive.name}\n"
        os.replace(archive, final_archive)
        final_checksum.write_text(checksum)
    print(f"Prebuilt release: {final_archive}")
    print(f"Checksum: {final_checksum}")
    print(f"Source revision: {revision}")
    print("The runtime image still needs Docker assembly from the verified runtime/ directory.")
    return final_archive


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", nargs="?", type=Path, default=Path("output/releases"))
    args = parser.parse_args()
    root = Path(__file__).resolve().parent.parent
    try:
        package(root, args.output.resolve())
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        print(f"Prebuilt release failed: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
