#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
exec ./scripts/trunk.sh serve --locked --skip-version-check "$@"
