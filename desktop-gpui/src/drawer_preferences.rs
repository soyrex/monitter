//! Device-local drawer sizes. Call these blocking SQLite operations off the UI thread.
use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior, params};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};

const DATABASE: &str = "drawer-preferences.sqlite3";
const MAX_ENTRIES: i64 = 512;

fn path() -> Result<PathBuf, String> {
    Ok(monitter_lib::experimental_desktop::profile_dir()?.join(DATABASE))
}
fn validate_task(task_id: &str) -> Result<(), String> {
    if task_id.len() != 36
        || !task_id.bytes().enumerate().all(|(i, byte)| {
            if matches!(i, 8 | 13 | 18 | 23) {
                byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        })
    {
        return Err("Drawer preference task identity must be a UUID.".into());
    }
    Ok(())
}
fn validate_fraction(fraction: f32) -> Result<(), String> {
    if !fraction.is_finite() || !(0.1..=0.8).contains(&fraction) {
        return Err("Drawer height fraction must be between 0.1 and 0.8.".into());
    }
    Ok(())
}
fn database(path: &Path) -> Result<Connection, String> {
    let parent = path.parent().ok_or("Drawer store has no directory.")?;
    std::fs::create_dir_all(parent).map_err(|e| format!("Drawer directory: {e}"))?;
    if std::fs::symlink_metadata(parent)
        .map_err(|e| e.to_string())?
        .file_type()
        .is_symlink()
    {
        return Err("Drawer directory cannot be a symbolic link.".into());
    }
    let mut db = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )
    .map_err(|e| format!("Drawer store: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| e.to_string())?;
    }
    db.busy_timeout(Duration::from_secs(2))
        .map_err(|e| e.to_string())?;
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS drawer_preferences (
        task_id TEXT PRIMARY KEY, fraction REAL NOT NULL,
        write_order INTEGER NOT NULL UNIQUE, request_version INTEGER NOT NULL DEFAULT 0)",
    )
    .map_err(|e| e.to_string())?;
    // Serialize the migration with other GUI writers opening the same store.
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    let has_version: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info('drawer_preferences') WHERE name = 'request_version')",
        [], |row| row.get(0)).map_err(|e| e.to_string())?;
    if !has_version {
        tx.execute_batch(
            "ALTER TABLE drawer_preferences ADD COLUMN request_version INTEGER NOT NULL DEFAULT 0",
        )
        .map_err(|e| e.to_string())?;
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(db)
}
pub fn load(task_id: &str) -> Result<Option<f32>, String> {
    validate_task(task_id)?;
    load_at(&path()?, task_id)
}
fn load_at(path: &Path, task_id: &str) -> Result<Option<f32>, String> {
    validate_task(task_id)?;
    let db = database(path)?;
    let fraction: Option<f32> = db
        .query_row(
            "SELECT fraction FROM drawer_preferences WHERE task_id = ?1",
            [task_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    if let Some(fraction) = fraction {
        validate_fraction(fraction)?;
    }
    Ok(fraction)
}
fn request_version() -> Result<i64, String> {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    i64::try_from(nanos).map_err(|_| "Drawer request timestamp exceeds its limits.".into())
}
pub fn save(task_id: &str, fraction: f32) -> Result<(), String> {
    save_versioned(task_id, fraction, request_version()?)
}
/// Capture `version` at the UI gesture release, before scheduling the writer.
pub fn save_versioned(task_id: &str, fraction: f32, version: i64) -> Result<(), String> {
    validate_task(task_id)?;
    validate_fraction(fraction)?;
    if version < 0 {
        return Err("Drawer request version cannot be negative.".into());
    }
    save_versioned_at(&path()?, task_id, fraction, version)
}
#[cfg(test)]
fn save_at(path: &Path, task_id: &str, fraction: f32) -> Result<(), String> {
    save_versioned_at(path, task_id, fraction, request_version()?)
}
fn save_versioned_at(
    path: &Path,
    task_id: &str,
    fraction: f32,
    version: i64,
) -> Result<(), String> {
    validate_task(task_id)?;
    validate_fraction(fraction)?;
    if version < 0 {
        return Err("Drawer request version cannot be negative.".into());
    }
    let mut db = database(path)?;
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    tx.execute("INSERT INTO drawer_preferences (task_id, fraction, write_order, request_version)
        VALUES (?1, ?2, (SELECT COALESCE(MAX(write_order), 0) + 1 FROM drawer_preferences), ?3)
        ON CONFLICT(task_id) DO UPDATE SET fraction = excluded.fraction, write_order = excluded.write_order, request_version = excluded.request_version
        WHERE excluded.request_version >= drawer_preferences.request_version",
        params![task_id, fraction, version]).map_err(|e| e.to_string())?;
    tx.execute(
        "DELETE FROM drawer_preferences WHERE task_id IN (
        SELECT task_id FROM drawer_preferences ORDER BY write_order DESC LIMIT -1 OFFSET ?1)",
        [MAX_ENTRIES],
    )
    .map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let nonce = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            Self(
                std::env::temp_dir()
                    // macOS commonly supplies /var/folders through the /var
                    // symlink. SQLite NOFOLLOW checks the complete filename;
                    // canonicalize only the trusted existing temp parent so
                    // the explicit file/parent symlink rejection tests remain.
                    .canonicalize()
                    .expect("canonical temporary directory")
                    .join(format!("monitter-drawer-{}-{nonce}", std::process::id()))
                    .join(DATABASE),
            )
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(self.0.parent().unwrap());
        }
    }
    fn id(number: usize) -> String {
        format!("00000000-0000-0000-0000-{number:012x}")
    }
    #[test]
    fn independently_roundtrips_panes_and_missing_defaults() {
        let fixture = Fixture::new();
        assert_eq!(load_at(&fixture.0, &id(1)).unwrap(), None);
        save_at(&fixture.0, &id(1), 0.5).unwrap();
        save_at(&fixture.0, &id(2), 0.8).unwrap();
        save_at(&fixture.0, &id(1), 0.1).unwrap();
        assert_eq!(load_at(&fixture.0, &id(1)).unwrap(), Some(0.1));
        assert_eq!(load_at(&fixture.0, &id(2)).unwrap(), Some(0.8));
    }
    #[test]
    fn older_off_thread_write_does_not_regress_latest_gesture() {
        let fixture = Fixture::new();
        save_versioned_at(&fixture.0, &id(1), 0.7, 200).unwrap();
        save_versioned_at(&fixture.0, &id(1), 0.3, 100).unwrap();
        assert_eq!(load_at(&fixture.0, &id(1)).unwrap(), Some(0.7));
        save_versioned_at(&fixture.0, &id(1), 0.6, 300).unwrap();
        assert_eq!(load_at(&fixture.0, &id(1)).unwrap(), Some(0.6));
    }
    #[test]
    fn invalid_inputs_do_not_create_store() {
        let fixture = Fixture::new();
        assert!(load_at(&fixture.0, "invalid").is_err());
        for fraction in [f32::NAN, f32::INFINITY, 0.09, 0.81] {
            assert!(save_at(&fixture.0, &id(1), fraction).is_err());
        }
        assert!(!fixture.0.exists());
    }
    #[test]
    fn bound_retains_latest_writes_including_updated_rows() {
        let fixture = Fixture::new();
        for number in 0..512 {
            save_at(&fixture.0, &id(number), 0.5).unwrap();
        }
        save_at(&fixture.0, &id(0), 0.7).unwrap();
        save_at(&fixture.0, &id(512), 0.6).unwrap();
        assert_eq!(load_at(&fixture.0, &id(0)).unwrap(), Some(0.7));
        assert_eq!(load_at(&fixture.0, &id(1)).unwrap(), None);
        assert_eq!(load_at(&fixture.0, &id(512)).unwrap(), Some(0.6));
        let db = database(&fixture.0).unwrap();
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM drawer_preferences", [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            512
        );
    }
    #[cfg(unix)]
    #[test]
    fn private_database_rejects_symlink_file_and_parent() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let fixture = Fixture::new();
        save_at(&fixture.0, &id(1), 0.5).unwrap();
        assert_eq!(
            std::fs::metadata(&fixture.0).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let link = fixture.0.parent().unwrap().join("linked.sqlite3");
        symlink(&fixture.0, &link).unwrap();
        assert!(load_at(&link, &id(1)).is_err());
        let linked_parent = fixture.0.parent().unwrap().join("linked-parent");
        symlink(fixture.0.parent().unwrap(), &linked_parent).unwrap();
        assert!(load_at(&linked_parent.join(DATABASE), &id(1)).is_err());
    }
}
