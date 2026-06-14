use serde::{Deserialize, Serialize};

use crate::error::AppResult;

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct HardwareIdentity {
    pub bios_serial: Option<String>,
    pub system_uuid: Option<String>,
    pub baseboard_serial: Option<String>,
    pub manufacturer: Option<String>,
    pub model: Option<String>,
}

impl HardwareIdentity {
    #[must_use]
    pub fn has_identifier(&self) -> bool {
        self.bios_serial.is_some() || self.system_uuid.is_some() || self.baseboard_serial.is_some()
    }
}

#[cfg(windows)]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Win32Bios {
    serial_number: Option<String>,
}

#[cfg(windows)]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Win32ComputerSystemProduct {
    uuid: Option<String>,
}

#[cfg(windows)]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Win32BaseBoard {
    serial_number: Option<String>,
}

#[cfg(windows)]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Win32ComputerSystem {
    manufacturer: Option<String>,
    model: Option<String>,
}

#[cfg(windows)]
pub fn collect_hardware_identity() -> AppResult<HardwareIdentity> {
    use wmi::{COMLibrary, WMIConnection};

    let com = COMLibrary::new()?;
    let wmi = WMIConnection::new(com)?;

    let bios: Vec<Win32Bios> = wmi.raw_query("SELECT SerialNumber FROM Win32_BIOS")?;
    let products: Vec<Win32ComputerSystemProduct> =
        wmi.raw_query("SELECT UUID FROM Win32_ComputerSystemProduct")?;
    let baseboards: Vec<Win32BaseBoard> =
        wmi.raw_query("SELECT SerialNumber FROM Win32_BaseBoard")?;
    let systems: Vec<Win32ComputerSystem> =
        wmi.raw_query("SELECT Manufacturer, Model FROM Win32_ComputerSystem")?;

    let system = systems.first();

    Ok(HardwareIdentity {
        bios_serial: bios
            .first()
            .and_then(|item| normalize_identifier(item.serial_number.as_deref())),
        system_uuid: products
            .first()
            .and_then(|item| normalize_identifier(item.uuid.as_deref())),
        baseboard_serial: baseboards
            .first()
            .and_then(|item| normalize_identifier(item.serial_number.as_deref())),
        manufacturer: system.and_then(|item| normalize_descriptor(item.manufacturer.as_deref())),
        model: system.and_then(|item| normalize_descriptor(item.model.as_deref())),
    })
}

#[cfg(not(windows))]
pub fn collect_hardware_identity() -> AppResult<HardwareIdentity> {
    Ok(HardwareIdentity::default())
}

#[must_use]
pub fn normalize_identifier(value: Option<&str>) -> Option<String> {
    let value = value?.trim();
    if value.is_empty() || is_placeholder(value) || is_all_zero_uuid(value) {
        None
    } else {
        Some(value.to_owned())
    }
}

#[must_use]
pub fn normalize_descriptor(value: Option<&str>) -> Option<String> {
    let value = value?.trim();
    if value.is_empty() || is_placeholder(value) {
        None
    } else {
        Some(value.to_owned())
    }
}

fn is_placeholder(value: &str) -> bool {
    matches!(
        value.to_ascii_lowercase().as_str(),
        "to be filled by o.e.m." | "default string" | "system serial number" | "none" | "unknown"
    )
}

fn is_all_zero_uuid(value: &str) -> bool {
    let mut saw_hex = false;
    for byte in value.bytes() {
        match byte {
            b'0' => saw_hex = true,
            b'-' | b'{' | b'}' => {}
            _ => return false,
        }
    }
    saw_hex
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_identifier_should_remove_invalid_serial_values() {
        assert_eq!(normalize_identifier(Some("To be filled by O.E.M.")), None);
        assert_eq!(normalize_identifier(Some("Default string")), None);
        assert_eq!(normalize_identifier(Some("System Serial Number")), None);
        assert_eq!(
            normalize_identifier(Some("00000000-0000-0000-0000-000000000000")),
            None
        );
    }

    #[test]
    fn normalize_identifier_should_keep_valid_value_trimmed() {
        assert_eq!(
            normalize_identifier(Some(" ABC123 ")),
            Some("ABC123".to_owned())
        );
    }
}
