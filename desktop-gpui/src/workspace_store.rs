//! Client-local workspace layouts. Compatible with already-running backend schemas.
//! All IO runs off the UI thread; SQLite transactions guard concurrent GUI writers.
use monitter_lib::experimental_desktop::{
    NativeTileTheme, NativeWorkspaceSet, validate_native_workspace_set,
};
use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
const MAX_BYTES: usize = 512 * 1024;
const MAX_SAVED_LAYOUT_BYTES: usize = 256 * 1024;
pub const MAX_SAVED_LAYOUTS_PER_WORKSPACE: usize = 12;
const MAX_SAVED_LAYOUT_WARNINGS: usize = 4;
const DATABASE: &str = "named-workspaces.sqlite3";

/// A named, client-local arrangement for one project space. The serialized
/// dock tree references the space's existing pane identities; it owns no task,
/// terminal, session, draft or transcript state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SavedWorkspaceLayout {
    pub id: String,
    pub workspace_id: String,
    pub label: String,
    pub layout: serde_json::Value,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

/// Detailed load result for callers with a visible diagnostic surface. The
/// compatibility loader returns only `layouts` and silently skips bad rows.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SavedWorkspaceLayoutLoad {
    pub layouts: Vec<SavedWorkspaceLayout>,
    pub warnings: Vec<String>,
}
fn path() -> Result<PathBuf, String> {
    Ok(monitter_lib::experimental_desktop::profile_dir()?.join(DATABASE))
}
fn database(path: &Path) -> Result<Connection, String> {
    let parent = path.parent().ok_or("Workspace store has no directory.")?;
    std::fs::create_dir_all(parent).map_err(|e| format!("Workspace directory: {e}"))?;
    if std::fs::symlink_metadata(parent)
        .map_err(|e| e.to_string())?
        .file_type()
        .is_symlink()
    {
        return Err("Workspace directory cannot be a symbolic link.".into());
    }
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )
    .map_err(|e| format!("Workspace store: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| e.to_string())?;
    }
    connection
        .busy_timeout(Duration::from_secs(2))
        .map_err(|e| e.to_string())?;
    connection.execute_batch("CREATE TABLE IF NOT EXISTS workspaces (id INTEGER PRIMARY KEY CHECK(id = 1), revision INTEGER NOT NULL, payload TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS saved_layouts (
            id TEXT PRIMARY KEY,
            workspace_id TEXT NOT NULL,
            label TEXT NOT NULL,
            layout TEXT NOT NULL,
            created_at_ms INTEGER NOT NULL,
            updated_at_ms INTEGER NOT NULL,
            UNIQUE(workspace_id, label COLLATE NOCASE)
        );
        CREATE INDEX IF NOT EXISTS saved_layouts_workspace_updated
            ON saved_layouts(workspace_id, updated_at_ms DESC, created_at_ms DESC)")
        .map_err(|e| e.to_string())?;
    Ok(connection)
}
/// Remove validated GPUI-only metadata from a clone for resident backends.
/// Local workspace and startup-cache writes keep the original layout.
pub fn backend_layout_projection(layout: &serde_json::Value) -> Result<serde_json::Value, String> {
    let mut projected = layout.clone();
    let focus = projected.as_object_mut().and_then(|layout| layout.remove("focusedPaneId"));
    if let Some(focus) = focus {
        let valid = focus.as_str().is_some_and(|id| {
            !id.is_empty() && id.len() <= 128
                && id.bytes().all(|byte| byte.is_ascii_alphanumeric() || b"-_:./".contains(&byte))
        });
        if !valid { return Err("Invalid local workspace focused pane identity.".into()); }
    }
    fn strip_mode(pane: &mut serde_json::Value, depth: usize) -> Result<(), String> {
        if depth > 32 { return Err("Native workspace layout exceeds nesting limit.".into()); }
        let native_leaf = pane["panel_name"] == "MonitterNativePane"
            && pane["children"].as_array().is_some_and(Vec::is_empty);
        if native_leaf {
            if let Some(info) = pane.get_mut("info").and_then(|info| info.get_mut("panel")).and_then(serde_json::Value::as_object_mut) {
                if let Some(mode) = info.remove("watchMode") {
                    if !mode.is_boolean() { return Err("Invalid local workspace Watch mode.".into()); }
                }
            }
        }
        if let Some(children) = pane.get_mut("children").and_then(serde_json::Value::as_array_mut) {
            for child in children { strip_mode(child, depth + 1)?; }
        }
        Ok(())
    }
    if let Some(center) = projected.get_mut("center") { strip_mode(center, 0)?; }
    for dock in ["left_dock", "right_dock", "bottom_dock"] {
        if let Some(panel) = projected.get_mut(dock).and_then(|dock| dock.get_mut("panel")) {
            strip_mode(panel, 0)?;
        }
    }
    let id = "00000000-0000-4000-8000-000000000001".to_owned();
    validate_native_workspace_set(&NativeWorkspaceSet {
        active_id: id.clone(),
        items: vec![monitter_lib::experimental_desktop::NativeWorkspace {
            id, name: "Projection".into(), color: "#000000".into(),
            icon: "folder".into(), url: None, layout: Some(projected.clone()),
        }],
    })?;
    Ok(projected)
}

