use std::time::Duration;

use reqwest::{StatusCode, header::RETRY_AFTER, multipart};
use serde::Deserialize;
use uuid::Uuid;

use crate::{config::AppConfig, error::AppResult};

#[derive(Debug, Clone, PartialEq)]
pub struct DeviceRegistration {
    pub serial_number: String,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub accuracy_meters: Option<f64>,
}

impl DeviceRegistration {
    fn into_form(self) -> multipart::Form {
        multipart::Form::new()
            .text("serial_number", self.serial_number)
            .text("latitude", form_text_value(self.latitude))
            .text("longitude", form_text_value(self.longitude))
            .text("accuracy_meters", form_text_value(self.accuracy_meters))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivationSuccess;

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

    pub async fn activate(
        &self,
        install_id: Uuid,
        device: &DeviceRegistration,
    ) -> Result<ActivationSuccess, ActivationFailure> {
        let token = self.fetch_token(install_id).await?;

        self.create_device(install_id, &token, device).await
    }

    async fn fetch_token(&self, install_id: Uuid) -> Result<String, ActivationFailure> {
        let url = format!("{}/validation/get_token/", self.base_url);
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
            if let Some(token) = extract_token(&body) {
                return Ok(token);
            }
            return Err(classify_token_missing(&body));
        }

        let body = response.text().await.unwrap_or_default();
        Err(classify_status(status, body, retry_after))
    }

    async fn create_device(
        &self,
        install_id: Uuid,
        token: &str,
        device: &DeviceRegistration,
    ) -> Result<ActivationSuccess, ActivationFailure> {
        let url = format!("{}/axioo_on/create", self.base_url);

        let response = self
            .client
            .post(&url)
            .bearer_auth(token)
            .header("Idempotency-Key", install_id.to_string())
            .multipart(device.clone().into_form())
            .send()
            .await
            .map_err(classify_reqwest_error)?;

        let status = response.status();
        let retry_after = retry_after(response.headers().get(RETRY_AFTER));
        if status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return parse_create_device_success(&body);
        }

        let body = response.text().await.unwrap_or_default();
        Err(classify_status(status, body, retry_after))
    }
}

fn form_text_value(value: Option<f64>) -> String {
    value.map_or_else(|| "null".to_owned(), |number| number.to_string())
}

