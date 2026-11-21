//! Configuration checks run in child processes so environment changes cannot race.
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};
use uuid::Uuid;

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("brunch-config-{}", Uuid::new_v4()));
        fs::create_dir(&path).unwrap();
        // Stop dotenv from finding unrelated environment files in a parent directory.
        fs::write(path.join(".env"), "").unwrap();
        Self(path)
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_brunch-server"));
        command
            .current_dir(&self.0)
            .env_clear()
            .arg("--check-config")
            .env("APP_ENV", "production")
            .env("PUBLIC_URL", "https://shop.brunch-ci.cz")
            .env(
                "DATABASE_URL",
                "postgres://brunch:DO-NOT-PRINT@127.0.0.1:1/brunch",
            )
            .env("ORDER_EMAIL", "orders@brunch-ci.cz")
            .env("SMTP_FROM", "Brunch Era <obchod@brunch-ci.cz>")
            .env("SMTP_HOST", "smtp.brunch-ci.cz")
            .env("SMTP_PORT", "587")
            .env("SMTP_TLS", "true")
            .env("DIST_DIR", self.0.join("dist"));
        command
    }

    fn secret(&self, name: &str, content: &str) -> PathBuf {
        let path = self.0.join(name);
        fs::write(&path, content).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        }
        path
    }

    fn preview_command(&self) -> Command {
        let mut command = self.command();
        command
            .env("APP_ENV", "preview")
            .env("PUBLIC_URL", "http://127.0.0.1:3100")
            .env("ORDER_EMAIL", "orders@example.test")
            .env("SMTP_FROM", "Brunch Era <shop@example.test>")
            .env("SMTP_HOST", "mailpit")
            .env("SMTP_PORT", "1025")
            .env("SMTP_TLS", "false");
        command
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn output_text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn rejected(command: &mut Command, setting: &str) {
    let output = command.output().unwrap();
    let text = output_text(&output);
    assert!(
        !output.status.success(),
        "accepted invalid {setting}: {text}"
    );
    assert!(text.contains(setting), "missing {setting} in: {text}");
    assert!(!text.contains("DO-NOT-PRINT"), "credential leaked: {text}");
}

#[test]
fn caddy_client_ip_trust_is_explicit_and_strict() {
    let fixture = Fixture::new();
    for value in ["false", "true"] {
        let output = fixture
            .command()
            .env("TRUST_CADDY_CLIENT_IP", value)
            .output()
            .unwrap();
        assert!(output.status.success(), "{}", output_text(&output));
    }
    for value in ["", "1", "TRUE", "yes"] {
        rejected(
            fixture.command().env("TRUST_CADDY_CLIENT_IP", value),
            "TRUST_CADDY_CLIENT_IP",
        );
    }
}

#[test]
fn deployment_check_accepts_valid_settings_without_connecting_to_any_service() {
    let fixture = Fixture::new();
    for environment in ["production", "staging"] {
        let output = fixture
            .command()
            .env("APP_ENV", environment)
            .output()
            .unwrap();
        assert!(output.status.success(), "{}", output_text(&output));
        let text = output_text(&output);
        assert!(text.contains(&format!("APP_ENV={environment}")));
        assert!(text.contains("Teya=disabled"));
        assert!(!text.contains("DO-NOT-PRINT"));
        assert!(!text.contains("postgres://"));
    }
}

#[test]
fn public_ipv4_requires_explicit_opt_in_and_https() {
    let fixture = Fixture::new();
    for environment in ["staging", "production"] {
        rejected(
            fixture
                .command()
                .env("APP_ENV", environment)
                .env("PUBLIC_URL", "https://37.46.209.59"),
            "PUBLIC_URL",
        );
        rejected(
            fixture
                .command()
                .env("APP_ENV", environment)
                .env("PUBLIC_URL", "https://37.46.209.59")
                .env("ALLOW_PUBLIC_IP", "false"),
            "PUBLIC_URL",
        );
        let output = fixture
            .command()
            .env("APP_ENV", environment)
            .env("PUBLIC_URL", "https://37.46.209.59")
            .env("ALLOW_PUBLIC_IP", "true")
            .output()
            .unwrap();
        assert!(output.status.success(), "{}", output_text(&output));
        assert!(output_text(&output).contains("Teya=disabled"));
        assert!(!output_text(&output).contains("DO-NOT-PRINT"));
        rejected(
            fixture
                .command()
                .env("APP_ENV", environment)
                .env("PUBLIC_URL", "http://37.46.209.59")
                .env("ALLOW_PUBLIC_IP", "true"),
            "PUBLIC_URL",
        );
    }
}

#[test]
fn public_ip_opt_in_refuses_nonpublic_ranges_and_ipv6() {
    let fixture = Fixture::new();
    for host in [
        "0.0.0.0",
        "0.255.255.255",
        "10.0.0.1",
        "10.255.255.255",
        "100.64.0.0",
        "100.127.255.255",
        "127.0.0.1",
        "127.255.255.255",
        "169.254.0.1",
        "169.254.255.255",
        "172.16.0.0",
        "172.31.255.255",
        "192.0.0.1",
        "192.0.0.9",
        "192.0.0.10",
        "192.0.2.1",
        "192.88.99.1",
        "192.168.0.1",
        "192.168.255.255",
        "198.18.0.0",
        "198.19.255.255",
        "198.51.100.1",
        "203.0.113.1",
        "224.0.0.1",
        "239.255.255.255",
        "240.0.0.1",
        "255.255.255.255",
        "[::1]",
        "[::ffff:37.46.209.59]",
        "[2606:4700:4700::1111]",
        "localhost",
        "shop.example.test",
    ] {
        rejected(
            fixture
                .command()
                .env("ALLOW_PUBLIC_IP", "true")
                .env("PUBLIC_URL", format!("https://{host}")),
            "PUBLIC_URL",
        );
    }
}

#[test]
fn public_ip_opt_in_is_strict_and_does_not_relax_mail_or_preview_configuration() {
    let fixture = Fixture::new();
    for value in ["", "1", "0", "TRUE", "False", "yes", " true", "false "] {
        rejected(
            fixture.command().env("ALLOW_PUBLIC_IP", value),
            "ALLOW_PUBLIC_IP",
        );
    }
    // A real domain remains valid with either setting.
    for value in ["false", "true"] {
        let output = fixture
            .command()
            .env("ALLOW_PUBLIC_IP", value)
            .output()
            .unwrap();
        assert!(output.status.success(), "{}", output_text(&output));
    }
    for key in ["ORDER_EMAIL", "SMTP_FROM"] {
        rejected(
            fixture
                .command()
                .env("ALLOW_PUBLIC_IP", "true")
                .env("PUBLIC_URL", "https://37.46.209.59")
                .env(key, "test@example.test"),
            key,
        );
    }
    rejected(
        fixture
            .command()
            .env("ALLOW_PUBLIC_IP", "true")
            .env("PUBLIC_URL", "https://37.46.209.59")
            .env("SMTP_HOST", "127.0.0.1"),
        "SMTP_HOST",
    );
    rejected(
        fixture
            .command()
            .env("ALLOW_PUBLIC_IP", "true")
            .env("PUBLIC_URL", "https://37.46.209.59")
            .env("SMTP_TLS", "false"),
        "SMTP",
    );
    rejected(
        fixture
            .preview_command()
            .env("ALLOW_PUBLIC_IP", "true")
            .env("PUBLIC_URL", "https://37.46.209.59"),
        "PUBLIC_URL",
    );
}

#[test]
fn deployment_rejects_insecure_placeholder_and_mismatched_payment_settings() {
    let fixture = Fixture::new();
    for url in [
        "http://127.0.0.1:3100",
        "https://localhost",
        "https://127.0.0.1",
        "https://shop.example",
        "https://shop.example.com",
        "https://shop.invalid",
        "https://shop.test",
    ] {
        rejected(fixture.command().env("PUBLIC_URL", url), "PUBLIC_URL");
    }
    for setting in ["ORDER_EMAIL", "SMTP_FROM"] {
        rejected(fixture.command().env(setting, "test@example.com"), setting);
    }
    rejected(
        fixture.command().env("SMTP_HOST", "smtp.example.com"),
        "SMTP_HOST",
    );
    rejected(fixture.command().env("SMTP_TLS", "false"), "SMTP");
    rejected(fixture.command().env("SMTP_PORT", "0"), "SMTP_PORT");
    rejected(fixture.command().env("APP_ENV", "prod"), "APP_ENV");
    rejected(
        fixture.command().env("TEYA_ENVIRONMENT", "staging"),
        "TEYA_ENVIRONMENT",
    );
    rejected(
        fixture
            .command()
            .env("APP_ENV", "staging")
            .env("TEYA_ENVIRONMENT", "production"),
        "TEYA_ENVIRONMENT",
    );
    rejected(
        fixture.command().env("TEYA_ENVIRONMENT", "production"),
        "TEYA_CLIENT_SECRET_FILE",
    );
    rejected(
        fixture
            .command()
            .env("DATABASE_URL", "not-a-database-DO-NOT-PRINT"),
        "DATABASE_URL",
    );
}

#[test]
fn protected_files_work_and_conflicting_sources_fail_without_printing_secrets() {
    let fixture = Fixture::new();
    let database = fixture.secret(
        "database-url",
        "postgres://brunch:DO-NOT-PRINT@127.0.0.1:1/brunch\n",
    );
    let password = fixture.secret("smtp-password", "DO-NOT-PRINT\n");
    let output = fixture
        .command()
        .env_remove("DATABASE_URL")
        .env("DATABASE_URL_FILE", &database)
        .env("SMTP_USERNAME", "smtp-user")
        .env("SMTP_PASSWORD_FILE", &password)
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", output_text(&output));
    assert!(!output_text(&output).contains("DO-NOT-PRINT"));
    rejected(
        fixture.command().env("DATABASE_URL_FILE", &database),
        "DATABASE_URL",
    );
    rejected(
        fixture
            .command()
            .env("SMTP_USERNAME", "smtp-user")
            .env("SMTP_PASSWORD", "DO-NOT-PRINT")
            .env("SMTP_PASSWORD_FILE", &password),
        "SMTP_PASSWORD",
    );
}

#[test]
fn unsafe_secret_file_paths_are_rejected() {
    let fixture = Fixture::new();
    let secret = fixture.secret("password", "DO-NOT-PRINT");
    let check_password = |path: &Path| {
        let mut command = fixture.command();
        command
            .env("SMTP_USERNAME", "smtp-user")
            .env("SMTP_PASSWORD_FILE", path);
        rejected(&mut command, "SMTP_PASSWORD_FILE");
    };
    check_password(Path::new("password"));
    check_password(&fixture.0.join("missing"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&secret, fs::Permissions::from_mode(0o644)).unwrap();
        check_password(&secret);
        fs::set_permissions(&secret, fs::Permissions::from_mode(0o600)).unwrap();
    }
    fs::create_dir(fixture.0.join("dist")).unwrap();
    let public_secret = fixture.secret("dist/password", "DO-NOT-PRINT");
    check_password(&public_secret);
    #[cfg(unix)]
    {
        let link = fixture.0.join("linked-password");
        std::os::unix::fs::symlink(&public_secret, &link).unwrap();
        check_password(&link);
    }
    let empty = fixture.secret("empty", "\n");
    check_password(&empty);
}

#[test]
fn development_configuration_keeps_local_mail_and_http_support() {
    let fixture = Fixture::new();
    let output = fixture
        .command()
        .env_remove("APP_ENV")
        .env("PUBLIC_URL", "http://127.0.0.1:3100")
        .env("ORDER_EMAIL", "orders@example.test")
        .env("SMTP_FROM", "brunch@example.test")
        .env("SMTP_HOST", "mailpit")
        .env("SMTP_TLS", "false")
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", output_text(&output));
    assert!(output_text(&output).contains("APP_ENV=development"));
}

#[test]
fn preview_accepts_only_private_loopback_urls_and_capture_mail() {
    let fixture = Fixture::new();
    for url in [
        "http://127.0.0.1:3100",
        "http://localhost:3100",
        "http://[::1]:3100",
    ] {
        let output = fixture
            .preview_command()
            .env("PUBLIC_URL", url)
            .output()
            .unwrap();
        assert!(output.status.success(), "{}", output_text(&output));
        assert!(output_text(&output).contains("APP_ENV=preview"));
        assert!(output_text(&output).contains("Teya=disabled"));
        assert!(!output_text(&output).contains("DO-NOT-PRINT"));
    }
    for url in [
        "http://0.0.0.0:3100",
        "http://192.168.1.2:3100",
        "http://203.0.113.10:3100",
        "https://shop.brunch-ci.cz",
        "https://127.0.0.1:3100",
        "http://localhost:3100/admin/",
    ] {
        rejected(
            fixture.preview_command().env("PUBLIC_URL", url),
            "PUBLIC_URL",
        );
    }
    for host in ["smtp.brunch-ci.cz", "192.168.1.2", "mailpit.example.test"] {
        rejected(
            fixture.preview_command().env("SMTP_HOST", host),
            "SMTP_HOST",
        );
    }
}

#[test]
fn preview_refuses_smtp_credentials_and_online_payments() {
    let fixture = Fixture::new();
    for key in ["SMTP_USERNAME", "SMTP_PASSWORD", "SMTP_PASSWORD_FILE"] {
        rejected(fixture.preview_command().env(key, "DO-NOT-PRINT"), "SMTP");
    }
    for environment in ["staging", "production", "preview"] {
        rejected(
            fixture
                .preview_command()
                .env("TEYA_ENVIRONMENT", environment),
            "TEYA_ENVIRONMENT",
        );
    }
}

#[test]
fn preview_configuration_is_not_accepted_in_staging_or_production() {
    let fixture = Fixture::new();
    for environment in ["staging", "production"] {
        rejected(
            fixture.preview_command().env("APP_ENV", environment),
            "PUBLIC_URL",
        );
        rejected(
            fixture
                .command()
                .env("APP_ENV", environment)
                .env("SMTP_HOST", "mailpit"),
            "SMTP_HOST",
        );
        rejected(
            fixture
                .command()
                .env("APP_ENV", environment)
                .env("SMTP_TLS", "false"),
            "SMTP",
        );
    }
}
