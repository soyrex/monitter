//! Local display time; persisted values remain Unix milliseconds.
use chrono::{DateTime, Local, Utc};

pub fn local_datetime(milliseconds: i64) -> Option<DateTime<Local>> {
    DateTime::<Utc>::from_timestamp_millis(milliseconds).map(|date| date.with_timezone(&Local))
}

pub fn datetime_label(milliseconds: i64) -> String {
    local_datetime(milliseconds)
        .map(|date| date.format("%b %-d, %Y, %-I:%M %p %Z").to_string())
        .unwrap_or_else(|| "Not reported".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[::core::prelude::v1::test]
    fn invalid_provider_timestamp_is_not_presented_as_a_date() {
        assert!(local_datetime(i64::MAX).is_none());
        assert_eq!(datetime_label(i64::MAX), "Not reported");
    }
}