/// Validate the backend projection while preserving the complete local JSON.
fn validate_local_workspace_set(set: &NativeWorkspaceSet) -> Result<(), String> {
    let mut projected = set.clone();
    for workspace in &mut projected.items {
        if let Some(layout) = &mut workspace.layout {
            *layout = backend_layout_projection(layout)?;
        }
    }
    validate_native_workspace_set(&projected)
}

fn validate_workspace_id(id: &str) -> Result<(), String> {
    uuid::Uuid::parse_str(id)
        .map(|_| ())
        .map_err(|_| "Invalid saved-layout workspace ID.".to_owned())
}

fn validate_layout_id(id: &str) -> Result<(), String> {
    uuid::Uuid::parse_str(id)
        .map(|_| ())
        .map_err(|_| "Invalid saved-layout ID.".to_owned())
}

fn normalize_label(label: &str) -> Result<String, String> {
    let label = label.trim();
    if label.is_empty() || label.chars().count() > 80 || label.chars().any(char::is_control) {
        return Err("Saved-layout names must contain 1 to 80 visible characters.".into());
    }
    Ok(label.to_owned())
}

/// Reuse the authoritative dock schema while allowing client-local focus
/// metadata, exactly as the continuously saved workspace layout does.
fn validate_saved_layout(layout: &serde_json::Value) -> Result<String, String> {
    let workspace_id = "00000000-0000-4000-8000-000000000001".to_owned();
    let set = NativeWorkspaceSet {
        active_id: workspace_id.clone(),
        items: vec![monitter_lib::experimental_desktop::NativeWorkspace {
            id: workspace_id,
            name: "Saved layout".into(),
            color: "#000000".into(),
            icon: "folder".into(),
            url: None,
            layout: Some(layout.clone()),
        }],
    };
    validate_local_workspace_set(&set)?;
    let serialized = serde_json::to_string(layout).map_err(|error| error.to_string())?;
    if serialized.len() > MAX_SAVED_LAYOUT_BYTES {
        return Err("Saved layout exceeds 256 KiB.".into());
    }
    Ok(serialized)
}

fn now_ms() -> Result<i64, String> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "System clock is before the Unix epoch.".to_owned())?
        .as_millis();
    i64::try_from(millis).map_err(|_| "System clock exceeds the saved-layout range.".to_owned())
}

fn decode_saved_layout(
    id: String,
    workspace_id: String,
    label: String,
    layout: String,
    created_at_ms: i64,
    updated_at_ms: i64,
) -> Result<SavedWorkspaceLayout, String> {
    validate_layout_id(&id)?;
    validate_workspace_id(&workspace_id)?;
    let label = normalize_label(&label)?;
    if created_at_ms < 0 || updated_at_ms < created_at_ms || layout.len() > MAX_SAVED_LAYOUT_BYTES {
        return Err("Saved-layout metadata exceeds its limits or is invalid.".into());
    }
    let layout = serde_json::from_str(&layout).map_err(|error| format!("Saved layout: {error}"))?;
    validate_saved_layout(&layout)?;
    Ok(SavedWorkspaceLayout {
        id,
        workspace_id,
        label,
        layout,
        created_at_ms,
        updated_at_ms,
    })
}

/// Return the arrangements for one project space, most recently changed first.
pub fn load_saved_layouts(workspace_id: &str) -> Result<Vec<SavedWorkspaceLayout>, String> {
    Ok(load_saved_layouts_with_warnings(workspace_id)?.layouts)
}

pub fn load_saved_layouts_with_warnings(
    workspace_id: &str,
) -> Result<SavedWorkspaceLayoutLoad, String> {
    load_saved_layouts_with_warnings_at(&path()?, workspace_id)
}

fn load_saved_layouts_at(
    path: &Path,
    workspace_id: &str,
) -> Result<Vec<SavedWorkspaceLayout>, String> {
    Ok(load_saved_layouts_with_warnings_at(path, workspace_id)?.layouts)
}

