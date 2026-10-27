#!/usr/bin/env bash
set -euo pipefail
umask 077
brunch_deploy_dir="$(cd "$(dirname "$0")" && pwd -P)"
brunch_env_file="${BRUNCH_ENV_FILE:-/etc/brunch/brunch.env}"
brunch_action="${1:-help}"

if [[ "$brunch_action" == help || "$brunch_action" == --help ]]; then
  printf '%s\n' 'Usage: brunch.sh check|up|restart|health|verify-release|status|logs|backup|restore-check DUMP' \
    'BRUNCH_ENV_FILE defaults to /etc/brunch/brunch.env' \
    'BRUNCH_BACKUP_DIR defaults to /var/backups/brunch, BRUNCH_BACKUP_DAYS to 7'
  exit 0
fi
case "$brunch_action" in
  check|up|restart|health|verify-release|status|logs|backup|restore-check) ;;
  *) printf '%s\n' 'Unknown operation. Use brunch.sh help.' >&2; exit 2 ;;
esac

python3 "$brunch_deploy_dir/preflight.py" --env-file "$brunch_env_file"
compose() {
  docker compose --project-directory "$brunch_deploy_dir" \
    --env-file "$brunch_env_file" -f "$brunch_deploy_dir/compose.yaml" "$@"
}
# Do not print or save the resolved Compose file, it contains credentials.
brunch_preflight_args=()
case "$brunch_action" in
  backup|restore-check|status|logs) brunch_preflight_args+=(--skip-secret-files) ;;
esac
brunch_metadata="$(compose config --format json | python3 "$brunch_deploy_dir/preflight.py" "${brunch_preflight_args[@]}")"
mapfile -t brunch_settings <<< "$brunch_metadata"
brunch_public_url="${brunch_settings[0]}"
brunch_project="${brunch_settings[1]}"

check() {
  compose run --rm --no-deps app --check-config
  compose run --rm --no-deps caddy caddy validate --config /etc/caddy/Caddyfile --adapter caddyfile
}

health() {
  compose exec -T app curl --fail --silent --show-error --max-time 10 http://127.0.0.1:3100/api/health |
    python3 -c 'import json,sys; assert json.load(sys.stdin) == {"status":"ok"}'
  compose exec -T app sh -c 'cd /app && sha256sum -c /app/assets.sha256' >/dev/null
  curl --fail --silent --show-error --proto '=https' --max-time 15 "$brunch_public_url/api/health" |
    python3 -c 'import json,sys; assert json.load(sys.stdin) == {"status":"ok"}'
  for brunch_path in / /admin/ /bootstrap.js; do
    curl --fail --silent --show-error --proto '=https' --max-time 15 \
      "$brunch_public_url$brunch_path" >/dev/null
  done
  printf '%s\n' 'Application, database, release assets and public HTTPS are healthy.'
}

