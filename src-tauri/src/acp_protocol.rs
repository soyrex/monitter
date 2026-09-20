//! Provider-independent ACP v1 wire helpers. No processes, persistence or UI.
use serde_json::{json, Value};
use std::io::BufRead;

pub const PROTOCOL_VERSION: u64 = 1;
pub const MAX_FRAME_BYTES: usize = 2 * 1024 * 1024;

/// Drain arbitrary stderr lines without allocating unbounded diagnostics. Keep
/// only a small prefix; unlike protocol frames, a partial final line is useful.
pub fn read_diagnostic_line(reader: &mut impl BufRead) -> std::io::Result<Option<String>> {
    const LIMIT: usize = 8192;
    let mut prefix = Vec::new();
    let mut seen = false;
    let mut truncated = false;
    loop {
        let buf = reader.fill_buf()?;
        if buf.is_empty() {
            if !seen {
                return Ok(None);
            }
            break;
        }
        seen = true;
        let end = buf.iter().position(|b| *b == b'\n');
        let count = end.map_or(buf.len(), |n| n + 1);
        let keep = count.min(LIMIT - prefix.len());
        prefix.extend_from_slice(&buf[..keep]);
        truncated |= keep < count;
        reader.consume(count);
        if end.is_some() {
            break;
        }
    }
    let mut text = String::from_utf8_lossy(&prefix)
        .trim_end_matches(['\r', '\n'])
        .to_string();
    if truncated {
        text.push_str(" [truncated]");
    }
    Ok(Some(text))
}

/// JSONL is LF framed. Unicode line separators are ordinary JSON string data.
/// A partial final frame is a transport failure, not an implicit successful turn.
pub fn read_frame(reader: &mut impl BufRead) -> Result<Option<Value>, String> {
    let mut frame = Vec::new();
    loop {
        let buf = reader
            .fill_buf()
            .map_err(|e| format!("ACP read failed: {e}"))?;
        if buf.is_empty() {
            return if frame.is_empty() {
                Ok(None)
            } else {
                Err("ACP closed during a JSON frame.".into())
            };
        }
        let end = buf.iter().position(|b| *b == b'\n');
        let count = end.map_or(buf.len(), |n| n + 1);
        if frame.len() + count > MAX_FRAME_BYTES {
            return Err("ACP frame exceeds the 2 MiB limit.".into());
        }
        frame.extend_from_slice(&buf[..count]);
        reader.consume(count);
        if end.is_some() {
            if frame.iter().all(u8::is_ascii_whitespace) {
                frame.clear();
                continue;
            }
            // Do not include malformed frames in errors: they can contain prompts/secrets.
            let value: Value = serde_json::from_slice(&frame).map_err(|_| {
                "Agent stdout is not valid ACP JSONL. Check its ACP launch arguments.".to_string()
            })?;
            validate_envelope(&value)?;
            return Ok(Some(value));
        }
    }
}

fn valid_id(id: &Value) -> bool {
    id.as_str().is_some_and(|s| s.len() <= 512) || id.as_i64().is_some() || id.as_u64().is_some()
}

pub fn validate_envelope(value: &Value) -> Result<(), String> {
    let Some(obj) = value.as_object() else {
        return Err("ACP requires a JSON-RPC object, not a batch.".into());
    };
    if value["jsonrpc"] != "2.0" {
        return Err("Agent stdout is not JSON-RPC 2.0.".into());
    }
    if let Some(method) = obj.get("method") {
        if !method
            .as_str()
            .is_some_and(|m| !m.is_empty() && m.len() <= 512)
            || obj.contains_key("result")
            || obj.contains_key("error")
            || obj.get("id").is_some_and(|id| !valid_id(id))
            || obj.get("params").is_some_and(|p| !p.is_object())
        {
            return Err("Malformed ACP request or notification.".into());
        }
    } else if !obj.get("id").is_some_and(valid_id)
        || obj.contains_key("result") == obj.contains_key("error")
        || obj
            .get("error")
            .is_some_and(|e| !e["code"].is_i64() || !e["message"].is_string())
    {
        return Err("Malformed ACP response.".into());
    }
    Ok(())
}

