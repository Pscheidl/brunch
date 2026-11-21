use crate::{ApiError, ApiResult};
use axum::http::HeaderMap;
use std::net::IpAddr;

const PROXY_HEADER: &str = "x-brunch-client-ip";

/// Proxy mode is only safe behind the dedicated Caddy on an unpublished app
/// port. Caddy replaces this header with its TCP peer, ignoring client headers.
pub(crate) fn from_request(
    headers: &HeaderMap,
    peer: Option<IpAddr>,
    trust_caddy: bool,
) -> ApiResult<Option<String>> {
    if !trust_caddy {
        return peer.map(|ip| Some(ip.to_string())).ok_or_else(|| {
            ApiError::new(
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                "Nelze ověřit adresu klientského spojení.",
            )
        });
    }
    let mut values = headers.get_all(PROXY_HEADER).iter();
    let address = values.next().and_then(|value| value.to_str().ok());
    if values.next().is_some() {
        return Err(ApiError::bad_request(
            "Neplatná adresa klientského spojení.",
        ));
    }
    let address = address
        .and_then(|value| value.parse::<IpAddr>().ok())
        .ok_or_else(|| ApiError::bad_request("Chybí platná adresa klientského spojení."))?;
    Ok(Some(address.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direct_connections_ignore_all_client_supplied_proxy_headers() {
        let mut headers = HeaderMap::new();
        for name in [PROXY_HEADER, "x-forwarded-for", "x-real-ip", "forwarded"] {
            headers.insert(name, "198.51.100.12".parse().unwrap());
        }
        assert_eq!(
            from_request(&headers, Some("::1".parse().unwrap()), false).unwrap(),
            Some("::1".into())
        );
        assert!(from_request(&headers, None, false).is_err());
    }

    #[test]
    fn explicit_proxy_mode_accepts_one_canonical_ipv4_or_ipv6_address() {
        for (input, expected) in [
            ("198.51.100.12", "198.51.100.12"),
            ("2001:0db8:0:0:0:0:0:12", "2001:db8::12"),
        ] {
            let mut headers = HeaderMap::new();
            headers.insert(PROXY_HEADER, input.parse().unwrap());
            headers.insert("x-forwarded-for", "203.0.113.99".parse().unwrap());
            assert_eq!(
                from_request(&headers, None, true).unwrap(),
                Some(expected.into())
            );
        }
    }

    #[test]
    fn proxy_mode_refuses_missing_duplicate_lists_ports_and_invalid_addresses() {
        assert!(from_request(&HeaderMap::new(), None, true).is_err());
        for invalid in [
            "",
            "198.51.100.1, 203.0.113.1",
            "198.51.100.1:80",
            "198.51.100.0/24",
            "[2001:db8::1]",
            "fe80::1%eth0",
            "unknown",
            " 198.51.100.1",
        ] {
            let mut headers = HeaderMap::new();
            headers.insert(PROXY_HEADER, invalid.parse().unwrap());
            assert!(
                from_request(&headers, None, true).is_err(),
                "accepted {invalid:?}"
            );
        }
        let mut headers = HeaderMap::new();
        headers.append(PROXY_HEADER, "198.51.100.1".parse().unwrap());
        headers.append(PROXY_HEADER, "198.51.100.2".parse().unwrap());
        assert!(from_request(&headers, None, true).is_err());
    }
}
