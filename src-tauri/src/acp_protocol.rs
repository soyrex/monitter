//! Provider-independent ACP v1 wire helpers. No processes, persistence or UI.
use serde_json::{Value, json};
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
        "clientCapabilities":{"subagents":{},"elicitation":{"form":{}},
        // Mcode's namespaced extensions are opt-in. Other ACP agents ignore
        // this opaque metadata, while Mcode uses it to expose its advertised
        // active-turn steering method.
        "_meta":{"minimax-code/extensions":{"version":1,"notifications":true}}}})
}

// ACP form elicitation is also the native MiniMax ask_user transport. Keep
// provider option IDs in the original schema; the desktop answers with labels.
struct ElicitationQuestion {
    id: String,
    question: String,
    options: Vec<(String, String, String)>,
    other: Option<String>,
    multiple: bool,
}
fn elicitation_questions(schema: &Value) -> Result<Vec<ElicitationQuestion>, String> {
    let unsupported = || "Unsupported ACP question schema.".to_string();
    if schema.get("type").and_then(Value::as_str) != Some("object") {
        return Err(unsupported());
    }
    if schema.as_object().is_none_or(|object| {
        object.keys().any(|key| {
            !matches!(
                key.as_str(),
                "type"
                    | "properties"
                    | "required"
                    | "additionalProperties"
                    | "title"
                    | "description"
                    | "$schema"
            )
        })
    }) || schema
        .get("additionalProperties")
        .is_some_and(|value| !value.is_boolean())
    {
        return Err(unsupported());
    }
    let properties = schema
        .get("properties")
        .and_then(Value::as_object)
        .ok_or_else(unsupported)?;
    if properties.is_empty() || properties.len() > 16 {
        return Err(unsupported());
    }
    if let Some(required) = schema.get("required") {
        let required = required.as_array().ok_or_else(unsupported)?;
        let mut unique = std::collections::HashSet::new();
        for key in required {
            let key = key.as_str().ok_or_else(unsupported)?;
            if !properties.contains_key(key) || !unique.insert(key) {
                return Err(unsupported());
            }
        }
    }
    // Admit only the emitted MiniMax/string-choice subset. Unknown validation
    // keywords cannot be silently ignored while promising a schema-valid reply.
    for field in properties.values() {
        let kind = field
            .get("type")
            .and_then(Value::as_str)
            .ok_or_else(unsupported)?;
        if field.as_object().is_none_or(|object| {
            object.keys().any(|key| match kind {
                "string" => !matches!(
                    key.as_str(),
                    "type" | "title" | "description" | "oneOf" | "enum"
                ),
                "array" => !matches!(key.as_str(), "type" | "title" | "description" | "items"),
                _ => true,
            })
        }) || (field.get("oneOf").is_some() && field.get("enum").is_some())
        {
            return Err(unsupported());
        }
        if kind == "array" {
            let items = field.get("items").ok_or_else(unsupported)?;
            if items.as_object().is_none_or(|object| {
                object.keys().any(|key| {
                    !matches!(
                        key.as_str(),
                        "type" | "title" | "description" | "anyOf" | "oneOf" | "enum"
                    )
                })
            }) || items
                .get("type")
                .is_some_and(|kind| kind.as_str() != Some("string"))
                || ["anyOf", "oneOf", "enum"]
                    .iter()
                    .filter(|key| items.get(**key).is_some())
                    .count()
                    != 1
            {
                return Err(unsupported());
            }
        }
    }
    let mut others = std::collections::HashMap::new();
    for (id, field) in properties {
        if field.get("type").and_then(Value::as_str) != Some("string") {
            continue;
        }
        for base in properties.keys() {
            let prefix = format!("{base}__other");
            if id
                .strip_prefix(&prefix)
                .is_some_and(|tail| tail.chars().all(|c| c == '_'))
            {
                if field.get("oneOf").is_some() || field.get("enum").is_some() {
                    return Err(unsupported());
                }
                if others.insert(base.clone(), id.clone()).is_some() {
                    return Err(unsupported());
                }
            }
        }
    }
    let mut questions = Vec::new();
    for (id, field) in properties {
        if others.values().any(|other| other == id) {
            continue;
        }
        if id.is_empty() || id.len() > 128 {
            return Err(unsupported());
        }
        let multiple = match field.get("type").and_then(Value::as_str) {
            Some("string") => false,
            Some("array") => true,
            _ => return Err(unsupported()),
        };
        let choices = if multiple {
            field.get("items").ok_or_else(unsupported)?
        } else {
            field
        };
        let mut options = Vec::new();
        if let Some(variants) = choices.get("oneOf").or_else(|| choices.get("anyOf")) {
            let variants = variants.as_array().ok_or_else(unsupported)?;
            if variants.is_empty() || variants.len() > 16 {
                return Err(unsupported());
            }
            for choice in variants {
                if choice.as_object().is_none_or(|object| {
                    object.keys().any(|key| {
                        !matches!(key.as_str(), "const" | "title" | "description" | "type")
                    })
                }) || choice
                    .get("type")
                    .is_some_and(|kind| kind.as_str() != Some("string"))
                {
                    return Err(unsupported());
                }
                let key = choice
                    .get("const")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty() && s.len() <= 240)
                    .ok_or_else(unsupported)?;
                let label = choice.get("title").and_then(Value::as_str).unwrap_or(key);
                let description = choice
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                if label.trim().is_empty() || label.len() > 240 || description.len() > 2000 {
                    return Err(unsupported());
                }
                options.push((key.into(), label.into(), description.into()));
            }
        } else if let Some(variants) = choices.get("enum") {
            let variants = variants.as_array().ok_or_else(unsupported)?;
            if variants.is_empty() || variants.len() > 16 {
                return Err(unsupported());
            }
            for choice in variants {
                let key = choice
                    .as_str()
                    .filter(|s| !s.is_empty() && s.len() <= 240)
                    .ok_or_else(unsupported)?;
                options.push((key.into(), key.into(), String::new()));
            }
        } else if multiple {
            return Err(unsupported());
        }
        let mut keys = std::collections::HashSet::new();
        let mut labels = std::collections::HashSet::new();
        if options
            .iter()
            .any(|(key, label, _)| !keys.insert(key) || !labels.insert(label))
        {
            return Err(unsupported());
        }
        let question = field.get("title").and_then(Value::as_str).unwrap_or(id);
        if question.trim().is_empty() || question.len() > 8000 {
            return Err(unsupported());
        }
        questions.push(ElicitationQuestion {
            id: id.clone(),
            question: question.into(),
            options,
            other: others.get(id).cloned(),
            multiple,
        });
    }
    if questions.is_empty() || questions.len() > 8 {
        return Err(unsupported());
    }
    Ok(questions)
}