pub fn request(id: Value, method: &str, params: Value) -> Value {
    json!({"jsonrpc":"2.0", "id":id, "method":method, "params":params})
}

pub fn notification(method: &str, params: Value) -> Value {
    json!({"jsonrpc":"2.0", "method":method, "params":params})
}

pub fn response(id: Value, result: Value) -> Value {
    json!({"jsonrpc":"2.0", "id":id, "result":result})
}

pub fn error_response(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc":"2.0", "id":id, "error":{"code":code,"message":message}})
}

pub fn initialize_params() -> Value {
    json!({"protocolVersion":PROTOCOL_VERSION,
        "clientInfo":{"name":"monitter","title":"Monitter","version":env!("CARGO_PKG_VERSION")},
        // Advertising `subagents` asks agents that support it (e.g. claude-agent-acp) to
        // announce Task/Agent-tool delegation as `subagent_spawned`/`subagent_state_update`
        // notifications instead of silently folding that activity into the root transcript.
        "clientCapabilities":{"subagents":{},
        // Mcode's namespaced extensions are opt-in. Other ACP agents ignore
        // this opaque metadata, while Mcode uses it to expose its advertised
        // active-turn steering method.
        "_meta":{"minimax-code/extensions":{"version":1,"notifications":true}}}})
}

/// Only explicit capabilities enable optional methods. Null is not support.
#[derive(Debug, Clone)]
pub struct Capabilities {
    pub load_session: bool,
    pub resume_session: bool,
    pub image: bool,
    pub audio: bool,
    pub embedded_context: bool,
}

impl Capabilities {
    pub fn from_initialize(result: &Value) -> Result<Self, String> {
        if result["protocolVersion"].as_u64() != Some(PROTOCOL_VERSION) {
            return Err(
                "This agent selected an unsupported ACP protocol version; Monitter supports v1."
                    .into(),
            );
        }
        let caps = &result["agentCapabilities"];
        Ok(Self {
            load_session: caps["loadSession"] == true,
            resume_session: caps["sessionCapabilities"]["resume"].is_object(),
            image: caps["promptCapabilities"]["image"] == true,
            audio: caps["promptCapabilities"]["audio"] == true,
            embedded_context: caps["promptCapabilities"]["embeddedContext"] == true,
        })
    }

    pub fn recovery_method(&self) -> Result<&'static str, String> {
        if self.resume_session {
            Ok("session/resume")
        } else if self.load_session {
            Ok("session/load")
        } else {
            Err("This ACP agent cannot reload saved sessions. The existing conversation was preserved; start a new chat to continue.".into())
        }
    }
}

/// Preserve opaque option IDs. Never reinterpret a remembered/always decision
/// as one-time authorization. Missing one-time rejection safely cancels instead.
pub fn permission_outcome(params: &Value, approve: bool) -> Result<Value, String> {
    let options = params["options"]
        .as_array()
        .ok_or("ACP permission options are missing.")?;
    if options.len() > 32 {
        return Err("Too many ACP permission options.".into());
    }
    let mut seen = std::collections::HashSet::new();
    for option in options {
        let id = option["optionId"]
            .as_str()
            .filter(|s| !s.is_empty() && s.len() <= 512)
            .ok_or("Invalid ACP permission option ID.")?;
        if !seen.insert(id) {
            return Err("Duplicate ACP permission option IDs.".into());
        }
    }
    let kind = if approve { "allow_once" } else { "reject_once" };
    if let Some(option) = options.iter().find(|o| o["kind"] == kind) {
        return Ok(json!({"outcome":{"outcome":"selected","optionId":option["optionId"]}}));
    }
    if approve {
        Err("This ACP request does not offer one-time approval.".into())
    } else {
        Ok(cancelled_permission())
    }
}

