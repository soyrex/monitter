//! Stable pane identifiers and restore-time lookup for pinned terminal panes.
//!
//! These helpers only project existing backend IDs. They never infer a new
//! terminal session or mutate the backend.

use serde_json::Value;

const TERMINAL_PANE_PREFIX: &str = "terminal:";

/// Builds the stable dock ID for an existing canonical, non-nil UUID.
pub(crate) fn terminal_pane_id(id: &str) -> Result<String, String> {
    if !is_canonical_non_nil_uuid(id) {
        return Err("Terminal pane ID requires a canonical non-nil UUID.".into());
    }
    Ok(format!("{TERMINAL_PANE_PREFIX}{id}"))
}

/// Parses a stable terminal pane ID without allocating or normalizing it.
pub(crate) fn parse_terminal_pane_id(id: &str) -> Option<&str> {
    let session_id = id.strip_prefix(TERMINAL_PANE_PREFIX)?;
    is_canonical_non_nil_uuid(session_id).then_some(session_id)
}

/// Result of resolving a saved terminal pane against the current authoritative
/// `list_terminals` response. Exited sessions remain distinguishable so callers
/// can show the backend's real exit state instead of silently dropping them.
pub(crate) enum TerminalSessionLookup<'a> {
    Running(&'a Value),
    Exited(&'a Value),
    Missing,
    UnknownStatus(&'a Value),
    Ambiguous,
}

/// Resolves only an exact canonical pane ID present exactly once in the
/// current backend session list. Invalid, stale, or duplicate IDs are never
/// mapped to a guessed session.
pub(crate) fn lookup_terminal_session<'a>(
    pane_id: &str,
    sessions: &'a [Value],
) -> TerminalSessionLookup<'a> {
    let Some(session_id) = parse_terminal_pane_id(pane_id) else {
        return TerminalSessionLookup::Missing;
    };
    let mut matching = sessions
        .iter()
        .filter(|session| session.get("id").and_then(Value::as_str) == Some(session_id));
    let Some(session) = matching.next() else {
        return TerminalSessionLookup::Missing;
    };
    if matching.next().is_some() {
        return TerminalSessionLookup::Ambiguous;
    }
    match session.get("status").and_then(Value::as_str) {
        Some("running") => TerminalSessionLookup::Running(session),
        Some("exited") => TerminalSessionLookup::Exited(session),
        _ => TerminalSessionLookup::UnknownStatus(session),
    }
}

fn is_canonical_non_nil_uuid(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 36
        || ![8, 13, 18, 23]
            .into_iter()
            .all(|index| bytes[index] == b'-')
    {
        return false;
    }
    let mut non_zero = false;
    for (index, byte) in bytes.iter().copied().enumerate() {
        if [8, 13, 18, 23].contains(&index) {
            continue;
        }
        match byte {
            b'0'..=b'9' => non_zero |= byte != b'0',
            b'a'..=b'f' => non_zero = true,
            _ => return false,
        }
    }
    non_zero
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const SESSION_ID: &str = "550e8400-e29b-41d4-a716-446655440000";

    #[::core::prelude::v1::test]
    fn pane_id_round_trips_only_canonical_non_nil_uuid() {
        let pane_id = terminal_pane_id(SESSION_ID).unwrap();
        assert_eq!(pane_id, format!("terminal:{SESSION_ID}"));
        assert_eq!(parse_terminal_pane_id(&pane_id), Some(SESSION_ID));

        for invalid in [
            "",
            "550E8400-E29B-41D4-A716-446655440000",
            "550e8400e29b41d4a716446655440000",
            "00000000-0000-0000-0000-000000000000",
            "550e8400-e29b-41d4-a716-446655440000/extra",
        ] {
            assert!(terminal_pane_id(invalid).is_err(), "accepted {invalid:?}");
            assert!(
                parse_terminal_pane_id(invalid).is_none(),
                "parsed {invalid:?}"
            );
        }
        assert!(terminal_pane_id(&pane_id).is_err());
        assert!(parse_terminal_pane_id("terminal:00000000-0000-0000-0000-000000000000").is_none());
    }

    #[::core::prelude::v1::test]
    fn lookup_distinguishes_running_exited_missing_and_ambiguous_sessions() {
        let running = json!({"id": SESSION_ID, "status": "running"});
        let exited = json!({"id": SESSION_ID, "status": "exited"});
        let pane_id = terminal_pane_id(SESSION_ID).unwrap();

        assert!(matches!(
            lookup_terminal_session(&pane_id, std::slice::from_ref(&running)),
            TerminalSessionLookup::Running(session) if session == &running
        ));
        assert!(matches!(
            lookup_terminal_session(&pane_id, std::slice::from_ref(&exited)),
            TerminalSessionLookup::Exited(session) if session == &exited
        ));
        assert!(matches!(
            lookup_terminal_session(&pane_id, &[]),
            TerminalSessionLookup::Missing
        ));
        assert!(matches!(
            lookup_terminal_session(&pane_id, &[running.clone(), running]),
            TerminalSessionLookup::Ambiguous
        ));
        assert!(matches!(
            lookup_terminal_session("terminal:not-a-uuid", std::slice::from_ref(&exited)),
            TerminalSessionLookup::Missing
        ));
    }

    #[::core::prelude::v1::test]
    fn unknown_backend_status_is_not_treated_as_running() {
        let session = json!({"id": SESSION_ID, "status": "restarting"});
        let pane_id = terminal_pane_id(SESSION_ID).unwrap();
        assert!(matches!(
            lookup_terminal_session(&pane_id, std::slice::from_ref(&session)),
            TerminalSessionLookup::UnknownStatus(found) if found == &session
        ));
    }
}