verify_release() {
  local brunch_manifest brunch_expected brunch_file brunch_actual
  brunch_manifest="$(compose exec -T app cat /app/assets.sha256)"
  [[ -n "$brunch_manifest" ]] || return 1
  while read -r brunch_expected brunch_file; do
    [[ "$brunch_expected" =~ ^[a-f0-9]{64}$ && "$brunch_file" =~ ^dist/[a-zA-Z0-9_./-]+$ && "$brunch_file" != *..* ]] || {
      printf '%s\n' 'Invalid release asset manifest.' >&2
      return 1
    }
    brunch_actual="$(curl --fail --silent --show-error --proto '=https' --max-time 30 \
      -H 'Cache-Control: no-cache' "$brunch_public_url/${brunch_file#dist/}" | sha256sum)"
    if [[ "${brunch_actual%% *}" != "$brunch_expected" ]]; then
      printf 'Public release asset differs: %s\n' "$brunch_file" >&2
      return 1
    fi
  done <<< "$brunch_manifest"
  printf '%s\n' 'Every public release asset matches the installed image.'
}

backup() {
  local brunch_backup_dir="${BRUNCH_BACKUP_DIR:-/var/backups/brunch}"
  local brunch_days="${BRUNCH_BACKUP_DAYS:-7}"
  [[ "$brunch_backup_dir" == /* && "$brunch_days" =~ ^[1-9][0-9]*$ ]] || {
    printf '%s\n' 'Use an absolute backup directory and a positive retention in days.' >&2
    return 1
  }
  mkdir -p "$brunch_backup_dir"
  chmod 700 "$brunch_backup_dir"
  exec 9>"$brunch_backup_dir/.backup.lock"
  flock -n 9 || { printf '%s\n' 'Another backup is running.' >&2; return 1; }
  local brunch_dump="$brunch_backup_dir/$brunch_project-$(date -u +%Y%m%dT%H%M%SZ)-$(python3 -c 'import secrets; print(secrets.token_hex(4))').dump"
  local brunch_partial
  brunch_partial="$(mktemp "$brunch_backup_dir/.partial.XXXXXX")"
  if ! compose exec -T db pg_dump -U postgres -d brunch --format=custom --no-owner --no-privileges >"$brunch_partial"; then
    rm -f "$brunch_partial"
    return 1
  fi
  if ! compose exec -T db pg_restore --list <"$brunch_partial" >/dev/null; then
    rm -f "$brunch_partial"
    return 1
  fi
  mv "$brunch_partial" "$brunch_dump"
  (cd "$brunch_backup_dir" && sha256sum "$(basename "$brunch_dump")" >"$(basename "$brunch_dump").sha256")
  # Delete only this project's expired backups, after a successful new backup.
  find "$brunch_backup_dir" -maxdepth 1 -type f -name "$brunch_project-*.dump*" -mtime "+$brunch_days" -delete
  printf 'Backup saved: %s\n' "$brunch_dump"
}

restore_check() {
  local brunch_dump="${2:-}"
  [[ -f "$brunch_dump" && -f "$brunch_dump.sha256" ]] || {
    printf '%s\n' 'Provide an existing backup with its .sha256 file.' >&2
    return 1
  }
  # Read the expected hash only. Never follow filenames from an external manifest.
  python3 - "$brunch_dump" <<'PY'
import hashlib, pathlib, re, sys
path = pathlib.Path(sys.argv[1])
expected = pathlib.Path(str(path) + '.sha256').read_text().split()[0]
if not re.fullmatch('[a-f0-9]{64}', expected):
    raise SystemExit('Invalid backup checksum')
with path.open('rb') as stream:
    actual = hashlib.file_digest(stream, 'sha256').hexdigest()
if actual != expected:
    raise SystemExit('Backup checksum mismatch')
PY
  brunch_restore_db="brunch_verify_$(python3 -c 'import secrets; print(secrets.token_hex(8))')"
  compose exec -T db createdb -U postgres "$brunch_restore_db"
  trap 'compose exec -T db dropdb -U postgres --if-exists "$brunch_restore_db" >/dev/null' EXIT
  compose exec -T db pg_restore -U postgres --dbname "$brunch_restore_db" \
    --no-owner --no-privileges --exit-on-error --single-transaction <"$brunch_dump"
  compose exec -T db psql -U postgres -d "$brunch_restore_db" -v ON_ERROR_STOP=1 -Atc \
    'SELECT count(*) FROM orders; SELECT count(*) FROM payment_attempts; SELECT count(*) FROM payment_receipts; SELECT count(*) FROM outbox;' >/dev/null
  compose exec -T db dropdb -U postgres "$brunch_restore_db"
  trap - EXIT
  printf '%s\n' 'Backup restored successfully to an isolated temporary database and removed.'
}

case "$brunch_action" in
  check) check ;;
  up)
    check
    compose up -d --wait --wait-timeout 180
    health
    verify_release
    ;;
  restart)
    check
    compose up -d --force-recreate --wait --wait-timeout 180 app
    health
    verify_release
    ;;
  health) health ;;
  verify-release) verify_release ;;
  status) compose ps ;;
  logs) compose logs --tail 100 app db caddy ;;
  backup) backup ;;
  restore-check) restore_check "$@" ;;
esac