fn load_saved_layouts_with_warnings_at(
    path: &Path,
    workspace_id: &str,
) -> Result<SavedWorkspaceLayoutLoad, String> {
    validate_workspace_id(workspace_id)?;
    let db = database(path)?;
    let mut query = db
        .prepare(
            "SELECT id, workspace_id, label, layout, created_at_ms, updated_at_ms
        FROM saved_layouts WHERE workspace_id=?1
        ORDER BY updated_at_ms DESC, created_at_ms DESC, id ASC",
        )
        .map_err(|error| error.to_string())?;
    let records = query
        .query_map([workspace_id], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
            ))
        })
        .map_err(|error| error.to_string())?;
    let mut result = SavedWorkspaceLayoutLoad::default();
    let mut skipped = 0_usize;
    for (index, record) in records.enumerate() {
        let decoded = record.map_err(|error| error.to_string()).and_then(
            |(id, workspace_id, label, layout, created_at_ms, updated_at_ms)| {
                decode_saved_layout(
                    id,
                    workspace_id,
                    label,
                    layout,
                    created_at_ms,
                    updated_at_ms,
                )
            },
        );
        match decoded {
            Ok(layout) if result.layouts.len() < MAX_SAVED_LAYOUTS_PER_WORKSPACE => {
                result.layouts.push(layout);
            }
            Ok(_) => {
                skipped += 1;
                if result.warnings.len() < MAX_SAVED_LAYOUT_WARNINGS {
                    result.warnings.push(format!(
                        "Skipped saved layout row {} because the workspace limit is {}.",
                        index + 1,
                        MAX_SAVED_LAYOUTS_PER_WORKSPACE
                    ));
                }
            }
            Err(error) => {
                skipped += 1;
                if result.warnings.len() < MAX_SAVED_LAYOUT_WARNINGS {
                    result.warnings.push(format!(
                        "Skipped invalid saved layout row {}: {error}",
                        index + 1
                    ));
                }
            }
        }
    }
    if skipped > MAX_SAVED_LAYOUT_WARNINGS {
        result.warnings.truncate(MAX_SAVED_LAYOUT_WARNINGS - 1);
        let suppressed = skipped - result.warnings.len();
        result.warnings.push(format!(
            "Skipped {suppressed} additional invalid saved layout row(s)."
        ));
    }
    Ok(result)
}

/// Capture a dock arrangement for an existing named workspace. This stores
/// presentation only and never copies the panes' backend-owned state.
pub fn capture_saved_layout(
    workspace_id: &str,
    label: &str,
    layout: &serde_json::Value,
) -> Result<SavedWorkspaceLayout, String> {
    capture_saved_layout_at(&path()?, workspace_id, label, layout)
}

