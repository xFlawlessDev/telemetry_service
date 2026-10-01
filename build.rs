use std::{env, fs, path::Path};

const APP_NAME: &str = "Axioo Telemetry Service";
const EXE_NAME: &str = "telemetry_service.exe";
const COPYRIGHT: &str = "Copyright © Axioo Indonesia";

const KEYS: &[&str] = &[
    "TELEMETRY_BASE_URL",
    "TELEMETRY_API_KEY",
    "TELEMETRY_USER_ID",
    "TELEMETRY_TASK_NAME",
    "TELEMETRY_BLOCK_LATITUDE",
    "TELEMETRY_BLOCK_LONGITUDE",
    "TELEMETRY_BLOCK_RADIUS_METERS",
    "TELEMETRY_IP_PUBLIC_URL",
    "TELEMETRY_SEND_IP_PUBLIC",
    "TELEMETRY_SEND_LOGS",
    "TELEMETRY_DEBUG",
];

fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    println!("cargo:rerun-if-changed=.env");

    #[cfg(windows)]
    {
        winresource::WindowsResource::new()
            .set_icon("assets/icon.ico")
            .set("FileDescription", APP_NAME)
            .set("ProductName", APP_NAME)
            .set("InternalName", APP_NAME)
            .set("OriginalFilename", EXE_NAME)
            .set("LegalCopyright", COPYRIGHT)
            .compile()
            .expect("failed to embed Windows executable resources");
    }

    if let Ok(contents) = fs::read_to_string(Path::new(".env")) {
        for line in contents.lines().filter_map(parse_env_line) {
            if KEYS.contains(&line.0) {
                println!("cargo:rustc-env={}={}", line.0, line.1);
            }
        }
    }

    for key in KEYS {
        println!("cargo:rerun-if-env-changed={key}");
        if env::var_os(key).is_some() {
            println!(
                "cargo:rustc-env={}={}",
                key,
                env::var(key).unwrap_or_default()
            );
        }
    }
}

fn parse_env_line(line: &str) -> Option<(&str, String)> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }

    let (key, value) = line.split_once('=')?;
    let key = key.trim();
    let value = value.trim();
    if key.is_empty() {
        return None;
    }

    Some((key, unquote(value)))
}

fn unquote(value: &str) -> String {
    if value.len() >= 2 {
        let bytes = value.as_bytes();
        if matches!(
            (bytes[0], bytes[value.len() - 1]),
            (b'"', b'"') | (b'\'', b'\'')
        ) {
            return value[1..value.len() - 1].to_owned();
        }
    }

    value.to_owned()
}
