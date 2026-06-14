use serde::Serialize;
use uuid::Uuid;

use crate::{hardware::HardwareIdentity, location::LocationSnapshot, state::ActivationState};

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ActivationPayload<'a> {
    pub install_id: Uuid,
    pub agent_version: &'a str,
    pub hardware: &'a HardwareIdentity,
    pub location: &'a LocationSnapshot,
    pub attempt: AttemptPayload<'a>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct AttemptPayload<'a> {
    pub count: u64,
    pub first_seen_utc: &'a str,
    pub last_attempt_utc: Option<&'a str>,
}

#[must_use]
pub fn build_activation_payload<'a>(
    state: &'a ActivationState,
    agent_version: &'a str,
    hardware: &'a HardwareIdentity,
    location: &'a LocationSnapshot,
) -> ActivationPayload<'a> {
    ActivationPayload {
        install_id: state.install_id,
        agent_version,
        hardware,
        location,
        attempt: AttemptPayload {
            count: state.attempt_count,
            first_seen_utc: &state.first_seen_utc,
            last_attempt_utc: state.last_attempt_utc.as_deref(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::ActivationState;

    #[test]
    fn build_activation_payload_should_serialize_missing_location_as_null() {
        let state = ActivationState::new("2026-01-01T00:00:00Z".to_owned());
        let hardware = HardwareIdentity::default();
        let location = LocationSnapshot::unavailable("denied");
        let payload = build_activation_payload(&state, "0.1.0", &hardware, &location);

        let json = serde_json::to_value(payload).unwrap();

        assert!(json["location"]["latitude"].is_null());
        assert!(json["location"]["longitude"].is_null());
    }
}
