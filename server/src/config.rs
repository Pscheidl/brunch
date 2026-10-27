use std::{
    env,
    io::Read,
    net::{Ipv4Addr, SocketAddr},
    path::Path,
    str::FromStr,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppEnvironment {
    Development,
    Preview,
    Staging,
    Production,
}

impl AppEnvironment {
    fn from_env() -> Result<Self, String> {
        match env::var("APP_ENV").as_deref().unwrap_or("development") {
            "development" => Ok(Self::Development),
            "preview" => Ok(Self::Preview),
            "staging" => Ok(Self::Staging),
            "production" => Ok(Self::Production),
            _ => Err("APP_ENV musí být development, preview, staging nebo production".into()),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Development => "development",
            Self::Preview => "preview",
            Self::Staging => "staging",
            Self::Production => "production",
        }
    }

    pub fn is_development(self) -> bool {
        self == Self::Development
    }

    fn requires_public_services(self) -> bool {
        matches!(self, Self::Staging | Self::Production)
    }
}

#[derive(Clone)]
pub struct Config {
    pub app_env: AppEnvironment,
    pub payment_timeout_minutes: u32,
    pub teya: Option<crate::payments::TeyaConfig>,
    pub database_url: String,
    pub bind: SocketAddr,
    pub public_url: String,
    pub order_email: String,
    pub admin_email: String,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub smtp_username: Option<String>,
    pub smtp_password: Option<String>,
    pub smtp_from: String,
    pub smtp_tls: bool,
    pub secure_cookies: bool,
    pub dist_dir: String,
}
impl Config {
    pub fn from_env() -> Result<Self, String> {
        let app_env = AppEnvironment::from_env()?;
        let allow_public_ip = match env::var("ALLOW_PUBLIC_IP") {
            Err(env::VarError::NotPresent) => false,
            Ok(value) if value == "false" => false,
            Ok(value) if value == "true" => true,
            _ => return Err("ALLOW_PUBLIC_IP musí být true nebo false".into()),
        };
        let required = |key: &str| {
            env::var(key)
                .ok()
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| format!("Chybí nastavení {key}"))
        };
        let dist_dir = env::var("DIST_DIR").unwrap_or_else(|_| "dist".into());
        let public_url = env::var("PUBLIC_URL").unwrap_or_else(|_| "http://127.0.0.1:3100".into());
        let url = url::Url::parse(&public_url).map_err(|_| "Neplatné PUBLIC_URL")?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || url.path() != "/"
            || url.query().is_some()
            || url.fragment().is_some()
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err(
                "PUBLIC_URL musí být kořenová adresa http(s) bez cesty nebo přihlášení".into(),
            );
        }
        let secure_cookies = url.scheme() == "https";
        let loopback_url = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
        if !secure_cookies && !loopback_url {
            return Err("Mimo localhost vyžaduje PUBLIC_URL HTTPS".into());
        }
        if app_env == AppEnvironment::Preview && (secure_cookies || !loopback_url) {
            return Err("PUBLIC_URL v preview musí být HTTP loopback adresa pro SSH tunel".into());
        }
        let deployable_host = url
            .host_str()
            .is_some_and(|host| deployable_domain(host) || (allow_public_ip && public_ipv4(host)));
        if app_env.requires_public_services() && (!secure_cookies || !deployable_host) {
            return Err(
                "PUBLIC_URL ve stagingu a produkci vyžaduje HTTPS a veřejnou doménu, nebo veřejnou IPv4 s ALLOW_PUBLIC_IP=true"
                    .into(),
            );
        }
        let order_email = required("ORDER_EMAIL")?.trim().to_lowercase();
        let admin_email = required("ADMIN_EMAIL")?.trim().to_lowercase();
        let smtp_from = required("SMTP_FROM")?;
        for (name, email) in [
            ("ORDER_EMAIL", &order_email),
            ("ADMIN_EMAIL", &admin_email),
            ("SMTP_FROM", &smtp_from),
        ] {
            let mailbox = email
                .parse::<lettre::message::Mailbox>()
                .map_err(|_| format!("Neplatná e-mailová adresa v {name}"))?;
            if name != "SMTP_FROM" && crate::auth::normalize_email(email).is_err() {
                return Err(format!(
                    "{name} musí obsahovat samotnou e-mailovou adresu bez jména"
                ));
            }
            if app_env.requires_public_services() && !deployable_domain(mailbox.email.domain()) {
                return Err(format!(
                    "{name} ve stagingu a produkci vyžaduje skutečnou e-mailovou adresu"
                ));
            }
        }
        let smtp_host = env::var("SMTP_HOST").unwrap_or_else(|_| "127.0.0.1".into());
        if smtp_host.trim().is_empty() {
            return Err("SMTP_HOST nesmí být prázdné".into());
        }
        if url::Host::parse(&smtp_host).is_err() {
            return Err("Neplatné SMTP_HOST".into());
        }
        if app_env.requires_public_services() && !deployable_domain(&smtp_host) {
            return Err(
                "SMTP_HOST ve stagingu a produkci vyžaduje skutečnou doménu SMTP serveru".into(),
            );
        }
        if app_env == AppEnvironment::Preview {
            if !matches!(
                smtp_host.as_str(),
                "localhost" | "127.0.0.1" | "::1" | "mailpit"
            ) {
                return Err("SMTP_HOST v preview musí být místní zachytávání pošty".into());
            }
            if ["SMTP_USERNAME", "SMTP_PASSWORD", "SMTP_PASSWORD_FILE"]
                .iter()
                .any(|key| env::var(key).is_ok_and(|value| !value.is_empty()))
            {
                return Err("SMTP přihlašovací údaje nejsou v preview povoleny".into());
            }
        }
        let smtp_tls = match env::var("SMTP_TLS").as_deref().unwrap_or("true") {
            "true" => true,
            "false" => false,
            _ => return Err("SMTP_TLS musí být true nebo false".into()),
        };
        let smtp_username = env::var("SMTP_USERNAME").ok().filter(|x| !x.is_empty());
        let smtp_password = secret_setting("SMTP_PASSWORD", &dist_dir)?;
        if smtp_username.is_some() != smtp_password.is_some() {
            return Err("SMTP_USERNAME a SMTP_PASSWORD musí být vyplněny společně".into());
        }
        if !smtp_tls
            && !matches!(
                smtp_host.as_str(),
                "localhost" | "127.0.0.1" | "::1" | "mailpit"
            )
        {
            return Err("SMTP bez TLS je dovoleno jen pro místní poštovní server".into());
        }
        if app_env.requires_public_services() && !smtp_tls {
            return Err("Staging a produkce vyžadují SMTP_TLS=true".into());
        }
        let database_url = secret_setting("DATABASE_URL", &dist_dir)?
            .ok_or_else(|| "Chybí nastavení DATABASE_URL nebo DATABASE_URL_FILE".to_owned())?;
        sqlx::postgres::PgConnectOptions::from_str(&database_url)
            .map_err(|_| "Neplatné DATABASE_URL")?;
        let teya_environment = env::var("TEYA_ENVIRONMENT").unwrap_or_default();
        if app_env == AppEnvironment::Preview && !teya_environment.is_empty() {
            return Err("TEYA_ENVIRONMENT musí zůstat prázdné, preview nemá online platby".into());
        }
        if app_env.requires_public_services()
            && !teya_environment.is_empty()
            && teya_environment != app_env.as_str()
        {
            return Err(
                "TEYA_ENVIRONMENT musí odpovídat APP_ENV, nebo zůstat prázdné pro vypnuté platby"
                    .into(),
            );
        }
        Ok(Self {
            app_env,
            payment_timeout_minutes: payment_timeout(
                env::var("ONLINE_PAYMENT_TIMEOUT_MINUTES").ok().as_deref(),
            )?,
            teya: crate::payments::TeyaConfig::from_env(&public_url)?,
            database_url,
            bind: env::var("BIND_ADDR")
                .unwrap_or_else(|_| "127.0.0.1:3100".into())
                .parse()
                .map_err(|_| "Neplatné BIND_ADDR")?,
            public_url: url.origin().ascii_serialization(),
            order_email,
            admin_email,
            smtp_host,
            smtp_port: env::var("SMTP_PORT")
                .unwrap_or_else(|_| "587".into())
                .parse::<u16>()
                .ok()
                .filter(|port| *port != 0)
                .ok_or("Neplatné SMTP_PORT")?,
            smtp_username,
            smtp_password,
            smtp_from,
            smtp_tls,
            secure_cookies,
            dist_dir,
        })
    }
}

