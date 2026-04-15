use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn parse_rfc3339_unix(value: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|dt| dt.timestamp())
}

pub fn is_valid_nanoid_id(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

pub fn format_system_time(time: SystemTime) -> String {
    match time.duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_secs().to_string(),
        Err(_) => "0".to_string(),
    }
}

pub fn expand_tilde_path(raw: &str) -> PathBuf {
    let trimmed = raw.trim();
    if trimmed == "~" {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home);
        }
        return PathBuf::from(trimmed);
    }
    if let Some(rest) = trimmed.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(trimmed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_nanoid_ids() {
        assert!(is_valid_nanoid_id("lobby"));
        assert!(is_valid_nanoid_id("abc-123_XYZ"));
        assert!(is_valid_nanoid_id("a"));
    }

    #[test]
    fn invalid_nanoid_ids() {
        assert!(!is_valid_nanoid_id(""));
        assert!(!is_valid_nanoid_id("hello world"));
        assert!(!is_valid_nanoid_id("foo.bar"));
        assert!(!is_valid_nanoid_id("a#b"));
    }

    #[test]
    fn rfc3339_parses() {
        assert!(parse_rfc3339_unix("2025-01-15T12:00:00Z").is_some());
        assert!(parse_rfc3339_unix("not-a-date").is_none());
    }

    #[test]
    fn system_time_formats() {
        let t = UNIX_EPOCH + std::time::Duration::from_secs(1000);
        assert_eq!(format_system_time(t), "1000");
    }

    #[test]
    fn tilde_expands() {
        let p = expand_tilde_path("/absolute/path");
        assert_eq!(p, PathBuf::from("/absolute/path"));
    }
}
