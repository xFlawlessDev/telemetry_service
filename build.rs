use std::{env, fs, path::Path};

const KEYS: &[&str] = &[
    "TELEMETRY_BASE_URL",
    "TELEMETRY_API_KEY",
    "TELEMETRY_TASK_NAME",
];

fn main() {
    println!("cargo:rerun-if-changed=.env");

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
