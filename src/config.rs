use std::time::Duration;

use crate::location::{BlockZone, GeoPoint};

pub const DEFAULT_BLOCK_ZONE_POLL_INTERVAL: Duration = Duration::from_secs(5 * 60);

/// Build-time debug toggle. Set `TELEMETRY_DEBUG=1` in `.env` (or the process
/// environment) to keep durable JSON activation state and write log files.
/// When disabled, the agent writes no local files at all, which is the
/// production default.
pub const DEBUG: bool = option_env!("TELEMETRY_DEBUG").is_some();

#[derive(Debug, Clone, Copy)]
pub struct AppConfig {
    pub base_url: &'static str,
    pub user_id: &'static str,
    pub api_key: &'static str,
    pub task_name: &'static str,
    pub request_timeout: Duration,
    pub geolocation_timeout: Duration,
    pub block_zone: Option<BlockZone>,
    pub block_zone_poll_interval: Duration,
    pub initial_backoff: Duration,
    pub max_backoff: Duration,
    pub jitter_percent: u8,
    pub retry_forever: bool,
}

impl AppConfig {
    #[must_use]
    pub fn production() -> Self {
        Self {
            base_url: match option_env!("TELEMETRY_BASE_URL") {
                Some(value) => value,
                None => "https://register.axiooworld.com",
            },
            user_id: match option_env!("TELEMETRY_USER_ID") {
                Some(value) => value,
                None => "replace-with-build-time-user-id",
            },
            api_key: match option_env!("TELEMETRY_API_KEY") {
                Some(value) => value,
                None => "replace-with-build-time-api-key",
            },
            task_name: match option_env!("TELEMETRY_TASK_NAME") {
                Some(value) => value,
                None => "TelemetryServiceActivation",
            },
            request_timeout: Duration::from_secs(20),
            geolocation_timeout: Duration::from_secs(10),
            block_zone: parse_block_zone(
                option_env!("TELEMETRY_BLOCK_LATITUDE"),
                option_env!("TELEMETRY_BLOCK_LONGITUDE"),
                option_env!("TELEMETRY_BLOCK_RADIUS_METERS"),
            ),
            block_zone_poll_interval: DEFAULT_BLOCK_ZONE_POLL_INTERVAL,
            initial_backoff: Duration::from_secs(15),
            max_backoff: Duration::from_secs(15 * 60),
            jitter_percent: 20,
            retry_forever: true,
        }
    }
}

fn parse_block_zone(
    latitude: Option<&'static str>,
    longitude: Option<&'static str>,
    radius_meters: Option<&'static str>,
) -> Option<BlockZone> {
    Some(BlockZone {
        center: GeoPoint {
            latitude: parse_f64(latitude?)?,
            longitude: parse_f64(longitude?)?,
        },
        radius_meters: parse_f64(radius_meters?)?,
    })
}

fn parse_f64(value: &str) -> Option<f64> {
    let parsed = value.trim().parse::<f64>().ok()?;
    parsed.is_finite().then_some(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_block_zone_should_return_none_when_disabled() {
        assert_eq!(parse_block_zone(None, None, None), None);
    }

    #[test]
    fn parse_block_zone_should_build_zone_from_values() {
        let zone = parse_block_zone(Some("-6.2"), Some("106.816666"), Some("1500")).unwrap();

        assert_eq!(zone.center.latitude, -6.2);
        assert_eq!(zone.center.longitude, 106.816_666);
        assert_eq!(zone.radius_meters, 1500.0);
    }

    #[test]
    fn parse_block_zone_should_return_none_for_invalid_numbers() {
        assert_eq!(
            parse_block_zone(Some("abc"), Some("106.8"), Some("1500")),
            None
        );
        assert_eq!(
            parse_block_zone(Some("-6.2"), Some("106.8"), Some("NaN")),
            None
        );
    }

    #[test]
    fn parse_block_zone_should_require_all_values() {
        assert_eq!(parse_block_zone(Some("-6.2"), None, Some("1500")), None);
    }

    #[test]
    fn production_should_disable_block_zone_without_env() {
        if option_env!("TELEMETRY_BLOCK_LATITUDE").is_none() {
            assert_eq!(AppConfig::production().block_zone, None);
        }
    }
}
