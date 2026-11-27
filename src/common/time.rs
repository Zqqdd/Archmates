//! Times and sizes as shown to people.

use std::time::{SystemTime, UNIX_EPOCH};

/// Milliseconds since 1970 (the timestamps stored in conversations and state.json).
pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// "just now", "5 min ago", "3 h ago", "2 d ago".
pub fn ago(ms: u64) -> String {
    let s = now_ms().saturating_sub(ms) / 1000;
    match s {
        0..=59 => "just now".into(),
        60..=3599 => format!("{} min ago", s / 60),
        3600..=86_399 => format!("{} h ago", s / 3600),
        _ => format!("{} d ago", s / 86_400),
    }
}

/// Bytes as gigabytes with one decimal ("12.3").
pub fn gb(bytes: u64) -> String {
    format!("{:.1}", bytes as f64 / 1_073_741_824.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readable_times() {
        assert_eq!(ago(now_ms()), "just now");
        assert_eq!(ago(now_ms() - 5 * 60_000), "5 min ago");
        assert_eq!(ago(now_ms() - 3 * 3_600_000), "3 h ago");
        assert_eq!(gb(1_073_741_824), "1.0");
    }
}
