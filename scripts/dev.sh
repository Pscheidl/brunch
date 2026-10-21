#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
if [[ ! -f .env && -z "${DATABASE_URL:-}" ]]; then
  printf '%s\n' 'Nejprve zkopírujte .env.example do .env a spusťte docker compose up -d.' >&2
  exit 1
fi
./scripts/trunk.sh build --locked --skip-version-check "$@"
python3 scripts/externalize-bootstrap.py dist
exec cargo run --locked --manifest-path server/Cargo.toml
