//! Private, bounded attachment storage rooted in a task workspace.
use crate::{model::Host, runner};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};
use uuid::Uuid;

pub const MAX_BYTES: usize = 20 * 1024 * 1024;
const MAX_PREVIEW_DATA_URL_BYTES: usize = 256 * 1024;
/// Generated tool images never become filesystem capabilities. They are
/// retained only as bounded inline data attached to the resulting message.
pub const MAX_GENERATED_IMAGE_DATA_URL_BYTES: usize = 512 * 1024;
const REMOTE_TIMEOUT: Duration = Duration::from_secs(12);
const REMOTE_OUTPUT_LIMIT: usize = 16 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Attachment {
    pub id: String,
    pub name: String,
    pub mime_type: String,
    pub size: usize,
    pub path: String,
    #[serde(default)]
    pub preview_data_url: Option<String>,
    #[serde(default)]
    pub source_id: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StoredAttachment {
    #[serde(flatten)]
    pub attachment: Attachment,
    pub host_id: String,
    pub cwd: String,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentTarget {
    pub task_id: Option<String>,
    pub agent_id: Option<String>,
    pub project_id: Option<String>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadAttachmentFile {
    pub filename: String,
    pub mime_type: String,
    pub data_base64: String,
}

pub fn safe_name(name: &str) -> Result<String, String> {
    if name.is_empty() || name.len() > 1024 || name.contains(['/', '\\']) {
        return Err("Attachment filename is invalid.".into());
    }
    let cleaned: String = name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | ' '))
        .take(120)
        .collect();
    if cleaned.is_empty() || cleaned == "." || cleaned == ".." {
        return Err("Attachment filename is invalid.".into());
    }
    Ok(cleaned)
}
pub fn decode_base64(data: &str) -> Result<Vec<u8>, String> {
    if data.len() > ((MAX_BYTES + 2) / 3) * 4 + 4 {
        return Err("Attachments are limited to 20 MiB.".into());
    }
    if data.len() % 4 != 0 {
        return Err("Attachment data is not valid base64.".into());
    }
    fn digit(b: u8) -> Option<u8> {
        match b {
            b'A'..=b'Z' => Some(b - b'A'),
            b'a'..=b'z' => Some(b - b'a' + 26),
            b'0'..=b'9' => Some(b - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            b'=' => Some(0),
            _ => None,
        }
    }
    let mut out = Vec::with_capacity(data.len() / 4 * 3);
    for (i, g) in data.as_bytes().chunks_exact(4).enumerate() {
        let pad = g.iter().rev().take_while(|b| **b == b'=').count();
        let v = [digit(g[0]), digit(g[1]), digit(g[2]), digit(g[3])];
        if pad > 2
            || (pad > 0 && (i + 1) * 4 != data.len())
            || g[..4 - pad].contains(&b'=')
            || v.iter().any(Option::is_none)
        {
            return Err("Attachment data is not valid base64.".into());
        }
        let n = (u32::from(v[0].unwrap()) << 18)
            | (u32::from(v[1].unwrap()) << 12)
            | (u32::from(v[2].unwrap()) << 6)
            | u32::from(v[3].unwrap());
        out.push((n >> 16) as u8);
        if pad < 2 {
            out.push((n >> 8) as u8)
        }
        if pad == 0 {
            out.push(n as u8)
        }
        if out.len() > MAX_BYTES {
            return Err("Attachments are limited to 20 MiB.".into());
        }
    }
    Ok(out)
}
pub fn encode_base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((bytes.len() + 2) / 3 * 4);
    for c in bytes.chunks(3) {
        let n = (u32::from(c[0]) << 16)
            | (u32::from(*c.get(1).unwrap_or(&0)) << 8)
            | u32::from(*c.get(2).unwrap_or(&0));
        out.push(T[((n >> 18) & 63) as usize] as char);
        out.push(T[((n >> 12) & 63) as usize] as char);
        out.push(if c.len() > 1 {
            T[((n >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if c.len() > 2 {
            T[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}
pub fn validate_preview(value: Option<&str>) -> Result<(), String> {
    let Some(value) = value else { return Ok(()) };
    if value.len() > MAX_PREVIEW_DATA_URL_BYTES {
        return Err("Attachment preview is too large.".into());
    }
    let payload = ["data:image/png;base64,", "data:image/jpeg;base64,"]
        .iter()
        .find_map(|p| value.strip_prefix(p))
        .ok_or("Attachment preview must be a PNG or JPEG data URL.")?;
    decode_base64(payload)
        .map(|_| ())
        .map_err(|_| "Attachment preview contains invalid base64 data.".into())
}
pub fn read_attachment_file(source_path: &str) -> Result<ReadAttachmentFile, String> {
    let path = Path::new(source_path);
    let m = fs::symlink_metadata(path).map_err(|_| "Attachment file is unavailable.")?;
    if !m.file_type().is_file() {
        return Err("Only regular files can be attached.".into());
    }
    if m.len() > MAX_BYTES as u64 {
        return Err("Attachments are limited to 20 MiB.".into());
    }
    let filename = safe_name(
        path.file_name()
            .and_then(|v| v.to_str())
            .ok_or("Attachment filename is invalid.")?,
    )?;
    let mut bytes = Vec::with_capacity(m.len() as usize);
    File::open(path)
        .map_err(|_| "Attachment file is unavailable.")?
        .take(MAX_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Could not read attachment file.")?;
    if bytes.len() > MAX_BYTES {
        return Err("Attachments are limited to 20 MiB.".into());
    }
    Ok(ReadAttachmentFile {
        filename: filename.clone(),
        mime_type: mime_for_name(&filename),
        data_base64: encode_base64(&bytes),
    })
}

pub fn generated_image(data_base64: &str) -> Result<Attachment, String> {
    if data_base64.len() + "data:image/jpeg;base64,".len() > MAX_GENERATED_IMAGE_DATA_URL_BYTES {
        return Err("Generated image is too large for inline display (512 KiB limit).".into());
    }
    let bytes = decode_base64(data_base64)
        .map_err(|_| "Generated image data is invalid or exceeds 20 MiB.".to_string())?;
    let mime_type = if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        "image/jpeg"
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        "image/png"
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        "image/webp"
    } else {
        return Err("Generated attachment is not a supported image.".into());
    };
    let preview_data_url = format!("data:{mime_type};base64,{data_base64}");
    if preview_data_url.len() > MAX_GENERATED_IMAGE_DATA_URL_BYTES {
        return Err("Generated image is too large for inline display (512 KiB limit).".into());
    }
    Ok(Attachment {
        id: Uuid::new_v4().to_string(),
        name: format!(
            "Generated image.{}",
            if mime_type == "image/jpeg" {
                "jpg"
            } else {
                mime_type.trim_start_matches("image/")
            }
        ),
        mime_type: mime_type.into(),
        size: bytes.len(),
        path: "Generated by Codex".into(),
        preview_data_url: Some(preview_data_url),
        source_id: None,
    })
}

pub fn store(
    host: &Host,
    cwd: &str,
    name: &str,
    mime_type: &str,
    data_base64: &str,
    preview_data_url: Option<String>,
    source_id: Option<String>,
) -> Result<Attachment, String> {
    let name = safe_name(name)?;
    let bytes = decode_base64(data_base64)?;
    validate_preview(preview_data_url.as_deref())?;
    if source_id
        .as_deref()
        .is_some_and(|v| Uuid::parse_str(v).is_err())
    {
        return Err("Attachment source ID must be a UUID.".into());
    }
    let stored_name = format!("{}-{}", Uuid::new_v4(), name);
    let path = match host.kind.as_str() {
        "local" => store_local(cwd, &stored_name, &bytes)?,
        "ssh" => store_remote(host, cwd, &stored_name, data_base64)?,
        _ => return Err("Host kind must be local or ssh.".into()),
    };
    Ok(Attachment {
        id: Uuid::new_v4().to_string(),
        name,
        mime_type: mime_type.trim().to_string(),
        size: bytes.len(),
        path,
        preview_data_url,
        source_id,
    })
}

fn local_cwd(cwd: &str) -> PathBuf {
    if cwd == "~" {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/"))
    } else if let Some(rest) = cwd.strip_prefix("~/") {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/"))
            .join(rest)
    } else {
        PathBuf::from(cwd)
    }
}
fn attachment_directory(root: &Path) -> Result<PathBuf, String> {
    let root = root
        .canonicalize()
        .map_err(|_| "Task folder is unavailable.")?;
    if !root.is_dir() {
        return Err("Task folder is unavailable.".into());
    }
    let monitter = root.join(".monitter");
    ensure_private_child(&root, &monitter)?;
    let attachments = monitter.join("attachments");
    ensure_private_child(&monitter, &attachments)?;
    if attachments
        .canonicalize()
        .map_err(|_| "Could not verify attachment folder.")?
        != attachments
    {
        return Err("Attachment folder escapes task folder.".into());
    }
    Ok(attachments)
}
fn ensure_private_child(parent: &Path, child: &Path) -> Result<(), String> {
    match fs::symlink_metadata(child) {
        Ok(m) => {
            if m.file_type().is_symlink() || !m.is_dir() {
                return Err("Attachment folder escapes task folder.".into());
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir(child).map_err(|_| "Could not create attachment folder.")?;
            private_dir(child)?
        }
        Err(_) => return Err("Could not verify attachment folder.".into()),
    }
    if child
        .canonicalize()
        .map_err(|_| "Could not verify attachment folder.")?
        != child
        || !child.starts_with(parent)
    {
        return Err("Attachment folder escapes task folder.".into());
    }
    private_dir(child)
}
fn store_local(cwd: &str, filename: &str, bytes: &[u8]) -> Result<String, String> {
    let directory = attachment_directory(&local_cwd(cwd))?;
    let path = directory.join(filename);
    #[cfg(unix)]
    use std::os::unix::fs::OpenOptionsExt;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        options.mode(0o600);
    }
    let mut file = options
        .open(&path)
        .map_err(|_| "Could not create attachment file.")?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|_| "Could not write attachment file.")?;
    Ok(path.to_string_lossy().into_owned())
}
#[cfg(unix)]
fn private_dir(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|_| "Could not secure attachment folder.".into())
}
#[cfg(not(unix))]
fn private_dir(_: &Path) -> Result<(), String> {
    Ok(())
}

fn remote_script() -> &'static str {
    r#"import base64,json,os,sys
def secure_child(parent,name):
 child=os.path.join(parent,name)
 try: st=os.lstat(child)
 except FileNotFoundError: os.mkdir(child,0o700)
 else:
  if os.path.islink(child) or not os.path.isdir(child): raise ValueError('Attachment folder escapes task folder.')
 if os.path.realpath(child)!=child or os.path.dirname(child)!=parent: raise ValueError('Attachment folder escapes task folder.')
 os.chmod(child,0o700)
 return child
try:
 p=json.loads(sys.stdin.buffer.read().decode('utf-8')); root=os.path.realpath(os.path.expanduser(p['cwd']))
 if not os.path.isdir(root): raise ValueError('Task folder is unavailable.')
 d=secure_child(root,'.monitter'); d=secure_child(d,'attachments')
 data=base64.b64decode(p['dataBase64'],validate=True)
 if len(data)>20971520: raise ValueError('Attachments are limited to 20 MiB.')
 path=os.path.join(d,p['filename']); fd=os.open(path,os.O_WRONLY|os.O_CREAT|os.O_EXCL,0o600)
 with os.fdopen(fd,'wb') as f: f.write(data); f.flush(); os.fsync(f.fileno())
 print(json.dumps({'path':path}))
except Exception as e: print(json.dumps({'error':str(e)})); sys.exit(1)"#
}
fn store_remote(
    host: &Host,
    cwd: &str,
    filename: &str,
    data_base64: &str,
) -> Result<String, String> {
    let payload =
        serde_json::json!({"cwd":cwd,"filename":filename,"dataBase64":data_base64}).to_string();
    let mut command = Command::new("ssh");
    runner::add_ssh_options(&mut command, host);
    command
        .arg(runner::ssh_target(host)?)
        .arg(format!(
            "python3 -c {}",
            runner::posix_quote(remote_script())
        ))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let out = run_bounded(command, payload.into_bytes())?;
    let value: serde_json::Value = serde_json::from_slice(&out)
        .map_err(|_| "Remote attachment storage returned invalid output.")?;
    value
        .get("path")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| {
            value
                .get("error")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("Could not store attachment on remote host.")
                .to_string()
        })
}
fn run_bounded(mut command: Command, input: Vec<u8>) -> Result<Vec<u8>, String> {
    let mut child = command
        .spawn()
        .map_err(|_| "Could not start remote attachment storage.")?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or("Could not send attachment to remote host.")?;
    let writer = thread::spawn(move || stdin.write_all(&input));
    let stdout = child
        .stdout
        .take()
        .ok_or("Could not read remote attachment storage.")?;
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut b = Vec::new();
        let _ = stdout
            .take((REMOTE_OUTPUT_LIMIT + 1) as u64)
            .read_to_end(&mut b);
        let _ = tx.send(b);
    });
    let deadline = Instant::now() + REMOTE_TIMEOUT;
    let status = loop {
        if let Some(s) = child
            .try_wait()
            .map_err(|_| "Could not wait for remote attachment storage.")?
        {
            break s;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Remote attachment storage timed out.".into());
        }
        thread::sleep(Duration::from_millis(20));
    };
    writer
        .join()
        .map_err(|_| "Could not send attachment to remote host.")?
        .map_err(|_| "Could not send attachment to remote host.")?;
    let out = rx
        .recv_timeout(Duration::from_secs(1))
        .map_err(|_| "Remote attachment output did not finish.")?;
    if out.len() > REMOTE_OUTPUT_LIMIT {
        return Err("Remote attachment output was too large.".into());
    }
    if !status.success() {
        return Err(serde_json::from_slice::<serde_json::Value>(&out)
            .ok()
            .and_then(|v| v["error"].as_str().map(str::to_owned))
            .unwrap_or_else(|| "Could not store attachment on remote host.".into()));
    }
    Ok(out)
}
fn mime_for_name(name: &str) -> String {
    match name
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "pdf" => "application/pdf",
        "txt" | "md" | "rs" | "go" | "ts" | "js" => "text/plain",
        _ => "application/octet-stream",
    }
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Stdio;
    fn host() -> Host {
        Host {
            id: "h".into(),
            name: "h".into(),
            kind: "local".into(),
            address: "".into(),
            user: "".into(),
            port: 0,
            identity_file: "".into(),
            default_cwd: "".into(),
            codex_path: "".into(),
            claude_path: "".into(),
            opencode_path: "".into(),
            hermes_path: "".into(),
        }
    }
    fn root() -> PathBuf {
        std::env::temp_dir().join(format!("monitter-attachment-{}", Uuid::new_v4()))
    }
    #[test]
    fn rejects_size_and_filename_escape() {
        assert!(safe_name("../x").is_err());
        assert!(safe_name("a/b").is_err());
        assert!(decode_base64(&encode_base64(&vec![0; MAX_BYTES + 1])).is_err());
    }
    #[test]
    fn generated_images_are_inline_and_have_a_separate_bound() {
        let image = generated_image("/9j/2Q==").unwrap();
        assert_eq!(image.mime_type, "image/jpeg");
        assert_eq!(
            image.preview_data_url.as_deref(),
            Some("data:image/jpeg;base64,/9j/2Q==")
        );
    }
    #[cfg(unix)]
    #[test]
    fn symlink_rejection_never_creates_outside_children() {
        use std::os::unix::fs::symlink;
        let root = root();
        let outside = root.with_extension("outside");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&outside).unwrap();
        symlink(&outside, root.join(".monitter")).unwrap();
        assert!(store(
            &host(),
            root.to_str().unwrap(),
            "x.txt",
            "text/plain",
            &encode_base64(b"x"),
            None,
            None
        )
        .is_err());
        assert!(!outside.join("attachments").exists());
        fs::remove_file(root.join(".monitter")).unwrap();
        fs::create_dir(root.join(".git")).unwrap();
        symlink(root.join(".git"), root.join(".monitter")).unwrap();
        assert!(store(
            &host(),
            root.to_str().unwrap(),
            "x.txt",
            "text/plain",
            &encode_base64(b"x"),
            None,
            None
        )
        .is_err());
        assert!(!root.join(".git/attachments").exists());
        let _ = fs::remove_dir_all(root);
        let _ = fs::remove_dir_all(outside);
    }
    #[cfg(unix)]
    #[test]
    fn local_storage_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let root = root();
        fs::create_dir(&root).unwrap();
        let attachment = store(
            &host(),
            root.to_str().unwrap(),
            "x.txt",
            "text/plain",
            &encode_base64(b"x"),
            None,
            None,
        )
        .unwrap();
        assert_eq!(
            fs::metadata(root.join(".monitter"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(root.join(".monitter/attachments"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(attachment.path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let _ = fs::remove_dir_all(root);
    }
    #[cfg(unix)]
    #[test]
    fn embedded_remote_script_rejects_symlink_before_write() {
        use std::os::unix::fs::symlink;
        let root = root();
        let outside = root.with_extension("outside");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&outside).unwrap();
        symlink(&outside, root.join(".monitter")).unwrap();
        let payload =
            serde_json::json!({"cwd":root,"filename":"x","dataBase64":encode_base64(b"x")})
                .to_string();
        let mut child = Command::new("python3")
            .arg("-c")
            .arg(remote_script())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(payload.as_bytes())
            .unwrap();
        assert!(!child.wait().unwrap().success());
        assert!(!outside.join("attachments").exists());
        let _ = fs::remove_dir_all(root);
        let _ = fs::remove_dir_all(outside);
    }
}
