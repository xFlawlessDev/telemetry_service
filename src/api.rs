use std::time::Duration;

use reqwest::{StatusCode, header::RETRY_AFTER, multipart};
use serde::Deserialize;
use uuid::Uuid;

use crate::{config::AppConfig, error::AppResult};

const DEVICE_SERIAL_NUMBER: &str = "0223290070363009024";
const DEVICE_LATITUDE: Option<f64> = Some(-6.914744);
const DEVICE_LONGITUDE: Option<f64> = Some(107.60981);
const DEVICE_ACCURACY_METERS: Option<f64> = Some(10.0);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivationSuccess {
    pub device_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActivationFailure {
    Retryable {
        reason: String,
        retry_after: Option<Duration>,
    },
    Fatal(String),
}

pub struct ActivationClient {
    client: reqwest::Client,
    base_url: &'static str,
    user_id: &'static str,
    api_key: &'static str,
}

impl ActivationClient {
    pub fn new(config: &AppConfig) -> AppResult<Self> {
        let client = reqwest::Client::builder()
            .timeout(config.request_timeout)
            .build()?;
        Ok(Self {
            client,
            base_url: config.base_url.trim_end_matches('/'),
            user_id: config.user_id,
            api_key: config.api_key,
        })
    }

    pub async fn activate(&self, install_id: Uuid) -> Result<ActivationSuccess, ActivationFailure> {
        let token = self.fetch_token(install_id).await?;

        self.create_device(install_id, &token).await
    }

    async fn fetch_token(&self, install_id: Uuid) -> Result<String, ActivationFailure> {
        let url = format!("{}/beta/validation/get_token/", self.base_url);
        let form = multipart::Form::new()
            .text("userId", self.user_id.to_owned())
            .text("apiKey", self.api_key.to_owned());

        let response = self
            .client
            .post(&url)
            .header("Idempotency-Key", install_id.to_string())
            .multipart(form)
            .send()
            .await
            .map_err(classify_reqwest_error)?;

        let status = response.status();
        let retry_after = retry_after(response.headers().get(RETRY_AFTER));
        if status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return extract_token(&body).ok_or_else(|| ActivationFailure::Retryable {
                reason: format!("token response missing token field: {body}"),
                retry_after: None,
            });
        }

        let body = response.text().await.unwrap_or_default();
        Err(classify_status(status, body, retry_after))
    }

    async fn create_device(
        &self,
        install_id: Uuid,
        token: &str,
    ) -> Result<ActivationSuccess, ActivationFailure> {
        let url = format!("{}/beta/axioo_on/create", self.base_url);
        let body = CreateDeviceRequest {
            serial_number: DEVICE_SERIAL_NUMBER,
            latitude: DEVICE_LATITUDE,
            longitude: DEVICE_LONGITUDE,
            accuracy_meters: DEVICE_ACCURACY_METERS,
        };

        let response = self
            .client
            .post(&url)
            .bearer_auth(token)
            .header("Idempotency-Key", install_id.to_string())
            .json(&body)
            .send()
            .await
            .map_err(classify_reqwest_error)?;

        let status = response.status();
        let retry_after = retry_after(response.headers().get(RETRY_AFTER));
        if status.is_success() {
            let raw = response.text().await.unwrap_or_default();
            let device_id = extract_device_id(&raw).unwrap_or(raw);
            return Ok(ActivationSuccess { device_id });
        }

        let body = response.text().await.unwrap_or_default();
        Err(classify_status(status, body, retry_after))
    }
}

#[derive(Debug, serde::Serialize)]
struct CreateDeviceRequest {
    serial_number: &'static str,
    latitude: Option<f64>,
    longitude: Option<f64>,
    accuracy_meters: Option<f64>,
}

fn classify_reqwest_error(error: reqwest::Error) -> ActivationFailure {
    if error.is_timeout() || error.is_connect() || error.is_request() || error.is_decode() {
        ActivationFailure::Retryable {
            reason: error.to_string(),
            retry_after: None,
        }
    } else {
        ActivationFailure::Fatal(error.to_string())
    }
}

#[must_use]
pub fn classify_status(
    status: StatusCode,
    body: String,
    retry_after: Option<Duration>,
) -> ActivationFailure {
    match status {
        StatusCode::TOO_MANY_REQUESTS => ActivationFailure::Retryable {
            reason: format!("HTTP {}: {body}", status.as_u16()),
            retry_after,
        },
        value if value.is_server_error() => ActivationFailure::Retryable {
            reason: format!("HTTP {}: {value}", value.as_u16()),
            retry_after,
        },
        StatusCode::BAD_REQUEST | StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
            ActivationFailure::Fatal(format!("HTTP {}: {body}", status.as_u16()))
        }
        value => ActivationFailure::Retryable {
            reason: format!("HTTP {}: {body}", value.as_u16()),
            retry_after,
        },
    }
}

fn retry_after(value: Option<&reqwest::header::HeaderValue>) -> Option<Duration> {
    value
        .and_then(|header| header.to_str().ok())
        .and_then(|text| text.parse::<u64>().ok())
        .map(Duration::from_secs)
}

