//! Client-local workspace layouts. Compatible with already-running backend schemas.
//! All IO runs off the UI thread; SQLite transactions guard concurrent GUI writers.
use monitter_lib::experimental_desktop::{
    NativeTileTheme, NativeWorkspaceSet, validate_native_workspace_set,
};
use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior, params};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
const MAX_BYTES: usize = 512 * 1024;
const DATABASE: &str = "named-workspaces.sqlite3";
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
    connection.execute_batch("CREATE TABLE IF NOT EXISTS workspaces (id INTEGER PRIMARY KEY CHECK(id = 1), revision INTEGER NOT NULL, payload TEXT NOT NULL)")
        .map_err(|e| e.to_string())?;
    Ok(connection)
}
/// Focus is GPUI client metadata, not part of the resident backend dock schema.
/// Validate a projection but persist the original so restoring focus survives.
fn validate_local_workspace_set(set: &NativeWorkspaceSet) -> Result<(), String> {
    let mut projected = set.clone();
    for workspace in &mut projected.items {
        if let Some(layout) = &mut workspace.layout {
            let focus = layout
                .as_object_mut()
                .and_then(|layout| layout.remove("focusedPaneId"));
            if let Some(focus) = focus {
                let valid = focus.as_str().is_some_and(|id| {
                    !id.is_empty()
                        && id.len() <= 128
                        && id
                            .bytes()
                            .all(|byte| byte.is_ascii_alphanumeric() || b"-_:./".contains(&byte))
                });
                if !valid {
                    return Err("Invalid local workspace focused pane identity.".into());
                }
                // A remembered pane can have since been removed. Restore
                // revalidates membership and falls back to surviving panes.
            }
        }
    }
    validate_native_workspace_set(&projected)
}
pub fn load() -> Result<(i64, Option<NativeWorkspaceSet>, Option<NativeTileTheme>, bool), String> {
    load_at(&path()?)
}
fn load_at(
    path: &Path,
) -> Result<(i64, Option<NativeWorkspaceSet>, Option<NativeTileTheme>, bool), String> {
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
    tx.commit().map_err(|e| e.to_string())?;
    Ok(next)
}
#[cfg(test)]
mod tests {
    use super::{load_at, save_at};
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
                url: Some("https://example.com/project".into()), layout: None,
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
    #[test]
    fn missing_auto_size_preference_defaults_on_and_off_roundtrips() {
        let path = temp();
        let set = fixture();
        let revision = save_at(&path, 0, &set, NativeTileTheme::Tern, true).unwrap();

        // Simulate a version-1 local record written before autoSizePanes existed.
        let db = super::database(&path).unwrap();
        let payload: String = db
            .query_row("SELECT payload FROM workspaces WHERE id=1", [], |row| row.get(0))
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
        assert!(!load_at(&path).unwrap().3, "explicitly disabled value should persist");
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
            (revision, Some(set.clone()), Some(NativeTileTheme::Tern), false)
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
        let path = temp(); let mut set = fixture();
        let first = set.items[0].id.clone();
        let mut second = set.items[0].clone();
        second.id = "22222222-2222-4222-8222-222222222222".into(); second.name = "Second".into();
        let second_id = second.id.clone(); set.items.push(second);
        let revision = save_at(&path, 0, &set, NativeTileTheme::Tern, true).unwrap();
        assert!(crate::workspace_order::reorder(&mut set, &first, &second_id));
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

}
