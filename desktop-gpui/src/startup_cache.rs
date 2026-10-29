//! Bounded, disposable presentation cache. Never used as backend authority.
use monitter_lib::experimental_desktop::{NativePreferences, NativeWorkspaceSet};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::{BTreeMap, HashMap}, io::Read, path::Path};

// Raw line previews from v1 cannot be rendered as a faithful transcript.
const VERSION: u32 = 2;
const MAX_BYTES: u64 = 2 * 1024 * 1024;
const FILE: &str = "workspace-presentation.json";

#[derive(Clone, Serialize, Deserialize)]
pub struct Presentation {
    version: u32,
    #[serde(default)]
    captured_at: u64,
    pub snapshot: Value,
    pub preferences: NativePreferences,
    pub workspaces: Option<NativeWorkspaceSet>,
    pub layout: Value,
    pub panes: BTreeMap<String, Pane>,
}
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Pane {
    pub title: String,
    pub messages: Vec<Value>,
}
fn short(text: &str, limit: usize) -> String { text.chars().take(limit).collect() }
fn preview_messages<'a>(messages: impl DoubleEndedIterator<Item = &'a Value>) -> Vec<Value> {
    let mut remaining_lines: usize = 100;
    let mut remaining_bytes: usize = 16 * 1024;
    let mut saved = Vec::new();
    for message in messages.rev().take(100) {
        // Tool/system payloads are projected into distinct live UI rows. They
        // must never flash as ordinary text in the startup presentation.
        if !matches!(message["role"].as_str(), Some("user" | "assistant")) { continue; }
        let text = message["text"].as_str().unwrap_or_default();
        if text.len() > remaining_bytes { break; }
        if text.trim().is_empty() { continue; }
        let lines = text.lines().count().max(1);
        // Keep complete message bodies (including markdown fences), rather
        // than clipping arbitrary lines or cutting the start off a code block.
        // The newest message may exceed the line target: keep it whole if it
        // fits the byte budget. Never substitute older content for a newest
        // body that is too large to cache.
        if text.len() > remaining_bytes || (lines > remaining_lines && !saved.is_empty()) { break; }
        let mut row = json!({});
        for key in ["id", "taskId", "role", "text", "createdAt", "phase", "senderAgentId", "collaborationId", "streamStatus"] {
            if let Some(value) = message.get(key) { row[key] = value.clone(); }
        }
        remaining_lines = remaining_lines.saturating_sub(lines);
        remaining_bytes -= text.len();
        saved.push(row);
        if remaining_lines == 0 || remaining_bytes == 0 { break; }
    }
    saved.reverse();
    saved
}
// Shell fields are scalar. Do not clone arbitrary nested payloads under a
// familiar key, and bound individual labels before allocating them.
fn project_fields(row: &Value, keys: &[&str]) -> Value {
    let mut projected = serde_json::Map::new();
    for key in keys {
        if let Some(value) = row.get(*key) {
            let value = match value {
                Value::String(text) => Value::String(short(text, 256)),
                Value::Bool(_) | Value::Number(_) | Value::Null => value.clone(),
                _ => continue,
            };
            projected.insert((*key).into(), value);
        }
    }
    Value::Object(projected)
}
impl Presentation {
    // Private drafts/attachment refs stay out of the disposable presentation.
    // Retained startup editors hydrate from authoritative preferences after
    // unlock, preserving both the saved baseline and any early local typing.
    pub fn shell_preferences(preferences: &NativePreferences) -> NativePreferences {
        let mut shell = NativePreferences::default();
        shell.revision = preferences.revision.clone();
        shell.data.appearance = preferences.data.appearance.clone();
        shell.data.activity_grouping = preferences.data.activity_grouping;
        shell.data.chat_panes = preferences.data.chat_panes.iter().take(64)
            .map(|(id, pane)| (id.clone(), pane.clone())).collect();
        shell
    }
    /// Project typed page rows without serializing attachments or metadata that
    /// the startup presentation will discard.
    pub fn preview_page(messages: &[monitter_lib::model::Message]) -> Vec<Value> {
        let mut rows = Vec::new();
        let mut remaining = 16 * 1024;
        for message in messages.iter().rev().take(100) {
            if !matches!(message.role.as_str(), "user" | "assistant") { continue; }
            if message.text.len() > remaining { break; }
            remaining -= message.text.len();
            rows.push(json!({"id":message.id,"taskId":message.task_id,
                "role":message.role,"text":message.text,"createdAt":message.created_at,
                "phase":message.phase,"senderAgentId":message.sender_agent_id,
                "collaborationId":message.collaboration_id,"streamStatus":message.stream_status}));
        }
        rows.reverse();
        preview_messages(rows.iter())
    }
    pub fn capture(snapshot: &Value, preferences: &NativePreferences, workspaces: Option<NativeWorkspaceSet>, layout: Value, ids: &[String], transcripts: &HashMap<String, Vec<Value>>) -> Self {
        let mut metadata = json!({});
        // Only fields needed to paint the saved shell; no credentials, runtime
        // approvals, events, attachment bodies, or authoritative cursors.
        let setting_keys = [
            "theme", "windowSurface", "windowTransparency", "tabStyle", "accent",
            "interfaceDensity", "interfaceScale", "interfaceFont", "interfaceFontSize",
            "interfaceFontWeight", "chatFont", "chatFontSize", "chatFontWeight",
            "chatLineHeight", "tintUserMessages", "showToolActivity",
            "showReasoningSummaries", "compressToolCalls", "showTabCloseButtons",
            "autoHideTabs", "shortcutMode", "showActivePaneBorder", "activePaneBorderWidth",
            "inactivePaneOpacity", "dimInactivePanes", "focusFollowsMouse", "reduceMotion",
            "ternGutter", "ternPadding", "ternCornerRadius",
        ];
        metadata["settings"] = project_fields(&snapshot["settings"], &setting_keys);
        let row_keys = ["id", "title", "name", "agentId", "projectId", "hostId", "archived", "internal", "status", "createdAt", "updatedAt", "order", "color", "icon"];
        for key in ["agents", "tasks", "projects", "hosts"] {
            // Project borrowed rows before cloning: prompts/configuration and
            // rows beyond the shell limit never enter this allocation.
            metadata[key] = Value::Array(snapshot[key].as_array().into_iter().flatten()
                .take(512).map(|row| project_fields(row, &row_keys)).collect());
        }
        let mut panes = BTreeMap::new();
        for id in ids.iter().take(64) {
            let task_id = id.strip_prefix("chat:");
            let title = task_id.and_then(|id| metadata["tasks"].as_array()?.iter().find(|task| task["id"] == id))
                .and_then(|task| task["title"].as_str()).unwrap_or_else(|| id.split(':').next().unwrap_or("Workspace"));
            let messages = if let Some(messages) = task_id.and_then(|id| transcripts.get(id)) {
                preview_messages(messages.iter())
            } else if let Some(task_id) = task_id {
                let rows = snapshot["messages"].as_array().map(Vec::as_slice).unwrap_or_default();
                let recent = &rows[rows.len().saturating_sub(512)..];
                preview_messages(recent.iter().filter(|message| message["taskId"] == task_id))
            } else { Vec::new() };
            panes.insert(id.clone(), Pane { title: short(title, 256), messages });
        }
        let preferences = Self::shell_preferences(preferences);
        let captured_at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as u64;
        Self { version: VERSION, captured_at, snapshot: metadata, preferences, workspaces, layout, panes }
    }
}
pub fn load() -> Option<Presentation> {
    let path = monitter_lib::experimental_desktop::profile_dir().ok()?.join(FILE);
    read(&path).ok()
}
fn read(path: &Path) -> Result<Presentation, String> {
    let meta = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !meta.is_file() || meta.len() > MAX_BYTES { return Err("Invalid presentation cache size/type".into()); }
    let mut data = Vec::new();
    std::fs::File::open(path).map_err(|e| e.to_string())?.take(MAX_BYTES + 1).read_to_end(&mut data).map_err(|e| e.to_string())?;
    if data.len() as u64 > MAX_BYTES { return Err("Presentation cache too large".into()); }
    let cache: Presentation = serde_json::from_slice(&data).map_err(|e| e.to_string())?;
    if cache.version != VERSION || cache.panes.len() > 64 { return Err("Unsupported presentation cache".into()); }
    Ok(cache)
}
pub fn save(cache: &Presentation) -> Result<(), String> {
    // A delayed transcript refresh must not replace a more recent layout save.
    static WRITER: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _writer = WRITER.lock().map_err(|_| "Presentation cache writer unavailable")?;
    let path = monitter_lib::experimental_desktop::profile_dir()?.join(FILE);
    if read(&path).is_ok_and(|saved| saved.captured_at > cache.captured_at) { return Ok(()); }
    let bytes = serde_json::to_vec(cache).map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_BYTES { return Err("Presentation cache exceeds limit".into()); }
    let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_err(|e| e.to_string())?.as_nanos();
    let temporary = path.with_extension(format!("{}.{nonce}.tmp", std::process::id()));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)] { use std::os::unix::fs::OpenOptionsExt; options.mode(0o600); }
    use std::io::Write;
    let mut file = options.open(&temporary).map_err(|e| e.to_string())?;
    file.write_all(&bytes).map_err(|e| e.to_string())?;
    std::fs::rename(&temporary, &path).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn startup_cache_is_presentation_only_and_bounds_transcript_text() {
        let snapshot = json!({"tasks":[{"id":"t","title":"Title","prompt":"private prompt"}], "agents":[{"id":"a","apiKey":"secret"}], "messages":[{"taskId":"t","role":"assistant","text":"```rust\nlet answer = 42;\n```","attachments":[{"previewDataUrl":"large"}]}], "approvalRequests":[{"payload":"secret"}]});
        let mut preferences = NativePreferences::default();
        preferences.data.drafts.insert("t".into(), "new draft".into());
        let cache = Presentation::capture(&snapshot, &preferences, None, json!({}), &["chat:t".into()], &HashMap::new());
        assert!(cache.snapshot.get("messages").is_none());
        assert!(cache.snapshot["tasks"][0].get("prompt").is_none());
        assert!(cache.snapshot["agents"][0].get("apiKey").is_none());
        assert!(cache.preferences.data.drafts.is_empty());
        assert_eq!(cache.panes["chat:t"].messages[0]["text"], "```rust\nlet answer = 42;\n```");
        assert!(cache.panes["chat:t"].messages[0].get("attachments").is_none());
        assert_eq!(preferences.data.drafts["t"], "new draft");
    }
    #[test]
    fn startup_cache_ignores_bad_versions_and_oversized_files() {
        let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!("monitter-startup-cache-{}-{nonce}.json", std::process::id()));
        std::fs::write(&path, b"not json").unwrap();
        assert!(read(&path).is_err());
        let mut cache = Presentation::capture(&json!({}), &NativePreferences::default(), None, json!({}), &[], &HashMap::new());
        cache.version += 1;
        std::fs::write(&path, serde_json::to_vec(&cache).unwrap()).unwrap();
        assert!(read(&path).is_err());
        std::fs::File::create(&path).unwrap().set_len(MAX_BYTES + 1).unwrap();
        assert!(read(&path).is_err());
        std::fs::remove_file(path).unwrap();
    }
}
