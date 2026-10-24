use std::{env, net::SocketAddr};

#[derive(Clone)]
pub struct Config {
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
        let required = |key: &str| env::var(key).map_err(|_| format!("Chybí nastavení {key}"));
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
        if !secure_cookies && !matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]")) {
            return Err("Mimo localhost vyžaduje PUBLIC_URL HTTPS".into());
        }
        let order_email = required("ORDER_EMAIL")?.trim().to_lowercase();
        let admin_email = required("ADMIN_EMAIL")?.trim().to_lowercase();
        let smtp_from = required("SMTP_FROM")?;
        for email in [&order_email, &admin_email, &smtp_from] {
            if email.parse::<lettre::message::Mailbox>().is_err() {
                return Err("Neplatná e-mailová adresa v konfiguraci".into());
            }
        }
        let smtp_host = env::var("SMTP_HOST").unwrap_or_else(|_| "127.0.0.1".into());
        let smtp_tls = match env::var("SMTP_TLS").as_deref().unwrap_or("true") {
            "true" => true,
            "false" => false,
            _ => return Err("SMTP_TLS musí být true nebo false".into()),
        };
        let smtp_username = env::var("SMTP_USERNAME").ok().filter(|x| !x.is_empty());
        let smtp_password = env::var("SMTP_PASSWORD").ok().filter(|x| !x.is_empty());
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
        Ok(Self {
            payment_timeout_minutes: payment_timeout(
                env::var("ONLINE_PAYMENT_TIMEOUT_MINUTES").ok().as_deref(),
            )?,
            teya: crate::payments::TeyaConfig::from_env(&public_url)?,
            database_url: required("DATABASE_URL")?,
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
                .parse()
                .map_err(|_| "Neplatné SMTP_PORT")?,
            smtp_username,
            smtp_password,
            smtp_from,
            smtp_tls,
            secure_cookies,
            dist_dir: env::var("DIST_DIR").unwrap_or_else(|_| "dist".into()),
        })
    }
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
    use super::payment_timeout;
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
