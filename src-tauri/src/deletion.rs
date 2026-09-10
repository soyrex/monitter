use crate::{
    model::{Host, Snapshot, Task},
    runner,
};
use serde::Serialize;
use std::{
    collections::HashSet,
    fs,
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

const MAX_SCAN_ENTRIES: usize = 4_096;
const MAX_SESSION_FILES: usize = 512;
const MAX_SCAN_DEPTH: usize = 32;
const MAX_METADATA_LINE: usize = 64 * 1024;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeletionPreview {
    pub supported: bool,
    pub reason: String,
    pub files: Vec<String>,
}

pub fn preview(snapshot: &Snapshot, task: &Task, host: &Host) -> DeletionPreview {
    match eligible(snapshot, task) {
        Err(reason) => unsupported(reason),
        Ok(()) if task.provider != "codex" => unsupported(
            "Native-file cleanup is currently supported only for Codex sessions.".into(),
        ),
        Ok(()) => match files_for(host, task.native_session_id.as_deref().unwrap()) {
            Ok(files) if files.is_empty() => {
                unsupported("No verified Codex session file was found.".into())
            }
            Ok(files) => DeletionPreview {
                supported: true,
                reason:
                    "Verified owned Codex session files can be removed with this archived chat."
                        .into(),
                files: display_paths(&files),
            },
            Err(reason) => unsupported(reason),
        },
    }
}
pub fn remove_verified(snapshot: &Snapshot, task: &Task, host: &Host) -> Result<(), String> {
    let preview = preview(snapshot, task, host);
    if !preview.supported {
        return Err(preview.reason);
    }
    let id = task.native_session_id.as_deref().unwrap();
    let files = files_for(host, id)?;
    if display_paths(&files) != preview.files {
        return Err("Codex session-file preview changed; review it again before deleting.".into());
    }
    if host.kind == "ssh" {
        return remote_delete(host, id, &files);
    }
    remove_local_files(&files, id)
}

fn remove_local_files(files: &[PathBuf], id: &str) -> Result<(), String> {
    let roots = local_roots()?;
    for file in files {
        if !local_file_is_owned(file, &roots, id)? {
            return Err("A verified Codex session file changed before deletion.".into());
        }
    }
    for file in files {
        // remove_file unlinks this directory entry; it never follows a replacement symlink.
        fs::remove_file(file)
            .map_err(|error| format!("Could not remove verified Codex session file: {error}"))?;
    }
    Ok(())
}
fn unsupported(reason: String) -> DeletionPreview {
    DeletionPreview {
        supported: false,
        reason,
        files: vec![],
    }
}
fn display_paths(files: &[PathBuf]) -> Vec<String> {
    files
        .iter()
        .map(|path| path.display().to_string())
        .collect()
}
fn files_for(host: &Host, id: &str) -> Result<Vec<PathBuf>, String> {
    match host.kind.as_str() {
        "local" => local_files(id),
        "ssh" => remote_files(host, id),
        _ => Err("Host kind must be local or ssh.".into()),
    }
}
fn eligible(snapshot: &Snapshot, task: &Task) -> Result<(), String> {
    if !task.archived {
        return Err("Archive this chat before permanently deleting it.".into());
    }
    if task.status == "running" {
        return Err("Cancel this running chat before permanently deleting it.".into());
    }
    let id = task
        .native_session_id
        .as_deref()
        .filter(|id| uuid::Uuid::parse_str(id).is_ok())
        .ok_or("This chat has no valid native session ID.")?;
    if snapshot.tasks.iter().any(|other| {
        other.id != task.id
            && other.host_id == task.host_id
            && other.provider == task.provider
            && other.native_session_id.as_deref() == Some(id)
    }) {
        return Err("Another Monitter chat references this native session.".into());
    }
    Ok(())
}
fn codex_home() -> Result<PathBuf, String> {
    std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".codex")))
        .ok_or("Codex home is unavailable.".into())
}
fn local_roots() -> Result<Vec<PathBuf>, String> {
    let home = codex_home()?;
    let meta = fs::symlink_metadata(&home).map_err(|_| "Codex home is unavailable.")?;
    if meta.file_type().is_symlink() || !meta.is_dir() {
        return Err("Codex home is not a real directory.".into());
    }
    let home = fs::canonicalize(home).map_err(|_| "Codex home is unavailable.")?;
    let mut roots = Vec::new();
    for name in ["sessions", "archived_sessions"] {
        let candidate = home.join(name);
        match fs::symlink_metadata(&candidate) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => return Err("Could not inspect Codex session storage.".into()),
            Ok(meta) if meta.file_type().is_symlink() || !meta.is_dir() => continue,
            Ok(_) => {
                let root = fs::canonicalize(&candidate)
                    .map_err(|_| "Could not verify Codex session storage.")?;
                if root.parent() != Some(home.as_path()) {
                    return Err("Codex session storage escaped its configured home.".into());
                }
                roots.push(root);
            }
        }
    }
    Ok(roots)
}
fn local_files(id: &str) -> Result<Vec<PathBuf>, String> {
    let roots = local_roots()?;
    let mut files = Vec::new();
    let mut visited = HashSet::new();
    let mut entries = 0;
    for root in &roots {
        scan(root, root, id, 0, &mut visited, &mut entries, &mut files)?;
    }
    files.sort();
    Ok(files)
}
fn scan(
    dir: &Path,
    root: &Path,
    id: &str,
    depth: usize,
    visited: &mut HashSet<PathBuf>,
    entries: &mut usize,
    files: &mut Vec<PathBuf>,
) -> Result<(), String> {
    if depth > MAX_SCAN_DEPTH {
        return Err("Codex session scan depth limit exceeded.".into());
    }
    let canonical_dir =
        fs::canonicalize(dir).map_err(|_| "Could not verify Codex session storage.")?;
    if !canonical_dir.starts_with(root) || !visited.insert(canonical_dir) {
        return Ok(());
    }
    for entry in fs::read_dir(dir).map_err(|_| "Could not read Codex session storage.")? {
        *entries += 1;
        if *entries > MAX_SCAN_ENTRIES {
            return Err("Codex session scan limit exceeded.".into());
        }
        let path = entry
            .map_err(|_| "Could not read Codex session storage.")?
            .path();
        let meta =
            fs::symlink_metadata(&path).map_err(|_| "Could not inspect Codex session storage.")?;
        if meta.file_type().is_symlink() {
            continue;
        }
        if meta.is_dir() {
            scan(&path, root, id, depth + 1, visited, entries, files)?;
        } else if meta.is_file()
            && path.extension().is_some_and(|ext| ext == "jsonl")
            && path
                .file_name()
                .is_some_and(|name| name.to_string_lossy().contains(id))
            && local_file_is_owned(&path, &[root.to_path_buf()], id)?
        {
            files.push(fs::canonicalize(path).map_err(|_| "Could not verify Codex session path.")?);
            if files.len() > MAX_SESSION_FILES {
                return Err("Codex session file limit exceeded.".into());
            }
        }
    }
    Ok(())
}
fn local_file_is_owned(path: &Path, roots: &[PathBuf], id: &str) -> Result<bool, String> {
    let meta = fs::symlink_metadata(path).map_err(|_| "Could not inspect Codex session file.")?;
    if meta.file_type().is_symlink() || !meta.is_file() {
        return Ok(false);
    }
    let canonical = fs::canonicalize(path).map_err(|_| "Could not verify Codex session path.")?;
    if !roots.iter().any(|root| canonical.starts_with(root)) {
        return Ok(false);
    }
    verifies(&canonical, id)
}
fn verifies(path: &Path, id: &str) -> Result<bool, String> {
    let meta = fs::symlink_metadata(path).map_err(|_| "Could not inspect Codex session file.")?;
    if meta.file_type().is_symlink() || !meta.is_file() {
        return Ok(false);
    }
    let mut first = Vec::new();
    BufReader::new(open_no_follow(path)?)
        .take((MAX_METADATA_LINE + 1) as u64)
        .read_until(b'\n', &mut first)
        .map_err(|_| "Could not read Codex session file.")?;
    if first.len() > MAX_METADATA_LINE || !first.ends_with(b"\n") {
        return Ok(false);
    }
    let value: serde_json::Value = match serde_json::from_slice(&first) {
        Ok(value) => value,
        Err(_) => return Ok(false),
    };
    Ok(
        value.get("type").and_then(serde_json::Value::as_str) == Some("session_meta")
            && value
                .pointer("/payload/id")
                .and_then(serde_json::Value::as_str)
                == Some(id),
    )
}
#[cfg(unix)]
fn open_no_follow(path: &Path) -> Result<fs::File, String> {
    use std::os::unix::fs::OpenOptionsExt;
    fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .map_err(|_| "Could not read Codex session file.".into())
}
#[cfg(not(unix))]
fn open_no_follow(path: &Path) -> Result<fs::File, String> {
    fs::File::open(path).map_err(|_| "Could not read Codex session file.".into())
}

