//! Device-local SQLite session PIN verifier and restart-persistent lock state.
//! All functions may block: call them from a background worker, never render.
use argon2::{
    Algorithm, Argon2, Params, Version,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
};
use rand_core::{OsRng, RngCore};
use std::{
    fmt,
    path::{Path, PathBuf},
    sync::Mutex,
};
const MEMORY_KIB: u32 = 64 * 1024;
const ITERATIONS: u32 = 3;
const LANES: u32 = 1;
const DATABASE: &str = "session-lock.sqlite3";
const LEGACY_MARKER: &str = "session-lock-state.json";
static DATABASE_MUTEX: Mutex<()> = Mutex::new(());

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoreError {
    InvalidPin,
    Missing,
    Unavailable,
    CorruptVerifier,
    Storage,
}
impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidPin => "Use 4–8 ASCII digits for the session PIN.",
            Self::Missing => "A session PIN has not been configured.",
            Self::Unavailable => {
                "The device-local session PIN store or random source is unavailable."
            }
            Self::CorruptVerifier => "The stored session PIN verifier is invalid.",
            Self::Storage => "The device-local session lock database could not be saved or read.",
        })
    }
}
impl std::error::Error for StoreError {}
pub fn validate_pin(pin: &str) -> Result<(), StoreError> {
    if (4..=8).contains(&pin.len()) && pin.bytes().all(|b| b.is_ascii_digit()) {
        Ok(())
    } else {
        Err(StoreError::InvalidPin)
    }
}
fn algorithm() -> Result<Argon2<'static>, StoreError> {
    let params = Params::new(MEMORY_KIB, ITERATIONS, LANES, Some(32))
        .map_err(|_| StoreError::CorruptVerifier)?;
    Ok(Argon2::new(Algorithm::Argon2id, Version::V0x13, params))
}
fn hash_with_salt(pin: &str, salt: &SaltString) -> Result<String, StoreError> {
    validate_pin(pin)?;
    algorithm()?
        .hash_password(pin.as_bytes(), salt)
        .map(|h| h.to_string())
        .map_err(|_| StoreError::CorruptVerifier)
}
fn parse_verifier(value: &str) -> Result<PasswordHash<'_>, StoreError> {
    if value.len() > 512 {
        return Err(StoreError::CorruptVerifier);
    }
    let hash = PasswordHash::new(value).map_err(|_| StoreError::CorruptVerifier)?;
    if hash.algorithm.as_str() != "argon2id"
        || hash.version != Some(19)
        || hash.params.get_decimal("m") != Some(MEMORY_KIB)
        || hash.params.get_decimal("t") != Some(ITERATIONS)
        || hash.params.get_decimal("p") != Some(LANES)
        || hash.hash.as_ref().is_none_or(|h| h.len() != 32)
        || hash.salt.is_none_or(|s| s.as_str().len() != 22)
    {
        return Err(StoreError::CorruptVerifier);
    }
    Ok(hash)
}
fn verify_hash(pin: &str, value: &str) -> Result<bool, StoreError> {
    validate_pin(pin)?;
    let hash = parse_verifier(value)?;
    // PasswordVerifier compares the computed digest through password-hash's
    // constant-time Output equality; never compare PINs or hash strings here.
    match algorithm()?.verify_password(pin.as_bytes(), &hash) {
        Ok(()) => Ok(true),
        Err(argon2::password_hash::Error::Password) => Ok(false),
        Err(_) => Err(StoreError::CorruptVerifier),
    }
}
fn database_path() -> Result<PathBuf, StoreError> {
    // GUI-owned data only: never open the authoritative task/backend database.
    let base = if cfg!(target_os = "windows") {
        std::env::var_os("LOCALAPPDATA")
            .or_else(|| std::env::var_os("APPDATA"))
            .map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support"))
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
    }
    .ok_or(StoreError::Unavailable)?;
    Ok(base.join("com.monitter.gpui.experimental").join(DATABASE))
}
fn random_verifier(pin: &str) -> Result<String, StoreError> {
    validate_pin(pin)?;
    let mut bytes = [0_u8; 16];
    OsRng
        .try_fill_bytes(&mut bytes)
        .map_err(|_| StoreError::Unavailable)?;
    let salt = SaltString::encode_b64(&bytes).map_err(|_| StoreError::CorruptVerifier)?;
    hash_with_salt(pin, &salt)
}
fn configured_at(path: &Path) -> Result<bool, StoreError> {
    let verifier = read_verifier_at(path)?;
    match verifier {
        Some(v) => {
            parse_verifier(&v)?;
            Ok(true)
        }
        None => Ok(false),
    }
}
fn set_pin_at(path: &Path, pin: &str) -> Result<(), StoreError> {
    let verifier = random_verifier(pin)?; // Slow hashing never holds the DB mutex.
    with_database(path, |tx| {
        tx.execute("UPDATE session_lock SET verifier=?1 WHERE id=1", [verifier])
            .map_err(|_| StoreError::Storage)?;
        Ok(())
    })
}
fn verify_pin_at(path: &Path, pin: &str) -> Result<bool, StoreError> {
    validate_pin(pin)?;
    let verifier = read_verifier_at(path)?.ok_or(StoreError::Missing)?;
    verify_hash(pin, &verifier) // Slow verification runs after closing the transaction.
}
pub fn configured_typed() -> Result<bool, StoreError> {
    configured_at(&database_path()?)
}
pub fn set_pin_typed(pin: &str) -> Result<(), StoreError> {
    set_pin_at(&database_path()?, pin)
}
pub fn verify_pin_typed(pin: &str) -> Result<bool, StoreError> {
    verify_pin_at(&database_path()?, pin)
}
pub fn configured() -> Result<bool, String> {
    configured_typed().map_err(|e| e.to_string())
}
pub fn set_pin(pin: &str) -> Result<(), String> {
    set_pin_typed(pin).map_err(|e| e.to_string())
}
pub fn verify_pin(pin: &str) -> Result<bool, String> {
    verify_pin_typed(pin).map_err(|e| e.to_string())
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LockSnapshot {
    pub locked: bool,
    pub attempts: u32,
    pub retry_until_ms: u64,
}
fn decode_snapshot(bytes: &[u8]) -> LockSnapshot {
    fn parse(bytes: &[u8]) -> Option<LockSnapshot> {
        let value = serde_json::from_slice::<serde_json::Value>(bytes).ok()?;
        if value["version"] != 1 {
            return None;
        }
        let locked = value["locked"].as_bool()?;
        let attempts = match value.get("attempts") {
            None => 0,
            Some(v) => u32::try_from(v.as_u64()?).ok()?,
        };
        let retry_until_ms = match value.get("retryUntilMs") {
            None => 0,
            Some(v) => v.as_u64()?,
        };
        Some(if locked {
            LockSnapshot {
                locked,
                attempts,
                retry_until_ms,
            }
        } else {
            LockSnapshot::default()
        })
    }
    // A truncated/altered marker must never unlock a previously locked UI.
    parse(bytes).unwrap_or(LockSnapshot {
        locked: true,
        ..LockSnapshot::default()
    })
}
#[cfg(test)]
fn decode_marker(bytes: &[u8]) -> bool {
    decode_snapshot(bytes).locked
}
fn legacy_snapshot(path: &Path) -> Result<LockSnapshot, StoreError> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(decode_snapshot(&bytes)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(LockSnapshot::default()),
        Err(_) => Err(StoreError::Storage),
    }
}
fn with_database<T>(
    path: &Path,
    operation: impl FnOnce(&rusqlite::Transaction<'_>) -> Result<T, StoreError>,
) -> Result<T, StoreError> {
    let _guard = DATABASE_MUTEX.lock().map_err(|_| StoreError::Storage)?;
    let parent = path.parent().ok_or(StoreError::Storage)?;
    std::fs::create_dir_all(parent).map_err(|_| StoreError::Storage)?;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(path).map_err(|_| StoreError::Storage)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(|_| StoreError::Storage)?;
    }
    drop(file);
    let mut connection = rusqlite::Connection::open(path).map_err(|_| StoreError::Storage)?;
    connection
        .busy_timeout(std::time::Duration::from_secs(2))
        .map_err(|_| StoreError::Storage)?;
    connection
        .pragma_update(None, "synchronous", "FULL")
        .map_err(|_| StoreError::Storage)?;
    let tx = connection
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|_| StoreError::Storage)?;
    tx.execute_batch("CREATE TABLE IF NOT EXISTS session_lock (id INTEGER PRIMARY KEY CHECK(id=1),verifier TEXT,locked INTEGER NOT NULL CHECK(locked IN(0,1)),attempts INTEGER NOT NULL CHECK(attempts>=0),retry_until_ms TEXT NOT NULL);").map_err(|_|StoreError::Storage)?;
    let exists: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM session_lock WHERE id=1)",
            [],
            |r| r.get(0),
        )
        .map_err(|_| StoreError::Storage)?;
    if !exists {
        // Import even after an interrupted first DB initialization. A legacy
        // locked state without a verifier stays locked, never silently resets.
        let old = legacy_snapshot(&parent.join(LEGACY_MARKER))?;
        tx.execute("INSERT INTO session_lock(id,verifier,locked,attempts,retry_until_ms) VALUES(1,NULL,?1,?2,?3)",rusqlite::params![old.locked,old.attempts,old.retry_until_ms.to_string()]).map_err(|_|StoreError::Storage)?;
    }
    let result = operation(&tx)?;
    tx.commit().map_err(|_| StoreError::Storage)?;
    Ok(result)
}
fn snapshot_in(tx: &rusqlite::Transaction<'_>) -> Result<LockSnapshot, StoreError> {
    let (locked, attempts, retry): (i64, i64, String) = tx
        .query_row(
            "SELECT locked,attempts,retry_until_ms FROM session_lock WHERE id=1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .map_err(|_| StoreError::Storage)?;
    if !matches!(locked, 0 | 1) {
        return Err(StoreError::Storage);
    }
    let snapshot = LockSnapshot {
        locked: locked == 1,
        attempts: u32::try_from(attempts).map_err(|_| StoreError::Storage)?,
        retry_until_ms: retry.parse().map_err(|_| StoreError::Storage)?,
    };
    Ok(if snapshot.locked {
        snapshot
    } else {
        LockSnapshot::default()
    })
}
fn read_snapshot_at(path: &Path) -> Result<LockSnapshot, StoreError> {
    with_database(path, snapshot_in)
}
fn read_verifier_at(path: &Path) -> Result<Option<String>, StoreError> {
    with_database(path, |tx| {
        tx.query_row("SELECT verifier FROM session_lock WHERE id=1", [], |r| {
            r.get(0)
        })
        .map_err(|_| StoreError::Storage)
    })
}
fn update_snapshot(
    path: &Path,
    change: impl FnOnce(LockSnapshot) -> LockSnapshot,
) -> Result<(), StoreError> {
    with_database(path, |tx| {
        let value = change(snapshot_in(tx)?);
        tx.execute(
            "UPDATE session_lock SET locked=?1,attempts=?2,retry_until_ms=?3 WHERE id=1",
            rusqlite::params![
                value.locked,
                value.attempts,
                value.retry_until_ms.to_string()
            ],
        )
        .map_err(|_| StoreError::Storage)?;
        Ok(())
    })
}
fn set_locked_at(path: &Path, locked: bool) -> Result<(), StoreError> {
    update_snapshot(path, |old| {
        if locked {
            LockSnapshot {
                locked: true,
                ..old
            }
        } else {
            LockSnapshot::default()
        }
    })
}
fn record_failure_at(path: &Path, attempts: u32, retry_until_ms: u64) -> Result<(), StoreError> {
    update_snapshot(path, |old| LockSnapshot {
        locked: true,
        attempts: old.attempts.max(attempts),
        retry_until_ms: old.retry_until_ms.max(retry_until_ms),
    })
}
pub fn read_snapshot() -> Result<LockSnapshot, String> {
    read_snapshot_at(&database_path().map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}
pub fn read_locked() -> Result<bool, String> {
    read_snapshot().map(|s| s.locked)
}
pub fn set_locked(locked: bool) -> Result<(), String> {
    set_locked_at(&database_path().map_err(|e| e.to_string())?, locked).map_err(|e| e.to_string())
}
pub fn record_failure(attempts: u32, retry_until_ms: u64) -> Result<(), String> {
    record_failure_at(
        &database_path().map_err(|e| e.to_string())?,
        attempts,
        retry_until_ms,
    )
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    // Spawn fresh processes so this checks committed state rather than a
    // surviving connection or in-memory value. Only synthetic test data is used.
    #[test]
    fn session_pin_lock_survives_gui_process_exit_until_explicit_unlock() {
        let directory = std::env::temp_dir().join(format!(
            "monitter-lock-restart-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        let path = directory.join(DATABASE);
        for operation in ["lock", "reopen-locked", "unlock", "reopen-unlocked"] {
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "session_lock_store::tests::session_pin_restart_child"])
                .env("MONITTER_TEST_LOCK_DATABASE", &path)
                .env("MONITTER_TEST_LOCK_OPERATION", operation)
                .status().unwrap();
            assert!(status.success(), "restart step failed: {operation}");
        }
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn session_pin_restart_child() {
        let Some(path) = std::env::var_os("MONITTER_TEST_LOCK_DATABASE") else { return; };
        let path = PathBuf::from(path);
        match std::env::var("MONITTER_TEST_LOCK_OPERATION").unwrap().as_str() {
            "lock" => {
                set_pin_at(&path, "0427").unwrap();
                set_locked_at(&path, true).unwrap();
                // Leave without normal GUI shutdown or test cleanup.
                std::process::exit(0);
            }
            "reopen-locked" => {
                assert!(read_snapshot_at(&path).unwrap().locked);
                assert!(configured_at(&path).unwrap());
            }
            "unlock" => {
                assert!(verify_pin_at(&path, "0427").unwrap());
                assert!(read_snapshot_at(&path).unwrap().locked);
                set_locked_at(&path, false).unwrap();
            }
            "reopen-unlocked" => assert!(!read_snapshot_at(&path).unwrap().locked),
            other => panic!("unexpected restart operation: {other}"),
        }
    }

    #[test]
    fn session_pin_lock_snapshot_roundtrip_preserves_failures_and_unlock_resets() {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let directory = std::env::temp_dir().join(format!(
            "monitter-session-lock-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let path = directory.join(DATABASE);
        assert_eq!(read_snapshot_at(&path).unwrap(), LockSnapshot::default());
        set_locked_at(&path, true).unwrap();
        record_failure_at(&path, 4, 123456789).unwrap();
        set_locked_at(&path, true).unwrap();
        assert_eq!(
            read_snapshot_at(&path).unwrap(),
            LockSnapshot {
                locked: true,
                attempts: 4,
                retry_until_ms: 123456789
            }
        );
        // A delayed earlier attempt cannot shorten a newer cooldown.
        record_failure_at(&path, 3, 100).unwrap();
        assert_eq!(read_snapshot_at(&path).unwrap().attempts, 4);
        assert_eq!(read_snapshot_at(&path).unwrap().retry_until_ms, 123456789);
        set_locked_at(&path, false).unwrap();
        assert_eq!(read_snapshot_at(&path).unwrap(), LockSnapshot::default());
        assert!(!configured_at(&path).unwrap());
        assert_eq!(verify_pin_at(&path, "0427"), Err(StoreError::Missing));
        set_pin_at(&path, "0427").unwrap();
        assert!(configured_at(&path).unwrap());
        assert!(verify_pin_at(&path, "0427").unwrap());
        assert!(!verify_pin_at(&path, "0428").unwrap());
        let first = read_verifier_at(&path).unwrap().unwrap();
        set_pin_at(&path, "0427").unwrap();
        assert_ne!(first, read_verifier_at(&path).unwrap().unwrap());
        assert_eq!(read_snapshot_at(&path).unwrap(), LockSnapshot::default());
        set_locked_at(&path, true).unwrap();
        record_failure_at(&path, 4, 123456789).unwrap();
        assert!(verify_pin_at(&path, "0427").unwrap());
        // Successful verification alone cannot remove the persistent lock.
        assert_eq!(
            read_snapshot_at(&path).unwrap(),
            LockSnapshot {
                locked: true,
                attempts: 4,
                retry_until_ms: 123456789
            }
        );
        set_locked_at(&path, false).unwrap();
        assert_eq!(read_snapshot_at(&path).unwrap(), LockSnapshot::default());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn session_pin_sqlite_imports_legacy_lock_and_corruption_fails_closed() {
        for (name, bytes) in [
            (
                "locked",
                br#"{"version":1,"locked":true,"attempts":4,"retryUntilMs":9999}"#.as_slice(),
            ),
            ("corrupt", b"{"),
        ] {
            let directory = std::env::temp_dir().join(format!(
                "monitter-lock-migration-{}-{name}",
                std::process::id()
            ));
            std::fs::create_dir_all(&directory).unwrap();
            std::fs::write(directory.join(LEGACY_MARKER), bytes).unwrap();
            let path = directory.join(DATABASE);
            let imported = read_snapshot_at(&path).unwrap();
            assert!(imported.locked);
            assert_eq!(imported.attempts, if name == "locked" { 4 } else { 0 });
            assert!(!configured_at(&path).unwrap());
            assert_eq!(verify_pin_at(&path, "0427"), Err(StoreError::Missing));
            // Reopening uses committed SQLite state, not a changed old marker.
            std::fs::write(
                directory.join(LEGACY_MARKER),
                br#"{"version":1,"locked":false}"#,
            )
            .unwrap();
            assert_eq!(read_snapshot_at(&path).unwrap(), imported);
            std::fs::remove_dir_all(directory).unwrap();
        }
    }

    #[test]
    fn session_pin_corrupt_sqlite_returns_error_instead_of_unlocked() {
        let directory =
            std::env::temp_dir().join(format!("monitter-lock-corrupt-db-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join(DATABASE);
        std::fs::write(&path, b"invalid SQLite database contents").unwrap();
        assert_eq!(read_snapshot_at(&path), Err(StoreError::Storage));
        assert_eq!(configured_at(&path), Err(StoreError::Storage));
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn session_pin_validation_rejects_non_ascii_and_length_errors() {
        for pin in ["1234", "00000000"] {
            assert!(validate_pin(pin).is_ok());
        }
        for pin in ["", "123", "123456789", "12 4", "١٢٣٤", "１２３４", "abcd"] {
            assert_eq!(validate_pin(pin), Err(StoreError::InvalidPin));
        }
    }
    #[test]
    fn session_pin_hash_verifies_and_random_salts_change_verifiers() {
        let a = SaltString::encode_b64(&[1; 16]).unwrap();
        let b = SaltString::encode_b64(&[2; 16]).unwrap();
        let first = hash_with_salt("0427", &a).unwrap();
        let second = hash_with_salt("0427", &b).unwrap();
        assert_ne!(first, second);
        assert!(verify_hash("0427", &first).unwrap());
        assert!(!verify_hash("0428", &first).unwrap());
        assert_eq!(
            verify_hash("0427", "not a verifier"),
            Err(StoreError::CorruptVerifier)
        );
        assert_eq!(
            parse_verifier(&first.replace("m=65536", "m=999999999")).err(),
            Some(StoreError::CorruptVerifier)
        );
    }
    #[test]
    fn session_pin_corrupt_markers_fail_closed() {
        assert!(!decode_marker(br#"{"version":1,"locked":false}"#));
        assert!(decode_marker(br#"{"version":1,"locked":true}"#));
        for bytes in [
            b"".as_slice(),
            b"{",
            br#"{"version":1}"#,
            br#"{"version":2,"locked":false}"#,
            br#"{"version":1,"locked":"false"}"#,
        ] {
            assert!(decode_marker(bytes));
        }
    }
}
