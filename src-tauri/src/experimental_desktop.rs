//! Private client/server adapter for the experimental GPUI desktop.
//! The persistent server alone owns execution and storage; GUI handles only
//! connect over same-user local IPC. Mobile/LAN allowlists remain unchanged.
//! Calls can perform disk or process I/O and must run on a background worker.
use crate::{Service, command_capabilities};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use std::{path::PathBuf, sync::Arc};

pub const APP_ID: &str = "com.monitter.gpui.experimental";
pub use crate::attachments::Attachment;
pub use crate::attachments::ReadAttachmentFile;
pub use crate::scheduler::ScheduleInput;
pub use crate::ui_sync::{
    ChannelMessageChange, ChannelMessagesPage, TaskMessagesPage, UiDelta, UiDeltaResponse,
};

#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ScheduleAction {
    Pause,
    Resume,
    RunNow,
    Delete,
}

/// Redacted private-owner assignment metadata; no configuration or credentials.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExtensionAssignmentKind {
    Server,
    Skill,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionAssignment {
    pub id: String,
    pub name: String,
    pub kind: ExtensionAssignmentKind,
    pub enabled: bool,
    pub assigned: bool,
    pub all_agents: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionAssignments {
    pub revision: String,
    pub entries: Vec<ExtensionAssignment>,
}
fn assignment_projection(
    config: &crate::extensions::ExtensionConfig,
    agent_id: &str,
) -> ExtensionAssignments {
    let entries = config
        .mcp_servers
        .iter()
        .map(|entry| ExtensionAssignment {
            id: entry.id.clone(),
            name: entry.name.clone(),
            kind: ExtensionAssignmentKind::Server,
            enabled: entry.enabled,
            assigned: entry.agent_ids.iter().any(|id| id == agent_id),
            all_agents: false,
        })
        .chain(config.skills.iter().map(|entry| ExtensionAssignment {
            id: entry.id.clone(),
            name: entry.name.clone(),
            kind: ExtensionAssignmentKind::Skill,
            enabled: entry.enabled,
            assigned: entry.all_agents || entry.agent_ids.iter().any(|id| id == agent_id),
            all_agents: entry.all_agents,
        }))
        .collect();
    ExtensionAssignments {
        revision: config.revision.clone().unwrap_or_default(),
        entries,
    }
}

const NATIVE_PREFERENCES_FILE: &str = "native-ui.json";
const MAX_NATIVE_PREFERENCE_BYTES: usize = 512 * 1024;
const MAX_NATIVE_BROWSER_TABS: usize = 64;
const MAX_NATIVE_BROWSER_URL_BYTES: usize = 8192;
const MAX_NATIVE_BROWSER_TITLE_BYTES: usize = 512;
const MAX_NATIVE_BROWSER_METADATA_BYTES: usize = 256 * 1024;
/// Native-only tile layout; independent of light/dark palettes and workspace settings.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NativeTileTheme {
    #[default]
    Monitter,
    Tern,
}

fn is_default_tile_theme(theme: &NativeTileTheme) -> bool { *theme == NativeTileTheme::Monitter }

