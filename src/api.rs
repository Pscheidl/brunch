use gloo_net::http::{Request, Response};
use serde::{Serialize, de::DeserializeOwned};

#[derive(Debug)]
pub struct ApiError {
    pub status: u16,
    pub message: String,
}
impl ApiError {
    fn network() -> Self {
        Self {
            status: 0,
            message: "Spojení se nepodařilo. Zkontrolujte internet a zkuste to znovu.".into(),
        }
    }
}
async fn decode<T: DeserializeOwned>(response: Response) -> Result<T, ApiError> {
    if !response.ok() {
        let status = response.status();
        let message = response
            .json::<serde_json::Value>()
            .await
            .ok()
            .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(str::to_owned))
            .unwrap_or_else(|| "Požadavek se nepodařil. Zkuste to prosím znovu.".into());
        return Err(ApiError { status, message });
    }
    response.json().await.map_err(|_| ApiError {
        status: 0,
        message: "Odpověď obchodu se nepodařilo načíst. Obnovte prosím stránku.".into(),
    })
}
pub async fn get<T: DeserializeOwned>(path: &str) -> Result<T, ApiError> {
    decode(
        Request::get(path)
            .send()
            .await
            .map_err(|_| ApiError::network())?,
    )
    .await
}
pub async fn post<T: DeserializeOwned>(path: &str, body: &impl Serialize) -> Result<T, ApiError> {
    let request = Request::post(path)
        .header("X-Brunch-Request", "1")
        .json(body)
        .map_err(|_| ApiError::network())?;
    decode(request.send().await.map_err(|_| ApiError::network())?).await
}
