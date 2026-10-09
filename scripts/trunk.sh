#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

# Trunk 0.21 expects a boolean even though NO_COLOR commonly contains "1".
if [[ -n "${NO_COLOR:-}" ]]; then
  export NO_COLOR=true
fi

if [[ -n "${BRUNCH_TRUNK_BIN:-}" ]]; then
  brunch_trunk="$BRUNCH_TRUNK_BIN"
elif command -v trunk >/dev/null 2>&1; then
  brunch_trunk="$(command -v trunk)"
elif [[ -x target/tools/trunk/trunk ]]; then
  brunch_trunk="target/tools/trunk/trunk"
elif [[ -x ../obecni-web/target/tools/trunk/trunk ]]; then
  brunch_trunk="../obecni-web/target/tools/trunk/trunk"
else
  printf '%s\n' 'Chybí Trunk. Nainstalujte: cargo install trunk --locked --version 0.21.14' >&2
  exit 1
fi

brunch_bindgen_cache="${XDG_CACHE_HOME:-$HOME/.cache}/trunk/wasm-bindgen-0.2.129"
if ! command -v wasm-bindgen >/dev/null 2>&1 && [[ -x "$brunch_bindgen_cache/wasm-bindgen" ]]; then
  export PATH="$brunch_bindgen_cache:$PATH"
fi

exec "$brunch_trunk" "$@"
