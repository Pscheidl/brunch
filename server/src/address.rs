use crate::{ApiError, ApiResult, AppState};
use axum::{Json, Router, routing::post};
use serde::{Deserialize, Serialize};
use std::{sync::LazyLock, time::Duration};
use tokio::sync::Semaphore;

const SUGGEST: &str =
    "https://ags.cuzk.gov.cz/arcgis/rest/services/RUIAN/MapServer/exts/GeocodeSOE/tables/1/suggest";
static CLIENT: LazyLock<reqwest::Client> = LazyLock::new(reqwest::Client::new);
static LOOKUPS: Semaphore = Semaphore::const_new(8);

pub fn routes() -> Router<AppState> {
    Router::new().route("/api/address-suggestions", post(suggest))
}

#[derive(Deserialize)]
struct Search {
    text: String,
}

#[derive(Deserialize)]
struct RegistryResponse {
    suggestions: Vec<RegistryAddress>,
}

#[derive(Deserialize)]
struct RegistryAddress {
    text: String,
}

#[derive(Debug, PartialEq, Serialize)]
struct Address {
    label: String,
    street: String,
    city: String,
    postal_code: String,
}

// RÚIAN places the postcode and delivery city after the last comma. Keep any
// preceding locality in the street line (important for villages sharing numbers).
fn address(label: &str) -> Option<Address> {
    let (street, locality) = label.rsplit_once(',')?;
    let street = street.trim();
    let mut parts = locality.split_whitespace();
    let mut postal_code = parts.next()?.to_owned();
    if postal_code.len() == 3 {
        postal_code.push_str(parts.next()?);
    }
    let city = parts.collect::<Vec<_>>().join(" ");
    if postal_code.len() != 5
        || !postal_code.bytes().all(|c| c.is_ascii_digit())
        || street.is_empty()
        || city.is_empty()
    {
        return None;
    }
    Some(Address {
        label: label.to_owned(),
        street: street.to_owned(),
        city,
        postal_code,
    })
}

async fn suggest(Json(search): Json<Search>) -> ApiResult<Json<Vec<Address>>> {
    let text = search.text.trim();
    if !(3..=100).contains(&text.chars().count()) || text.chars().any(char::is_control) {
        return Err(ApiError::bad_request(
            "Pro vyhledání zadejte 3 až 100 znaků adresy.",
        ));
    }
    let unavailable = || {
        ApiError::new(
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            "Našeptávač je dočasně nedostupný. Adresu můžete zadat ručně.",
        )
    };
    let _permit = LOOKUPS.try_acquire().map_err(|_| unavailable())?;
    let response = CLIENT
        .get(SUGGEST)
        .query(&[("text", text), ("maxSuggestions", "6"), ("f", "json")])
        .timeout(Duration::from_secs(4))
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|_| unavailable())?
        .json::<RegistryResponse>()
        .await
        .map_err(|_| unavailable())?;
    let addresses = response
        .suggestions
        .iter()
        .filter_map(|row| address(&row.text))
        .take(6)
        .collect();
    Ok(Json(addresses))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_addresses_keep_localities_and_support_both_postcode_formats() {
        let urban = address("Pod sídlištěm 1800/9, Kobylisy, 18200 Praha 8").unwrap();
        assert_eq!(urban.street, "Pod sídlištěm 1800/9, Kobylisy");
        assert_eq!(urban.postal_code, "18200");
        assert_eq!(urban.city, "Praha 8");
        let rural = address("Vyskeř č.p. 10, 512 64 Vyskeř").unwrap();
        assert_eq!(rural.street, "Vyskeř č.p. 10");
        assert_eq!(rural.postal_code, "51264");
        assert_eq!(rural.city, "Vyskeř");
        for invalid in [
            "Vyskeř",
            "Vyskeř, bez PSČ",
            "Vyskeř, 51300",
            ", 51300 Semily",
        ] {
            assert!(address(invalid).is_none());
        }
    }
}