fn capture_saved_layout_at(
    path: &Path,
    workspace_id: &str,
    label: &str,
    layout: &serde_json::Value,
) -> Result<SavedWorkspaceLayout, String> {
    validate_workspace_id(workspace_id)?;
    let label = normalize_label(label)?;
    let layout_json = validate_saved_layout(layout)?;
    let id = uuid::Uuid::new_v4().to_string();
    let timestamp = now_ms()?;
    let mut db = database(path)?;
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    let count: i64 = tx
        .query_row(
            "SELECT COUNT(*) FROM saved_layouts WHERE workspace_id=?1",
            [workspace_id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if count >= MAX_SAVED_LAYOUTS_PER_WORKSPACE as i64 {
        return Err(format!(
            "A workspace can save at most {MAX_SAVED_LAYOUTS_PER_WORKSPACE} layouts."
        ));
    }
    tx.execute(
        "INSERT INTO saved_layouts(id,workspace_id,label,layout,created_at_ms,updated_at_ms)
        VALUES(?1,?2,?3,?4,?5,?5)",
        params![id, workspace_id, label, layout_json, timestamp],
    )
    .map_err(|error| {
        if error.to_string().contains("UNIQUE constraint failed") {
            "A saved layout with this name already exists in the workspace.".to_owned()
        } else {
            error.to_string()
        }
    })?;
    tx.commit().map_err(|error| error.to_string())?;
    Ok(SavedWorkspaceLayout {
        id,
        workspace_id: workspace_id.to_owned(),
        label,
        layout: layout.clone(),
        created_at_ms: timestamp,
        updated_at_ms: timestamp,
    })
}

pub fn rename_saved_layout(
    workspace_id: &str,
    layout_id: &str,
    label: &str,
) -> Result<SavedWorkspaceLayout, String> {
    rename_saved_layout_at(&path()?, workspace_id, layout_id, label)
}

fn rename_saved_layout_at(
    path: &Path,
    workspace_id: &str,
    layout_id: &str,
    label: &str,
) -> Result<SavedWorkspaceLayout, String> {
    validate_workspace_id(workspace_id)?;
    validate_layout_id(layout_id)?;
    let label = normalize_label(label)?;
    let timestamp = now_ms()?;
    let db = database(path)?;
    let changed = db
        .execute(
            "UPDATE saved_layouts SET label=?3, updated_at_ms=?4
        WHERE id=?1 AND workspace_id=?2",
            params![layout_id, workspace_id, label, timestamp],
        )
        .map_err(|error| {
            if error.to_string().contains("UNIQUE constraint failed") {
                "A saved layout with this name already exists in the workspace.".to_owned()
            } else {
                error.to_string()
            }
        })?;
    if changed != 1 {
        return Err("Saved layout does not exist in this workspace.".into());
    }
    load_saved_layouts_at(path, workspace_id)?
        .into_iter()
        .find(|layout| layout.id == layout_id)
        .ok_or_else(|| "Saved layout does not exist in this workspace.".to_owned())
}

pub fn delete_saved_layout(workspace_id: &str, layout_id: &str) -> Result<bool, String> {
    delete_saved_layout_at(&path()?, workspace_id, layout_id)
}

fn delete_saved_layout_at(
    path: &Path,
    workspace_id: &str,
    layout_id: &str,
) -> Result<bool, String> {
    validate_workspace_id(workspace_id)?;
    validate_layout_id(layout_id)?;
    let db = database(path)?;
    db.execute(
        "DELETE FROM saved_layouts WHERE id=?1 AND workspace_id=?2",
        params![layout_id, workspace_id],
    )
    .map(|changed| changed == 1)
    .map_err(|error| error.to_string())
}

pub fn load() -> Result<
    (
        i64,
        Option<NativeWorkspaceSet>,
        Option<NativeTileTheme>,
        bool,
    ),
    String,
> {
    load_at(&path()?)
}
fn load_at(
    path: &Path,
) -> Result<
    (
        i64,
        Option<NativeWorkspaceSet>,
        Option<NativeTileTheme>,
        bool,
    ),
    String,
> {
    let db = database(path)?;
    let row: Option<(i64, String)> = db
        .query_row(
            "SELECT revision, payload FROM workspaces WHERE id=1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    match row {
        None => Ok((0, None, None, true)),
        Some((revision, payload)) => {
            if revision <= 0 || payload.len() > MAX_BYTES {
                return Err("Workspace store exceeds its limits or is invalid.".into());
            }
            let value: serde_json::Value =
                serde_json::from_str(&payload).map_err(|e| format!("Saved workspaces: {e}"))?;
            let (set, theme, auto_size_panes) = if value.get("items").is_some() {
                (
                    serde_json::from_value(value.clone()).map_err(|e| e.to_string())?,
                    None,
                    true,
                )
            } else {
                if value["version"] != 1 {
                    return Err("Unsupported local workspace version.".into());
                }
                (
                    serde_json::from_value(value["workspaces"].clone())
                        .map_err(|e| e.to_string())?,
                    Some(
                        serde_json::from_value(value["tileTheme"].clone())
                            .map_err(|e| e.to_string())?,
                    ),
                    value["autoSizePanes"].as_bool().unwrap_or(true),
                )
            };
            validate_local_workspace_set(&set)?;
            Ok((revision, Some(set), theme, auto_size_panes))
        }
    }
}
pub fn save(
    revision: i64,
    set: &NativeWorkspaceSet,
    theme: NativeTileTheme,
    auto_size_panes: bool,
) -> Result<i64, String> {
    save_at(&path()?, revision, set, theme, auto_size_panes)
}
fn save_at(
    path: &Path,
    revision: i64,
    set: &NativeWorkspaceSet,
    theme: NativeTileTheme,
    auto_size_panes: bool,
) -> Result<i64, String> {
    validate_local_workspace_set(set)?;
    let payload =
        serde_json::to_string(&serde_json::json!({"version":1,"workspaces":set,"tileTheme":theme,"autoSizePanes":auto_size_panes}))
            .map_err(|e| e.to_string())?;
    if payload.len() > MAX_BYTES {
        return Err("Named workspaces exceed 512 KiB. Reduce the saved layouts.".into());
    }
    let mut db = database(path)?;
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    let current: i64 = tx
        .query_row("SELECT revision FROM workspaces WHERE id=1", [], |row| {
            row.get(0)
        })
        .optional()
        .map_err(|e| e.to_string())?
        .unwrap_or(0);
    if current != revision {
        return Err("Workspaces changed in another window. Reload saved workspace preferences before retrying.".into());
    }
    let next = current
        .checked_add(1)
        .ok_or("Workspace revision limit reached.")?;
    tx.execute("INSERT INTO workspaces(id,revision,payload) VALUES(1,?1,?2) ON CONFLICT(id) DO UPDATE SET revision=excluded.revision,payload=excluded.payload",params![next,payload]).map_err(|e|e.to_string())?;
    let placeholders = (1..=set.items.len())
        .map(|index| format!("?{index}"))
        .collect::<Vec<_>>()
        .join(",");
    tx.execute(
        &format!("DELETE FROM saved_layouts WHERE workspace_id NOT IN ({placeholders})"),
        rusqlite::params_from_iter(set.items.iter().map(|workspace| &workspace.id)),
    )
    .map_err(|error| error.to_string())?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(next)
}
#[cfg(test)]
mod tests {
    use super::{
        MAX_SAVED_LAYOUTS_PER_WORKSPACE, capture_saved_layout_at, delete_saved_layout_at, load_at,
        load_saved_layouts_at, load_saved_layouts_with_warnings_at, rename_saved_layout_at,
        save_at,
    };
    use monitter_lib::experimental_desktop::{
        NativeTileTheme, NativeWorkspace, NativeWorkspaceSet,
    };
    fn fixture() -> NativeWorkspaceSet {
        let id = "11111111-1111-4111-8111-111111111111".to_owned();
        NativeWorkspaceSet {
            active_id: id.clone(),
            items: vec![NativeWorkspace {
                id,
                name: "Dev".into(),
                icon: "code".into(),
                color: "#41C968".into(),
                url: Some("https://example.com/project".into()),
                layout: None,
            }],
        }
    }
    fn temp() -> std::path::PathBuf {
        std::fs::canonicalize(std::env::temp_dir())
            .unwrap()
            .join(format!(
                "workspace-store-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ))
            .join("workspaces.sqlite3")
    }
    fn layout(id: &str) -> serde_json::Value {
        serde_json::json!({
            "version": 1,
            "focusedPaneId": id,
            "center": {
                "panel_name": "MonitterNativePane",
                "children": [],
                "info": {"panel": {"id": id}}
            }
        })
    }
    #[test]
    fn pane_mode_local_workspace_and_saved_layout_roundtrip_preserve_metadata() {
        let path = temp();
        let mut set = fixture();
        let mut saved_layout = layout("chat:watch");
        saved_layout["center"]["info"]["panel"]["watchMode"] = serde_json::json!(true);
        set.items[0].layout = Some(saved_layout.clone());
        save_at(&path, 0, &set, NativeTileTheme::Tern, true).unwrap();
        assert_eq!(load_at(&path).unwrap().1, Some(set.clone()));
        let saved = capture_saved_layout_at(&path, &set.active_id, "Watch", &saved_layout).unwrap();
        assert_eq!(saved.layout, saved_layout);
        assert_eq!(load_saved_layouts_at(&path, &set.active_id).unwrap(), vec![saved]);
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn pane_mode_backend_projection_strips_only_valid_client_leaf_metadata() {
        let mut original = layout("chat:watch");
        original["center"]["info"]["panel"]["watchMode"] = serde_json::json!(true);
        let mut raw_set = fixture();
        raw_set.items[0].layout = Some(original.clone());
        assert!(monitter_lib::experimental_desktop::validate_native_workspace_set(&raw_set).is_err(),
            "unprojected client metadata reproduces resident backend validation failure");
        let projected = super::backend_layout_projection(&original).unwrap();
        assert!(projected.get("focusedPaneId").is_none());
        assert!(projected["center"]["info"]["panel"].get("watchMode").is_none());
        assert_eq!(original["center"]["info"]["panel"]["watchMode"], true);
        for invalid in [serde_json::json!("watch"), serde_json::json!(0), serde_json::Value::Null] {
            let mut bad = original.clone();
            bad["center"]["info"]["panel"]["watchMode"] = invalid;
            assert!(super::backend_layout_projection(&bad).is_err());
            assert!(super::validate_saved_layout(&bad).is_err());
        }
        let mut unknown = original.clone();
        unknown["center"]["info"]["panel"]["futureMode"] = serde_json::json!(true);
        assert!(super::backend_layout_projection(&unknown).is_err());
        let mut misplaced = layout("chat:watch");
        misplaced["center"]["watchMode"] = serde_json::json!(true);
        assert!(super::backend_layout_projection(&misplaced).is_err());
        let mut chat = original.clone();
        chat["center"]["info"]["panel"]["watchMode"] = serde_json::json!(false);
        assert!(super::backend_layout_projection(&chat).is_ok());
        assert!(super::backend_layout_projection(&layout("chat:legacy")).is_ok());
    }

    #[test]
    fn missing_auto_size_preference_defaults_on_and_off_roundtrips() {
        let path = temp();
        let set = fixture();
        let revision = save_at(&path, 0, &set, NativeTileTheme::Tern, true).unwrap();

        // Simulate a version-1 local record written before autoSizePanes existed.
        let db = super::database(&path).unwrap();
        let payload: String = db
            .query_row("SELECT payload FROM workspaces WHERE id=1", [], |row| {
                row.get(0)
            })
            .unwrap();
        let mut value: serde_json::Value = serde_json::from_str(&payload).unwrap();
        value.as_object_mut().unwrap().remove("autoSizePanes");
        db.execute(
            "UPDATE workspaces SET payload=?1 WHERE id=1",
            [serde_json::to_string(&value).unwrap()],
        )
        .unwrap();
        drop(db);

        let loaded = load_at(&path).unwrap();
        assert_eq!(loaded.0, revision);
        assert_eq!(loaded.1, Some(set.clone()));
        assert_eq!(loaded.2, Some(NativeTileTheme::Tern));
        assert!(loaded.3, "legacy preference should default to enabled");

        let next_revision = save_at(&path, revision, &set, NativeTileTheme::Tern, false).unwrap();
        assert!(
            !load_at(&path).unwrap().3,
            "explicitly disabled value should persist"
        );
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
        assert_eq!(next_revision, revision + 1);
    }
    #[test]
    fn workspace_store_persists_active_layout_and_rejects_stale_writer() {
        let path = temp();
        let mut set = fixture();
        assert_eq!(load_at(&path).unwrap(), (0, None, None, true));
        set.items[0].layout = Some(
            serde_json::json!({"version":1,"center":{"panel_name":"StackPanel","children":[],"info":{"stack":{"sizes":[],"axis":0}}}}),
        );
        let revision = save_at(&path, 0, &set, NativeTileTheme::Tern, false).unwrap();
        assert_eq!(
            load_at(&path).unwrap(),
            (
                revision,
                Some(set.clone()),
                Some(NativeTileTheme::Tern),
                false
            )
        );
        let mut other = set.clone();
        other.items[0].name = "Other".into();
        assert!(
            save_at(&path, 0, &other, NativeTileTheme::Monitter, true)
                .unwrap_err()
                .contains("another window")
        );
        assert_eq!(load_at(&path).unwrap().1, Some(set));
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
    #[test]
    fn workspace_store_rejects_invalid_metadata_without_overwriting() {
        let path = temp();
        let set = fixture();
        let revision = save_at(&path, 0, &set, NativeTileTheme::Tern, true).unwrap();
        let mut bad = set.clone();
        bad.active_id = "missing".into();
        assert!(save_at(&path, revision, &bad, NativeTileTheme::Monitter, true).is_err());
        assert_eq!(
            load_at(&path).unwrap(),
            (revision, Some(set), Some(NativeTileTheme::Tern), true)
        );
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
    #[test]
    fn reordered_and_removed_workspaces_survive_reload() {
        let path = temp();
        let mut set = fixture();
        let first = set.items[0].id.clone();
        let mut second = set.items[0].clone();
        second.id = "22222222-2222-4222-8222-222222222222".into();
        second.name = "Second".into();
        let second_id = second.id.clone();
        set.items.push(second);
        let revision = save_at(&path, 0, &set, NativeTileTheme::Tern, true).unwrap();
        assert!(crate::workspace_order::reorder(
            &mut set, &first, &second_id
        ));
        let revision = save_at(&path, revision, &set, NativeTileTheme::Tern, true).unwrap();
        assert_eq!(load_at(&path).unwrap().1, Some(set.clone()));
        assert!(crate::workspace_order::remove(&mut set, &first));
        save_at(&path, revision, &set, NativeTileTheme::Tern, true).unwrap();
        assert_eq!(load_at(&path).unwrap().1, Some(set));
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
    #[test]
    fn workspace_store_roundtrips_local_focus_and_keeps_dock_schema_strict() {
        let path = temp();
        let mut set = fixture();
        set.items[0].layout = Some(serde_json::json!({
            "version":1,
            "focusedPaneId":"chat:main",
            "center":{"panel_name":"MonitterNativePane","children":[],"info":{"panel":{"id":"chat:main"}}}
        }));
        let revision = save_at(&path, 0, &set, NativeTileTheme::Tern, true).unwrap();
        assert_eq!(load_at(&path).unwrap().1, Some(set.clone()));
        for invalid in [serde_json::json!(7), serde_json::json!("chat:bad id")] {
            let mut bad = set.clone();
            bad.items[0].layout.as_mut().unwrap()["focusedPaneId"] = invalid;
            assert!(save_at(&path, revision, &bad, NativeTileTheme::Tern, true).is_err());
        }
        let mut bad = set.clone();
        bad.items[0].layout.as_mut().unwrap()["unknownClientField"] = serde_json::json!(true);
        assert!(save_at(&path, revision, &bad, NativeTileTheme::Tern, true).is_err());
        assert_eq!(load_at(&path).unwrap().1, Some(set.clone()));
        set.items[0].layout.as_mut().unwrap()["focusedPaneId"] = serde_json::json!("chat:removed");
        save_at(&path, revision, &set, NativeTileTheme::Tern, true).unwrap();
        assert_eq!(load_at(&path).unwrap().1, Some(set));
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn saved_layouts_are_scoped_named_and_manageable_without_changing_live_workspaces() {
        let path = temp();
        let set = fixture();
        let workspace_id = set.items[0].id.clone();
        let revision = save_at(&path, 0, &set, NativeTileTheme::Tern, true).unwrap();
        let saved =
            capture_saved_layout_at(&path, &workspace_id, "  Review  ", &layout("chat:main"))
                .unwrap();
        assert_eq!(saved.workspace_id, workspace_id);
        assert_eq!(saved.label, "Review");
        assert!(uuid::Uuid::parse_str(&saved.id).is_ok());
        assert_eq!(
            load_saved_layouts_at(&path, &workspace_id).unwrap(),
            vec![saved.clone()]
        );

        let renamed = rename_saved_layout_at(&path, &workspace_id, &saved.id, "Ship").unwrap();
        assert_eq!(
            renamed.id, saved.id,
            "rename must retain the stable identity"
        );
        assert_eq!(renamed.label, "Ship");
        assert_eq!(renamed.layout, saved.layout);
        assert_eq!(
            load_at(&path).unwrap(),
            (revision, Some(set), Some(NativeTileTheme::Tern), true)
        );

        let other_workspace = "22222222-2222-4222-8222-222222222222";
        assert!(rename_saved_layout_at(&path, other_workspace, &saved.id, "Wrong owner").is_err());
        assert!(!delete_saved_layout_at(&path, other_workspace, &saved.id).unwrap());
        assert!(delete_saved_layout_at(&path, &workspace_id, &saved.id).unwrap());
        assert!(!delete_saved_layout_at(&path, &workspace_id, &saved.id).unwrap());
        assert!(
            load_saved_layouts_at(&path, &workspace_id)
                .unwrap()
                .is_empty()
        );
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn stale_workspace_removal_preserves_saved_layouts() {
        let path = temp();
        let mut set = fixture();
        let first = set.items[0].id.clone();
        let mut second = set.items[0].clone();
        second.id = "22222222-2222-4222-8222-222222222222".into();
        second.name = "Second".into();
        let second_id = second.id.clone();
        set.items.push(second);
        let _revision = save_at(&path, 0, &set, NativeTileTheme::Tern, true).unwrap();
        capture_saved_layout_at(&path, &first, "First", &layout("route:projects")).unwrap();
        capture_saved_layout_at(&path, &second_id, "Second", &layout("route:archives")).unwrap();

        let mut removed = set.clone();
        removed.items.remove(0);
        removed.active_id = second_id.clone();
        assert!(
            save_at(&path, 0, &removed, NativeTileTheme::Tern, true)
                .unwrap_err()
                .contains("another window")
        );
        assert_eq!(load_saved_layouts_at(&path, &first).unwrap().len(), 1);
        assert_eq!(load_saved_layouts_at(&path, &second_id).unwrap().len(), 1);
        assert_eq!(load_at(&path).unwrap().1, Some(set));
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn successful_workspace_removal_atomically_deletes_only_its_saved_layouts() {
        let path = temp();
        let mut set = fixture();
        let first = set.items[0].id.clone();
        let mut second = set.items[0].clone();
        second.id = "22222222-2222-4222-8222-222222222222".into();
        second.name = "Second".into();
        let second_id = second.id.clone();
        set.items.push(second);
        let revision = save_at(&path, 0, &set, NativeTileTheme::Tern, true).unwrap();
        capture_saved_layout_at(&path, &first, "First", &layout("route:projects")).unwrap();
        capture_saved_layout_at(&path, &second_id, "Second", &layout("route:archives")).unwrap();
        let mut removed = set;
        removed.items.remove(0);
        removed.active_id = second_id.clone();
        let next = save_at(&path, revision, &removed, NativeTileTheme::Tern, true).unwrap();
        assert_eq!(next, revision + 1);
        assert!(load_saved_layouts_at(&path, &first).unwrap().is_empty());
        assert_eq!(load_saved_layouts_at(&path, &second_id).unwrap().len(), 1);
        assert_eq!(load_at(&path).unwrap().1, Some(removed));
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn corrupt_saved_layout_row_is_skipped_without_hiding_valid_rows() {
        let path = temp();
        let workspace_id = fixture().items[0].id.clone();
        let first =
            capture_saved_layout_at(&path, &workspace_id, "First", &layout("chat:first")).unwrap();
        let second =
            capture_saved_layout_at(&path, &workspace_id, "Second", &layout("chat:second"))
                .unwrap();
        let db = super::database(&path).unwrap();
        db.execute(
            "INSERT INTO saved_layouts(id,workspace_id,label,layout,created_at_ms,updated_at_ms)
             VALUES(?1,?2,'Corrupt','{',1,1)",
            rusqlite::params![uuid::Uuid::new_v4().to_string(), workspace_id],
        )
        .unwrap();
        drop(db);

        let loaded = load_saved_layouts_with_warnings_at(&path, &workspace_id).unwrap();
        assert_eq!(loaded.layouts.len(), 2);
        assert!(loaded.layouts.iter().any(|layout| layout.id == first.id));
        assert!(loaded.layouts.iter().any(|layout| layout.id == second.id));
        assert_eq!(loaded.warnings.len(), 1);
        assert!(loaded.warnings[0].contains("Skipped invalid saved layout row"));
        assert_eq!(
            load_saved_layouts_at(&path, &workspace_id).unwrap().len(),
            2
        );
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn existing_version_one_workspace_rows_load_before_and_after_saved_layout_migration() {
        let path = temp();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let db = rusqlite::Connection::open(&path).unwrap();
        db.execute_batch("CREATE TABLE workspaces (id INTEGER PRIMARY KEY CHECK(id = 1), revision INTEGER NOT NULL, payload TEXT NOT NULL)").unwrap();
        let set = fixture();
        db.execute(
            "INSERT INTO workspaces(id,revision,payload) VALUES(1,7,?1)",
            [serde_json::to_string(&set).unwrap()],
        )
        .unwrap();
        drop(db);

        assert_eq!(load_at(&path).unwrap(), (7, Some(set.clone()), None, true));
        let workspace_id = &set.items[0].id;
        assert!(
            load_saved_layouts_at(&path, workspace_id)
                .unwrap()
                .is_empty()
        );
        capture_saved_layout_at(&path, workspace_id, "Review", &layout("route:projects")).unwrap();
        assert_eq!(load_at(&path).unwrap(), (7, Some(set), None, true));
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn saved_layout_limits_and_schema_are_enforced_without_partial_writes() {
        let path = temp();
        let workspace_id = fixture().items[0].id.clone();
        assert!(
            capture_saved_layout_at(
                &path,
                &workspace_id,
                "Bad",
                &serde_json::json!({"version": 99})
            )
            .is_err()
        );
        assert!(
            load_saved_layouts_at(&path, &workspace_id)
                .unwrap()
                .is_empty()
        );

        capture_saved_layout_at(&path, &workspace_id, "Layout 1", &layout("terminal:first"))
            .unwrap();
        assert!(
            capture_saved_layout_at(&path, &workspace_id, "layout 1", &layout("chat:duplicate"))
                .unwrap_err()
                .contains("already exists")
        );

        for index in 1..MAX_SAVED_LAYOUTS_PER_WORKSPACE {
            capture_saved_layout_at(
                &path,
                &workspace_id,
                &format!("Layout {}", index + 1),
                &layout(&format!("terminal:{index}")),
            )
            .unwrap();
        }
        assert_eq!(
            load_saved_layouts_at(&path, &workspace_id).unwrap().len(),
            MAX_SAVED_LAYOUTS_PER_WORKSPACE
        );
        assert!(
            capture_saved_layout_at(&path, &workspace_id, "One too many", &layout("chat:extra"))
                .unwrap_err()
                .contains("at most")
        );

        let layouts = load_saved_layouts_at(&path, &workspace_id).unwrap();
        assert_eq!(layouts.len(), MAX_SAVED_LAYOUTS_PER_WORKSPACE);
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
}