/// No mail/browser bodies, credentials, harness config or workspace snapshots.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NativeAppearance {
    #[serde(default, skip_serializing_if = "is_default_tile_theme")]
    pub tile_theme: NativeTileTheme,
    #[serde(default)]
    pub sidebar_width: Option<f32>,
    #[serde(default)]
    pub warm_paper: bool,
    #[serde(default)]
    pub last_route: Option<String>,
    #[serde(default = "default_light_palette")]
    pub light_palette: String,
    #[serde(default = "default_dark_palette")]
    pub dark_palette: String,
    #[serde(default)]
    pub custom_accent: Option<String>,
    #[serde(default)]
    pub contrast: u16,
    #[serde(default = "default_surface_tint")]
    pub surface_tint: u16,
    #[serde(default = "default_border_opacity")]
    pub border_opacity: u16,
    #[serde(default = "default_interface_scale")]
    pub interface_scale: u16,
}
fn default_light_palette() -> String {
    "monitter".into()
}
fn default_dark_palette() -> String {
    "monitter".into()
}
fn default_surface_tint() -> u16 {
    5
}
fn default_border_opacity() -> u16 {
    100
}
fn default_interface_scale() -> u16 {
    125
}
impl Default for NativeAppearance {
    fn default() -> Self {
        Self {
            tile_theme: NativeTileTheme::default(),
            sidebar_width: None,
            warm_paper: false,
            last_route: None,
            light_palette: default_light_palette(),
            dark_palette: default_dark_palette(),
            custom_accent: None,
            contrast: 0,
            surface_tint: default_surface_tint(),
            border_opacity: default_border_opacity(),
            interface_scale: default_interface_scale(),
        }
    }
}
#[derive(Clone, Default, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NativePreferenceData {
    /// GPUI DockAreaState schema; validation admits identity/layout fields only.
    #[serde(default)]
    pub layout: Option<Value>,
    /// Named owner-local layouts; domain tasks and sessions remain global.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspaces: Option<NativeWorkspaceSet>,
    #[serde(default)]
    pub appearance: NativeAppearance,
    /// Explicitly saved chat composer drafts, keyed by task UUID. Never filled
    /// from message bodies, mail details, browser state or credential fields.
    #[serde(default)]
    pub drafts: std::collections::BTreeMap<String, String>,
    /// Per-task native chat sidebar state. Empty maps are omitted so older
    /// preference readers continue to accept the serialized payload.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub chat_panes: std::collections::BTreeMap<String, NativeChatPanePreferences>,
    /// Task-keyed unsent attachment references. This is local composer
    /// association metadata, not backend ownership authorization. Values hold
    /// display metadata and captured destination only, never paths, bytes, or
    /// previews.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub attachment_drafts: std::collections::BTreeMap<String, NativeAttachmentDraft>,
    /// Optional, bounded safe UI metadata for the isolated native browser.
    /// Never includes page bodies, cookies, browser history, or credentials.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub browser_metadata: Option<NativeBrowserMetadata>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NativeWorkspace {
    pub id: String,
    pub name: String,
    pub color: String,
    pub icon: String,
    #[serde(default)]
    pub layout: Option<Value>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NativeWorkspaceSet {
    pub active_id: String,
    pub items: Vec<NativeWorkspace>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NativeBrowserMetadata {
    pub tabs: Vec<NativeBrowserTab>,
    pub selected_id: Option<u64>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NativeBrowserTab {
    pub id: u64,
    pub requested_url: String,
    pub actual_url: Option<String>,
    pub title: String,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NativeAttachmentDraft {
    pub host_id: String,
    pub cwd: String,
    #[serde(default)]
    pub attachments: Vec<NativeAttachmentRef>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NativeAttachmentRef {
    pub id: String,
    pub name: String,
    pub mime_type: String,
    pub size: u64,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NativeChatPanePreferences {
    pub visible: bool,
    pub tab: NativeDetailTab,
    /// Width in design pixels.
    pub width: f32,
}
impl Default for NativeChatPanePreferences {
    fn default() -> Self {
        Self {
            visible: true,
            tab: NativeDetailTab::Details,
            width: 292.0,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NativeDetailTab {
    Details,
    Timeline,
    Approvals,
    Subagents,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NativePreferences {
    pub revision: String,
    pub data: NativePreferenceData,
}
impl Default for NativePreferences {
    fn default() -> Self {
        Self {
            revision: "0".into(),
            data: NativePreferenceData::default(),
        }
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PreferenceLayout {
    version: Option<usize>,
    center: PreferencePane,
    left_dock: Option<PreferenceDock>,
    right_dock: Option<PreferenceDock>,
    bottom_dock: Option<PreferenceDock>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PreferenceDock {
    panel: PreferencePane,
    placement: String,
    size: f32,
    open: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PreferencePane {
    panel_name: String,
    children: Vec<PreferencePane>,
    info: PreferencePaneInfo,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
enum PreferencePaneInfo {
    #[serde(rename = "stack")]
    Stack { sizes: Vec<f32>, axis: usize },
    #[serde(rename = "tabs")]
    Tabs { active_index: usize },
    #[serde(rename = "panel")]
    Panel(PreferencePaneIdentity),
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PreferencePaneIdentity {
    id: String,
}
fn validate_native_data(data: &NativePreferenceData) -> Result<(), String> {
    const PALETTES: &[&str] = &[
        "monitter",
        "catppuccin",
        "dracula",
        "gruvbox",
        "nord",
        "one",
        "solarized",
        "tokyo-night",
        "wombat",
        "xterm",
    ];
    let appearance = &data.appearance;
    if !PALETTES.contains(&appearance.light_palette.as_str())
        || !PALETTES.contains(&appearance.dark_palette.as_str())
        || appearance.contrast > 100
        || appearance.surface_tint > 50
        || appearance.border_opacity > 100
        || !(80..=200).contains(&appearance.interface_scale)
        || appearance.interface_scale % 5 != 0
        || appearance.custom_accent.as_ref().is_some_and(|accent| {
            accent.len() != 7
                || !accent.starts_with('#')
                || !accent[1..].bytes().all(|byte| byte.is_ascii_hexdigit())
        })
    {
        return Err("Invalid native appearance preference.".into());
    }
    if data
        .appearance
        .sidebar_width
        .is_some_and(|width| !width.is_finite() || !(160.0..=1000.0).contains(&width))
    {
        return Err("Invalid native sidebar width.".into());
    }
    if data.appearance.last_route.as_deref().is_some_and(|route| {
        ![
            "chats",
            "agents",
            "projects",
            "browser",
            "settings",
            "mail",
            "channels",
            "terminal",
            "usage",
            "collaboration",
            "schedules",
            "git",
            "archives",
        ]
        .contains(&route)
    }) {
        return Err("Invalid native route.".into());
    }
    if data.drafts.len() > 64
        || data
            .drafts
            .iter()
            .any(|(id, text)| uuid::Uuid::parse_str(id).is_err() || text.len() > 32 * 1024)
    {
        return Err("Native chat drafts exceed their limits or have invalid task IDs.".into());
    }
    if data.chat_panes.len() > 512
        || data.chat_panes.iter().any(|(id, pane)| {
            uuid::Uuid::parse_str(id).is_err()
                || !pane.width.is_finite()
                || !(260.0..=1600.0).contains(&pane.width)
        })
    {
        return Err("Native chat pane preferences exceed their limits or are invalid.".into());
    }
    let mut owned_attachment_ids = std::collections::HashSet::new();
    if data.attachment_drafts.len() > 64
        || data.attachment_drafts.iter().any(|(task_id, draft)| {
            uuid::Uuid::parse_str(task_id).is_err()
                || uuid::Uuid::parse_str(&draft.host_id).is_err()
                || draft.cwd.is_empty()
                || draft.cwd.len() > 4096
                || draft.cwd.contains('\0')
                || draft.attachments.iter().any(|attachment| {
                    let safe_name = !attachment.name.is_empty()
                        && attachment.name.len() <= 120
                        && attachment.name != "."
                        && attachment.name != ".."
                        && attachment.name.chars().all(|c| {
                            c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | ' ')
                        });
                    uuid::Uuid::parse_str(&attachment.id).is_err()
                        || !owned_attachment_ids.insert(attachment.id.as_str())
                        || !safe_name
                        || attachment.mime_type.len() > 256
                        || attachment.mime_type.chars().any(char::is_control)
                        || attachment.size > crate::attachments::MAX_BYTES as u64
                })
        })
    {
        return Err(
            "Native chat attachment drafts exceed their limits or have invalid task/scope metadata."
                .into(),
        );
    }
    if let Some(metadata) = &data.browser_metadata {
        let mut ids = std::collections::HashSet::with_capacity(
            metadata.tabs.len().min(MAX_NATIVE_BROWSER_TABS),
        );
        let selection_is_valid = match (metadata.tabs.is_empty(), metadata.selected_id) {
            (true, None) => true,
            (false, Some(selected_id)) => metadata.tabs.iter().any(|tab| tab.id == selected_id),
            _ => false,
        };
        let mut total_bytes = 0usize;
        if metadata.tabs.len() > MAX_NATIVE_BROWSER_TABS
            || !selection_is_valid
            || metadata.tabs.iter().any(|tab| {
                if tab.id == 0 || !ids.insert(tab.id) {
                    return true;
                }
                if tab.title.len() > MAX_NATIVE_BROWSER_TITLE_BYTES
                    || tab.title.chars().any(char::is_control)
                    || tab.requested_url.len() > MAX_NATIVE_BROWSER_URL_BYTES
                    || tab
                        .actual_url
                        .as_ref()
                        .is_some_and(|url| url.len() > MAX_NATIVE_BROWSER_URL_BYTES)
                    || validate_native_browser_url(&tab.requested_url).is_err()
                    || tab
                        .actual_url
                        .as_deref()
                        .is_some_and(|url| validate_native_browser_url(url).is_err())
                {
                    return true;
                }
                total_bytes = total_bytes
                    .checked_add(tab.requested_url.len())
                    .and_then(|size| {
                        size.checked_add(tab.actual_url.as_ref().map_or(0, String::len))
                    })
                    .and_then(|size| size.checked_add(tab.title.len()))
                    .unwrap_or(usize::MAX);
                total_bytes > MAX_NATIVE_BROWSER_METADATA_BYTES
            })
        {
            return Err("Native browser metadata exceeds its limits or is invalid.".into());
        }
    }
    if let Some(workspaces) = &data.workspaces { validate_native_workspace_set(workspaces)?; }
    if let Some(layout) = &data.layout {
        validate_native_layout(layout)?;
    }
    Ok(())
}

/// Validate client-local workspace metadata and layouts without changing backend state.
pub fn validate_native_workspace_set(workspaces: &NativeWorkspaceSet) -> Result<(), String> {
        let mut ids = std::collections::HashSet::new();
        if workspaces.items.is_empty() || workspaces.items.len() > 24 {
            return Err("Native workspaces must contain between 1 and 24 items.".into());
        }
        for workspace in &workspaces.items {
            let id = uuid::Uuid::parse_str(&workspace.id)
                .map_err(|_| "Invalid native workspace ID.")?;
            if !ids.insert(id)
                || workspace.name.trim().is_empty()
                || workspace.name.chars().count() > 80
                || workspace.name.chars().any(char::is_control)
                || workspace.color.len() != 7
                || !workspace.color.starts_with('#')
                || !workspace.color[1..].bytes().all(|byte| byte.is_ascii_hexdigit())
                || !["folder", "code", "terminal", "globe", "sparkles"].contains(&workspace.icon.as_str())
            {
                return Err("Invalid native workspace metadata or duplicate ID.".into());
            }
            if let Some(layout) = &workspace.layout {
                validate_native_layout(layout)?;
            }
        }
        if !workspaces.items.iter().any(|item| item.id == workspaces.active_id) {
            return Err("Native active workspace does not exist.".into());
        }
    Ok(())
}

fn validate_native_layout(layout: &Value) -> Result<(), String> {
    let layout: PreferenceLayout =
        serde_json::from_value(layout.clone()).map_err(|_| "Invalid native layout schema.")?;
    if layout.version != Some(1) {
        return Err("Unsupported native layout version.".into());
    }
    let mut count = 0;
    let mut ids = std::collections::HashSet::new();
    validate_preference_pane(&layout.center, 0, &mut count, &mut ids)?;
    for dock in [layout.left_dock, layout.right_dock, layout.bottom_dock]
        .into_iter()
        .flatten()
    {
        if !["left", "right", "bottom"].contains(&dock.placement.as_str())
            || !dock.size.is_finite()
            || !(0.0..=100000.0).contains(&dock.size)
        {
            return Err("Invalid native dock placement/size.".into());
        }
        let _ = dock.open;
        validate_preference_pane(&dock.panel, 0, &mut count, &mut ids)?;
    }
    Ok(())
}

fn validate_native_browser_url(value: &str) -> Result<(), String> {
    let url = tauri::Url::parse(value.trim())
        .map_err(|_| "Native browser metadata contains an invalid URL.".to_owned())?;
    if url.as_str() == "about:blank" {
        return Ok(());
    }
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("Native browser metadata URL is not allowed.".into());
    }
    Ok(())
}
fn validate_preference_pane(
    pane: &PreferencePane,
    depth: usize,
    count: &mut usize,
    ids: &mut std::collections::HashSet<String>,
) -> Result<(), String> {
    *count += 1;
    if depth > 32 || *count > 256 {
        return Err("Native layout exceeds its limits.".into());
    }
    match &pane.info {
        PreferencePaneInfo::Panel(identity) => {
            if pane.panel_name != "MonitterNativePane"
                || !pane.children.is_empty()
                || identity.id.is_empty()
                || identity.id.len() > 128
                || !identity
                    .id
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"-_:./".contains(&byte))
                || !ids.insert(identity.id.clone())
            {
                return Err("Invalid/duplicate native pane identity.".into());
            }
        }
        PreferencePaneInfo::Stack { sizes, axis } => {
            if pane.panel_name != "StackPanel"
                || *axis > 1
                || sizes.len() > 64
                || sizes
                    .iter()
                    .any(|size| !size.is_finite() || !(0.0..=100000.0).contains(size))
            {
                return Err("Invalid native split layout.".into());
            }
        }
        PreferencePaneInfo::Tabs { active_index } => {
            if pane.panel_name != "TabPanel" || *active_index > 64 {
                return Err("Invalid native tab layout.".into());
            }
        }
    }
    for child in &pane.children {
        validate_preference_pane(child, depth + 1, count, ids)?;
    }
    Ok(())
}
struct NativePreferenceStore {
    dir: PathBuf,
    writes: std::sync::Mutex<()>,
}
impl NativePreferenceStore {
    fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            writes: std::sync::Mutex::new(()),
        }
    }
    fn load(&self) -> Result<NativePreferences, String> {
        let _guard = self
            .writes
            .lock()
            .map_err(|_| "Native preference lock failed.")?;
        native_preferences_read(&self.dir)
    }
    fn save(
        &self,
        revision: &str,
        data: NativePreferenceData,
    ) -> Result<NativePreferences, String> {
        let _guard = self
            .writes
            .lock()
            .map_err(|_| "Native preference lock failed.")?;
        validate_native_data(&data)?;
        let previous = native_preferences_read(&self.dir)?;
        if previous.revision != revision {
            return Err(
                "Native preferences changed; reload before saving. Draft was not overwritten."
                    .into(),
            );
        }
        let next = NativePreferences {
            revision: uuid::Uuid::new_v4().to_string(),
            data,
        };
        let bytes = serde_json::to_vec(&next).map_err(|error| error.to_string())?;
        if bytes.len() > MAX_NATIVE_PREFERENCE_BYTES {
            return Err("Native preferences exceed 512 KiB.".into());
        }
        native_preferences_write(&self.dir, &bytes)?;
        Ok(next)
    }
}
#[cfg(unix)]
fn preference_directory(dir: &std::path::Path) -> Result<std::fs::File, String> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    // O_NOFOLLOW anchors the profile directory itself; subsequent operations
    // use openat/renameat against this retained FD, avoiding path-swap races.
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
        .open(dir)
        .map_err(|error| error.to_string())?;
    file.set_permissions(std::fs::Permissions::from_mode(0o700))
        .map_err(|error| error.to_string())?;
    Ok(file)
}
#[cfg(unix)]
fn preference_open(
    dir: &std::fs::File,
    name: &std::ffi::CStr,
    flags: i32,
) -> std::io::Result<std::fs::File> {
    use std::os::fd::{AsRawFd, FromRawFd};
    // SAFETY: live directory FD, nul-terminated name, newly owned resulting FD.
    let fd = unsafe {
        libc::openat(
            dir.as_raw_fd(),
            name.as_ptr(),
            flags | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            0o600,
        )
    };
    if fd < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(unsafe { std::fs::File::from_raw_fd(fd) })
    }
}
#[cfg(unix)]
fn native_preferences_read(dir: &std::path::Path) -> Result<NativePreferences, String> {
    use std::io::Read;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let directory = preference_directory(dir)?;
    let name = std::ffi::CString::new(NATIVE_PREFERENCES_FILE).unwrap();
    let file = match preference_open(&directory, &name, libc::O_RDONLY | libc::O_NONBLOCK) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(NativePreferences::default());
        }
        Err(error) => return Err(error.to_string()),
    };
    let metadata = file.metadata().map_err(|error| error.to_string())?;
    if !metadata.is_file()
        || metadata.nlink() != 1
        || metadata.len() > MAX_NATIVE_PREFERENCE_BYTES as u64
    {
        return Err("Unsafe or oversized native preferences file.".into());
    }
    file.set_permissions(std::fs::Permissions::from_mode(0o600))
        .map_err(|error| error.to_string())?;
    let mut bytes = Vec::new();
    file.take(MAX_NATIVE_PREFERENCE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() > MAX_NATIVE_PREFERENCE_BYTES {
        return Err("Native preferences exceed 512 KiB.".into());
    }
    let prefs: NativePreferences =
        serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
    if uuid::Uuid::parse_str(&prefs.revision).is_err() {
        return Err("Invalid native preference revision.".into());
    }
    validate_native_data(&prefs.data)?;
    Ok(prefs)
}
#[cfg(unix)]
fn native_preferences_write(dir: &std::path::Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    use std::os::fd::AsRawFd;
    let directory = preference_directory(dir)?;
    let temp = std::ffi::CString::new(format!(".native-ui-{}.tmp", uuid::Uuid::new_v4())).unwrap();
    let destination = std::ffi::CString::new(NATIVE_PREFERENCES_FILE).unwrap();
    let result = (|| {
        let mut file = preference_open(
            &directory,
            &temp,
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL,
        )
        .map_err(|error| error.to_string())?;
        file.write_all(bytes)
            .and_then(|_| file.sync_all())
            .map_err(|error| error.to_string())?;
        // SAFETY: same retained directory and valid nul-terminated basenames.
        if unsafe {
            libc::renameat(
                directory.as_raw_fd(),
                temp.as_ptr(),
                directory.as_raw_fd(),
                destination.as_ptr(),
            )
        } != 0
        {
            return Err(std::io::Error::last_os_error().to_string());
        }
        directory.sync_all().map_err(|error| error.to_string())
    })();
    if result.is_err() {
        unsafe { libc::unlinkat(directory.as_raw_fd(), temp.as_ptr(), 0) };
    }
    result
}
#[cfg(not(unix))]
fn native_preferences_read(_: &std::path::Path) -> Result<NativePreferences, String> {
    Err("Native preferences require the supported Unix desktop.".into())
}
#[cfg(not(unix))]
fn native_preferences_write(_: &std::path::Path, _: &[u8]) -> Result<(), String> {
    Err("Native preferences require the supported Unix desktop.".into())
}

const LOCAL: &[&str] = &[
    "get_command_capabilities",
    "list_system_fonts",
    "choose_local_folder",
    "plan_jev_route",
    "record_jev_route",
    "plan_jev_command",
    "mail_get_config",
    "mail_configure_gmail",
    "mail_start_connect",
    "mail_connect_status",
    "mail_cancel_connect",
    "mail_set_account_agents",
    "mail_disconnect",
    "refresh_mail_inbox",
    "request_mail_detail",
    "get_mail_detail",
    "summarize_mail_thread",
    // Owner-local settings. These are advertised only on the authenticated
    // GPUI IPC path; they are intentionally not added to Service::lan_invoke.
    "get_extension_config",
    "save_extension_config",
    "list_environment_secrets",
    "set_environment_secret",
    "delete_environment_secret",
];

/// Owner-only runtime status. The access code never enters a Snapshot or LAN command.
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeLanStatus {
    pub running: bool,
    pub info: crate::lan::Info,
}

fn lan_asset_roots(
    profile: &std::path::Path,
    executable: &std::path::Path,
) -> Result<Vec<PathBuf>, String> {
    validate_profile(profile)?;
    let mut candidates = vec![profile.join("lan-web")];
    if let Some(macos) = executable
        .parent()
        .filter(|p| p.file_name().is_some_and(|name| name == "MacOS"))
    {
        if let Some(contents) = macos
            .parent()
            .filter(|p| p.file_name().is_some_and(|name| name == "Contents"))
        {
            candidates.push(contents.join("Resources/lan-web"));
        }
    }
    let mut roots = Vec::new();
    for candidate in candidates {
        let Ok(metadata) = std::fs::symlink_metadata(&candidate) else {
            continue;
        };
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err("LAN assets must be a real directory, not a link.".into());
        }
        let entry = candidate.join("index.html");
        let Ok(metadata) = std::fs::symlink_metadata(&entry) else {
            continue;
        };
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err("LAN entry page must be a real file, not a link.".into());
        }
        roots.push(candidate);
    }
    if roots.is_empty() {
        return Err("Remote-client assets are missing. Publish lan-web into the experimental profile or package Resources/lan-web before starting LAN access.".into());
    }
    Ok(roots)
}

struct Session {
    lan_control: std::sync::Mutex<()>,
    preferences: NativePreferenceStore,
    service: Arc<Service>,
}

impl Drop for Session {
    fn drop(&mut self) {
        self.service.cleanup();
    }
}

/// Cloneable handle to exactly one isolated service; clones do not reopen data.
#[derive(Clone)]
struct BackendService(Arc<Session>);

impl BackendService {
    /// Owner-native picker input only. Never exposed through LAN/mobile dispatch.
    /// Call off the UI thread for a path explicitly selected by the user.
    pub fn read_attachment_file(&self, source_path: &str) -> Result<ReadAttachmentFile, String> {
        self.ensure_running()?;
        crate::attachments::read_attachment_file(source_path)
    }

    /// Capture/check the picker destination under the same state lock used to
    /// resolve storage. Later task moves cannot redirect this file transfer.
    pub fn store_selected_attachment(
        &self,
        task_id: &str,
        expected_host_id: &str,
        expected_cwd: &str,
        file: ReadAttachmentFile,
        preview_data_url: Option<String>,
    ) -> Result<Attachment, String> {
        self.ensure_running()?;
        self.0.service.store_attachment_checked(
            crate::attachments::AttachmentTarget {
                task_id: Some(task_id.to_owned()),
                agent_id: None,
                project_id: None,
            },
            file.filename,
            file.mime_type,
            file.data_base64,
            preview_data_url,
            None,
            Some((expected_host_id, expected_cwd)),
        )
    }

    /// Explicit owner-native listener controls; never dispatched via LAN/relay.
    /// All three methods can perform I/O and belong on a background worker.
    pub fn read_lan_status(&self) -> Result<NativeLanStatus, String> {
        let _control = self
            .0
            .lan_control
            .lock()
            .map_err(|_| "LAN control lock failed.")?;
        self.lan_status_locked()
    }
    fn lan_status_locked(&self) -> Result<NativeLanStatus, String> {
        let running = self
            .0
            .service
            .lan
            .lock()
            .map_err(|_| "LAN server lock failed.")?
            .is_some();
        let mut info = self.0.service.lan_info();
        info.access_code_required = true;
        if !running && info.error.as_deref() == Some("LAN server is unavailable.") {
            info.error = None;
        }
        Ok(NativeLanStatus { running, info })
    }
    pub fn start_lan(&self) -> Result<NativeLanStatus, String> {
        let _control = self
            .0
            .lan_control
            .lock()
            .map_err(|_| "LAN control lock failed.")?;
        if self
            .0
            .service
            .stopping
            .load(std::sync::atomic::Ordering::Acquire)
        {
            return Err("The experimental desktop is shutting down.".into());
        }
        let mut slot = self
            .0
            .service
            .lan
            .lock()
            .map_err(|_| "LAN server lock failed.")?;
        if slot.is_none() {
            let result = (|| {
                let executable = std::env::current_exe().map_err(|e| e.to_string())?;
                let roots = lan_asset_roots(&self.0.preferences.dir, &executable)?;
                crate::lan::Server::start_authenticated(
                    Arc::clone(&self.0.service),
                    roots,
                    self.0.preferences.dir.join("lan-tls.json"),
                )
            })();
            match result {
                Ok(server) => {
                    if self
                        .0
                        .service
                        .stopping
                        .load(std::sync::atomic::Ordering::Acquire)
                    {
                        drop(server);
                        return Err("The experimental desktop is shutting down.".into());
                    }
                    *slot = Some(server);
                    *self
                        .0
                        .service
                        .lan_error
                        .lock()
                        .map_err(|_| "LAN status lock failed.")? = None;
                }
                Err(error) => {
                    *self
                        .0
                        .service
                        .lan_error
                        .lock()
                        .map_err(|_| "LAN status lock failed.")? = Some(error.clone());
                    return Err(error);
                }
            }
        }
        drop(slot);
        self.lan_status_locked()
    }
    pub fn stop_lan(&self) -> Result<NativeLanStatus, String> {
        let _control = self
            .0
            .lan_control
            .lock()
            .map_err(|_| "LAN control lock failed.")?;
        self.0
            .service
            .lan
            .lock()
            .map_err(|_| "LAN server lock failed.")?
            .take();
        *self
            .0
            .service
            .lan_error
            .lock()
            .map_err(|_| "LAN status lock failed.")? = None;
        self.lan_status_locked()
    }

    /// Open a fixed experimental profile. There is deliberately no arbitrary
    /// data-directory argument. History imports use the separately bounded,
    /// owner-local record method, never open another profile through Service.
    pub fn open() -> Result<Self, String> {
        #[cfg(not(target_os = "macos"))]
        return Err("The experimental GPUI service currently supports macOS only.".into());

        #[cfg(target_os = "macos")]
        {
            let dir = profile_dir()?;
            validate_profile(&dir)?;
            let preferences = NativePreferenceStore::new(dir.clone());
            let service = Service::open(None, dir)?;
            if let Err(error) = service.initialize_collaboration() {
                service.cleanup();
                return Err(error);
            }
            service.dispatch_startup_queues();
            service.start_idle_collector();
            service.start_scheduler();
            let backend = Self(Arc::new(Session {
                lan_control: std::sync::Mutex::new(()),
                service,
                preferences,
            }));
            // Serve browsers on LAN and Tailscale whenever the backend starts.
            // start_lan retains errors for the owner UI; web startup failure must
            // not prevent local agents or private desktop IPC from starting.
            let _ = backend.start_lan();
            Ok(backend)
        }
    }

    /// Private native layout/preferences only. Run these filesystem calls off UI.
    pub fn load_native_preferences(&self) -> Result<NativePreferences, String> {
        self.ensure_running()?;
        self.0.preferences.load()
    }
    pub(crate) fn import_project_history(
        &self,
        input: crate::gpui_history_import::GpuiHistoryImport,
    ) -> Result<crate::gpui_history_import::GpuiHistoryImportReport, String> {
        self.ensure_running()?;
        self.0.service.import_gpui_history(input)
    }
    pub(crate) fn validate_project_history(
        &self,
        input: crate::gpui_history_import::GpuiHistoryImport,
    ) -> Result<(), String> {
        self.ensure_running()?;
        self.0.service.validate_gpui_history(input)
    }
    pub fn save_native_preferences(
        &self,
        expected_revision: &str,
        data: NativePreferenceData,
    ) -> Result<NativePreferences, String> {
        if self
            .0
            .service
            .stopping
            .load(std::sync::atomic::Ordering::Acquire)
        {
            return Err("The experimental desktop is shutting down.".into());
        }
        self.0.preferences.save(expected_revision, data)
    }
    pub fn shutdown(&self) {
        self.0.service.cleanup();
    }

    pub fn save_schedule(&self, input: ScheduleInput) -> Result<crate::model::Snapshot, String> {
        self.ensure_running()?;
        self.0.service.save_schedule(input)
    }

    pub fn schedule_action(
        &self,
        id: String,
        action: ScheduleAction,
    ) -> Result<crate::model::Snapshot, String> {
        self.ensure_running()?;
        match action {
            ScheduleAction::Pause => self.0.service.set_schedule_enabled(id, false),
            ScheduleAction::Resume => self.0.service.set_schedule_enabled(id, true),
            ScheduleAction::RunNow => self.0.service.run_schedule_now(id),
            ScheduleAction::Delete => self.0.service.delete_schedule(id),
        }
    }

    fn ensure_running(&self) -> Result<(), String> {
        if self
            .0
            .service
            .stopping
            .load(std::sync::atomic::Ordering::Acquire)
        {
            Err("The experimental desktop is shutting down.".into())
        } else {
            Ok(())
        }
    }

    /// Typed native state transfer; no JSON serialization or renderer IPC hop.
    /// The journal and committed revision remain owned by the existing service.
    pub fn read_ui_changes(&self, revision: Option<&str>) -> Result<UiDeltaResponse, String> {
        self.ensure_running()?;
        self.0.service.ui_delta(revision)
    }

    /// Read-only local diagnostics for the persistent server's idle collector.
    /// Deliberately not part of the shared/mobile command allowlist.
    pub fn read_idle_runtime_status(&self) -> Result<Value, String> {
        self.ensure_running()?;
        self.0.service.idle_runtime_status()
    }

    pub fn read_task_messages(
        &self,
        task_id: &str,
        before_id: Option<&str>,
        limit: Option<u32>,
    ) -> Result<TaskMessagesPage, String> {
        self.ensure_running()?;
        self.0.service.task_messages(task_id, before_id, limit)
    }

    pub fn read_channel_messages(
        &self,
        channel_id: &str,
        before_id: Option<&str>,
        limit: Option<u32>,
    ) -> Result<ChannelMessagesPage, String> {
        self.ensure_running()?;
        self.0
            .service
            .channel_messages(channel_id, before_id, limit)
    }

    /// Wake a native view after committed state changes. A single pending wake
    /// coalesces bursts; the revisioned snapshot remains authoritative.
    pub fn subscribe_changes(&self) -> Result<async_channel::Receiver<()>, String> {
        self.ensure_running()?;
        let (sender, receiver) = async_channel::bounded(1);
        self.0
            .service
            .native_subscribers
            .lock()
            .map_err(|_| "Native state subscription lock failed.")?
            .push(sender);
        Ok(receiver)
    }

    /// Native PTY input is binary (for example legacy terminal mouse reports).
    /// This typed in-process path preserves the existing session lookup, input
    /// size limit and exited-session checks without widening the LAN protocol.
    pub fn write_terminal_input(&self, id: &str, bytes: &[u8]) -> Result<(), String> {
        if self
            .0
            .service
            .stopping
            .load(std::sync::atomic::Ordering::Acquire)
        {
            return Err("The experimental desktop is shutting down.".into());
        }
        self.0.service.terminal(id)?.write(bytes)
    }

    /// Private owner-only typed metadata. The renderer never receives secrets.
    pub fn get_agent_extension_assignments(
        &self,
        agent_id: &str,
    ) -> Result<ExtensionAssignments, String> {
        if self
            .0
            .service
            .stopping
            .load(std::sync::atomic::Ordering::Acquire)
        {
            return Err("The experimental desktop is shutting down.".into());
        }
        if !self.0.service.eligible_agent_ids()?.contains(agent_id) {
            return Err("Choose a saved non-internal agent.".into());
        }
        let config = self.0.service.extension_config()?;
        Ok(assignment_projection(&config, agent_id))
    }
    /// Modify only one membership, keeping private configuration entirely native.
    /// Existing normalization, stale-write checks and grant revocation still apply.
    pub fn set_agent_extension_assignment(
        &self,
        agent_id: &str,
        entry_id: &str,
        kind: ExtensionAssignmentKind,
        assigned: bool,
        expected_revision: &str,
    ) -> Result<ExtensionAssignments, String> {
        if self
            .0
            .service
            .stopping
            .load(std::sync::atomic::Ordering::Acquire)
        {
            return Err("The experimental desktop is shutting down.".into());
        }
        if !self.0.service.eligible_agent_ids()?.contains(agent_id) {
            return Err("Choose a saved non-internal agent.".into());
        }
        let mut config = self.0.service.extension_config()?;
        if config.revision.as_deref() != Some(expected_revision) {
            return Err("Extension assignments changed. Refresh before editing.".into());
        }
        let ids = match kind {
            ExtensionAssignmentKind::Server => {
                &mut config
                    .mcp_servers
                    .iter_mut()
                    .find(|entry| entry.id == entry_id)
                    .ok_or("Extension server no longer exists.")?
                    .agent_ids
            }
            ExtensionAssignmentKind::Skill => {
                let entry = config
                    .skills
                    .iter_mut()
                    .find(|entry| entry.id == entry_id)
                    .ok_or("Managed skill no longer exists.")?;
                if entry.all_agents {
                    return Err("This skill applies to all agents; change its global scope in extension settings.".into());
                }
                &mut entry.agent_ids
            }
        };
        ids.retain(|id| id != agent_id);
        if assigned {
            ids.push(agent_id.to_owned());
        }
        let saved = self.0.service.save_extension_config(config)?;
        Ok(assignment_projection(&saved, agent_id))
    }

    pub fn invoke(&self, command: &str, args: Value) -> Result<Value, String> {
        if self
            .0
            .service
            .stopping
            .load(std::sync::atomic::Ordering::Acquire)
        {
            return Err("The experimental desktop is shutting down.".into());
        }
        let service = &self.0.service;
        match command {
            "get_command_capabilities" => {
                let mut capabilities = command_capabilities("lan")?;
                capabilities
                    .commands
                    .extend(LOCAL.iter().map(|name| name.to_string()));
                capabilities.commands.sort();
                capabilities.commands.dedup();
                encode(capabilities)
            }
            "get_extension_config" => encode(service.extension_config()?),
            "save_extension_config" => encode(service.save_extension_config(arg(&args, "config")?)?),
            "list_environment_secrets" => encode(service.environment_secrets()?),
            "set_environment_secret" => encode(service.set_environment_secret(
                arg(&args, "revision")?,
                arg(&args, "name")?,
                arg(&args, "value")?,
                arg(&args, "description")?,
            )?),
            "delete_environment_secret" => encode(service.delete_environment_secret(
                arg(&args, "revision")?,
                arg(&args, "name")?,
            )?),
            "mail_get_config" => encode(service.mail.config()?),
            "mail_configure_gmail" => encode(service.mail.configure_gmail(
                arg(&args, "revision")?,
                arg(&args, "clientId")?,
                optional(&args, "clientSecret")?,
            )?),
            "mail_start_connect" => {
                let start = service.mail.start_connect()?;
                // The callback/token handling remains in MailService. Opening
                // its generated Google URL is an explicit local UI action.
                let opened = std::process::Command::new("/usr/bin/open")
                    .arg(&start.authorization_url)
                    .status();
                if !opened.is_ok_and(|status| status.success()) {
                    let _ = service.mail.cancel_connect(&start.attempt_id);
                    return Err(
                        "Could not open the system browser for Gmail sign-in. Try again.".into(),
                    );
                }
                encode(start)
            }
            "mail_connect_status" => encode(
                service
                    .mail
                    .connect_status(&arg::<String>(&args, "attemptId")?)?,
            ),
            "mail_cancel_connect" => {
                service
                    .mail
                    .cancel_connect(&arg::<String>(&args, "attemptId")?)?;
                Ok(Value::Null)
            }
            "mail_set_account_agents" => encode(service.set_mail_account_agents(
                arg(&args, "revision")?,
                arg(&args, "accountId")?,
                arg(&args, "agentIds")?,
            )?),
            "mail_disconnect" => encode(
                service
                    .mail
                    .disconnect(arg(&args, "revision")?, arg(&args, "accountId")?)?,
            ),
            "refresh_mail_inbox" => service.refresh_mail_inbox(
                arg(&args, "taskId")?,
                arg(&args, "accountId")?,
                arg(&args, "query")?,
            ),
            "request_mail_detail" => {
                encode(service.request_mail_detail(arg(&args, "taskId")?, arg(&args, "mailId")?)?)
            }
            "get_mail_detail" => encode(service.get_mail_detail(
                &arg::<String>(&args, "taskId")?,
                &arg::<String>(&args, "mailId")?,
            )?),
            "summarize_mail_thread" => encode(service.summarize_mail_thread(
                &arg::<String>(&args, "taskId")?,
                &arg::<String>(&args, "mailId")?,
            )?),
            "list_system_fonts" => encode(crate::load_system_font_families()),
            "choose_local_folder" => encode(crate::choose_local_folder_blocking(
                optional::<String>(&args, "initial")?.unwrap_or_default(),
            )?),
            "plan_jev_route" => encode(service.plan_jev_route(
                &arg::<String>(&args, "agentId")?,
                &arg::<String>(&args, "prompt")?,
            )?),
            "record_jev_route" => {
                service.record_jev_route(
                    &arg::<String>(&args, "taskId")?,
                    &arg::<String>(&args, "traceId")?,
                )?;
                Ok(Value::Null)
            }
            "plan_jev_command" => encode(service.plan_jev_command(
                &arg::<String>(&args, "query")?,
                &arg::<Vec<crate::model_router::JevCommandCandidate>>(&args, "candidates")?,
            )?),
            // Reuse the bounded existing dispatcher for already portable domain
            // operations. This does not add any command to the remote surface.
            _ => service.lan_invoke(command, args),
        }
    }
}

#[path = "experimental_desktop_ipc.rs"]
mod ipc;
pub use ipc::DesktopServer;

/// Only the menu-bar server calls this before acquiring its private IPC lease.
/// GUI callers may inspect the path but must never open its database themselves.
pub fn profile_dir() -> Result<PathBuf, String> {
    #[cfg(target_os = "macos")]
    {
        let home = std::env::var_os("HOME").ok_or("Cannot resolve home directory.")?;
        Ok(PathBuf::from(home)
            .join("Library/Application Support")
            .join(APP_ID))
    }
    #[cfg(not(target_os = "macos"))]
    Err("The experimental GPUI service currently supports macOS only.".into())
}

/// A transport-only startup probe. Present means a same-user server accepted
/// a local connection; protocol compatibility must still be checked by connect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackendAvailability {
    Absent,
    Present,
}

/// Cloneable GUI facade. Dropping/closing any GUI handle disconnects only;
/// execution and native preferences remain in the independent menu-bar server.
#[derive(Clone)]
pub struct DesktopService(ipc::Client);

impl DesktopService {
    pub fn open() -> Result<Self, String> {
        Self::connect()
    }
    pub fn connect() -> Result<Self, String> {
        ipc::Client::connect(&profile_dir()?).map(Self)
    }
    /// Only Absent permits the launcher to start a missing/stale backend.
    /// Permission, authentication, and live protocol failures remain visible.
    pub fn server_availability() -> Result<BackendAvailability, String> {
        ipc::server_availability(&profile_dir()?)
    }
    pub fn server_instance_id(&self) -> &str {
        self.0.instance_id()
    }
    /// Compatibility with the old GUI cleanup call. This never stops runs.
    pub fn shutdown(&self) {
        self.0.disconnect();
    }
    pub fn invoke(&self, command: &str, args: Value) -> Result<Value, String> {
        self.0
            .call("invoke", serde_json::json!({"command":command,"args":args}))
    }
    pub fn read_ui_changes(&self, revision: Option<&str>) -> Result<UiDeltaResponse, String> {
        self.0
            .call("read_ui_changes", serde_json::json!({"revision":revision}))
    }
    pub fn read_task_messages(
        &self,
        task_id: &str,
        before_id: Option<&str>,
        limit: Option<u32>,
    ) -> Result<TaskMessagesPage, String> {
        self.0.call(
            "read_task_messages",
            serde_json::json!({"taskId":task_id,"beforeId":before_id,"limit":limit}),
        )
    }
    pub fn read_channel_messages(
        &self,
        channel_id: &str,
        before_id: Option<&str>,
        limit: Option<u32>,
    ) -> Result<ChannelMessagesPage, String> {
        self.0.call(
            "read_channel_messages",
            serde_json::json!({"channelId":channel_id,"beforeId":before_id,"limit":limit}),
        )
    }
    pub fn subscribe_changes(&self) -> Result<async_channel::Receiver<()>, String> {
        self.0.subscribe()
    }
    pub fn read_attachment_file(&self, source_path: &str) -> Result<ReadAttachmentFile, String> {
        self.0.call(
            "read_attachment_file",
            serde_json::json!({"sourcePath":source_path}),
        )
    }
    pub fn store_selected_attachment(
        &self,
        task_id: &str,
        expected_host_id: &str,
        expected_cwd: &str,
        file: ReadAttachmentFile,
        preview_data_url: Option<String>,
    ) -> Result<Attachment, String> {
        self.0.call("store_selected_attachment", serde_json::json!({"taskId":task_id,"expectedHostId":expected_host_id,"expectedCwd":expected_cwd,"file":file,"previewDataUrl":preview_data_url}))
    }
    pub fn write_terminal_input(&self, id: &str, bytes: &[u8]) -> Result<(), String> {
        self.0.call(
            "write_terminal_input",
            serde_json::json!({"id":id,"bytes":bytes}),
        )
    }
    pub fn load_native_preferences(&self) -> Result<NativePreferences, String> {
        self.0.call("load_native_preferences", Value::Null)
    }
    pub fn save_native_preferences(
        &self,
        expected_revision: &str,
        data: NativePreferenceData,
    ) -> Result<NativePreferences, String> {
        self.0.call(
            "save_native_preferences",
            serde_json::json!({"expectedRevision":expected_revision,"data":data}),
        )
    }
    pub fn save_schedule(&self, input: ScheduleInput) -> Result<crate::model::Snapshot, String> {
        self.0
            .call("save_schedule", serde_json::json!({"input":input}))
    }
    pub fn schedule_action(
        &self,
        id: String,
        action: ScheduleAction,
    ) -> Result<crate::model::Snapshot, String> {
        self.0.call(
            "schedule_action",
            serde_json::json!({"id":id,"action":action}),
        )
    }
    pub fn read_lan_status(&self) -> Result<NativeLanStatus, String> {
        self.0.call("read_lan_status", Value::Null)
    }
    pub fn start_lan(&self) -> Result<NativeLanStatus, String> {
        self.0.call("start_lan", Value::Null)
    }
    pub fn stop_lan(&self) -> Result<NativeLanStatus, String> {
        self.0.call("stop_lan", Value::Null)
    }
    pub fn get_agent_extension_assignments(
        &self,
        agent_id: &str,
    ) -> Result<ExtensionAssignments, String> {
        self.0.call(
            "get_agent_extension_assignments",
            serde_json::json!({"agentId":agent_id}),
        )
    }
    pub fn set_agent_extension_assignment(
        &self,
        agent_id: &str,
        entry_id: &str,
        kind: ExtensionAssignmentKind,
        assigned: bool,
        expected_revision: &str,
    ) -> Result<ExtensionAssignments, String> {
        self.0.call("set_agent_extension_assignment", serde_json::json!({"agentId":agent_id,"entryId":entry_id,"kind":kind,"assigned":assigned,"expectedRevision":expected_revision}))
    }
}

fn arg<T: DeserializeOwned>(args: &Value, name: &str) -> Result<T, String> {
    serde_json::from_value(
        args.get(name)
            .cloned()
            .ok_or_else(|| format!("Missing {name}."))?,
    )
    .map_err(|_| format!("Invalid {name}."))
}

fn optional<T: DeserializeOwned>(args: &Value, name: &str) -> Result<Option<T>, String> {
    args.get(name)
        .filter(|v| !v.is_null())
        .cloned()
        .map(serde_json::from_value)
        .transpose()
        .map_err(|_| format!("Invalid {name}."))
}

fn encode(value: impl serde::Serialize) -> Result<Value, String> {
    serde_json::to_value(value).map_err(|error| format!("Cannot encode desktop result: {error}"))
}

fn validate_profile(dir: &std::path::Path) -> Result<(), String> {
    if dir.file_name().and_then(|v| v.to_str()) != Some(APP_ID) {
        return Err("Only the isolated GPUI profile is allowed.".into());
    }
    if let Ok(metadata) = std::fs::symlink_metadata(dir) {
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err("The experimental profile must be a directory, not a symlink.".into());
        }
    }
    // A linked database or lock would defeat the separate profile identity.
    if dir.exists() {
        for entry in std::fs::read_dir(dir).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            if entry.file_type().map_err(|e| e.to_string())?.is_symlink() {
                return Err("The experimental profile cannot contain linked live data.".into());
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn lan_assets_require_isolated_profile_and_real_entry_file() {
        let root = std::env::temp_dir().join(format!("gpui-lan-assets-{}", uuid::Uuid::new_v4()));
        let profile = root.join(APP_ID);
        std::fs::create_dir_all(&profile).unwrap();
        assert!(lan_asset_roots(&profile, &root.join("binary")).is_err());
        let assets = profile.join("lan-web");
        std::fs::create_dir_all(&assets).unwrap();
        std::fs::write(assets.join("index.html"), "fixture").unwrap();
        assert_eq!(
            lan_asset_roots(&profile, &root.join("binary")).unwrap(),
            vec![assets]
        );
        assert!(lan_asset_roots(&root.join("com.monitter.desktop"), &root.join("binary")).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn lan_assets_use_profile_before_packaged_resources_without_source_fallback() {
        let root = std::env::temp_dir().join(format!("gpui-lan-bundle-{}", uuid::Uuid::new_v4()));
        let profile = root.join(APP_ID);
        let live = profile.join("lan-web");
        let bundle = root.join("Monitter GPUI.app/Contents/Resources/lan-web");
        for assets in [&live, &bundle] {
            std::fs::create_dir_all(assets).unwrap();
            std::fs::write(assets.join("index.html"), "fixture").unwrap();
        }
        let executable = root.join("Monitter GPUI.app/Contents/MacOS/monitter-gpui-desktop");
        assert_eq!(
            lan_asset_roots(&profile, &executable).unwrap(),
            vec![live, bundle]
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn lan_assets_reject_symlinked_profile_assets_and_entry_page() {
        use std::os::unix::fs::symlink;
        let root = std::env::temp_dir().join(format!("gpui-lan-links-{}", uuid::Uuid::new_v4()));
        let profile = root.join(APP_ID);
        let outside = root.join("outside");
        std::fs::create_dir_all(&profile).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("index.html"), "fixture").unwrap();
        symlink(&outside, profile.join("lan-web")).unwrap();
        assert!(lan_asset_roots(&profile, &root.join("binary")).is_err());
        std::fs::remove_file(profile.join("lan-web")).unwrap();
        std::fs::create_dir(profile.join("lan-web")).unwrap();
        symlink(
            outside.join("index.html"),
            profile.join("lan-web/index.html"),
        )
        .unwrap();
        assert!(lan_asset_roots(&profile, &root.join("binary")).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn headless_service_uses_real_state_and_revision_contract() {
        let dir = std::env::temp_dir().join(format!("gpui-service-test-{}", uuid::Uuid::new_v4()));
        let service = Service::open(None, dir.clone()).unwrap();
        let desktop = BackendService(Arc::new(Session {
            lan_control: std::sync::Mutex::new(()),
            service,
            preferences: NativePreferenceStore::new(dir.clone()),
        }));
        let attachment = dir.join("selected.txt");
        std::fs::write(&attachment, b"selected fixture").unwrap();
        let selected = desktop
            .read_attachment_file(attachment.to_str().unwrap())
            .unwrap();
        assert_eq!(selected.filename, "selected.txt");
        assert_eq!(selected.data_base64, "c2VsZWN0ZWQgZml4dHVyZQ==");
        assert!(desktop.read_attachment_file(dir.to_str().unwrap()).is_err());
        let changes = desktop.subscribe_changes().unwrap();
        assert!(changes.try_recv().is_err());
        let first = desktop.invoke("get_ui_snapshot", json!({})).unwrap();
        let typed_first = desktop.read_ui_changes(None).unwrap();
        assert_eq!(typed_first.revision, first["revision"].as_str().unwrap());
        assert!(typed_first.snapshot.is_some());
        let typed_unchanged = desktop
            .read_ui_changes(Some(&typed_first.revision))
            .unwrap();
        assert!(typed_unchanged.snapshot.is_none() && typed_unchanged.delta.is_none());
        let agent_id = first["snapshot"]["agents"][0]["id"].clone();
        let unchanged = desktop
            .invoke("get_ui_snapshot", json!({"revision": first["revision"]}))
            .unwrap();
        assert!(unchanged["snapshot"].is_null());
        let created = desktop
            .invoke(
                "create_task",
                json!({"input": {
                    "agentId": agent_id, "title": "Isolated service test"
                }}),
            )
            .unwrap();
        let next = desktop
            .invoke("get_ui_snapshot", json!({"revision": first["revision"]}))
            .unwrap();
        let rejected = desktop
            .store_selected_attachment(
                created["id"].as_str().unwrap(),
                "stale-host-id",
                "/stale-workspace",
                selected,
                None,
            )
            .unwrap_err();
        assert!(rejected.contains("attachment destination changed"));
        assert_ne!(first["revision"], next["revision"]);
        assert!(changes.try_recv().is_ok());
        assert!(changes.try_recv().is_err());
        assert!(
            next["snapshot"]["tasks"]
                .as_array()
                .unwrap()
                .iter()
                .any(|task| task["id"] == created["id"])
        );
        // Creating a task may insert its system instructions, but never starts
        // a model turn. The history endpoint must expose the same stored data.
        let page = desktop
            .invoke(
                "get_task_messages",
                json!({"taskId": created["id"], "limit":100}),
            )
            .unwrap();
        let task_messages: Vec<_> = next["snapshot"]["messages"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|message| message["taskId"] == created["id"])
            .cloned()
            .collect();
        assert_eq!(page["messages"], json!(task_messages));
        let typed_page = desktop
            .read_task_messages(created["id"].as_str().unwrap(), None, Some(100))
            .unwrap();
        assert_eq!(json!(typed_page.messages), page["messages"]);
        assert_eq!(typed_page.revision, page["revision"].as_str().unwrap());
        assert!(
            desktop
                .read_task_messages("missing-task", None, None)
                .is_err()
        );
        assert!(
            desktop
                .read_channel_messages("missing-channel", None, None)
                .is_err()
        );
        assert!(
            task_messages
                .iter()
                .all(|message| message["role"] == "system")
        );
        let capabilities = desktop
            .invoke("get_command_capabilities", json!({}))
            .unwrap();
        let commands = capabilities["commands"].as_array().unwrap();
        assert!(commands.iter().any(|command| command == "save_settings"));
        for command in ["get_extension_config", "save_extension_config", "list_environment_secrets", "set_environment_secret", "delete_environment_secret"] {
            assert!(commands.iter().any(|value| value == command), "owner capability missing: {command}");
            let error = desktop.0.service.lan_invoke(command, json!({})).unwrap_err();
            assert!(error.contains("not available"), "owner settings escaped into LAN: {command}: {error}");
        }
        assert!(!commands.iter().any(|command| command == "browser_open"));
        assert!(desktop.invoke("browser_open", json!({})).is_err());
        let settings = next["snapshot"]["settings"].clone();
        desktop
            .invoke("save_settings", json!({"settings": settings}))
            .unwrap();
        // Deletion/settings used to require a Tauri AppHandle. Exercise them
        // with real isolated storage but never start a harness or touch mail.
        assert!(
            desktop
                .invoke("delete_task", json!({"id":created["id"]}))
                .is_err()
        );
        desktop
            .invoke(
                "set_task_archived",
                json!({"taskId":created["id"],"archived":true}),
            )
            .unwrap();
        desktop
            .invoke("delete_task", json!({"id":created["id"]}))
            .unwrap();
        // Archive and deletion arrive as one wake; a fresh snapshot contains
        // both changes. Slow native consumers cannot accumulate an event backlog.
        assert!(changes.try_recv().is_ok());
        assert!(changes.try_recv().is_err());
        desktop.shutdown();
        assert!(
            desktop
                .read_attachment_file(attachment.to_str().unwrap())
                .is_err()
        );
        assert!(desktop.invoke("get_snapshot", json!({})).is_err());
        assert!(desktop.read_ui_changes(None).is_err());
        assert!(
            desktop
                .read_task_messages("missing-task", None, None)
                .is_err()
        );
        drop(desktop);
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[cfg(unix)]
    #[test]
    fn native_preferences_are_private_bounded_and_revision_guarded() {
        use std::os::unix::fs::PermissionsExt;
        let dir =
            std::env::temp_dir().join(format!("gpui-preference-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        let store = NativePreferenceStore::new(dir.clone());
        let empty = store.load().unwrap();
        assert_eq!(empty.revision, "0");
        let mut data = NativePreferenceData::default();
        data.appearance.sidebar_width = Some(252.0);
        data.drafts.insert(
            uuid::Uuid::new_v4().to_string(),
            "unsent composer draft".into(),
        );
        let pane_id = uuid::Uuid::new_v4().to_string();
        let pane_state = NativeChatPanePreferences {
            visible: false,
            tab: NativeDetailTab::Approvals,
            width: 416.0,
        };
        data.chat_panes.insert(pane_id.clone(), pane_state.clone());
        let attachment_task = uuid::Uuid::new_v4().to_string();
        let attachment_draft = NativeAttachmentDraft {
            host_id: uuid::Uuid::new_v4().to_string(),
            cwd: "/work/project".into(),
            attachments: vec![NativeAttachmentRef {
                id: uuid::Uuid::new_v4().to_string(),
                name: "notes.txt".into(),
                mime_type: "text/plain".into(),
                size: 128,
            }],
        };
        data.attachment_drafts
            .insert(attachment_task.clone(), attachment_draft.clone());
        let saved = store.save("0", data.clone()).unwrap();
        let loaded = store.load().unwrap();
        assert_eq!(loaded.revision, saved.revision);
        assert_eq!(loaded.data.chat_panes.get(&pane_id), Some(&pane_state));
        assert_eq!(
            loaded.data.attachment_drafts.get(&attachment_task),
            Some(&attachment_draft)
        );
        assert_eq!(loaded.data.drafts, data.drafts);
        assert!(store.save("0", data.clone()).is_err());
        assert_eq!(
            std::fs::metadata(dir.join(NATIVE_PREFERENCES_FILE))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
            0o700
        );
        data.drafts
            .insert(uuid::Uuid::new_v4().to_string(), "x".repeat(32 * 1024 + 1));
        assert!(store.save(&saved.revision, data).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn native_appearance_defaults_and_bounds_are_backward_compatible() {
        let legacy: NativeAppearance = serde_json::from_value(json!({
            "sidebarWidth": 252.0,
            "warmPaper": true,
            "lastRoute": "chats"
        }))
        .unwrap();
        assert_eq!(legacy.light_palette, "monitter");
        assert_eq!(legacy.dark_palette, "monitter");
        assert_eq!(legacy.custom_accent, None);
        assert_eq!(legacy.contrast, 0);
        assert_eq!(legacy.surface_tint, 5);
        assert_eq!(legacy.border_opacity, 100);
        assert_eq!(legacy.interface_scale, 125);
        assert_eq!(legacy.tile_theme, NativeTileTheme::Monitter);
        validate_native_data(&NativePreferenceData::default()).unwrap();

        let mut data = NativePreferenceData::default();
        data.appearance.light_palette = "unknown".into();
        assert!(validate_native_data(&data).is_err());
        data.appearance.light_palette = "monitter".into();
        data.appearance.contrast = 101;
        assert!(validate_native_data(&data).is_err());
        data.appearance.contrast = 0;
        data.appearance.surface_tint = 51;
        assert!(validate_native_data(&data).is_err());
        data.appearance.surface_tint = 5;
        data.appearance.interface_scale = 126;
        assert!(validate_native_data(&data).is_err());
        data.appearance.interface_scale = 125;
        data.appearance.custom_accent = Some("blue".into());
        assert!(validate_native_data(&data).is_err());
    }
    #[test]
    fn native_tile_theme_is_strict_and_round_trips() {
        for (theme, value) in [
            (NativeTileTheme::Monitter, "monitter"),
            (NativeTileTheme::Tern, "tern"),
        ] {
            let mut data = NativePreferenceData::default();
            data.appearance.tile_theme = theme;
            validate_native_data(&data).unwrap();
            let serialized = serde_json::to_value(&data).unwrap();
            if theme == NativeTileTheme::Monitter {
                assert!(serialized["appearance"].get("tileTheme").is_none(), "Default appearance must remain compatible with older backends");
            } else { assert_eq!(serialized["appearance"]["tileTheme"], value); }
            let restored: NativePreferenceData = serde_json::from_value(serialized).unwrap();
            assert_eq!(restored.appearance.tile_theme, theme);
        }
        assert!(serde_json::from_value::<NativeAppearance>(json!({"tileTheme": "unknown"})).is_err());
        assert!(serde_json::from_value::<NativeAppearance>(json!({"tileTheme": "Tern"})).is_err());
    }
    #[test]
    fn native_browser_metadata_is_optional_strict_and_bounded() {
        let legacy: NativePreferenceData = serde_json::from_value(json!({})).unwrap();
        assert_eq!(legacy.browser_metadata, None);
        let serialized = serde_json::to_value(&legacy).unwrap();
        assert!(serialized.get("browserMetadata").is_none());
        validate_native_data(&legacy).unwrap();

        let metadata = NativeBrowserMetadata {
            tabs: vec![NativeBrowserTab {
                id: 1,
                requested_url: "https://example.com/start".into(),
                actual_url: Some("https://example.com/page".into()),
                title: "Example".into(),
            }],
            selected_id: Some(1),
        };
        let mut data = NativePreferenceData::default();
        data.browser_metadata = Some(metadata.clone());
        validate_native_data(&data).unwrap();
        let encoded = serde_json::to_value(&data).unwrap();
        assert_eq!(
            encoded["browserMetadata"]["tabs"][0]["requestedUrl"],
            "https://example.com/start"
        );
        assert_eq!(
            serde_json::from_value::<NativePreferenceData>(encoded)
                .unwrap()
                .browser_metadata,
            Some(metadata)
        );

        let empty_metadata = NativeBrowserMetadata::default();
        data.browser_metadata = Some(empty_metadata);
        validate_native_data(&data).unwrap();
        assert!(
            serde_json::from_value::<NativeBrowserMetadata>(json!({
                "tabs": [], "selectedId": null, "cookies": []
            }))
            .is_err()
        );
        assert!(
            serde_json::from_value::<NativeBrowserTab>(json!({
                "id": 1,
                "requestedUrl": "about:blank",
                "actualUrl": null,
                "title": "",
                "body": "must not persist"
            }))
            .is_err()
        );
    }

    #[test]
    fn native_browser_metadata_rejects_invalid_ids_selection_urls_and_bounds() {
        let tab = |id, requested_url: String, actual_url: Option<String>, title: String| {
            NativeBrowserTab {
                id,
                requested_url,
                actual_url,
                title,
            }
        };
        let valid = || NativeBrowserMetadata {
            tabs: vec![tab(1, "about:blank".into(), None, "".into())],
            selected_id: Some(1),
        };
        let mut data = NativePreferenceData::default();
        let mut metadata = valid();
        metadata.tabs[0].id = 0;
        data.browser_metadata = Some(metadata);
        assert!(validate_native_data(&data).is_err());

        let mut metadata = valid();
        let duplicate = metadata.tabs[0].clone();
        metadata.tabs.push(duplicate);
        data.browser_metadata = Some(metadata);
        assert!(validate_native_data(&data).is_err());

        let mut metadata = valid();
        metadata.selected_id = None;
        data.browser_metadata = Some(metadata);
        assert!(validate_native_data(&data).is_err());

        let mut metadata = valid();
        metadata.selected_id = Some(2);
        data.browser_metadata = Some(metadata);
        assert!(validate_native_data(&data).is_err());

        let mut metadata = NativeBrowserMetadata::default();
        metadata.selected_id = Some(1);
        data.browser_metadata = Some(metadata);
        assert!(validate_native_data(&data).is_err());

        for url in [
            "javascript:alert(1)".to_owned(),
            "about:blank#fragment".to_owned(),
            "https://user:secret@example.com/".to_owned(),
            format!(
                "https://example.com/{}",
                "x".repeat(MAX_NATIVE_BROWSER_URL_BYTES)
            ),
        ] {
            let mut metadata = valid();
            metadata.tabs[0].requested_url = url;
            data.browser_metadata = Some(metadata);
            assert!(validate_native_data(&data).is_err());
        }

        let mut metadata = valid();
        metadata.tabs[0].actual_url = Some("http://user@example.com".into());
        data.browser_metadata = Some(metadata);
        assert!(validate_native_data(&data).is_err());

        let mut metadata = valid();
        metadata.tabs[0].title = "x".repeat(MAX_NATIVE_BROWSER_TITLE_BYTES + 1);
        data.browser_metadata = Some(metadata);
        assert!(validate_native_data(&data).is_err());

        let mut metadata = valid();
        metadata.tabs[0].title = "bad\ntitle".into();
        data.browser_metadata = Some(metadata);
        assert!(validate_native_data(&data).is_err());

        let mut metadata = NativeBrowserMetadata {
            tabs: (1..=MAX_NATIVE_BROWSER_TABS as u64 + 1)
                .map(|id| tab(id, "about:blank".into(), None, String::new()))
                .collect(),
            selected_id: Some(1),
        };
        data.browser_metadata = Some(metadata.clone());
        assert!(validate_native_data(&data).is_err());
        metadata.tabs.truncate(MAX_NATIVE_BROWSER_TABS);
        metadata.selected_id = Some(1);
        data.browser_metadata = Some(metadata);
        validate_native_data(&data).unwrap();

        let url_prefix = "https://example.com/";
        let url = format!(
            "{url_prefix}{}",
            "x".repeat(MAX_NATIVE_BROWSER_URL_BYTES - url_prefix.len())
        );
        let metadata = NativeBrowserMetadata {
            tabs: (1..=33)
                .map(|id| tab(id, url.clone(), None, String::new()))
                .collect(),
            selected_id: Some(1),
        };
        data.browser_metadata = Some(metadata);
        assert!(validate_native_data(&data).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn native_browser_metadata_round_trips_through_cas_preferences() {
        let dir = std::env::temp_dir().join(format!(
            "gpui-browser-preference-test-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir(&dir).unwrap();
        let store = NativePreferenceStore::new(dir.clone());
        let mut data = NativePreferenceData::default();
        let metadata = NativeBrowserMetadata {
            tabs: vec![
                NativeBrowserTab {
                    id: 4,
                    requested_url: "https://example.com/requested".into(),
                    actual_url: Some("https://example.com/actual".into()),
                    title: "Example page".into(),
                },
                NativeBrowserTab {
                    id: 7,
                    requested_url: "about:blank".into(),
                    actual_url: None,
                    title: String::new(),
                },
            ],
            selected_id: Some(7),
        };
        data.browser_metadata = Some(metadata.clone());
        let saved = store.save("0", data).unwrap();
        let loaded = store.load().unwrap();
        assert_eq!(loaded.revision, saved.revision);
        assert_eq!(loaded.data.browser_metadata, Some(metadata));
        assert!(store.save("0", loaded.data).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn native_workspaces_are_bounded_and_legacy_compatible() {
        let legacy: NativePreferenceData = serde_json::from_value(json!({})).unwrap();
        assert!(legacy.workspaces.is_none());
        assert!(serde_json::to_value(&legacy).unwrap().get("workspaces").is_none());
        let id = uuid::Uuid::new_v4().to_string();
        let workspace = NativeWorkspace {
            id: id.clone(), name: "Development".into(), color: "#Aabb09".into(),
            icon: "code".into(),
            layout: Some(json!({"version":1,"center":{"panel_name":"MonitterNativePane","children":[],"info":{"panel":{"id":"chat:main"}}}})),
        };
        let mut data = NativePreferenceData::default();
        data.workspaces = Some(NativeWorkspaceSet { active_id: id, items: vec![workspace] });
        validate_native_data(&data).unwrap();
        let encoded = serde_json::to_value(&data).unwrap();
        assert!(encoded["workspaces"]["activeId"].is_string());
        let decoded: NativePreferenceData = serde_json::from_value(encoded).unwrap();
        assert_eq!(decoded.workspaces, data.workspaces);
        for mutation in 0..9 {
            let mut invalid = data.clone();
            let set = invalid.workspaces.as_mut().unwrap();
            match mutation {
                0 => set.active_id = uuid::Uuid::new_v4().to_string(),
                1 => set.items.push(set.items[0].clone()),
                2 => set.items[0].color = "#GG0000".into(),
                3 => set.items[0].icon = "unknown".into(),
                4 => set.items[0].layout = Some(json!({"version": 99})),
                5 => set.items[0].name = " ".into(),
                6 => set.items[0].name = "x".repeat(81),
                7 => set.items[0].id = "bad-id".into(),
                _ => set.items = (0..25).map(|_| {
                    let mut item = set.items[0].clone();
                    item.id = uuid::Uuid::new_v4().to_string(); item
                }).collect(),
            }
            assert!(validate_native_data(&invalid).is_err(), "mutation {mutation}");
        }
    }
    #[test]
    fn native_chat_pane_preferences_are_bounded_and_legacy_compatible() {
        let legacy: NativePreferenceData = serde_json::from_value(json!({})).unwrap();
        assert!(legacy.chat_panes.is_empty());
        assert!(
            serde_json::to_value(&legacy)
                .unwrap()
                .get("chatPanes")
                .is_none()
        );

        let task_id = uuid::Uuid::new_v4().to_string();
        let mut data = NativePreferenceData::default();
        data.chat_panes.insert(
            task_id.clone(),
            NativeChatPanePreferences {
                visible: true,
                tab: NativeDetailTab::Subagents,
                width: 420.5,
            },
        );
        validate_native_data(&data).unwrap();
        let encoded = serde_json::to_value(&data).unwrap();
        assert_eq!(encoded["chatPanes"][&task_id]["tab"], "subagents");
        assert_eq!(
            serde_json::from_value::<NativePreferenceData>(encoded)
                .unwrap()
                .chat_panes[&task_id]
                .tab,
            NativeDetailTab::Subagents
        );

        let mut invalid_id = NativePreferenceData::default();
        invalid_id.chat_panes.insert(
            "not-a-uuid".into(),
            NativeChatPanePreferences {
                visible: false,
                tab: NativeDetailTab::Details,
                width: 300.0,
            },
        );
        assert!(validate_native_data(&invalid_id).is_err());

        for width in [f32::NAN, f32::INFINITY, 259.99, 1600.01] {
            let mut invalid_width = NativePreferenceData::default();
            invalid_width.chat_panes.insert(
                uuid::Uuid::new_v4().to_string(),
                NativeChatPanePreferences {
                    visible: true,
                    tab: NativeDetailTab::Timeline,
                    width,
                },
            );
            assert!(validate_native_data(&invalid_width).is_err());
        }

        assert!(
            serde_json::from_value::<NativeChatPanePreferences>(json!({
                "visible": true, "tab": "timeline", "width": 360.0, "extra": 1
            }))
            .is_err()
        );
        assert!(
            serde_json::from_value::<NativeChatPanePreferences>(json!({
                "visible": true, "tab": "Timeline", "width": 360.0
            }))
            .is_err()
        );

        let mut too_many = NativePreferenceData::default();
        for _ in 0..513 {
            too_many.chat_panes.insert(
                uuid::Uuid::new_v4().to_string(),
                NativeChatPanePreferences {
                    visible: true,
                    tab: NativeDetailTab::Approvals,
                    width: 360.0,
                },
            );
        }
        assert!(validate_native_data(&too_many).is_err());
    }
    #[test]
    fn native_attachment_drafts_are_scoped_bounded_and_legacy_compatible() {
        let legacy: NativePreferenceData = serde_json::from_value(json!({})).unwrap();
        assert!(legacy.attachment_drafts.is_empty());
        assert!(
            serde_json::to_value(&legacy)
                .unwrap()
                .get("attachmentDrafts")
                .is_none()
        );

        let task_id = uuid::Uuid::new_v4().to_string();
        let host_id = uuid::Uuid::new_v4().to_string();
        let attachment = NativeAttachmentDraft {
            host_id: host_id.clone(),
            cwd: "/work/project".into(),
            attachments: vec![NativeAttachmentRef {
                id: uuid::Uuid::new_v4().to_string(),
                name: "report.pdf".into(),
                mime_type: "application/pdf".into(),
                size: crate::attachments::MAX_BYTES as u64,
            }],
        };
        let mut data = NativePreferenceData::default();
        data.attachment_drafts
            .insert(task_id.clone(), attachment.clone());
        validate_native_data(&data).unwrap();
        let encoded = serde_json::to_value(&data).unwrap();
        assert_eq!(encoded["attachmentDrafts"][&task_id]["hostId"], host_id);
        assert!(
            encoded["attachmentDrafts"][&task_id]["attachments"][0]
                .get("path")
                .is_none()
        );
        assert_eq!(
            serde_json::from_value::<NativePreferenceData>(encoded)
                .unwrap()
                .attachment_drafts[&task_id],
            attachment
        );
        assert!(
            serde_json::from_value::<NativeAttachmentRef>(json!({
                "id": uuid::Uuid::new_v4().to_string(),
                "name": "report.pdf",
                "mimeType": "application/pdf",
                "size": 1,
                "path": "/must/not/persist"
            }))
            .is_err()
        );

        let mut invalid = NativePreferenceData::default();
        invalid
            .attachment_drafts
            .insert("not-a-task-uuid".into(), attachment.clone());
        assert!(validate_native_data(&invalid).is_err());

        let mut invalid = NativePreferenceData::default();
        let mut wrong_host = attachment.clone();
        wrong_host.host_id = "not-a-host-uuid".into();
        invalid
            .attachment_drafts
            .insert(task_id.clone(), wrong_host);
        assert!(validate_native_data(&invalid).is_err());

        let mut invalid = NativePreferenceData::default();
        let mut duplicate = attachment.clone();
        duplicate.attachments.push(duplicate.attachments[0].clone());
        invalid.attachment_drafts.insert(task_id.clone(), duplicate);
        assert!(validate_native_data(&invalid).is_err());

        let mut invalid = NativePreferenceData::default();
        invalid
            .attachment_drafts
            .insert(task_id.clone(), attachment.clone());
        let other_task = uuid::Uuid::new_v4().to_string();
        invalid
            .attachment_drafts
            .insert(other_task, attachment.clone());
        assert!(validate_native_data(&invalid).is_err());

        let mut invalid = NativePreferenceData::default();
        let mut too_large = attachment;
        too_large.attachments[0].size += 1;
        invalid.attachment_drafts.insert(task_id, too_large);
        assert!(validate_native_data(&invalid).is_err());
    }
    #[test]
    fn native_layout_cannot_smuggle_private_configuration() {
        let mut data = NativePreferenceData::default();
        data.layout = Some(
            json!({"version":1,"center":{"panel_name":"MonitterNativePane","children":[],"info":{"panel":{"id":"browser","headers":{"Authorization":"never persist"}}}}}),
        );
        assert!(validate_native_data(&data).is_err());
        data.layout = Some(
            json!({"version":1,"center":{"panel_name":"StackPanel","children":[],"info":{"stack":{"sizes":[],"axis":0,"mailBody":"never persist"}}}}),
        );
        assert!(validate_native_data(&data).is_err());
        data.layout = Some(
            json!({"version":1,"center":{"panel_name":"StackPanel","children":[{"panel_name":"TabPanel","children":[{"panel_name":"MonitterNativePane","children":[],"info":{"panel":{"id":"chat:main"}}}],"info":{"tabs":{"active_index":0}}}],"info":{"stack":{"sizes":[252.0],"axis":0}}}}),
        );
        assert!(validate_native_data(&data).is_ok());
    }
    #[cfg(unix)]
    #[test]
    fn native_preferences_reject_links_without_touching_target() {
        use std::os::unix::fs::symlink;
        let dir = std::env::temp_dir().join(format!(
            "gpui-preference-link-test-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir(&dir).unwrap();
        let target = dir.join("target");
        std::fs::write(&target, b"preserve").unwrap();
        symlink(&target, dir.join(NATIVE_PREFERENCES_FILE)).unwrap();
        let store = NativePreferenceStore::new(dir.clone());
        assert!(store.load().is_err());
        assert!(store.save("0", NativePreferenceData::default()).is_err());
        assert_eq!(std::fs::read(&target).unwrap(), b"preserve");
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn rejects_production_profile() {
        assert!(validate_profile(std::path::Path::new("/tmp/com.monitter.desktop")).is_err());
    }
    #[cfg(unix)]
    #[test]
    fn rejects_linked_profile() {
        let temp = std::env::temp_dir().join(format!("gpui-profile-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&temp).unwrap();
        let profile = temp.join(APP_ID);
        std::os::unix::fs::symlink(&temp, &profile).unwrap();
        assert!(validate_profile(&profile).is_err());
        std::fs::remove_file(profile).unwrap();
        std::fs::remove_dir(temp).unwrap();
    }
}