const REMOTE_SCAN: &str = r#"import json,os,sys,uuid
r=json.loads(sys.stdin.read()); sid=r['id']; uuid.UUID(sid); home=os.environ.get('CODEX_HOME',os.path.join(os.path.expanduser('~'),'.codex'))
def real_dir(p):
 st=os.lstat(p)
 if os.path.islink(p) or not os.path.isdir(p): raise ValueError('unsafe directory')
 return os.path.realpath(p)
home=real_dir(home); out=[]; seen=0; visited=set()
for name in ('sessions','archived_sessions'):
 try: base=real_dir(os.path.join(home,name))
 except FileNotFoundError: continue
 except ValueError: continue
 if os.path.dirname(base)!=home: raise SystemExit('session storage escaped CODEX_HOME')
 stack=[(base,0)]
 while stack:
  current,depth=stack.pop()
  if depth>32: raise SystemExit('session scan depth limit exceeded')
  current=os.path.realpath(current)
  if current in visited or (not current.startswith(base+os.sep) and current!=base): continue
  visited.add(current)
  with os.scandir(current) as it:
   for entry in it:
    seen+=1
    if seen>4096: raise SystemExit('session scan limit exceeded')
    if entry.is_symlink(): continue
    if entry.is_dir(follow_symlinks=False): stack.append((entry.path,depth+1)); continue
    if not entry.is_file(follow_symlinks=False) or not entry.name.endswith('.jsonl') or sid not in entry.name: continue
    real=os.path.realpath(entry.path)
    if not real.startswith(base+os.sep): continue
    if len(out)>=512: raise SystemExit('session file limit exceeded')
    try:
     with open(real,'rb') as f: line=f.readline(65537)
     if len(line)>65536 or not line.endswith(b'\n'): continue
     value=json.loads(line); payload=value.get('payload') if isinstance(value,dict) else None
     if value.get('type')=='session_meta' and isinstance(payload,dict) and payload.get('id')==sid: out.append(real)
    except (OSError,ValueError,json.JSONDecodeError): pass
