//! Private, expiring GUI close requests. An acknowledgment means the GUI has
//! completed its normal durable close path; it never means merely "quit sent".
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::{
        fd::AsRawFd,
        unix::fs::{MetadataExt, OpenOptionsExt},
    },
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

const TTL_MS: u64 = 20_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloseRequest {
    pub token: String,
    pub pid: u32,
    pub deadline_ms: u64,
}

fn now_ms() -> Result<u64, String> {
    u64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_millis(),
    )
    .map_err(|e| e.to_string())
}

fn validate_profile(profile: &Path) -> Result<(), String> {
    let metadata = fs::metadata(profile).map_err(|e| e.to_string())?;
    if !profile.is_absolute()
        || !metadata.is_dir()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o022 != 0
    {
        return Err("GUI restart close profile must be an owned absolute directory without group/world write access.".into());
    }
    Ok(())
}

fn request_path(profile: &Path, pid: u32) -> PathBuf {
    profile.join(format!("restart-close-{pid}.json"))
}

fn ack_path(profile: &Path, request: &CloseRequest) -> PathBuf {
    profile.join(format!(
        "restart-close-{}-{}.ack.json",
        request.pid, request.token
    ))
}

fn validate_request(request: &CloseRequest, pid: u32, now: u64) -> Result<(), String> {
    let token =
        uuid::Uuid::parse_str(&request.token).map_err(|_| "Invalid GUI close request nonce.")?;
    if request.pid <= 1
        || request.pid != pid
        || token.to_string() != request.token
        || token.is_nil()
        || request.deadline_ms == 0
        || request.deadline_ms > now.saturating_add(TTL_MS + 2_000)
    {
        return Err("Invalid GUI close request identity or expiry.".into());
    }
    Ok(())
}

fn read_request(path: &Path) -> Result<Option<CloseRequest>, String> {
    let mut file = match OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("Cannot read private GUI close request: {error}")),
    };
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    if !metadata.is_file()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o777 != 0o600
        || metadata.len() > 4096
    {
        return Err("GUI close handshake must be an owned private regular file.".into());
    }
    let mut bytes = Vec::new();
    (&mut file)
        .take(4097)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 4096 {
        return Err("GUI close handshake exceeds its size limit.".into());
    }
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|e| format!("Invalid GUI close handshake: {e}"))
}

fn atomic_write(path: &Path, request: &CloseRequest) -> Result<(), String> {
    let temporary = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&temporary)
            .map_err(|e| e.to_string())?;
        serde_json::to_writer(&mut file, request).map_err(|e| e.to_string())?;
        file.flush()
            .and_then(|_| file.sync_all())
            .map_err(|e| e.to_string())?;
        fs::rename(&temporary, path).map_err(|e| e.to_string())?;
        File::open(path.parent().ok_or("Close handshake lacks a profile.")?)
            .and_then(|directory| directory.sync_all())
            .map_err(|e| e.to_string())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

/// A per-PID advisory lock serializes helpers replacing expired requests.
/// The lock file is retained; kernel ownership ends even after a helper crash.
fn lock(profile: &Path, pid: u32) -> Result<File, String> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(profile.join(format!("restart-close-{pid}.lock")))
        .map_err(|e| e.to_string())?;
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    if !metadata.is_file()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o777 != 0o600
    {
        return Err("GUI close request lock is not private and owned.".into());
    }
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err("Another restart coordinator is preparing this GUI close request.".into());
    }
    Ok(file)
}

pub fn request_close(profile: &Path, pid: u32) -> Result<CloseRequest, String> {
    validate_profile(profile)?;
    if pid <= 1 {
        return Err("Invalid GUI close process ID.".into());
    }
    let _lock = lock(profile, pid)?;
    let now = now_ms()?;
    let path = request_path(profile, pid);
    if let Some(existing) = read_request(&path)? {
        validate_request(&existing, pid, now)?;
        if existing.deadline_ms > now {
            return Err("A GUI close request is already active for this process.".into());
        }
    }
    let request = CloseRequest {
        token: uuid::Uuid::new_v4().to_string(),
        pid,
        deadline_ms: now.saturating_add(TTL_MS),
    };
    atomic_write(&path, &request)?;
    Ok(request)
}