fn public_ipv4(host: &str) -> bool {
    let Ok(address) = host.parse::<Ipv4Addr>() else {
        return false;
    };
    let [a, b, c, _] = address.octets();
    // Exclude local, shared, documentation and special-purpose address ranges.
    // The 192.0.0/24 protocol anycast exceptions are not storefront endpoints.
    // https://www.iana.org/assignments/iana-ipv4-special-registry/
    !(a == 0
        || a == 10
        || (a == 100 && (64..=127).contains(&b))
        || a == 127
        || (a == 169 && b == 254)
        || (a == 172 && (16..=31).contains(&b))
        || (a == 192 && b == 0 && matches!(c, 0 | 2))
        || (a == 192 && b == 88 && c == 99)
        || (a == 192 && b == 168)
        || (a == 198 && (18..=19).contains(&b))
        || (a == 198 && b == 51 && c == 100)
        || (a == 203 && b == 0 && c == 113)
        || a >= 224)
}

fn deployable_domain(host: &str) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    host.contains('.')
        && host.parse::<std::net::IpAddr>().is_err()
        && ![
            "localhost",
            "local",
            "test",
            "example",
            "invalid",
            "example.com",
            "example.org",
            "example.net",
        ]
        .iter()
        .any(|reserved| host == *reserved || host.ends_with(&format!(".{reserved}")))
}

