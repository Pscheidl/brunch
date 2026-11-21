#!/usr/bin/env python3
"""Validate resolved Compose configuration without displaying credentials."""
import argparse
import ipaddress
import json
import os
from pathlib import Path
import re
import stat
import sys


# Keep in step with server/src/config.rs. Protocol anycast exceptions in
# 192.0.0/24 are intentionally excluded as storefront endpoints.
NONPUBLIC_IPV4 = tuple(ipaddress.IPv4Network(prefix) for prefix in (
    "0.0.0.0/8", "10.0.0.0/8", "100.64.0.0/10", "127.0.0.0/8",
    "169.254.0.0/16", "172.16.0.0/12", "192.0.0.0/24", "192.0.2.0/24",
    "192.88.99.0/24", "192.168.0.0/16", "198.18.0.0/15",
    "198.51.100.0/24", "203.0.113.0/24", "224.0.0.0/3",
))


def public_ipv4(host):
    if not isinstance(host, str):
        return False
    try:
        address = ipaddress.IPv4Address(host)
    except ipaddress.AddressValueError:
        return False
    return not any(address in network for network in NONPUBLIC_IPV4)


def private_file(path):
    info = Path(path).stat()
    if not stat.S_ISREG(info.st_mode) or info.st_mode & 0o077:
        raise ValueError("Configuration must be a regular file with mode 0600 or 0400")
    if info.st_uid not in (0, os.getuid()):
        raise ValueError("Configuration must belong to root or the invoking user")


def placeholder(value):
    if not isinstance(value, str):
        return True
    return any(part in value.lower() for part in ("example", "replace", "changeme", ".invalid", ".test"))


def validate(config, skip_secret_files=False):
    services = config["services"]
    app = services["app"]
    env = app["environment"]
    db = services["db"]["environment"]
    proxy = services["caddy"]["environment"]
    mode = env["APP_ENV"]
    if mode not in ("production", "staging"):
        raise ValueError("APP_ENV must be production or staging")
    if config["name"] != f"brunch-{mode}":
        raise ValueError("COMPOSE_PROJECT_NAME must match brunch-APP_ENV")
    domain = proxy["DOMAIN"]
    allow_public_ip = env.get("ALLOW_PUBLIC_IP", "false")
    if allow_public_ip not in ("true", "false"):
        raise ValueError("ALLOW_PUBLIC_IP must be true or false")
    profile = proxy.get("ACME_PROFILE", "classic")
    if profile not in ("classic", "shortlived"):
        raise ValueError("ACME_PROFILE must be classic or shortlived")
    is_public_ipv4 = public_ipv4(domain)
    is_domain = not placeholder(domain) and re.fullmatch(
        r"(?=.{1,253}$)(?:[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?\.)+[a-z]{2,63}", domain)
    if not is_domain and not (allow_public_ip == "true" and is_public_ipv4):
        raise ValueError("DOMAIN must be a real DNS hostname, or a public IPv4 with ALLOW_PUBLIC_IP=true")
    if is_public_ipv4 and profile != "shortlived":
        raise ValueError("ACME_PROFILE must be shortlived for public IPv4 HTTPS")
    if env["PUBLIC_URL"] != f"https://{domain}":
        raise ValueError("PUBLIC_URL must match the HTTPS domain")
    image = app["image"]
    if placeholder(image) or image.endswith(":latest") or not re.fullmatch(r"brunch:[a-f0-9]{12}(?:-dirty-[a-zA-Z0-9-]+)?", image):
        raise ValueError("BRUNCH_IMAGE must identify a versioned Brunch release")
    if mode == "production" and "-dirty-" in image:
        raise ValueError("Production requires a release built from a clean commit")
    passwords = [db["POSTGRES_PASSWORD"], db["BRUNCH_PASSWORD"]]
    if any(not re.fullmatch(r"[a-fA-F0-9]{64}", value) for value in passwords):
        raise ValueError("Database passwords must each contain 64 random hexadecimal characters")
    if passwords[0] == passwords[1]:
        raise ValueError("Administrator and application database passwords must differ")
    if env["DATABASE_URL"] != f"postgres://brunch:{passwords[1]}@db:5432/brunch":
        raise ValueError("Application database configuration does not match the database service")
    for key in ("ORDER_EMAIL", "SMTP_FROM", "SMTP_HOST"):
        if not env.get(key) or placeholder(env[key]):
            raise ValueError(f"Set a real value for {key}")
    if placeholder(proxy.get("ACME_EMAIL", "")) or "@" not in proxy.get("ACME_EMAIL", ""):
        raise ValueError("Set a real ACME_EMAIL")
    if str(env["SMTP_TLS"]).lower() != "true":
        raise ValueError("SMTP_TLS must be true")
    if str(env.get("TRUST_CADDY_CLIENT_IP", "false")).lower() != "true":
        raise ValueError("TRUST_CADDY_CLIENT_IP must be true behind the dedicated Caddy proxy")
    if app.get("ports") or app.get("network_mode"):
        raise ValueError("Trusted Caddy client IP requires an unpublished app port on its private Docker network")
    if env.get("TEYA_ENVIRONMENT", "") not in ("", mode):
        raise ValueError("TEYA_ENVIRONMENT must match APP_ENV or stay empty")
    # Database backups and diagnostics must still work if an application key is lost.
    if skip_secret_files:
        return env["PUBLIC_URL"], config["name"]
    mount = next(v for v in app["volumes"] if v["target"] == "/run/brunch-secrets")
    secrets = Path(mount["source"])
    if not secrets.is_absolute() or not secrets.is_dir():
        raise ValueError("Create the absolute SECRETS_DIR directory before starting")
    for key in ("SMTP_PASSWORD_FILE", "TEYA_CLIENT_SECRET_FILE", "TEYA_WEBHOOK_PUBLIC_KEY_FILE", "TEYA_WEBHOOK_PREVIOUS_PUBLIC_KEY_FILE"):
        value = env.get(key, "")
        if not value:
            continue
        path = Path(value)
        if path.parent != Path("/run/brunch-secrets"):
            raise ValueError(f"{key} must name a file directly in /run/brunch-secrets")
        host_path = secrets / path.name
        if host_path.is_symlink() or not host_path.is_file():
            raise ValueError(f"{key} must be an existing regular file, not a symbolic link")
        if key in ("SMTP_PASSWORD_FILE", "TEYA_CLIENT_SECRET_FILE"):
            info = host_path.stat()
            if info.st_mode & 0o077 or info.st_uid != 10001:
                raise ValueError(f"{key} must be owned by UID 10001 with mode 0600 or 0400")
    return env["PUBLIC_URL"], config["name"]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--env-file", type=Path)
    parser.add_argument("--skip-secret-files", action="store_true")
    args = parser.parse_args()
    try:
        if args.env_file:
            private_file(args.env_file)
        else:
            url, project = validate(json.load(sys.stdin), args.skip_secret_files)
            print(url)
            print(project)
    except (ValueError, OSError, KeyError, StopIteration, TypeError):
        # Validation messages contain key names only, never the composed values.
        error = sys.exc_info()[1]
        message = str(error) if isinstance(error, ValueError) and not isinstance(error, json.JSONDecodeError) else "Missing or invalid deployment configuration"
        print(f"Preflight failed: {message}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