pub fn pending(profile: &Path, pid: u32) -> Result<Option<CloseRequest>, String> {
    validate_profile(profile)?;
    if pid <= 1 {
        return Err("Invalid GUI close process ID.".into());
    }
    let Some(request) = read_request(&request_path(profile, pid))? else {
        return Ok(None);
    };
    let now = now_ms()?;
    validate_request(&request, pid, now)?;
    Ok((request.deadline_ms > now).then_some(request))
}

pub fn acknowledge(profile: &Path, request: &CloseRequest) -> Result<(), String> {
    validate_profile(profile)?;
    validate_request(request, request.pid, now_ms()?)?;
    let _lock = lock(profile, request.pid)?;
    if pending(profile, request.pid)?.as_ref() != Some(request) {
        return Err("GUI close request expired or changed before durable close completed.".into());
    }
    let path = ack_path(profile, request);
    if let Some(existing) = read_request(&path)? {
        if &existing == request {
            return Ok(());
        }
        return Err("GUI close acknowledgment belongs to a different request.".into());
    }
    atomic_write(&path, request)
}

pub fn acknowledged(profile: &Path, request: &CloseRequest) -> Result<bool, String> {
    validate_profile(profile)?;
    let now = now_ms()?;
    validate_request(request, request.pid, now)?;
    if now >= request.deadline_ms {
        return Ok(false);
    }
    match read_request(&ack_path(profile, request))? {
        Some(ack) => Ok(ack == *request),
        None => Ok(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};

    fn profile() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "monitter-restart-close-test-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir(&path).unwrap();
        path
    }

    #[test]
    fn close_requires_an_explicit_matching_ack_and_preserves_active_request() {
        let profile = profile();
        let request = request_close(&profile, 4242).unwrap();
        assert_eq!(pending(&profile, 4242).unwrap(), Some(request.clone()));
        assert!(!acknowledged(&profile, &request).unwrap());
        assert!(request_close(&profile, 4242).is_err());
        acknowledge(&profile, &request).unwrap();
        assert!(acknowledged(&profile, &request).unwrap());
        let different = CloseRequest {
            token: uuid::Uuid::new_v4().to_string(),
            ..request
        };
        assert!(!acknowledged(&profile, &different).unwrap());
        assert!(acknowledge(&profile, &different).is_err());
        fs::remove_dir_all(profile).unwrap();
    }

    #[test]
    fn expired_request_cannot_quit_gui_and_new_request_has_a_new_nonce() {
        let profile = profile();
        let request = request_close(&profile, 4242).unwrap();
        let expired = CloseRequest {
            deadline_ms: now_ms().unwrap() - 1,
            ..request
        };
        atomic_write(&request_path(&profile, 4242), &expired).unwrap();
        assert!(pending(&profile, 4242).unwrap().is_none());
        assert!(acknowledge(&profile, &expired).is_err());
        let next = request_close(&profile, 4242).unwrap();
        assert_ne!(next.token, expired.token);
        assert!(!acknowledged(&profile, &next).unwrap());
        fs::remove_dir_all(profile).unwrap();
    }

    #[test]
    fn permissions_symlinks_and_wrong_process_identity_are_rejected() {
        let profile = profile();
        let request = request_close(&profile, 4242).unwrap();
        let path = request_path(&profile, 4242);
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(pending(&profile, 4242).is_err());
        fs::remove_file(&path).unwrap();
        let other = profile.join("other.json");
        atomic_write(&other, &request).unwrap();
        symlink(&other, &path).unwrap();
        assert!(pending(&profile, 4242).is_err());
        fs::remove_file(&path).unwrap();
        atomic_write(
            &path,
            &CloseRequest {
                pid: 4243,
                ..request
            },
        )
        .unwrap();
        assert!(pending(&profile, 4242).is_err());
        fs::remove_dir_all(profile).unwrap();
    }
}