fn secret_setting(name: &str, dist_dir: &str) -> Result<Option<String>, String> {
    let direct = env::var(name).ok().filter(|value| !value.is_empty());
    let file_name = format!("{name}_FILE");
    let file = env::var(&file_name).ok().filter(|value| !value.is_empty());
    if direct.is_some() && file.is_some() {
        return Err(format!("Nastavte pouze {name}, nebo {file_name}, ne obojí"));
    }
    let Some(file) = file else {
        return Ok(direct);
    };
    read_secret_file(
        Path::new(&file),
        &file_name,
        &[Path::new(dist_dir), Path::new("public")],
    )
    .map(Some)
}

fn read_secret_file(path: &Path, name: &str, public_dirs: &[&Path]) -> Result<String, String> {
    if !path.is_absolute() {
        return Err(format!("{name} musí být absolutní cesta"));
    }
    let resolved = path
        .canonicalize()
        .map_err(|_| format!("Nelze číst {name}"))?;
    if public_dirs
        .iter()
        .filter_map(|dir| dir.canonicalize().ok())
        .any(|dir| resolved.starts_with(dir))
    {
        return Err(format!("{name} nesmí být ve veřejném adresáři webu"));
    }
    // Reject special files before opening them so a FIFO cannot block preflight.
    let metadata = std::fs::metadata(&resolved).map_err(|_| format!("Nelze číst {name}"))?;
    if !metadata.is_file() {
        return Err(format!("Neplatný soubor {name}"));
    }
    let file = std::fs::File::open(resolved).map_err(|_| format!("Nelze číst {name}"))?;
    let metadata = file.metadata().map_err(|_| format!("Nelze číst {name}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if !matches!(metadata.permissions().mode() & 0o777, 0o400 | 0o600) {
            return Err(format!("{name} musí mít oprávnění 0600 nebo 0400"));
        }
    }
    if !metadata.is_file() || metadata.len() > 8192 {
        return Err(format!("Neplatný soubor {name}"));
    }
    let mut value = String::new();
    file.take(8193)
        .read_to_string(&mut value)
        .map_err(|_| format!("Nelze číst {name}"))?;
    let value = value.trim_end_matches(['\r', '\n']);
    if value.len() > 8192 || value.trim().is_empty() || value.contains(['\r', '\n', '\0']) {
        return Err(format!("Neplatný nebo prázdný soubor {name}"));
    }
    Ok(value.to_owned())
}

fn payment_timeout(value: Option<&str>) -> Result<u32, String> {
    match value {
        None => Ok(60),
        Some(value) => value
            .parse::<u32>()
            .ok()
            .filter(|minutes| (1..=1440).contains(minutes))
            .ok_or_else(|| {
                "ONLINE_PAYMENT_TIMEOUT_MINUTES musí být celé číslo od 1 do 1440".into()
            }),
    }
}

#[cfg(test)]
mod tests {
    use super::{AppEnvironment, payment_timeout};
    #[test]
    fn private_preview_never_enables_development_demo_seeding() {
        assert!(!AppEnvironment::Preview.is_development());
        assert!(!AppEnvironment::Staging.is_development());
        assert!(!AppEnvironment::Production.is_development());
        assert!(AppEnvironment::Development.is_development());
    }

    #[test]
    fn online_order_timeout_defaults_to_one_hour_and_rejects_invalid_values() {
        assert_eq!(payment_timeout(None).unwrap(), 60);
        for minutes in [1, 15, 60, 120, 1440] {
            assert_eq!(
                payment_timeout(Some(&minutes.to_string())).unwrap(),
                minutes
            );
        }
        for invalid in ["", "0", "-1", "1.5", "1441", "4294967296", "minute"] {
            assert!(
                payment_timeout(Some(invalid)).is_err(),
                "accepted {invalid}"
            );
        }
    }
}
