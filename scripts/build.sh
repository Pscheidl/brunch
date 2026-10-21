#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
./scripts/trunk.sh build --release --locked --skip-version-check "$@"
python3 scripts/externalize-bootstrap.py dist
