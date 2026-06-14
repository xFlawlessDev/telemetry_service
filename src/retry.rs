use std::time::Duration;

use rand::Rng;

use crate::config::AppConfig;

#[must_use]
pub fn backoff_delay(config: &AppConfig, attempt_count: u64) -> Duration {
    let base = exponential_backoff(config.initial_backoff, config.max_backoff, attempt_count);
    apply_jitter(base, config.jitter_percent)
}

#[must_use]
pub fn exponential_backoff(initial: Duration, max: Duration, attempt_count: u64) -> Duration {
    let shift = attempt_count.min(16) as u32;
    let multiplier = 1_u32.checked_shl(shift).unwrap_or(u32::MAX);
    initial.saturating_mul(multiplier).min(max)
}

#[must_use]
pub fn apply_jitter(delay: Duration, jitter_percent: u8) -> Duration {
    if jitter_percent == 0 || delay.is_zero() {
        return delay;
    }

    let millis = delay.as_millis();
    let jitter = millis.saturating_mul(u128::from(jitter_percent)) / 100;
    let min = millis.saturating_sub(jitter);
    let max = millis.saturating_add(jitter);
    let sampled = rand::thread_rng().gen_range(min..=max);
    let millis = u64::try_from(sampled).unwrap_or(u64::MAX);
    Duration::from_millis(millis)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exponential_backoff_should_cap_at_maximum() {
        let delay = exponential_backoff(Duration::from_secs(15), Duration::from_secs(60), 10);

        assert_eq!(delay, Duration::from_secs(60));
    }

    #[test]
    fn apply_jitter_should_stay_inside_expected_bounds() {
        let base = Duration::from_secs(100);

        for _ in 0..100 {
            let delay = apply_jitter(base, 20);
            assert!(delay >= Duration::from_secs(80));
            assert!(delay <= Duration::from_secs(120));
        }
    }
}
