//! Bounded, read-only resolution of a snapshotted Work Log context reference.
//! The coordinator supplies a canonical read-only `state.sqlite3` connection
//! and a fresh grant lookup; no provider/native transcript is opened here.

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use super::access::{authorize, CallerIdentity, Operation, OwnerGrant, SourceIdentity};
use super::types::{ContextAvailability, ContextRef};

const MAX_PAGE: u32 = 64;
// Budget after both JSON encodings used by MCP content text. Leave ample
// room for the JSON-RPC envelope and request identity under the128KiB broker.
const MAX_TEXT_BYTES: usize = 80 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ContextMessage {
    pub id: String,
    pub role: String,
    pub text: String,
    pub created_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_offset_bytes: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_complete: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ContextPage {
    pub availability: ContextAvailability,
    pub messages: Vec<ContextMessage>,
    pub next_cursor: Option<String>,
    pub source_message_id: Option<String>,
    pub through_message_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextReadError {
    Denied,
    Unavailable,
}

fn empty(reference: &ContextRef, availability: ContextAvailability) -> ContextPage {
    ContextPage {
        availability,
        messages: Vec::new(),
        next_cursor: None,
        source_message_id: reference.source_message_id.clone(),
        through_message_id: reference.through_message_id.clone(),
    }
}

fn position(
    connection: &Connection,
    task_id: &str,
    id: &str,
) -> Result<Option<(i64, String)>, ContextReadError> {
    connection.query_row(
        "SELECT position,payload FROM entities WHERE collection='messages' AND task_id=?1 AND id=?2",
        params![task_id,id], |row| Ok((row.get(0)?,row.get(1)?)),
    ).optional().map_err(|_| ContextReadError::Unavailable)
}

/// `fresh_grants` must read the current owner-private grant records on *each*
/// invocation. It is invoked before and after the SQLite page read so a revoked
/// grant cannot return data after an asynchronous/coordinator boundary.
pub fn read_page<F>(
    connection: &Connection,
    reference: &ContextRef,
    source: &SourceIdentity,
    caller: &CallerIdentity,
    mut fresh_grants: F,
    cursor: Option<&str>,
    limit: Option<u32>,
) -> Result<ContextPage, ContextReadError>
where
    F: FnMut() -> Result<Vec<OwnerGrant>, ContextReadError>,
{
    let check = |grants: &[OwnerGrant]| {
        authorize(caller, source, Operation::ReadContext, grants)
            .map_err(|_| ContextReadError::Denied)
    };
    check(&fresh_grants()?)?;
    if reference.installation_id != source.installation_id
        || reference.profile_id != source.profile_id
        || reference.task_id != source.task_id
        || reference.project_id != source.project_id
    {
        return Err(ContextReadError::Denied);
    }
    let result = read_canonical_page(connection, reference, cursor, limit)?;
    check(&fresh_grants()?)?;
    Ok(result)
}

fn read_canonical_page(
    connection: &Connection,
    reference: &ContextRef,
    cursor: Option<&str>,
    limit: Option<u32>,
) -> Result<ContextPage, ContextReadError> {
    if reference.availability != ContextAvailability::Available {
        return Ok(empty(reference, reference.availability.clone()));
    }
    let task_exists: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM entities WHERE collection='tasks' AND id=?1)",
            [&reference.task_id],
            |row| row.get(0),
        )
        .map_err(|_| ContextReadError::Unavailable)?;
    if !task_exists {
        return Ok(empty(reference, ContextAvailability::Deleted));
    }

    // The upper anchor is frozen at capture. Never read through the current
    // native session or the latest message after a clear, fork, or restart.
    let end_id = reference
        .through_message_id
        .as_deref()
        .or(reference.source_message_id.as_deref());
    let Some(end_id) = end_id else {
        return Ok(empty(reference, ContextAvailability::NotRecorded));
    };
    let Some((through, _)) = position(connection, &reference.task_id, end_id)? else {
        return Ok(empty(reference, ContextAvailability::NotRecorded));
    };
    let source_position = match reference.source_message_id.as_deref() {
        Some(source_id) => {
            let Some((position, _)) = position(connection, &reference.task_id, source_id)? else {
                return Ok(empty(reference, ContextAvailability::NotRecorded));
            };
            Some(position)
        }
        None => None,
    };
    let start = match reference.segment_start_id.as_deref() {
        Some(id) => {
            let Some((position, payload)) = position(connection, &reference.task_id, id)? else {
                return Ok(empty(reference, ContextAvailability::NotRecorded));
            };
            let value: serde_json::Value =
                serde_json::from_str(&payload).map_err(|_| ContextReadError::Unavailable)?;
            let clear_marker = value.get("role").and_then(|v| v.as_str()) == Some("system")
                && value.get("text").and_then(|v| v.as_str()) == Some("Context Cleared");
            if clear_marker {
                position
            } else {
                position.saturating_sub(1)
            }
        }
        None => i64::MIN,
    };
    if start >= through {
        return Ok(empty(reference, ContextAvailability::NotRecorded));
    }
    if source_position.is_some_and(|position| position <= start || position > through) {
        return Ok(empty(reference, ContextAvailability::NotRecorded));
    }
    let (cursor, chunk_offset) = match cursor {
        Some(value) if value.starts_with("chunk:") => {
            let (id, offset) = value[6..]
                .rsplit_once(':')
                .ok_or(ContextReadError::Unavailable)?;
            let offset = offset
                .parse::<usize>()
                .ok()
                .filter(|n| *n > 0 && *n <= 16 * 1024 * 1024)
                .ok_or(ContextReadError::Unavailable)?;
            (Some(id), offset)
        }
        other => (other, 0),
    };
    let after = match cursor {
        Some(id) => {
            let Some((position, payload)) = position(connection, &reference.task_id, id)? else {
                return Err(ContextReadError::Unavailable);
            };
            if position <= start || position > through {
                return Err(ContextReadError::Denied);
            }
            let value: serde_json::Value =
                serde_json::from_str(&payload).map_err(|_| ContextReadError::Unavailable)?;
            if !matches!(
                value.get("role").and_then(|v| v.as_str()),
                Some("user" | "assistant")
            ) {
                return Err(ContextReadError::Denied);
            }
            if chunk_offset > 0 {
                position.saturating_sub(1)
            } else {
                position
            }
        }
        None => start,
    };
    let limit = limit.unwrap_or(MAX_PAGE).clamp(1, MAX_PAGE) as usize;
    let mut statement = connection
        .prepare(
            "SELECT payload FROM entities WHERE collection='messages' AND task_id=?1 \
         AND position>?2 AND position<=?3 \
         AND json_extract(payload,'$.role') IN ('user','assistant') \
         ORDER BY position ASC LIMIT ?4",
        )
        .map_err(|_| ContextReadError::Unavailable)?;
    let rows = statement
        .query_map(
            params![reference.task_id, after, through, (limit + 1) as i64],
            |row| row.get::<_, String>(0),
        )
        .map_err(|_| ContextReadError::Unavailable)?;
    let mut messages = Vec::new();
    let mut bytes = 0usize;
    let mut more = false;
    let mut chunk_cursor = None;
    for row in rows {
        let payload = row.map_err(|_| ContextReadError::Unavailable)?;
        let value: serde_json::Value =
            serde_json::from_str(&payload).map_err(|_| ContextReadError::Unavailable)?;
        let string = |key| value.get(key).and_then(|v| v.as_str()).map(str::to_owned);
        let (Some(id), Some(task_id), Some(role), Some(text)) = (
            string("id"),
            string("taskId"),
            string("role"),
            string("text"),
        ) else {
            return Err(ContextReadError::Unavailable);
        };
        if task_id != reference.task_id || !matches!(role.as_str(), "user" | "assistant") {
            return Err(ContextReadError::Unavailable);
        }
        let created_at = value
            .get("createdAt")
            .and_then(|v| v.as_i64())
            .ok_or(ContextReadError::Unavailable)?;
        let text = if reference.through_message_id.as_deref() == Some(id.as_str()) {
            if let Some(cap) = reference.through_message_text_bytes {
                let mut cap = (cap as usize).min(text.len());
                while !text.is_char_boundary(cap) {
                    cap -= 1;
                }
                text[..cap].to_owned()
            } else {
                text
            }
        } else {
            text
        };
        if text.len() > 16 * 1024 * 1024 {
            return Ok(empty(
                reference,
                ContextAvailability::TemporarilyUnavailable,
            ));
        }
        let offset = if cursor == Some(id.as_str()) {
            chunk_offset
        } else {
            0
        };
        if offset > text.len() || !text.is_char_boundary(offset) {
            return Err(ContextReadError::Unavailable);
        }
        let mut end = (offset + 8192).min(text.len());
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        let partial = offset > 0 || end < text.len();
        let message = ContextMessage {
            id,
            role,
            text: text[offset..end].into(),
            created_at,
            text_offset_bytes: partial.then_some(offset as u32),
            text_complete: partial.then_some(end == text.len()),
        };
        let wire_bytes = serde_json::to_string(&message)
            .ok()
            .and_then(|s| serde_json::to_string(&s).ok())
            .map(|s| s.len())
            .ok_or(ContextReadError::Unavailable)?;
        if messages.len() == limit || bytes.saturating_add(wire_bytes) > MAX_TEXT_BYTES {
            if messages.is_empty() {
                return Ok(empty(
                    reference,
                    ContextAvailability::TemporarilyUnavailable,
                ));
            }
            more = true;
            break;
        }
        let unfinished = end < text.len();
        if unfinished {
            chunk_cursor = Some(format!("chunk:{}:{}", message.id, end));
        }
        bytes += wire_bytes;
        messages.push(message);
        if unfinished {
            break;
        }
    }
    let next_cursor = if chunk_cursor.is_some() {
        chunk_cursor
    } else if more {
        messages.last().map(|message| message.id.clone())
    } else {
        None
    };
    Ok(ContextPage {
        availability: ContextAvailability::Available,
        messages,
        next_cursor,
        source_message_id: reference.source_message_id.clone(),
        through_message_id: reference.through_message_id.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::super::access::{GrantScope, Operation};
    use super::*;

    pub(super) fn fixture() -> (
        Connection,
        ContextRef,
        SourceIdentity,
        CallerIdentity,
        OwnerGrant,
    ) {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE entities (collection TEXT NOT NULL, id TEXT NOT NULL, position INTEGER NOT NULL, task_id TEXT, payload TEXT NOT NULL, PRIMARY KEY(collection,id));").unwrap();
        db.execute(
            "INSERT INTO entities VALUES('tasks','source',0,NULL,'{}')",
            [],
        )
        .unwrap();
        for (id, task, position, role, text) in [
            ("old", "source", 1, "user", "old"),
            ("clear", "source", 2, "system", "Context Cleared"),
            ("one", "source", 3, "user", "one"),
            ("hidden", "source", 4, "system", "saved instructions"),
            ("two", "source", 5, "assistant", "two"),
            ("later", "source", 6, "user", "later"),
            ("alien", "other", 7, "user", "private"),
        ] {
            let payload = serde_json::json!({"id":id,"taskId":task,"role":role,"text":text,"createdAt":position});
            db.execute(
                "INSERT INTO entities VALUES('messages',?1,?2,?3,?4)",
                params![id, position, task, payload.to_string()],
            )
            .unwrap();
        }
        let reference = ContextRef {
            context_ref: "opaque".into(),
            installation_id: "install".into(),
            profile_id: "profile".into(),
            project_id: Some("project".into()),
            task_id: "source".into(),
            run_id: "run".into(),
            provider_turn_id: None,
            segment_start_id: Some("clear".into()),
            source_message_id: Some("one".into()),
            through_message_id: Some("two".into()),
            through_message_text_bytes: None,
            title: "renamed".into(),
            agent_name: "agent".into(),
            harness: "codex".into(),
            lineage_kind: None,
            lineage_root_task_id: None,
            source_task_id: None,
            source_through_message_id: None,
            native_session_id: Some("secret-native-id".into()),
            availability: ContextAvailability::Available,
            unavailable_reason: None,
        };
        let source = SourceIdentity {
            installation_id: "install".into(),
            profile_id: "profile".into(),
            task_id: "source".into(),
            project_id: Some("project".into()),
            lineage_id: Some("tree".into()),
        };
        let caller = CallerIdentity {
            installation_id: "install".into(),
            profile_id: "profile".into(),
            task_id: "reader".into(),
            run_id: "reader-run".into(),
        };
        let grant = OwnerGrant {
            installation_id: "install".into(),
            profile_id: "profile".into(),
            caller_task_id: "reader".into(),
            operation: Operation::ReadContext,
            scope: GrantScope::Lineage("tree".into()),
            revoked: false,
        };
        (db, reference, source, caller, grant)
    }

    #[test]
    fn segment_boundary_paging_and_private_fields() {
        let (db, reference, source, caller, grant) = fixture();
        let first = read_page(
            &db,
            &reference,
            &source,
            &caller,
            || Ok(vec![grant.clone()]),
            None,
            Some(1),
        )
        .unwrap();
        assert_eq!(
            first
                .messages
                .iter()
                .map(|m| m.id.as_str())
                .collect::<Vec<_>>(),
            vec!["one"]
        );
        assert_eq!(first.next_cursor.as_deref(), Some("one"));
        let second = read_page(
            &db,
            &reference,
            &source,
            &caller,
            || Ok(vec![grant.clone()]),
            first.next_cursor.as_deref(),
            Some(64),
        )
        .unwrap();
        assert_eq!(
            second
                .messages
                .iter()
                .map(|m| m.id.as_str())
                .collect::<Vec<_>>(),
            vec!["two"]
        );
        assert!(second.next_cursor.is_none());
        let serialized = serde_json::to_string(&second).unwrap();
        assert!(!serialized.contains("secret-native-id"));
        assert!(!serialized.contains("saved instructions"));
        assert!(!serialized.contains("later"));
    }

    #[test]
    fn cross_task_and_out_of_segment_cursors_fail() {
        let (db, reference, source, caller, grant) = fixture();
        for id in ["old", "later"] {
            assert!(read_page(
                &db,
                &reference,
                &source,
                &caller,
                || Ok(vec![grant.clone()]),
                Some(id),
                None
            )
            .is_err());
        }
        assert!(read_page(
            &db,
            &reference,
            &source,
            &caller,
            || Ok(vec![grant.clone()]),
            Some("alien"),
            None
        )
        .is_err());
    }

    #[test]
    fn deletion_missing_anchor_and_revocation_are_safe() {
        let (db, mut reference, source, caller, grant) = fixture();
        let mut checks = 0;
        let denied = read_page(
            &db,
            &reference,
            &source,
            &caller,
            || {
                checks += 1;
                let mut current = grant.clone();
                current.revoked = checks == 2;
                Ok(vec![current])
            },
            None,
            None,
        );
        assert_eq!(denied, Err(ContextReadError::Denied));
        reference.through_message_id = Some("missing".into());
        let page = read_page(
            &db,
            &reference,
            &source,
            &caller,
            || Ok(vec![grant.clone()]),
            None,
            None,
        )
        .unwrap();
        assert_eq!(page.availability, ContextAvailability::NotRecorded);
        db.execute(
            "DELETE FROM entities WHERE collection='tasks' AND id='source'",
            [],
        )
        .unwrap();
        let page = read_page(
            &db,
            &reference,
            &source,
            &caller,
            || Ok(vec![grant.clone()]),
            None,
            None,
        )
        .unwrap();
        assert_eq!(page.availability, ContextAvailability::Deleted);
    }

    #[test]
    fn byte_bound_chunks_oversized_first_message() {
        let (db, mut reference, source, caller, grant) = fixture();
        let payload = serde_json::json!({"id":"big","taskId":"source","role":"user",
            "text":"x".repeat(MAX_TEXT_BYTES+1),"createdAt":8});
        db.execute(
            "INSERT INTO entities VALUES('messages','big',8,'source',?1)",
            [payload.to_string()],
        )
        .unwrap();
        reference.segment_start_id = Some("big".into());
        reference.source_message_id = Some("big".into());
        reference.through_message_id = Some("big".into());
        let page = read_page(
            &db,
            &reference,
            &source,
            &caller,
            || Ok(vec![grant.clone()]),
            None,
            None,
        )
        .unwrap();
        assert_eq!(page.availability, ContextAvailability::Available);
        assert_eq!(page.messages.len(), 1);
        assert!(page.messages[0].text.len() <= MAX_TEXT_BYTES);
        assert_eq!(page.messages[0].text_complete, Some(false));
        assert!(page.next_cursor.is_some());
    }
}

#[cfg(test)]
mod wire_tests {
    use super::*;
    #[test]
    fn encoded_mcp_page_stays_below_broker_limit_for_escaped_text() {
        let message = ContextMessage {
            id: "id".into(),
            role: "assistant".into(),
            text: "\\\"\n\t".repeat(2000),
            created_at: 1,
            text_offset_bytes: None,
            text_complete: None,
        };
        let page = ContextPage {
            availability: ContextAvailability::Available,
            messages: vec![message],
            next_cursor: None,
            source_message_id: None,
            through_message_id: None,
        };
        let value = serde_json::to_value(&page).unwrap();
        let handler = move |_: &str, _: &str, _: serde_json::Value| Ok(value.clone());
        let result=crate::collaboration_mcp::dispatch("fixture",&handler,serde_json::json!({"jsonrpc":"2.0","id":"fixture-request","method":"tools/call","params":{"name":"work_log_read_context","arguments":{"context_ref":uuid::Uuid::new_v4().to_string()}}})).unwrap();
        assert!(serde_json::to_vec(&result).unwrap().len() < 128 * 1024);
    }
    #[test]
    fn oversized_utf8_and_escaped_message_is_recoverable_through_bounded_chunks() {
        let (db, mut reference, source, caller, grant) = super::tests::fixture();
        let text = "é\\\"\n\t".repeat(20_000);
        db.execute("UPDATE entities SET payload=json_set(payload,'$.text',?1) WHERE collection='messages' AND id='one'",[&text]).unwrap();
        reference.through_message_id = Some("one".into());
        reference.through_message_text_bytes = Some(text.len() as u32);
        let mut cursor = None;
        let mut recovered = String::new();
        loop {
            let page = read_page(
                &db,
                &reference,
                &source,
                &caller,
                || Ok(vec![grant.clone()]),
                cursor.as_deref(),
                Some(1),
            )
            .unwrap();
            assert_eq!(page.availability, ContextAvailability::Available);
            assert!(serde_json::to_vec(&page).unwrap().len() < 100 * 1024);
            assert_eq!(page.messages.len(), 1);
            if let Some(offset) = page.messages[0].text_offset_bytes {
                assert_eq!(offset as usize, recovered.len());
            }
            recovered.push_str(&page.messages[0].text);
            match page.next_cursor {
                Some(next) => cursor = Some(next),
                None => break,
            }
        }
        assert_eq!(recovered, text);
        // A continued stream cannot enlarge an already captured endpoint.
        reference.through_message_text_bytes = Some(10);
        let page = read_page(
            &db,
            &reference,
            &source,
            &caller,
            || Ok(vec![grant.clone()]),
            None,
            Some(64),
        )
        .unwrap();
        assert!(page.messages[0].text.len() <= 10);
        assert!(text.starts_with(&page.messages[0].text));
    }
}