pub fn cancelled_permission() -> Value {
    json!({"outcome":{"outcome":"cancelled"}})
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn diagnostics_are_bounded_and_drain_the_entire_line() {
        let mut bytes = vec![b'x'; MAX_FRAME_BYTES + 1];
        bytes.extend_from_slice(b"\nnext\npartial");
        let mut reader = std::io::BufReader::with_capacity(97, Cursor::new(bytes));
        let first = read_diagnostic_line(&mut reader).unwrap().unwrap();
        assert!(first.len() < 8300 && first.ends_with("[truncated]"));
        assert_eq!(read_diagnostic_line(&mut reader).unwrap().unwrap(), "next");
        assert_eq!(
            read_diagnostic_line(&mut reader).unwrap().unwrap(),
            "partial"
        );
        assert!(read_diagnostic_line(&mut reader).unwrap().is_none());
    }

    #[test]
    fn lf_framing_preserves_unicode_and_crlf() {
        let mut input = Cursor::new("\n{\"jsonrpc\":\"2.0\",\"method\":\"_update\",\"params\":{\"text\":\"a\u{2028}b\u{2029}c\"}}\r\n");
        assert_eq!(
            read_frame(&mut input).unwrap().unwrap()["params"]["text"],
            "a\u{2028}b\u{2029}c"
        );
        assert!(read_frame(&mut input).unwrap().is_none());
    }

    #[test]
    fn partial_malformed_and_oversize_frames_fail_without_echoing_payload() {
        assert!(read_frame(&mut Cursor::new(b"{\"secret\":\"private\"}")).is_err());
        let error = read_frame(&mut Cursor::new(b"private\n")).unwrap_err();
        assert!(!error.contains("private"));
        assert!(read_frame(&mut Cursor::new(vec![b'x'; MAX_FRAME_BYTES + 1])).is_err());
        assert!(read_frame(&mut Cursor::new(b"[]\n")).is_err());
    }

    #[test]
    fn rpc_request_ids_keep_string_and_numeric_identity() {
        for id in [json!(1), json!("1")] {
            let frame = response(id.clone(), json!({}));
            validate_envelope(&frame).unwrap();
            assert_eq!(frame["id"], id);
        }
        assert!(
            validate_envelope(&json!({"jsonrpc":"2.0","id":1,"result":{},"error":{}})).is_err()
        );
        assert!(validate_envelope(&json!({"jsonrpc":"2.0","id":null,"method":"x"})).is_err());
        validate_envelope(&notification("_future_method", json!({"future":true}))).unwrap();
    }

    #[test]
    fn initialization_and_recovery_require_explicit_support() {
        let caps = Capabilities::from_initialize(&json!({"protocolVersion":1})).unwrap();
        assert!(caps.recovery_method().is_err());
        assert!(!caps.image && !caps.audio && !caps.embedded_context);
        assert!(Capabilities::from_initialize(&json!({"protocolVersion":2})).is_err());
        let caps = Capabilities::from_initialize(&json!({"protocolVersion":1,"agentCapabilities":{"loadSession":true,"sessionCapabilities":{"resume":null}}})).unwrap();
        assert_eq!(caps.recovery_method().unwrap(), "session/load");
        let caps = Capabilities::from_initialize(&json!({"protocolVersion":1,"agentCapabilities":{"loadSession":true,"sessionCapabilities":{"resume":{}}}})).unwrap();
        assert_eq!(caps.recovery_method().unwrap(), "session/resume");
        assert_eq!(
            initialize_params()["clientCapabilities"],
            json!({"subagents":{}})
        );
    }

    #[test]
    fn permission_selection_never_escalates_to_always() {
        let params = json!({"options":[{"kind":"allow_always","optionId":"forever"},{"kind":"allow_once","optionId":"opaque-yes"},{"kind":"reject_once","optionId":"opaque-no"}]});
        assert_eq!(
            permission_outcome(&params, true).unwrap()["outcome"]["optionId"],
            "opaque-yes"
        );
        assert_eq!(
            permission_outcome(&params, false).unwrap()["outcome"]["optionId"],
            "opaque-no"
        );
        let only_always = json!({"options":[{"kind":"allow_always","optionId":"yes"},{"kind":"reject_always","optionId":"no"}]});
        assert!(permission_outcome(&only_always, true).is_err());
        assert_eq!(
            permission_outcome(&only_always, false).unwrap(),
            cancelled_permission()
        );
        assert!(permission_outcome(&json!({"options":[{"optionId":"x","kind":"allow_once"},{"optionId":"x","kind":"reject_once"}]}),true).is_err());
    }
}
