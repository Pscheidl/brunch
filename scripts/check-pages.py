#!/usr/bin/env python3
"""Check a Trunk output's local assets as served under a Pages base path.

This checks generated HTML, CSS URLs and static JS/WASM references. It does
not run the WebAssembly app, so interactive behaviour still needs a browser.
"""

import argparse
from html.parser import HTMLParser
from pathlib import Path
import re
import sys
from urllib.parse import unquote, urljoin, urlsplit


ORIGIN = "https://brunch.invalid"
CSS_URL = re.compile(r"url\(\s*(['\"]?)(.*?)\1\s*\)", re.IGNORECASE)
JS_ASSET = re.compile(r"['\"]([^'\"\\\n]+\.(?:js|wasm)(?:[?#][^'\"\\\n]*)?)['\"]")
JS_TOKEN = re.compile(
    r"(?P<comment>//[^\n]*|/\*[\s\S]*?\*/)"
    r"|(?P<string>'(?:\\.|[^'\\])*'|\"(?:\\.|[^\"\\])*\")"
    r"|(?P<template>`(?:\\.|[^`\\])*`)"
    r"|(?P<word>[A-Za-z_$][\w$]*)"
    r"|(?P<symbol>[^\s])"
)


def module_references(script: str):
    """Yield literal import/re-export specifiers, ignoring other JS strings.

    wasm-bindgen contains logical import-object keys and a fallback WASM URL.
    Trunk supplies the real, hashed WASM URL from its inline HTML startup.
    Neither of those generated fallback strings is a module import.
    """
    tokens = [
        (token.lastgroup, token.group())
        for token in JS_TOKEN.finditer(script)
        if token.lastgroup != "comment"
    ]
    for index, token in enumerate(tokens[:-1]):
        if token not in (("word", "import"), ("word", "export")):
            continue
        kind, value = tokens[index + 1]
        if token[1] == "import":
            if kind == "string":
                yield value[1:-1]
                continue
            if value == "(":
                if index + 2 < len(tokens) and tokens[index + 2][0] == "string":
                    yield tokens[index + 2][1][1:-1]
                continue
            if value == ".":
                continue
        elif value not in ("{", "*"):
            continue
        for cursor in range(index + 1, len(tokens) - 1):
            current = tokens[cursor]
            if current[1] == ";":
                break
            if current == ("word", "from") and tokens[cursor + 1][0] == "string":
                yield tokens[cursor + 1][1][1:-1]
                break


class Document(HTMLParser):
    def __init__(self):
        super().__init__()
        self.bases = []
        self.references = []
        self.inline_scripts = []
        self.in_script = False

    def handle_starttag(self, tag, attributes):
        attrs = dict(attributes)
        if tag == "base":
            self.bases.append(attrs.get("href", ""))
            return
        for attribute in ("href", "src", "poster"):
            if attrs.get(attribute):
                self.references.append(attrs[attribute])
        if tag == "script":
            self.in_script = True

    def handle_endtag(self, tag):
        if tag == "script":
            self.in_script = False

    def handle_data(self, data):
        if self.in_script:
            self.inline_scripts.append(data)


def check(dist: Path, public_url: str) -> list[str]:
    errors = []
    root = dist.resolve()
    if not (root / "index.html").is_file():
        return [f"Missing generated entry point: {root / 'index.html'}"]

    def asset(reference: str, context: str, source: Path):
        reference = reference.strip()
        if not reference or reference.startswith("#"):
            return
        parsed = urlsplit(urljoin(context, reference))
        if parsed.scheme != "https" or parsed.netloc != "brunch.invalid":
            return
        path = unquote(parsed.path)
        if not path.startswith(public_url):
            errors.append(f"{source.name}: {reference!r} escapes {public_url}")
            return
        local = (root / path[len(public_url):]).resolve()
        if not local.is_relative_to(root):
            errors.append(f"{source.name}: {reference!r} escapes the output directory")
            return
        if local.is_dir():
            local /= "index.html"
        if not local.is_file():
            errors.append(f"{source.name}: missing asset {reference!r} ({local.relative_to(root)})")

    for path in sorted(root.rglob("*.html")):
        document_url = ORIGIN + public_url + path.relative_to(root).as_posix()
        document = Document()
        document.feed(path.read_text(encoding="utf-8"))
        if document.bases != [public_url]:
            errors.append(f"{path.name}: expected one <base href={public_url!r}>, got {document.bases!r}")
        context = urljoin(document_url, document.bases[0]) if document.bases else document_url
        for reference in document.references:
            asset(reference, context, path)
        for script in document.inline_scripts:
            for reference in JS_ASSET.findall(script):
                asset(reference, context, path)

    for path in sorted(root.rglob("*.css")):
        context = ORIGIN + public_url + path.relative_to(root).as_posix()
        for _, reference in CSS_URL.findall(path.read_text(encoding="utf-8")):
            asset(reference, context, path)

    for path in sorted(root.rglob("*.js")):
        context = ORIGIN + public_url + path.relative_to(root).as_posix()
        for reference in module_references(path.read_text(encoding="utf-8")):
            asset(reference, context, path)

    wasm_files = list(root.rglob("*.wasm"))
    if not wasm_files:
        errors.append("No generated WebAssembly module found")
    for path in wasm_files:
        with path.open("rb") as module:
            if module.read(4) != b"\x00asm":
                errors.append(f"{path.name}: invalid WebAssembly header")
    for suffix in ("*.js", "*.css"):
        if not any(root.rglob(suffix)):
            errors.append(f"No generated {suffix} asset found")
    return errors


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--dist", type=Path, default=Path("dist"))
    parser.add_argument("--public-url", required=True)
    args = parser.parse_args()
    if not re.fullmatch(r"/(?:[A-Za-z0-9._~-]+/)*", args.public_url) or any(
        part in (".", "..") for part in args.public_url.split("/")
    ):
        parser.error("--public-url must be / or an absolute path ending in /, e.g. /brunch/")
    errors = check(args.dist, args.public_url)
    if errors:
        print("Static Pages checks failed:", file=sys.stderr)
        for error in errors:
            print(f"  {error}", file=sys.stderr)
        return 1
    print(f"Static Pages checks passed for {args.public_url}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
