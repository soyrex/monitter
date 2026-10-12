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
pub fn load() -> Result<(i64, Option<NativeWorkspaceSet>, Option<NativeTileTheme>), String> {
    load_at(&path()?)
}
fn load_at(
    path: &Path,
) -> Result<(i64, Option<NativeWorkspaceSet>, Option<NativeTileTheme>), String> {
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
        None => Ok((0, None, None)),
        Some((revision, payload)) => {
            if revision <= 0 || payload.len() > MAX_BYTES {
                return Err("Workspace store exceeds its limits or is invalid.".into());
            }
            let value: serde_json::Value =
                serde_json::from_str(&payload).map_err(|e| format!("Saved workspaces: {e}"))?;
            let (set, theme) = if value.get("items").is_some() {
                (
                    serde_json::from_value(value).map_err(|e| e.to_string())?,
                    None,
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
                )
            };
            validate_native_workspace_set(&set)?;
            Ok((revision, Some(set), theme))
        }
    }
}
pub fn save(
    revision: i64,
    set: &NativeWorkspaceSet,
    theme: NativeTileTheme,
) -> Result<i64, String> {
    save_at(&path()?, revision, set, theme)
}
fn save_at(
    path: &Path,
    revision: i64,
    set: &NativeWorkspaceSet,
    theme: NativeTileTheme,
) -> Result<i64, String> {
    validate_native_workspace_set(set)?;
    let payload =
        serde_json::to_string(&serde_json::json!({"version":1,"workspaces":set,"tileTheme":theme}))
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
    #[test]
    fn workspace_store_persists_active_layout_and_rejects_stale_writer() {
        let path = temp();
        let mut set = fixture();
        assert_eq!(load_at(&path).unwrap(), (0, None, None));
        set.items[0].layout = Some(
            serde_json::json!({"version":1,"center":{"panel_name":"StackPanel","children":[],"info":{"stack":{"sizes":[],"axis":0}}}}),
        );
        let revision = save_at(&path, 0, &set, NativeTileTheme::Tern).unwrap();
        assert_eq!(
            load_at(&path).unwrap(),
            (revision, Some(set.clone()), Some(NativeTileTheme::Tern))
        );
        let mut other = set.clone();
        other.items[0].name = "Other".into();
        assert!(
            save_at(&path, 0, &other, NativeTileTheme::Monitter)
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
        let revision = save_at(&path, 0, &set, NativeTileTheme::Tern).unwrap();
        let mut bad = set.clone();
        bad.active_id = "missing".into();
        assert!(save_at(&path, revision, &bad, NativeTileTheme::Monitter).is_err());
        assert_eq!(
            load_at(&path).unwrap(),
            (revision, Some(set), Some(NativeTileTheme::Tern))
        );
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
}
