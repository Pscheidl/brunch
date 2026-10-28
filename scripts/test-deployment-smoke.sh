#!/usr/bin/env bash
# Test a prebuilt image locally. No ports are published and no payment or mail is sent.
set -euo pipefail
umask 077
brunch_root="$(cd "$(dirname "$0")/.." && pwd)"
brunch_image="${BRUNCH_SMOKE_IMAGE:-brunch:000000000000}"
[[ "$brunch_image" =~ ^brunch:[a-f0-9]{12}$ ]] || {
  printf '%s\n' 'BRUNCH_SMOKE_IMAGE must be a versioned Brunch image.' >&2
  exit 1
}
docker image inspect "$brunch_image" >/dev/null
if [[ -n "${DOCKER_CONTEXT:-}" ]]; then
  brunch_docker_host="$(docker context inspect "$DOCKER_CONTEXT" --format '{{.Endpoints.docker.Host}}')"
else
  brunch_docker_host="${DOCKER_HOST:-$(docker context inspect --format '{{.Endpoints.docker.Host}}')}"
fi
[[ "$brunch_docker_host" == unix://* ]] || {
  printf '%s\n' 'Deployment smoke requires a local Docker socket.' >&2
  exit 1
}
for brunch_kind in container volume network; do
  brunch_list_args=(ls -q --filter label=com.docker.compose.project=brunch-staging)
  [[ "$brunch_kind" != container ]] || brunch_list_args+=(-a)
  brunch_existing="$(docker "$brunch_kind" "${brunch_list_args[@]}")"
  if [[ -n "$brunch_existing" ]]; then
    printf '%s\n' 'Refusing to use existing brunch-staging resources. Use an isolated Docker environment.' >&2
    exit 1
  fi
done
brunch_fixture="$(mktemp -d /tmp/brunch-deployment.XXXXXX)"
export BRUNCH_ENV_FILE="$brunch_fixture/brunch.env"
export BRUNCH_BACKUP_DIR="$brunch_fixture/backups"
brunch_owns_project=false
brunch_environment=("PATH=$PATH" "HOME=$HOME" "BRUNCH_ENV_FILE=$BRUNCH_ENV_FILE" "BRUNCH_BACKUP_DIR=$BRUNCH_BACKUP_DIR")
for brunch_key in DOCKER_HOST DOCKER_CONTEXT DOCKER_CONFIG XDG_RUNTIME_DIR; do
  [[ ! -v "$brunch_key" ]] || brunch_environment+=("$brunch_key=${!brunch_key}")
done
clean_env() {
  env -i "${brunch_environment[@]}" "$@"
}
compose() {
  clean_env docker compose --project-directory "$brunch_root/deploy" \
    --env-file "$BRUNCH_ENV_FILE" -f "$brunch_root/deploy/compose.yaml" \
    -f "$brunch_root/deploy/compose.test.yaml" "$@"
}
cleanup() {
  local brunch_result=$?
  set +e
  if [[ "$brunch_owns_project" == true ]]; then
    compose down --volumes --remove-orphans >/dev/null || brunch_result=1
  fi
  rm -rf "$brunch_fixture"
  exit "$brunch_result"
}
trap cleanup EXIT
python3 - "$brunch_fixture" "$brunch_image" <<'PY'
from pathlib import Path
import secrets
import sys
root = Path(sys.argv[1])
(root / "secrets").mkdir(mode=0o755)
(root / "secrets").chmod(0o755)
settings = {
    "COMPOSE_PROJECT_NAME": "brunch-staging", "APP_ENV": "staging",
    "BRUNCH_IMAGE": sys.argv[2], "DOMAIN": "shop.brunch-ci.cz",
    "ACME_EMAIL": "admin@brunch-ci.cz",
    "ORDER_EMAIL": "orders@brunch-ci.cz", "SMTP_FROM": "shop@brunch-ci.cz",
    "SMTP_HOST": "smtp.brunch-ci.cz", "SMTP_PORT": "587",
    "SMTP_USERNAME": "", "SMTP_PASSWORD": "", "SMTP_PASSWORD_FILE": "",
    "POSTGRES_ADMIN_PASSWORD": secrets.token_hex(32),
    "POSTGRES_PASSWORD": secrets.token_hex(32),
    "SECRETS_DIR": str(root / "secrets"), "TEYA_ENVIRONMENT": "",
    "ONLINE_PAYMENT_TIMEOUT_MINUTES": "60",
}
(root / "brunch.env").write_text("".join(f"{key}={value}\n" for key, value in settings.items()))
(root / "brunch.env").chmod(0o600)
PY
brunch_owns_project=true
# One-off validation does not publish the proxy's service ports or request certificates.
clean_env "$brunch_root/deploy/brunch.sh" check
compose up -d --wait --wait-timeout 180 db app
brunch_app_id="$(compose ps -q app)"
brunch_db_id="$(compose ps -q db)"
docker inspect "$brunch_app_id" "$brunch_db_id" | python3 -c '
import json, sys
for container in json.load(sys.stdin):
    assert not container["HostConfig"]["PortBindings"], "Smoke services must not publish ports"
'
sql() {
  compose exec -T db psql -U postgres -d brunch -v ON_ERROR_STOP=1 -Atc "$1"
}
[[ "$(sql "SELECT rolsuper OR rolcreatedb OR rolcreaterole FROM pg_roles WHERE rolname='brunch'")" == f ]]
[[ "$(sql "SELECT pg_get_userbyid(datdba) FROM pg_database WHERE datname='brunch'")" == brunch ]]
[[ "$(sql 'SELECT count(*) FROM products')" == 0 ]]
[[ "$(sql 'SELECT count(*) FROM shop_settings')" == 1 ]]
[[ "$(sql 'SELECT bool_and(success) FROM _sqlx_migrations')" == t ]]
compose exec -T app curl --fail --silent --show-error http://127.0.0.1:3100/api/health |
  python3 -c 'import json,sys; assert json.load(sys.stdin) == {"status":"ok"}'
# Verify both the packaged files and every HTTP response against the release manifest.
compose exec -T app sh -ec '
  cd /app
  sha256sum -c assets.sha256 >/dev/null
  sample=$(mktemp)
  trap '\''rm -f "$sample"'\'' EXIT
  while read -r expected path; do
    curl --fail --silent --show-error --max-time 10 \
      "http://127.0.0.1:3100/${path#dist/}" --output "$sample"
    printf "%s  %s\n" "$expected" "$sample" | sha256sum -c - >/dev/null
  done < assets.sha256
'
sql 'UPDATE shop_settings SET delivery_fee_cents=7100 WHERE id=1' >/dev/null
clean_env "$brunch_root/deploy/brunch.sh" backup
mapfile -t brunch_dumps < <(find "$BRUNCH_BACKUP_DIR" -maxdepth 1 -name '*.dump' -type f)
[[ "${#brunch_dumps[@]}" == 1 ]]
clean_env "$brunch_root/deploy/brunch.sh" restore-check "${brunch_dumps[0]}"
[[ "$(sql "SELECT count(*) FROM pg_database WHERE datname LIKE 'brunch_verify_%'")" == 0 ]]
compose stop app
[[ "$(docker inspect --format '{{.State.ExitCode}}' "$brunch_app_id")" == 0 ]]
compose stop db
compose up -d --wait --wait-timeout 180 db app
[[ "$(sql 'SELECT delivery_fee_cents FROM shop_settings WHERE id=1')" == 7100 ]]
[[ "$(sql 'SELECT count(*) FROM products')" == 0 ]]
printf '%s\n' 'Deployment smoke passed: configuration, database permissions, empty catalogue, HTTP assets, backup restore and graceful restart.'