print(json.dumps({'files':sorted(out)},separators=(',',':')))
"#;
fn remote_files(host: &Host, id: &str) -> Result<Vec<PathBuf>, String> {
    let output = remote_python(host, REMOTE_SCAN, serde_json::json!({"id": id}))?;
    output
        .get("files")
        .and_then(serde_json::Value::as_array)
        .ok_or("Remote Codex session scan returned invalid data.")?
        .iter()
        .map(|value| {
            value
                .as_str()
                .filter(|path| path.starts_with('/'))
                .map(PathBuf::from)
                .ok_or("Remote Codex session scan returned an invalid path.".into())
        })
        .collect()
}
fn remote_delete(host: &Host, id: &str, expected: &[PathBuf]) -> Result<(), String> {
    let current = remote_files(host, id)?;
    if current != expected {
        return Err("Codex session-file preview changed; review it again before deleting.".into());
    }
    let script = REMOTE_SCAN.replace("print(json.dumps({'files':sorted(out)},separators=(',',':')))", "expected=r.get('files')\nif not isinstance(expected,list) or expected != sorted(out): raise SystemExit('verified session paths changed')\nfor p in out:\n st=os.lstat(p)\n if os.path.islink(p) or not os.path.isfile(p): raise SystemExit('unsafe session path')\n fd=os.open(p,os.O_RDONLY|getattr(os,'O_NOFOLLOW',0))\n try:\n  opened=os.fstat(fd)\n  if (opened.st_dev,opened.st_ino)!=(st.st_dev,st.st_ino): raise SystemExit('session file changed')\n  line=os.read(fd,65537); end=line.find(b'\\n')\n  if end<0 or end>=65536: raise SystemExit('session metadata changed')\n  value=json.loads(line[:end]); payload=value.get('payload') if isinstance(value,dict) else None\n  if value.get('type')!='session_meta' or not isinstance(payload,dict) or payload.get('id')!=sid: raise SystemExit('session metadata changed')\n finally: os.close(fd)\n os.unlink(p)\nprint('{\"ok\":true}')");
    remote_python(
        host,
        &script,
        serde_json::json!({"id": id, "files": display_paths(&current)}),
    )?;
    Ok(())
}
fn remote_python(
    host: &Host,
    script: &str,
    input: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let mut command = Command::new("ssh");
    runner::add_ssh_options(&mut command, host);
    command
        .arg(runner::ssh_target(host)?)
        .arg(format!("python3 -c {}", runner::posix_quote(script)))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = command
        .spawn()
        .map_err(|_| "Could not start remote Codex session scan.".to_string())?;
    child
        .stdin
        .take()
        .ok_or("Could not open remote session scan input.")?
        .write_all(input.to_string().as_bytes())
        .map_err(|_| "Could not send remote session scan request.")?;
    let mut out = Vec::new();
    child
        .stdout
        .take()
        .ok_or("Could not read remote session scan output.")?
        .take(128 * 1024)
        .read_to_end(&mut out)
        .map_err(|_| "Could not read remote session scan output.")?;
    let deadline = Instant::now() + Duration::from_secs(8);
    while child
        .try_wait()
        .map_err(|_| "Could not read remote session scan status.")?
        .is_none()
    {
        if Instant::now() >= deadline {
            let _ = child.kill();
            return Err("Remote Codex session scan timed out.".into());
        }
        thread::sleep(Duration::from_millis(20));
    }
    serde_json::from_slice(&out)
        .map_err(|_| "Remote Codex session scan returned invalid data.".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        process::Command,
        sync::{Mutex, OnceLock},
    };
    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }
    fn fixture() -> (PathBuf, String) {
        let home =
            std::env::temp_dir().join(format!("monitter-delete-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(home.join("sessions/nested")).unwrap();
        (home, uuid::Uuid::new_v4().to_string())
    }
    fn write_session(path: &Path, id: &str) {
        fs::write(
            path,
            format!("{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"{id}\"}}}}\nrest\n"),
        )
        .unwrap();
    }
    #[test]
    fn local_scan_requires_exact_metadata_and_refuses_symlinks() {
        let _guard = env_lock().lock().unwrap();
        let (home, id) = fixture();
        let good = home.join(format!("sessions/nested/rollout-{id}.jsonl"));
        write_session(&good, &id);
        let victim = home.join(format!("sessions/rollout-{id}-spoof.jsonl"));
        write_session(&victim, &uuid::Uuid::new_v4().to_string());
        fs::write(
            home.join(format!("sessions/overlong-{id}.jsonl")),
            format!("{}\n", "x".repeat(MAX_METADATA_LINE)),
        )
        .unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&good, home.join(format!("sessions/link-{id}.jsonl"))).unwrap();
        std::env::set_var("CODEX_HOME", &home);
        let files = local_files(&id).unwrap();
        std::env::remove_var("CODEX_HOME");
        assert_eq!(files, vec![fs::canonicalize(&good).unwrap()]);
        assert!(victim.exists());
        let _ = fs::remove_dir_all(home);
    }
    #[test]
    fn local_scan_skips_symlink_base() {
        let _guard = env_lock().lock().unwrap();
        let (home, id) = fixture();
        let outside =
            std::env::temp_dir().join(format!("monitter-outside-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&outside).unwrap();
        #[cfg(unix)]
        {
            fs::remove_dir_all(home.join("sessions")).unwrap();
            std::os::unix::fs::symlink(&outside, home.join("sessions")).unwrap();
        }
        std::env::set_var("CODEX_HOME", &home);
        assert!(local_files(&id).unwrap().is_empty());
        std::env::remove_var("CODEX_HOME");
        let _ = fs::remove_dir_all(home);
        let _ = fs::remove_dir_all(outside);
    }
    #[test]
    fn local_scan_enforces_entry_bound() {
        let _guard = env_lock().lock().unwrap();
        let (home, id) = fixture();
        for number in 0..=MAX_SCAN_ENTRIES {
            fs::write(home.join(format!("sessions/{number}.txt")), "x").unwrap();
        }
        std::env::set_var("CODEX_HOME", &home);
        let error = local_files(&id).unwrap_err();
        std::env::remove_var("CODEX_HOME");
        assert!(error.contains("scan limit"));
        let _ = fs::remove_dir_all(home);
    }
    #[test]
    fn remote_script_matches_local_fixture_rules() {
        let (home, id) = fixture();
        let good = home.join(format!("sessions/rollout-{id}.jsonl"));
        write_session(&good, &id);
        let victim = home.join(format!("sessions/rollout-{id}-spoof.jsonl"));
        write_session(&victim, &uuid::Uuid::new_v4().to_string());
        fs::write(
            home.join(format!("sessions/overlong-{id}.jsonl")),
            format!("{}\n", "x".repeat(MAX_METADATA_LINE)),
        )
        .unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&good, home.join(format!("sessions/link-{id}.jsonl")))
                .unwrap();
            std::os::unix::fs::symlink(std::env::temp_dir(), home.join("archived_sessions"))
                .unwrap();
        }
        let mut child = Command::new("python3")
            .arg("-c")
            .arg(REMOTE_SCAN)
            .env("CODEX_HOME", &home)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .as_mut()
            .unwrap()
            .write_all(format!("{{\"id\":\"{id}\"}}").as_bytes())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success());
        let files = serde_json::from_slice::<serde_json::Value>(&output.stdout)
            .unwrap()
            .get("files")
            .unwrap()
            .as_array()
            .unwrap()
            .clone();
        assert_eq!(files.len(), 1);
        assert!(files[0]
            .as_str()
            .unwrap()
            .ends_with(&format!("rollout-{id}.jsonl")));
        assert!(victim.exists());
        let _ = fs::remove_dir_all(home);
    }
    #[test]
    fn remote_script_enforces_entry_bound() {
        let (home, id) = fixture();
        for number in 0..=MAX_SCAN_ENTRIES {
            fs::write(home.join(format!("sessions/{number}.txt")), "x").unwrap();
        }
        let mut child = Command::new("python3")
            .arg("-c")
            .arg(REMOTE_SCAN)
            .env("CODEX_HOME", &home)
            .stdin(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .as_mut()
            .unwrap()
            .write_all(format!("{{\"id\":\"{id}\"}}").as_bytes())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("session scan limit"));
        let _ = fs::remove_dir_all(home);
    }
    #[test]
    fn owned_file_is_deleted_after_reverification() {
        let _guard = env_lock().lock().unwrap();
        let (home, id) = fixture();
        let file = home.join(format!("sessions/rollout-{id}.jsonl"));
        write_session(&file, &id);
        std::env::set_var("CODEX_HOME", &home);
        remove_local_files(&[fs::canonicalize(&file).unwrap()], &id).unwrap();
        std::env::remove_var("CODEX_HOME");
        assert!(!file.exists());
        let _ = fs::remove_dir_all(home);
    }
}
