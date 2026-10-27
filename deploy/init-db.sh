#!/usr/bin/env bash
set -euo pipefail
# Runs once on an empty PostgreSQL volume. App credentials have no superuser rights.
psql --username "$POSTGRES_USER" --dbname "$POSTGRES_DB" --set=ON_ERROR_STOP=1 \
  --set=brunch_password="$BRUNCH_PASSWORD" <<'SQL'
CREATE ROLE brunch LOGIN PASSWORD :'brunch_password' NOSUPERUSER NOCREATEDB NOCREATEROLE;
CREATE DATABASE brunch OWNER brunch;
REVOKE ALL ON DATABASE brunch FROM PUBLIC;
SQL