#[must_use]
pub fn payload_debug_string(device: &DeviceRegistration) -> String {
    format!(
        "serial_number={} coordinates={}",
        device.serial_number,
        if device.latitude.is_some() && device.longitude.is_some() {
            "present"
        } else {
            "null"
        }
    )
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

#[must_use]
pub fn extract_token(body: &str) -> Option<String> {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed)
        && let Some(token) = find_token_field(&value)
    {
        return Some(token);
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

fn find_token_field(value: &serde_json::Value) -> Option<String> {
    const TOKEN_KEYS: &[&str] = &["token", "Token", "access_token", "accessToken"];

    match value {
        serde_json::Value::Object(map) => {
            for key in TOKEN_KEYS {
                if let Some(serde_json::Value::String(token)) = map.get(*key) {
                    let token = token.trim();
                    if !token.is_empty() {
                        return Some(token.to_owned());
                    }
                }
            }
            map.values().find_map(find_token_field)
        }
        serde_json::Value::Array(items) => items.iter().find_map(find_token_field),
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
enum ResultCode {
    Number(i64),
    Text(String),
}

impl ResultCode {
    fn is_success(&self) -> bool {
        match self {
            Self::Number(value) => *value == 0,
            Self::Text(value) => value == "0",
        }
    }
}

impl std::fmt::Display for ResultCode {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Number(value) => write!(formatter, "{value}"),
            Self::Text(value) => formatter.write_str(value),
        }
    }
}

#[derive(Debug, Deserialize)]
struct BusinessEnvelope {
    result: ResultCode,
    #[serde(default)]
    message: String,
}

fn parse_create_device_success(body: &str) -> Result<ActivationSuccess, ActivationFailure> {
    let envelope = serde_json::from_str::<BusinessEnvelope>(body).map_err(|error| {
        ActivationFailure::Retryable {
            reason: format!("create response invalid JSON: {error}"),
            retry_after: None,
        }
    })?;

    if envelope.result.is_success() {
        Ok(ActivationSuccess)
    } else {
        Err(ActivationFailure::Fatal(format!(
            "create response result {}: {}",
            envelope.result, envelope.message
        )))
    }
}

fn classify_token_missing(body: &str) -> ActivationFailure {
    match serde_json::from_str::<BusinessEnvelope>(body) {
        Ok(envelope) if !envelope.result.is_success() => ActivationFailure::Fatal(format!(
            "token request result {}: {}",
            envelope.result, envelope.message
        )),
        _ => ActivationFailure::Retryable {
            reason: format!("token response missing token field: {body}"),
            retry_after: None,
        },
    }
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
    fn extract_token_should_read_data_array_token_field() {
        assert_eq!(
            extract_token(
                r#"{"result":"0","message":"Success","data":[{"ExpiredDate":"2026-09-11 10:46:40.000","Token":"abc123"}]}"#
            ),
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
    fn parse_create_device_success_should_accept_zero_result() {
        assert!(parse_create_device_success(r#"{"result":"0","message":"activated"}"#).is_ok());
    }

    #[test]
    fn parse_create_device_success_should_accept_integer_zero_result() {
        assert_eq!(
            parse_create_device_success(r#"{"result":0,"message":"Save succeed"}"#).unwrap(),
            ActivationSuccess
        );
    }

    #[test]
    fn parse_create_device_success_should_reject_non_zero_result() {
        assert!(matches!(
            parse_create_device_success(r#"{"result":"1","message":"blocked"}"#),
            Err(ActivationFailure::Fatal(_))
        ));
    }

    #[test]
    fn parse_create_device_success_should_surface_integer_error_message() {
        match parse_create_device_success(
            r#"{"result":-1,"message":"Serial number must be filled"}"#,
        ) {
            Err(ActivationFailure::Fatal(reason)) => {
                assert!(
                    reason.contains("Serial number must be filled"),
                    "reason: {reason}"
                );
            }
            other => panic!("expected fatal failure, got {other:?}"),
        }
    }

    #[test]
    fn classify_token_missing_should_treat_business_error_as_fatal() {
        match classify_token_missing(r#"{"result":-1,"message":"Expired token"}"#) {
            ActivationFailure::Fatal(reason) => {
                assert!(reason.contains("Expired token"), "reason: {reason}");
            }
            other => panic!("expected fatal failure, got {other:?}"),
        }
    }

    #[test]
    fn classify_token_missing_should_treat_unrecognized_body_as_retryable() {
        assert!(matches!(
            classify_token_missing(r#"{"status":"ok"}"#),
            ActivationFailure::Retryable { .. }
        ));
    }

    #[test]
    fn form_text_value_should_encode_number_as_text() {
        assert_eq!(form_text_value(Some(-6.914744)), "-6.914744");
        assert_eq!(form_text_value(Some(107.60981)), "107.60981");
        assert_eq!(form_text_value(Some(10.0)), "10");
    }

    #[test]
    fn form_text_value_should_encode_missing_coordinate_as_null_text() {
        assert_eq!(form_text_value(None), "null");
    }

    #[test]
    fn payload_debug_string_should_include_serial_number_without_coordinates() {
        let device = DeviceRegistration {
            serial_number: "0223290070363009024".to_owned(),
            latitude: Some(-6.914744),
            longitude: Some(107.60981),
            accuracy_meters: Some(10.0),
        };
        let payload = payload_debug_string(&device);

        assert!(
            payload.contains("serial_number=0223290070363009024"),
            "payload: {payload}"
        );
        assert!(
            payload.contains("coordinates=present"),
            "payload: {payload}"
        );
        assert!(
            !payload.contains("latitude") && !payload.contains("longitude"),
            "payload: {payload}"
        );
    }
}
