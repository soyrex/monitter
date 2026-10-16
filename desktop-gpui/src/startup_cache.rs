//! Bounded, disposable presentation cache. Never used as backend authority.
use monitter_lib::experimental_desktop::{NativePreferences, NativeWorkspaceSet};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeMap, io::Read, path::Path};

const VERSION: u32 = 1;
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
    pub lines: Vec<String>,
}
fn short(text: &str, limit: usize) -> String { text.chars().take(limit).collect() }
impl Presentation {
    pub fn capture(snapshot: &Value, preferences: &NativePreferences, workspaces: Option<NativeWorkspaceSet>, layout: Value, ids: &[String]) -> Self {
        let mut metadata = json!({});
        // Only fields needed to paint the saved shell; no credentials, runtime
        // approvals, events, attachment bodies, or authoritative cursors.
        for key in ["settings", "agents", "tasks", "projects", "hosts"] {
            if let Some(value) = snapshot.get(key) { metadata[key] = value.clone(); }
        }
        if let Some(settings) = metadata["settings"].as_object_mut() {
            settings.retain(|key, _| matches!(key.as_str(), "theme" | "windowSurface" | "tabStyle" | "accent" | "interfaceDensity" | "interfaceFont" | "interfaceFontSize" | "interfaceFontWeight" | "chatFont" | "chatFontSize" | "chatFontWeight" | "chatLineHeight" | "shortcutMode" | "showActivePaneBorder" | "inactivePaneOpacity" | "dimInactivePanes" | "focusFollowsMouse" | "reduceMotion" | "ternGutter" | "ternPadding" | "ternCornerRadius"));
        }
        // The shell only needs names/identity, not task prompts or agent config.
        for key in ["agents", "tasks", "projects", "hosts"] {
            if let Some(rows) = metadata[key].as_array_mut() {
                rows.truncate(512);
                for row in rows {
                    if let Some(row) = row.as_object_mut() {
                        row.retain(|key, _| matches!(key.as_str(), "id" | "title" | "name" | "agentId" | "projectId" | "hostId" | "archived" | "internal" | "status" | "createdAt" | "updatedAt" | "order" | "color" | "icon"));
                    }
                }
            }
        }
        let mut panes = BTreeMap::new();
        for id in ids.iter().take(64) {
            let task_id = id.strip_prefix("chat:");
            let title = task_id.and_then(|id| snapshot["tasks"].as_array()?.iter().find(|task| task["id"] == id))
                .and_then(|task| task["title"].as_str()).unwrap_or_else(|| id.split(':').next().unwrap_or("Workspace"));
            let mut lines = task_id.map(|id| snapshot["messages"].as_array().into_iter().flatten()
                .filter(|message| message["taskId"] == id)
                .rev().take(8).map(|message| short(message["text"].as_str().or_else(|| message["content"].as_str()).unwrap_or_default(), 2048))
                .collect::<Vec<_>>()).unwrap_or_default();
            lines.reverse();
            panes.insert(id.clone(), Pane { title: short(title, 256), lines });
        }
        let mut preferences = preferences.clone();
        preferences.data.drafts.clear();
        preferences.data.attachment_drafts.clear();
        preferences.data.browser_metadata = None;
        preferences.data.layout = None;
        preferences.data.workspaces = None;
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
        let snapshot = json!({"tasks":[{"id":"t","title":"Title","prompt":"private prompt"}], "agents":[{"id":"a","apiKey":"secret"}], "messages":[{"taskId":"t","text":"x".repeat(9000),"attachments":[{"previewDataUrl":"large"}]}], "approvalRequests":[{"payload":"secret"}]});
        let mut preferences = NativePreferences::default();
        preferences.data.drafts.insert("t".into(), "new draft".into());
        let cache = Presentation::capture(&snapshot, &preferences, None, json!({}), &["chat:t".into()]);
        assert!(cache.snapshot.get("messages").is_none());
        assert!(cache.snapshot["tasks"][0].get("prompt").is_none());
        assert!(cache.snapshot["agents"][0].get("apiKey").is_none());
        assert!(cache.preferences.data.drafts.is_empty());
        assert_eq!(cache.panes["chat:t"].lines[0].len(), 2048);
        assert_eq!(preferences.data.drafts["t"], "new draft");
    }
    #[test]
    fn startup_cache_ignores_bad_versions_and_oversized_files() {
        let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!("monitter-startup-cache-{}-{nonce}.json", std::process::id()));
        std::fs::write(&path, b"not json").unwrap();
        assert!(read(&path).is_err());
        let mut cache = Presentation::capture(&json!({}), &NativePreferences::default(), None, json!({}), &[]);
        cache.version += 1;
        std::fs::write(&path, serde_json::to_vec(&cache).unwrap()).unwrap();
        assert!(read(&path).is_err());
        std::fs::File::create(&path).unwrap().set_len(MAX_BYTES + 1).unwrap();
        assert!(read(&path).is_err());
        std::fs::remove_file(path).unwrap();
    }
}
