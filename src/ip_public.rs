use std::time::Duration;

use tracing::warn;

/// Built-in fallback endpoint. `api.ipify.org` returns the caller's public IP
/// as plain text when no `format` query is set.
pub const IPIFY_URL: &str = "https://api.ipify.org";

/// Resolve the device's public IP in plain-text form. Tries the configured
/// endpoint first (when present), then falls back to `api.ipify.org`. Any
/// failure returns `None` and is logged; the activation request then sends
/// `ip_public=null` instead of failing the whole run.
pub async fn fetch_public_ip(
    client: &reqwest::Client,
    timeout: Duration,
    primary_url: Option<&str>,
) -> Option<String> {
    let primary = primary_url.map(str::trim).filter(|url| !url.is_empty());

    if let Some(url) = primary {
        match lookup(client, url, timeout).await {
            Ok(ip) => return Some(ip),
            Err(message) => {
                warn!(%message, %url, "configured public IP endpoint failed; trying fallback")
            }
        }
    } else if primary_url.is_some() {
        warn!("public IP endpoint configured but empty; using fallback only");
    }

    if primary == Some(IPIFY_URL) {
        return None;
    }

    match lookup(client, IPIFY_URL, timeout).await {
        Ok(ip) => Some(ip),
        Err(message) => {
            warn!(%message, url = IPIFY_URL, "public IP fallback failed");
            None
        }
    }
}

async fn lookup(client: &reqwest::Client, url: &str, timeout: Duration) -> Result<String, String> {
    match tokio::time::timeout(timeout, lookup_inner(client, url)).await {
        Ok(Ok(body)) => {
            let parsed = parse_public_ip(&body);
            if parsed == "null" {
                Err("endpoint did not return a valid IP".to_owned())
            } else {
                Ok(parsed)
            }
        }
        Ok(Err(error)) => Err(error.to_string()),
        Err(_) => Err("lookup timed out".to_owned()),
    }
}

async fn lookup_inner(client: &reqwest::Client, url: &str) -> Result<String, reqwest::Error> {
    client.get(url).send().await?.text().await
}

/// Public IP responses are plain text. Be defensive: trim whitespace, reject
/// anything with unexpected characters, and fail closed to the text `"null"`
/// so the form field format never changes.
#[must_use]
pub fn parse_public_ip(body: &str) -> String {
    let trimmed = body.trim();
    if is_valid_ip_text(trimmed) {
        trimmed.to_owned()
    } else {
        "null".to_owned()
    }
}

fn is_valid_ip_text(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 45
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() || matches!(byte, b'.' | b':'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_public_ip_should_accept_ipv4() {
        assert_eq!(parse_public_ip("203.0.113.7"), "203.0.113.7");
        assert_eq!(parse_public_ip("  203.0.113.7\n"), "203.0.113.7");
    }

    #[test]
    fn parse_public_ip_should_accept_ipv6() {
        assert_eq!(
            parse_public_ip("2001:0db8:85a3:0000:0000:8a2e:0370:7334"),
            "2001:0db8:85a3:0000:0000:8a2e:0370:7334"
        );
    }

    #[test]
    fn parse_public_ip_should_reject_html_error_pages() {
        assert_eq!(parse_public_ip("<html>rate limited</html>"), "null");
        assert_eq!(parse_public_ip(""), "null");
        assert_eq!(parse_public_ip("not an ip"), "null");
    }

    #[tokio::test]
    async fn fetch_public_ip_should_return_none_without_primary_when_fallback_unreachable() {
        // Reserved TEST-NET-1 address, never routable, so the fallback lookup
        // must fail quickly and yield None rather than hanging.
        let client = reqwest::Client::builder()
            .timeout(Duration::from_millis(200))
            .build()
            .unwrap();

        let result = fetch_public_ip(
            &client,
            Duration::from_millis(200),
            Some("http://192.0.2.1/ip"),
        )
        .await;

        assert_eq!(result, None);
    }
}
