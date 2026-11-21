#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
: "${TEST_DATABASE_URL:?Set TEST_DATABASE_URL to an isolated local PostgreSQL database. Tests use temporary schemas.}"
cargo fmt --check
cargo fmt --manifest-path server/Cargo.toml --check
cargo test --locked --lib
cargo test --locked --manifest-path server/Cargo.toml --lib
cargo test --locked --manifest-path server/Cargo.toml --test config
cargo test --locked --manifest-path server/Cargo.toml --test api --test mail --test security --test administrators --test media --test payments --test customer_experience --test team_orders --test pickup --test banquet -- --ignored --test-threads=1
cargo clippy --locked --manifest-path server/Cargo.toml --all-targets -- -D warnings
./scripts/build.sh --public-url /
python3 scripts/check-pages.py --dist dist --public-url /
