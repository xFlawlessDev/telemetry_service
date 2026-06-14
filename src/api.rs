use std::time::Duration;

use reqwest::{StatusCode, header::RETRY_AFTER};
use serde::Deserialize;
use uuid::Uuid;

use crate::{config::AppConfig, error::AppResult, payload::ActivationPayload};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivationSuccess {
    pub activation_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActivationFailure {
    Retryable {
        reason: String,
        retry_after: Option<Duration>,
    },
    Fatal(String),
}

#[derive(Debug, Deserialize)]
struct ActivationResponse {
    status: String,
    activation_id: String,
}

pub struct ActivationClient {
    client: reqwest::Client,
    base_url: &'static str,
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
            api_key: config.api_key,
        })
    }

    pub async fn activate(
        &self,
        install_id: Uuid,
        payload: &ActivationPayload<'_>,
    ) -> Result<ActivationSuccess, ActivationFailure> {
        let url = format!("{}/device-activations", self.base_url);
        let response = self
            .client
            .post(url)
            .bearer_auth(self.api_key)
            .header("Idempotency-Key", install_id.to_string())
            .json(payload)
            .send()
            .await
            .map_err(classify_reqwest_error)?;

        let status = response.status();
        let retry_after = retry_after(response.headers().get(RETRY_AFTER));
        if status.is_success() {
            let body = response
                .json::<ActivationResponse>()
                .await
                .map_err(classify_reqwest_error)?;
            if body.status == "activated" && !body.activation_id.trim().is_empty() {
                return Ok(ActivationSuccess {
                    activation_id: body.activation_id,
                });
            }
            return Err(ActivationFailure::Retryable {
                reason: "invalid activation response".to_owned(),
                retry_after: None,
            });
        }

        let body = response.text().await.unwrap_or_default();
        Err(classify_status(status, body, retry_after))
    }
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
            reason: format!("HTTP {}: {body}", value.as_u16()),
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
}