#[derive(Debug, Deserialize)]
struct TokenEnvelope {
    token: Option<String>,
    #[serde(alias = "access_token", alias = "accessToken")]
    access_token: Option<String>,
    data: Option<TokenData>,
}

#[derive(Debug, Deserialize)]
struct TokenData {
    token: Option<String>,
    #[serde(alias = "access_token", alias = "accessToken")]
    access_token: Option<String>,
}

#[must_use]
pub fn extract_token(body: &str) -> Option<String> {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(envelope) = serde_json::from_str::<TokenEnvelope>(trimmed)
        && let Some(token) = envelope.token.or(envelope.access_token).or(envelope
            .data
            .and_then(|data| data.token.or(data.access_token)))
    {
        let token = token.trim();
        if !token.is_empty() {
            return Some(token.to_owned());
        }
    }
    if let Some(raw) = trimmed
        .strip_prefix("Bearer ")
        .or_else(|| trimmed.strip_prefix("bearer "))
    {
        let token = raw.trim();
        if !token.is_empty() {
            return Some(token.to_owned());
        }
    }
    None
}

#[derive(Debug, Deserialize)]
struct CreateDeviceEnvelope {
    #[serde(alias = "device_id", alias = "deviceId")]
    device_id: Option<String>,
    id: Option<String>,
    data: Option<CreateDeviceData>,
}

#[derive(Debug, Deserialize)]
struct CreateDeviceData {
    #[serde(alias = "device_id", alias = "deviceId")]
    device_id: Option<String>,
    id: Option<String>,
}

#[must_use]
pub fn extract_device_id(body: &str) -> Option<String> {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(envelope) = serde_json::from_str::<CreateDeviceEnvelope>(trimmed)
        && let Some(id) = envelope
            .device_id
            .or(envelope.id)
            .or(envelope.data.and_then(|data| data.device_id.or(data.id)))
    {
        let id = id.trim();
        if !id.is_empty() {
            return Some(id.to_owned());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_status_should_treat_429_as_retryable() {
        assert!(matches!(
            classify_status(StatusCode::TOO_MANY_REQUESTS, String::new(), None),
            ActivationFailure::Retryable { .. }
        ));
    }

    #[test]
    fn classify_status_should_treat_500_as_retryable() {
        assert!(matches!(
            classify_status(StatusCode::INTERNAL_SERVER_ERROR, String::new(), None),
            ActivationFailure::Retryable { .. }
        ));
    }

    #[test]
    fn classify_status_should_treat_401_as_fatal() {
        assert!(matches!(
            classify_status(StatusCode::UNAUTHORIZED, String::new(), None),
            ActivationFailure::Fatal(_)
        ));
    }

    #[test]
    fn extract_token_should_read_token_field() {
        assert_eq!(
            extract_token(r#"{"token":"abc123"}"#),
            Some("abc123".to_owned())
        );
    }

    #[test]
    fn extract_token_should_read_access_token_field() {
        assert_eq!(
            extract_token(r#"{"access_token":"abc123"}"#),
            Some("abc123".to_owned())
        );
    }

    #[test]
    fn extract_token_should_read_nested_data_field() {
        assert_eq!(
            extract_token(r#"{"data":{"token":"abc123"}}"#),
            Some("abc123".to_owned())
        );
    }

    #[test]
    fn extract_token_should_strip_bearer_prefix() {
        assert_eq!(extract_token("Bearer abc123"), Some("abc123".to_owned()));
    }

    #[test]
    fn extract_token_should_return_none_when_missing() {
        assert!(extract_token("").is_none());
        assert!(extract_token(r#"{"status":"ok"}"#).is_none());
    }

    #[test]
    fn extract_device_id_should_read_top_level_field() {
        assert_eq!(
            extract_device_id(r#"{"device_id":"d-1"}"#),
            Some("d-1".to_owned())
        );
    }

    #[test]
    fn extract_device_id_should_read_nested_data() {
        assert_eq!(
            extract_device_id(r#"{"data":{"id":"d-2"}}"#),
            Some("d-2".to_owned())
        );
    }

    #[test]
    fn create_device_request_should_match_specified_payload() {
        let request = CreateDeviceRequest {
            serial_number: DEVICE_SERIAL_NUMBER,
            latitude: DEVICE_LATITUDE,
            longitude: DEVICE_LONGITUDE,
            accuracy_meters: DEVICE_ACCURACY_METERS,
        };

        let json = serde_json::to_value(request).unwrap();

        assert_eq!(json["serial_number"], "0223290070363009024");
        assert_eq!(json["latitude"], -6.914744);
        assert_eq!(json["longitude"], 107.60981);
        assert_eq!(json["accuracy_meters"], 10.0);

        let nullable_request = CreateDeviceRequest {
            serial_number: DEVICE_SERIAL_NUMBER,
            latitude: None,
            longitude: None,
            accuracy_meters: None,
        };
        let nullable_json = serde_json::to_value(nullable_request).unwrap();
        assert!(nullable_json["latitude"].is_null());
        assert!(nullable_json["longitude"].is_null());
        assert!(nullable_json["accuracy_meters"].is_null());
    }
}
