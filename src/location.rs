use std::time::Duration;

use serde::{Deserialize, Serialize};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use tokio::time::timeout;
#[cfg(windows)]
use windows::Devices::Geolocation::GeolocationAccessStatus;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LocationSnapshot {
    pub access_status: String,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub accuracy_meters: Option<f64>,
    pub timestamp_utc: Option<String>,
    pub error: Option<String>,
}

impl LocationSnapshot {
    #[must_use]
    pub fn unavailable(error: impl Into<String>) -> Self {
        Self {
            access_status: "Unavailable".to_owned(),
            latitude: None,
            longitude: None,
            accuracy_meters: None,
            timestamp_utc: None,
            error: Some(error.into()),
        }
    }
}

#[cfg(windows)]
pub async fn get_location(wait: Duration) -> LocationSnapshot {
    match timeout(wait, get_location_inner()).await {
        Ok(snapshot) => snapshot,
        Err(_) => LocationSnapshot::unavailable("geolocation timeout"),
    }
}

#[cfg(windows)]
async fn get_location_inner() -> LocationSnapshot {
    match windows_location().await {
        Ok(snapshot) => snapshot,
        Err(error) => LocationSnapshot::unavailable(error.to_string()),
    }
}

#[cfg(windows)]
async fn windows_location() -> crate::error::AppResult<LocationSnapshot> {
    use windows::Devices::Geolocation::Geolocator;
    let request = Geolocator::RequestAccessAsync()?;
    let access_status = request.get()?;
    let access_status_label = access_status_label(access_status);
    if access_status != GeolocationAccessStatus::Allowed {
        return Ok(LocationSnapshot {
            access_status: access_status_label.to_owned(),
            latitude: None,
            longitude: None,
            accuracy_meters: None,
            timestamp_utc: None,
            error: None,
        });
    }

    let geolocator = Geolocator::new()?;
    let position = geolocator.GetGeopositionAsync()?.get()?;
    let coordinate = position.Coordinate()?;
    let point = coordinate.Point()?;
    let basic = point.Position()?;
    let timestamp = coordinate.Timestamp()?;
    Ok(LocationSnapshot {
        access_status: access_status_label.to_owned(),
        latitude: Some(basic.Latitude),
        longitude: Some(basic.Longitude),
        accuracy_meters: Some(coordinate.Accuracy()?),
        timestamp_utc: windows_ticks_to_rfc3339(timestamp.UniversalTime),
        error: None,
    })
}

#[cfg(windows)]
fn access_status_label(status: GeolocationAccessStatus) -> &'static str {
    match status {
        GeolocationAccessStatus::Unspecified => "Unspecified",
        GeolocationAccessStatus::Allowed => "Allowed",
        GeolocationAccessStatus::Denied => "Denied",
        _ => "Unknown",
    }
}

#[cfg(not(windows))]
pub async fn get_location(_wait: Duration) -> LocationSnapshot {
    LocationSnapshot::unavailable("geolocation requires Windows")
}

fn windows_ticks_to_rfc3339(windows_ticks: i64) -> Option<String> {
    const WINDOWS_TICKS_PER_SECOND: i128 = 10_000_000;
    const UNIX_EPOCH_WINDOWS_TICKS: i128 = 116_444_736_000_000_000;

    let unix_nanos = (i128::from(windows_ticks) - UNIX_EPOCH_WINDOWS_TICKS)
        .checked_mul(1_000_000_000 / WINDOWS_TICKS_PER_SECOND)?;
    OffsetDateTime::from_unix_timestamp_nanos(unix_nanos)
        .ok()?
        .format(&Rfc3339)
        .ok()
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unavailable_should_omit_coordinates() {
        let snapshot = LocationSnapshot::unavailable("denied");

        assert_eq!(snapshot.latitude, None);
        assert_eq!(snapshot.longitude, None);
    }

    #[test]
    fn windows_ticks_to_rfc3339_should_convert_unix_epoch() {
        assert_eq!(
            windows_ticks_to_rfc3339(116_444_736_000_000_000),
            Some("1970-01-01T00:00:00Z".to_owned())
        );
    }

    #[test]
    fn windows_ticks_to_rfc3339_should_convert_fractional_seconds() {
        assert_eq!(
            windows_ticks_to_rfc3339(116_444_736_012_345_678),
            Some("1970-01-01T00:00:01.2345678Z".to_owned())
        );
    }

    #[cfg(windows)]
    #[test]
    fn access_status_label_should_humanize_variants() {
        use windows::Devices::Geolocation::GeolocationAccessStatus;
        assert_eq!(
            access_status_label(GeolocationAccessStatus::Allowed),
            "Allowed"
        );
        assert_eq!(
            access_status_label(GeolocationAccessStatus::Denied),
            "Denied"
        );
        assert_eq!(
            access_status_label(GeolocationAccessStatus::Unspecified),
            "Unspecified"
        );
    }
}
