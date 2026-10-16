//! Pure bounds and stable identity for authenticated agent checkpoint reports.
//! Durable receipt, retries, rate limits and running-turn authentication live
//! at the primary Store/coordinator boundary.

use sha2::{Digest, Sha256};

use super::access::CallerIdentity;
use super::types::ReportInput;

fn safe_text(value: &str) -> bool {
    !value.chars().any(|ch| ch.is_control()
        || matches!(ch, '\u{2028}' | '\u{2029}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'))
}

pub fn validate(input: &ReportInput) -> Result<(), String> {
    if input.request_id.is_empty()
        || input.request_id.len() > 128
        || input.request_id.trim() != input.request_id
        || !safe_text(&input.request_id)
    {
        return Err("Invalid work report request ID.".into());
    }
    if !matches!(
        input.phase.as_str(),
        "investigate" | "plan" | "implement" | "verify" | "review" | "other"
    ) {
        return Err("Invalid work report phase.".into());
    }
    let summary_chars = input.summary.chars().count();
    if summary_chars == 0
        || summary_chars > 1000
        || input.summary.trim().is_empty()
        || !safe_text(&input.summary)
    {
        return Err("Invalid work report summary.".into());
    }
    if let Some(id) = &input.plan_item_id {
        if id.is_empty() || id.len() > 128 || id.trim() != id || !safe_text(id) {
            return Err("Invalid work report plan item ID.".into());
        }
    }
    if input.labels.len() > 8
        || input.labels.iter().any(|label| {
            let length = label.chars().count();
            length == 0 || length > 64 || label.trim().is_empty() || !safe_text(label)
        })
    {
        return Err("Invalid work report labels.".into());
    }
    Ok(())
}

/// Stable hash of report content, excluding retry/request identity.
pub fn fingerprint(input: &ReportInput) -> Result<String, String> {
    validate(input)?;
    let canonical = serde_json::to_vec(&(
        &input.phase,
        &input.summary,
        &input.plan_item_id,
        &input.labels,
    ))
    .map_err(|_| "Cannot encode work report.".to_string())?;
    Ok(format!("{:x}", Sha256::digest(canonical)))
}

/// Exact retry identity, scoped to the backend-authenticated caller run.
/// Coordinator compares `fingerprint` on duplicate keys before returning a
/// durable receipt, rejecting a reused request ID with different content.
pub fn source_key(caller: &CallerIdentity, input: &ReportInput) -> Result<String, String> {
    validate(input)?;
    if caller.installation_id.is_empty()
        || caller.profile_id.is_empty()
        || caller.task_id.is_empty()
        || caller.run_id.is_empty()
    {
        return Err("Work report caller is unavailable.".into());
    }
    let encoded = serde_json::to_vec(&(
        &caller.installation_id,
        &caller.profile_id,
        &caller.task_id,
        &caller.run_id,
        &input.request_id,
    ))
    .map_err(|_| "Cannot encode work report identity.".to_string())?;
    Ok(format!("report:{:x}", Sha256::digest(encoded)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input() -> ReportInput {
        ReportInput {
            request_id: "request".into(),
            phase: "verify".into(),
            summary: "Tests passed".into(),
            plan_item_id: None,
            labels: vec!["tests".into()],
        }
    }

    #[test]
    fn bounds_and_control_restrictions() {
        let mut report = input();
        assert!(validate(&report).is_ok());
        report.phase = "unknown".into();
        assert!(validate(&report).is_err());
        report.phase = "verify".into();
        report.summary = "\u{202e}spoof".into();
        assert!(validate(&report).is_err());
        report.summary = "a".repeat(1001);
        assert!(validate(&report).is_err());
        report.summary = "ok".into();
        report.labels = vec!["a".into(); 9];
        assert!(validate(&report).is_err());
        report.labels = vec!["x".repeat(65)];
        assert!(validate(&report).is_err());
        report.labels.clear();
        report.request_id = "x".repeat(129);
        assert!(validate(&report).is_err());
    }

    #[test]
    fn content_and_retry_identity_are_independent() {
        let caller = CallerIdentity {
            installation_id: "i".into(),
            profile_id: "p".into(),
            task_id: "t".into(),
            run_id: "r".into(),
        };
        let a = input();
        let mut b = a.clone();
        b.request_id = "retry".into();
        assert_eq!(fingerprint(&a), fingerprint(&b));
        assert_ne!(source_key(&caller, &a), source_key(&caller, &b));
        b = a.clone();
        b.summary = "Changed".into();
        assert_ne!(fingerprint(&a), fingerprint(&b));
        assert_eq!(source_key(&caller, &a), source_key(&caller, &b));
    }
}
