use std::time::Duration;

#[derive(Debug, Clone, Copy)]
pub struct AppConfig {
    pub base_url: &'static str,
    pub api_key: &'static str,
    pub agent_version: &'static str,
    pub task_name: &'static str,
    pub request_timeout: Duration,
    pub geolocation_timeout: Duration,
    pub initial_backoff: Duration,
    pub max_backoff: Duration,
    pub jitter_percent: u8,
    pub retry_forever: bool,
}

impl AppConfig {
    #[must_use]
    pub const fn production() -> Self {
        Self {
            base_url: match option_env!("TELEMETRY_BASE_URL") {
                Some(value) => value,
                None => "https://activation.example.invalid",
            },
            api_key: match option_env!("TELEMETRY_API_KEY") {
                Some(value) => value,
                None => "replace-with-build-time-api-key",
            },
            agent_version: env!("CARGO_PKG_VERSION"),
            task_name: match option_env!("TELEMETRY_TASK_NAME") {
                Some(value) => value,
                None => "TelemetryServiceActivation",
            },
            request_timeout: Duration::from_secs(20),
            geolocation_timeout: Duration::from_secs(15),
            initial_backoff: Duration::from_secs(15),
            max_backoff: Duration::from_secs(15 * 60),
            jitter_percent: 20,
            retry_forever: true,
        }
    }
}