pub(crate) fn elicitation_input(params: &Value) -> Result<crate::model::InteractionInput, String> {
    if params.get("mode").and_then(Value::as_str) != Some("form")
        || params.to_string().len() > 64 * 1024
    {
        return Err("Unsupported ACP elicitation request.".into());
    }
    let schema = params
        .get("requestedSchema")
        .ok_or("Missing ACP question schema.")?;
    let fields = elicitation_questions(schema)?;
    Ok(crate::model::InteractionInput {
        kind: "questions".into(),
        questions: fields
            .into_iter()
            .map(|field| crate::model::InputQuestion {
                id: field.id,
                header: String::new(),
                question: field.question,
                is_secret: false,
                selection_mode: Some(if field.multiple { "multiple" } else { "single" }.into()),
                options: field
                    .options
                    .into_iter()
                    .map(|(_, label, description)| crate::model::InputOption { label, description })
                    .collect(),
            })
            .collect(),
        schema: Some(schema.clone()),
        url: None,
    })
}

/// Validate before resolve_input persists the canonical response, and convert
/// only at the exact provider request boundary. Cancellation uses action:cancel.
pub(crate) fn elicitation_response(
    input: &crate::model::InteractionInput,
    response: &Value,
) -> Result<Value, String> {
    let fields = elicitation_questions(
        input
            .schema
            .as_ref()
            .ok_or("Missing ACP question schema.")?,
    )?;
    let answers = response
        .get("answers")
        .and_then(Value::as_object)
        .ok_or("Missing question answers.")?;
    if answers.len() != fields.len() {
        return Err("Answer each ACP question exactly once.".into());
    }
    let mut content = serde_json::Map::new();
    for field in fields {
        let values = answers
            .get(&field.id)
            .and_then(|v| v.get("answers"))
            .and_then(Value::as_array)
            .ok_or("Missing ACP question answer.")?;
        let maximum = (field.options.len() + usize::from(field.other.is_some())).max(1);
        if values.is_empty() || values.len() > maximum || (!field.multiple && values.len() != 1) {
            return Err("Invalid ACP answer selection count.".into());
        }
        let mut selected = Vec::new();
        let mut other = None;
        let mut seen = std::collections::HashSet::new();
        for value in values {
            let text = value
                .as_str()
                .filter(|s| !s.trim().is_empty() && s.len() <= 16_384)
                .ok_or("ACP answers must contain text.")?;
            if !seen.insert(text) {
                return Err("Duplicate ACP answer selection.".into());
            }
            if let Some((key, _, _)) = field.options.iter().find(|(_, label, _)| label == text) {
                selected.push(Value::String(key.clone()));
            } else if field.options.is_empty() {
                selected.push(Value::String(text.into()));
            } else if field.other.is_some() && other.is_none() {
                other = Some(Value::String(text.into()));
            } else {
                return Err("Choose an offered option for this question.".into());
            }
        }
        if !selected.is_empty() {
            content.insert(
                field.id,
                if field.multiple {
                    Value::Array(selected)
                } else {
                    selected.remove(0)
                },
            );
        }
        if let Some(text) = other {
            content.insert(field.other.unwrap(), text);
        }
    }
    for key in input
        .schema
        .as_ref()
        .and_then(|schema| schema.get("required"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if !content.contains_key(key.as_str().ok_or("Invalid required ACP field.")?) {
            return Err("The native question requires an offered choice for this field.".into());
        }
    }
    Ok(json!({"action":"accept","content":content}))
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
    fn input_request_acp_minimax_questionnaire_roundtrip_preserves_native_ids_and_multiple() {
        let params = json!({"mode":"form","requestedSchema":{"type":"object","properties":{
            "single":{"type":"string","title":"Pick a route","oneOf":[{"const":"opaque-one","title":"First"},{"const":"opaque-two","title":"Second"}]},
            "single__other":{"type":"string","title":"Pick a route — Other"},
            "many":{"type":"array","title":"Pick features","items":{"anyOf":[{"const":"feature-a","title":"Alpha"},{"const":"feature-b","title":"Beta"}]}}
        }}});
        let input = elicitation_input(&params).unwrap();
        assert_eq!(input.questions.len(), 2);
        assert_eq!(
            input
                .questions
                .iter()
                .find(|q| q.id == "many")
                .unwrap()
                .selection_mode
                .as_deref(),
            Some("multiple")
        );
        let response = json!({"answers":{"single":{"answers":["Custom route"]},"many":{"answers":["Alpha","Beta"]}}});
        assert_eq!(
            elicitation_response(&input, &response).unwrap(),
            json!({"action":"accept","content":{"single__other":"Custom route","many":["feature-a","feature-b"]}})
        );
        assert!(elicitation_response(&input,&json!({"answers":{"single":{"answers":["First","Second"]},"many":{"answers":["Alpha"]}}})).is_err());
        assert!(
            elicitation_response(
                &input,
                &json!({"answers":{"single":{"answers":["First"]},"many":{"answers":["Unknown"]}}})
            )
            .is_err()
        );
    }
    #[test]
    fn input_request_acp_all_options_plus_other_roundtrip() {
        let options = (0..16)
            .map(|n| json!({"const":format!("id-{n}"),"title":format!("Option {n}")}))
            .collect::<Vec<_>>();
        let params = json!({"mode":"form","requestedSchema":{"type":"object","properties":{
            "many":{"type":"array","title":"Pick features","items":{"anyOf":options}},
            "many__other":{"type":"string","title":"Other"}
        }}});
        let input = elicitation_input(&params).unwrap();
        let mut selections = (0..16).map(|n| format!("Option {n}")).collect::<Vec<_>>();
        selections.push("Custom choice".into());
        let response = json!({"answers":{"many":{"answers":selections}}});
        let native = elicitation_response(&input, &response).unwrap();
        assert_eq!(native["content"]["many"].as_array().unwrap().len(), 16);
        assert_eq!(native["content"]["many__other"], "Custom choice");
        let invalid = json!({"answers":{"many":{"answers":["Custom one","Custom two"]}}});
        assert!(elicitation_response(&input, &invalid).is_err());
    }

    #[test]
    fn input_request_acp_declines_unimplemented_constraints_and_enforces_required_other_layout() {
        for field in [
            json!({"type":"string","maxLength":1}),
            json!({"type":"string","pattern":"^a$"}),
            json!({"type":"string","enum":["a"],"oneOf":[{"const":"b"}]}),
            json!({"type":"array","enum":["a"],"items":{"enum":["a"]}}),
            json!({"type":"array","items":{"enum":["a"],"anyOf":[{"const":"b"}]}}),
            json!({"type":"array","maxItems":1,"items":{"enum":["a","b"]}}),
        ] {
            assert!(elicitation_input(&json!({"mode":"form","requestedSchema":{"type":"object","properties":{"q":field}}})).is_err());
        }
        let params = json!({"mode":"form","requestedSchema":{"type":"object","required":["q"],"properties":{
            "q":{"type":"string","oneOf":[{"const":"a","title":"Alpha"}]},
            "q__other":{"type":"string","title":"Other"}
        }}});
        let input = elicitation_input(&params).unwrap();
        assert!(
            elicitation_response(&input, &json!({"answers":{"q":{"answers":["Custom"]}}})).is_err()
        );
        assert_eq!(
            elicitation_response(&input, &json!({"answers":{"q":{"answers":["Alpha"]}}})).unwrap()
                ["content"],
            json!({"q":"a"})
        );
    }

    #[test]
    fn input_request_acp_rejects_ambiguous_unsupported_and_oversized_questionnaires() {
        let mut params = json!({"mode":"form","requestedSchema":{"type":"object","properties":{"q":{"type":"string","oneOf":[{"const":"a","title":"Same"},{"const":"b","title":"Same"}]}}}});
        assert!(elicitation_input(&params).is_err());
        params["requestedSchema"]["properties"]["q"] = json!({"type":"object"});
        assert!(elicitation_input(&params).is_err());
        params["mode"] = json!("url");
        assert!(elicitation_input(&params).is_err());
        params = json!({"mode":"form","requestedSchema":{"type":"object","properties":{"q":{"type":"string","title":"x".repeat(8001)}}}});
        assert!(elicitation_input(&params).is_err());
    }

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
        let mut input = Cursor::new(
            "\n{\"jsonrpc\":\"2.0\",\"method\":\"_update\",\"params\":{\"text\":\"a\u{2028}b\u{2029}c\"}}\r\n",
        );
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
            json!({"subagents":{},"elicitation":{"form":{}},"_meta":{"minimax-code/extensions":{"version":1,"notifications":true}}})
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
