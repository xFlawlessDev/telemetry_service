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

pub const EARTH_RADIUS_METERS: f64 = 6_371_000.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GeoPoint {
    pub latitude: f64,
    pub longitude: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BlockZone {
    pub center: GeoPoint,
    pub radius_meters: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockDecision {
    Blocked,
    Clear,
    Unknown,
}

impl LocationSnapshot {
    #[must_use]
    pub fn point(&self) -> Option<GeoPoint> {
        Some(GeoPoint {
            latitude: self.latitude?,
            longitude: self.longitude?,
        })
    }
}

impl BlockZone {
    #[must_use]
    pub fn contains(&self, point: GeoPoint) -> bool {
        haversine_meters(self.center, point) <= self.radius_meters
    }

    #[must_use]
    pub fn evaluate(&self, location: &LocationSnapshot) -> BlockDecision {
        match location.point() {
            Some(point) if self.contains(point) => BlockDecision::Blocked,
            Some(_) => BlockDecision::Clear,
            None => BlockDecision::Unknown,
        }
    }
}

#[must_use]
pub fn haversine_meters(from: GeoPoint, to: GeoPoint) -> f64 {
    let from_latitude = from.latitude.to_radians();
    let to_latitude = to.latitude.to_radians();
    let delta_latitude = (to.latitude - from.latitude).to_radians();
    let delta_longitude = (to.longitude - from.longitude).to_radians();

    let a = (delta_latitude / 2.0).sin().powi(2)
        + from_latitude.cos() * to_latitude.cos() * (delta_longitude / 2.0).sin().powi(2);
    let c = 2.0 * a.sqrt().asin();
    EARTH_RADIUS_METERS * c
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

    fn snapshot_at(latitude: f64, longitude: f64) -> LocationSnapshot {
        LocationSnapshot {
            access_status: "Allowed".to_owned(),
            latitude: Some(latitude),
            longitude: Some(longitude),
            accuracy_meters: Some(10.0),
            timestamp_utc: None,
            error: None,
        }
    }

    fn jakarta_zone() -> BlockZone {
        BlockZone {
            center: GeoPoint {
                latitude: -6.2,
                longitude: 106.816_666,
            },
            radius_meters: 1_000.0,
        }
    }

    #[test]
    fn haversine_should_return_zero_for_identical_points() {
        let point = GeoPoint {
            latitude: -6.2,
            longitude: 106.816_666,
        };

        assert_eq!(haversine_meters(point, point), 0.0);
    }

    #[test]
    fn haversine_should_measure_known_distance() {
        let paris = GeoPoint {
            latitude: 48.856_6,
            longitude: 2.352_2,
        };
        let london = GeoPoint {
            latitude: 51.507_4,
            longitude: -0.127_8,
        };

        let distance = haversine_meters(paris, london);

        assert!(
            (343_000.0..=345_000.0).contains(&distance),
            "distance: {distance}"
        );
    }

    #[test]
    fn block_zone_should_block_point_inside_radius() {
        let zone = jakarta_zone();
        let inside = GeoPoint {
            latitude: -6.2,
            longitude: 106.816_666,
        };

        assert!(zone.contains(inside));
    }

    #[test]
    fn block_zone_should_allow_point_outside_radius() {
        let zone = jakarta_zone();
        let outside = GeoPoint {
            latitude: -6.91,
            longitude: 107.61,
        };

        assert!(!zone.contains(outside));
    }

    #[test]
    fn evaluate_should_report_blocked_inside_zone() {
        let zone = jakarta_zone();

        assert_eq!(
            zone.evaluate(&snapshot_at(-6.2, 106.816_666)),
            BlockDecision::Blocked
        );
    }

    #[test]
    fn evaluate_should_report_clear_outside_zone() {
        let zone = jakarta_zone();

        assert_eq!(
            zone.evaluate(&snapshot_at(-6.91, 107.61)),
            BlockDecision::Clear
        );
    }

    #[test]
    fn evaluate_should_report_unknown_without_coordinates() {
        let zone = jakarta_zone();

        assert_eq!(
            zone.evaluate(&LocationSnapshot::unavailable("denied")),
            BlockDecision::Unknown
        );
    }

    #[test]
    fn point_should_be_none_without_coordinates() {
        assert_eq!(LocationSnapshot::unavailable("denied").point(), None);
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
