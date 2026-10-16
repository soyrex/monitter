//! Owner-local embedded Neovim. Only the private GPUI server calls this module.
use rmpv::Value as Mp;
use serde_json::{Value, json};
use std::{
    collections::{HashMap, VecDeque},
    io::{BufReader, Read, Write},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{
        Arc, LazyLock, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_PENDING: usize = 64;
const MAX_INPUT: usize = 1024 * 1024;
const MAX_MESSAGE: u64 = 16 * 1024 * 1024;
const MAX_BATCHES: usize = 8192;
const MAX_LOG_BYTES: usize = 32 * 1024 * 1024;
const MAX_SESSIONS: usize = 32;
static SESSIONS: LazyLock<Mutex<HashMap<String, Arc<Session>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
type Reply = Result<Mp, String>;
struct Batch {
    seq: u64,
    value: Value,
    bytes: usize,
}
#[derive(Default)]
struct Replay {
    seq: u64,
    bytes: usize,
    batches: VecDeque<Batch>,
}
impl Replay {
    fn push(&mut self, value: Value) {
        self.seq += 1;
        let bytes = value.to_string().len();
        self.bytes += bytes;
        self.batches.push_back(Batch {
            seq: self.seq,
            value,
            bytes,
        });
        while self.batches.len() > MAX_BATCHES || self.bytes > MAX_LOG_BYTES {
            if let Some(old) = self.batches.pop_front() {
                self.bytes -= old.bytes;
            } else {
                break;
            }
        }
    }
    fn gap(&self, after: u64) -> bool {
        after > self.seq
            || self
                .batches
                .front()
                .is_some_and(|b| after.saturating_add(1) < b.seq)
            || (self.batches.is_empty() && after < self.seq)
    }
}
struct Session {
    id: String,
    cwd: String,
    executable: String,
    version: String,
    child: Mutex<Child>,
    writer: mpsc::SyncSender<Vec<u8>>,
    pending: Mutex<HashMap<u64, mpsc::SyncSender<Reply>>>,
    next: AtomicU64,
    replay: Mutex<Replay>,
    exited: AtomicBool,
    error: Mutex<Option<String>>,
    stderr: Mutex<VecDeque<u8>>,
    metadata: Mutex<Value>,
    metadata_refreshing: AtomicBool,
    size: Mutex<(u64, u64)>,
    recovery: Mutex<()>,
}
fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}
fn array(values: Vec<Mp>) -> Mp {
    Mp::Array(values)
}
fn mp_json(v: &Mp) -> Value {
    match v {
        Mp::Nil => Value::Null,
        Mp::Boolean(b) => json!(b),
        Mp::Integer(i) => i
            .as_u64()
            .map(|n| json!(n))
            .unwrap_or_else(|| json!(i.as_i64())),
        Mp::F32(n) => json!(n),
        Mp::F64(n) => json!(n),
        Mp::String(s) => json!(s.as_str().unwrap_or("�")),
        Mp::Binary(b) => json!(b),
        Mp::Array(a) => Value::Array(a.iter().map(mp_json).collect()),
        Mp::Map(entries) => Value::Object(
            entries
                .iter()
                .filter_map(|(k, v)| k.as_str().map(|s| (s.to_owned(), mp_json(v))))
                .collect(),
        ),
        Mp::Ext(_, bytes) => rmpv::decode::read_value(&mut bytes.as_slice())
            .ok()
            .map(|v| mp_json(&v))
            .unwrap_or(Value::Null),
    }
}
fn options() -> Mp {
    Mp::Map(
        [
            "rgb",
            "ext_linegrid",
            "ext_multigrid",
            "ext_cmdline",
            "ext_popupmenu",
            "ext_messages",
        ]
        .into_iter()
        .map(|s| (Mp::from(s), Mp::from(true)))
        .collect(),
    )
}
fn discover() -> Result<(String, String), String> {
    let mut paths = Vec::new();
    if let Some(path) = std::env::var_os("PATH") {
        paths.extend(std::env::split_paths(&path).map(|p| p.join("nvim")));
    }
    paths.extend([
        PathBuf::from("/opt/homebrew/bin/nvim"),
        PathBuf::from("/usr/local/bin/nvim"),
        PathBuf::from("/usr/bin/nvim"),
    ]);
    let path = paths
        .into_iter()
        .find(|p| p.is_file())
        .ok_or("Neovim is not installed or is not on PATH.")?;
    let mut child = Command::new(&path)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Cannot run Neovim: {e}"))?;
    let out = child
        .stdout
        .take()
        .ok_or("Neovim version stdout unavailable.")?;
    let reader = thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = out.take(4096).read_to_end(&mut bytes);
        bytes
    });
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break,
            Ok(Some(status)) => return Err(format!("Neovim version check failed: {status}")),
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("Neovim version check timed out.".into());
            }
        }
    }
    let bytes = reader.join().unwrap_or_default();
    let version = String::from_utf8_lossy(&bytes)
        .lines()
        .next()
        .unwrap_or_default()
        .to_owned();
    if !version.starts_with("NVIM v") {
        return Err("Executable did not identify itself as Neovim.".into());
    }
    Ok((path.to_string_lossy().into_owned(), version))
}
impl Session {
    fn notify(&self, method: &str, args: Vec<Mp>) -> Result<(), String> {
        if self.exited.load(Ordering::Acquire) {
            return Err("Neovim has exited.".into());
        }
        let mut bytes = Vec::new();
        rmpv::encode::write_value(
            &mut bytes,
            &array(vec![2.into(), method.into(), array(args)]),
        )
        .map_err(|e| e.to_string())?;
        self.writer
            .try_send(bytes)
            .map_err(|e| format!("Neovim writer unavailable: {e}"))
    }
    fn failure(&self, error: String) {
        *lock(&self.error) = Some(error.clone());
        for (_, sender) in lock(&self.pending).drain() {
            let _ = sender.try_send(Err(error.clone()));
        }
    }
    fn enqueue(&self, method: &str, args: Vec<Mp>) -> Result<(u64, mpsc::Receiver<Reply>), String> {
        if self.exited.load(Ordering::Acquire) {
            return Err("Neovim has exited.".into());
        }
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = mpsc::sync_channel(1);
        {
            let mut pending = lock(&self.pending);
            if pending.len() >= MAX_PENDING {
                return Err("Neovim request queue is full.".into());
            }
            pending.insert(id, tx);
        }
        let mut bytes = Vec::new();
        let encoded = rmpv::encode::write_value(
            &mut bytes,
            &array(vec![0.into(), id.into(), method.into(), array(args)]),
        )
        .map_err(|e| e.to_string());
        if let Err(e) = encoded {
            lock(&self.pending).remove(&id);
            return Err(e);
        }
        if bytes.len() > MAX_INPUT + 4096 {
            lock(&self.pending).remove(&id);
            return Err("Neovim request is too large.".into());
        }
        if let Err(e) = self.writer.try_send(bytes) {
            lock(&self.pending).remove(&id);
            return Err(format!("Neovim writer unavailable: {e}"));
        }
        Ok((id, rx))
    }
    fn request(&self, method: &str, args: Vec<Mp>) -> Reply {
        let (id, rx) = self.enqueue(method, args)?;
        let result = rx
            .recv_timeout(REQUEST_TIMEOUT)
            .map_err(|_| format!("Neovim {method} timed out; outcome may be uncertain."));
        lock(&self.pending).remove(&id);
        result?
    }
    fn command(&self, command: &str) -> Reply {
        self.request("nvim_command", vec![command.into()])
    }
    fn snapshot(&self) -> Result<Value, String> {
        if self.exited.load(Ordering::Acquire) {
            return Ok(lock(&self.metadata).clone());
        }
        let script = r#"local wins={}; for _,w in ipairs(vim.api.nvim_tabpage_list_wins(0)) do local b=vim.api.nvim_win_get_buf(w); table.insert(wins,{id=w,buffer=b,name=vim.api.nvim_buf_get_name(b),cwd=vim.fn.getcwd(vim.fn.win_id2win(w),0),filetype=vim.bo[b].filetype,modified=vim.bo[b].modified,width=vim.api.nvim_win_get_width(w),height=vim.api.nvim_win_get_height(w)}) end; return vim.json.encode({layout=vim.fn.winlayout(),windows=wins,currentWindow=vim.api.nvim_get_current_win(),mode=vim.api.nvim_get_mode().mode})"#;
        let value = self.request("nvim_exec_lua", vec![script.into(), array(vec![])])?;
        let text = value.as_str().ok_or("Invalid Neovim metadata response.")?;
        let metadata =
            serde_json::from_str(text).map_err(|e| format!("Invalid Neovim metadata: {e}"))?;
        *lock(&self.metadata) = metadata;
        Ok(lock(&self.metadata).clone())
    }
    fn descriptor(&self) -> Value {
        json!({"id":self.id,"cwd":self.cwd,"executable":self.executable,"version":self.version,"exited":self.exited.load(Ordering::Acquire)})
    }
    fn refresh_metadata(self: &Arc<Self>) {
        if self.exited.load(Ordering::Acquire)
            || self
                .metadata_refreshing
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_err()
        {
            return;
        }
        let s = self.clone();
        thread::spawn(move || {
            let script = r#"local wins={}; for _,w in ipairs(vim.api.nvim_tabpage_list_wins(0)) do local b=vim.api.nvim_win_get_buf(w); table.insert(wins,{id=w,buffer=b,name=vim.api.nvim_buf_get_name(b),cwd=vim.fn.getcwd(vim.fn.win_id2win(w),0),filetype=vim.bo[b].filetype,modified=vim.bo[b].modified,width=vim.api.nvim_win_get_width(w),height=vim.api.nvim_win_get_height(w)}) end; return vim.json.encode({layout=vim.fn.winlayout(),windows=wins,currentWindow=vim.api.nvim_get_current_win(),mode=vim.api.nvim_get_mode().mode})"#;
            let result = (|| {
                let (id, rx) = s.enqueue("nvim_exec_lua", vec![script.into(), array(vec![])])?;
                let reply = loop {
                    match rx.recv_timeout(REQUEST_TIMEOUT) {
                        Ok(reply) => break reply,
                        Err(mpsc::RecvTimeoutError::Timeout)
                            if !s.exited.load(Ordering::Acquire) =>
                        {
                            continue;
                        }
                        Err(_) => break Err("Neovim metadata stream closed.".into()),
                    }
                };
                lock(&s.pending).remove(&id);
                let value = reply?;
                let metadata = serde_json::from_str(
                    value.as_str().ok_or("Invalid Neovim metadata response.")?,
                )
                .map_err(|e| format!("Invalid Neovim metadata: {e}"))?;
                *lock(&s.metadata) = metadata;
                Ok::<(), String>(())
            })();
            if let Err(error) = result {
                if !s.exited.load(Ordering::Acquire) {
                    *lock(&s.error) = Some(error);
                }
            }
            s.metadata_refreshing.store(false, Ordering::Release);
        });
    }
    fn kill_failed_start(&self) {
        let _ = lock(&self.child).kill();
    }
}
fn open(args: &Value) -> Result<Value, String> {
    lock(&SESSIONS).retain(|_, session| !session.exited.load(Ordering::Acquire));
    if lock(&SESSIONS).len() >= MAX_SESSIONS {
        return Err("Too many Neovim sessions (maximum 32).".into());
    }
    let (executable, version) = discover()?;
    let cwd = args
        .get("cwd")
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .unwrap_or(std::env::current_dir().map_err(|e| e.to_string())?);
    if !cwd.is_absolute() || !cwd.is_dir() {
        return Err("Neovim cwd must be an existing absolute local directory.".into());
    }
    let cols = number(args, "cols", 100)?.clamp(2, 1000);
    let rows = number(args, "rows", 30)?.clamp(2, 500);
    let mut command = Command::new(&executable);
    command.arg("--embed");
    if args.get("clean").and_then(Value::as_bool).unwrap_or(false) {
        command.arg("--clean");
    }
    command
        .current_dir(&cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|e| format!("Cannot start Neovim: {e}"))?;
    let mut stdin = child.stdin.take().ok_or("Neovim stdin unavailable.")?;
    let stdout = child.stdout.take().ok_or("Neovim stdout unavailable.")?;
    let mut stderr = child.stderr.take().ok_or("Neovim stderr unavailable.")?;
    let (writer, writes) = mpsc::sync_channel::<Vec<u8>>(MAX_PENDING);
    let session = Arc::new(Session {
        id: uuid::Uuid::new_v4().to_string(),
        cwd: cwd.to_string_lossy().into_owned(),
        executable,
        version,
        child: Mutex::new(child),
        writer,
        pending: Mutex::new(HashMap::new()),
        next: AtomicU64::new(1),
        replay: Mutex::new(Replay::default()),
        exited: AtomicBool::new(false),
        error: Mutex::new(None),
        stderr: Mutex::new(VecDeque::new()),
        metadata_refreshing: AtomicBool::new(false),
        metadata: Mutex::new(json!({"layout":[],"windows":[],"currentWindow":0,"mode":"n"})),
        size: Mutex::new((cols, rows)),
        recovery: Mutex::new(()),
    });
    let weak = Arc::downgrade(&session);
    thread::spawn(move || {
        while let Ok(bytes) = writes.recv() {
            if let Err(e) = stdin.write_all(&bytes).and_then(|_| stdin.flush()) {
                if let Some(s) = weak.upgrade() {
                    s.failure(format!("Neovim input failed: {e}"));
                }
                break;
            }
        }
    });
    let weak = Arc::downgrade(&session);
    thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        loop {
            let value =
                rmpv::decode::read_value_with_max_depth(&mut (&mut reader).take(MAX_MESSAGE), 64);
            let Some(s) = weak.upgrade() else { break };
            match value {
                Ok(Mp::Array(parts))
                    if parts.first().and_then(Mp::as_u64) == Some(1) && parts.len() == 4 =>
                {
                    if let Some(id) = parts[1].as_u64() {
                        if let Some(tx) = lock(&s.pending).remove(&id) {
                            let reply = if parts[2].is_nil() {
                                Ok(parts[3].clone())
                            } else {
                                Err(format!("Neovim RPC: {}", mp_json(&parts[2])))
                            };
                            let _ = tx.try_send(reply);
                        }
                    }
                }
                Ok(Mp::Array(parts))
                    if parts.first().and_then(Mp::as_u64) == Some(2) && parts.len() == 3 =>
                {
                    if parts[1].as_str() == Some("redraw") {
                        lock(&s.replay).push(mp_json(&parts[2]));
                    } else if parts[1].as_str() == Some("nvim_error_event") {
                        *lock(&s.error) = Some(format!("Neovim: {}", mp_json(&parts[2])));
                    }
                }
                Ok(Mp::Array(parts))
                    if parts.first().and_then(Mp::as_u64) == Some(0) && parts.len() == 4 =>
                {
                    let mut bytes = Vec::new();
                    let _ = rmpv::encode::write_value(
                        &mut bytes,
                        &array(vec![
                            1.into(),
                            parts[1].clone(),
                            array(vec![
                                0.into(),
                                "Monitter does not expose host RPC methods".into(),
                            ]),
                            Mp::Nil,
                        ]),
                    );
                    let _ = s.writer.try_send(bytes);
                }
                Ok(_) => {
                    s.failure("Malformed Neovim RPC message.".into());
                    s.kill_failed_start();
                    break;
                }
                Err(e) => {
                    if e.kind() != std::io::ErrorKind::UnexpectedEof {
                        s.failure(format!("Neovim output failed: {e}"));
                        s.kill_failed_start();
                    } else {
                        for (_, tx) in lock(&s.pending).drain() {
                            let _ = tx.try_send(Err("Neovim RPC stream closed.".into()));
                        }
                    }
                    break;
                }
            }
        }
    });
    let weak = Arc::downgrade(&session);
    thread::spawn(move || {
        let mut buf = [0u8; 2048];
        while let Ok(n) = stderr.read(&mut buf) {
            if n == 0 {
                break;
            }
            let Some(s) = weak.upgrade() else {
                break;
            };
            let mut tail = lock(&s.stderr);
            tail.extend(&buf[..n]);
            while tail.len() > 16384 {
                tail.pop_front();
            }
        }
    });
    let monitored = session.clone();
    thread::spawn(move || {
        loop {
            let s = &monitored;
            let status = lock(&s.child).try_wait();
            match status {
                Ok(Some(status)) => {
                    s.exited.store(true, Ordering::Release);
                    if !status.success() {
                        let text = String::from_utf8_lossy(
                            &lock(&s.stderr).iter().copied().collect::<Vec<_>>(),
                        )
                        .into_owned();
                        s.failure(format!("Neovim exited ({status}): {text}"));
                    } else {
                        for (_, tx) in lock(&s.pending).drain() {
                            let _ = tx.try_send(Err("Neovim has exited.".into()));
                        }
                    }
                    break;
                }
                Err(e) => {
                    s.failure(format!("Cannot monitor Neovim: {e}"));
                    break;
                }
                _ => {}
            }
            thread::sleep(Duration::from_millis(50));
        }
    });
    let start = (|| {
        session.request("nvim_ui_attach", vec![cols.into(), rows.into(), options()])?;
        if let Some(path) = args.get("path").and_then(Value::as_str) {
            if path.len() > 8192 || path.contains('\0') {
                return Err("Invalid local Neovim path.".into());
            }
            session.request(
                "nvim_exec_lua",
                vec![
                    "vim.cmd.edit(vim.fn.fnameescape(...))".into(),
                    array(vec![path.into()]),
                ],
            )?;
        }
        session.snapshot()?;
        Ok::<(), String>(())
    })();
    if let Err(error) = start {
        session.kill_failed_start();
        return Err(error);
    }
    let mut registry = lock(&SESSIONS);
    registry.retain(|_, session| !session.exited.load(Ordering::Acquire));
    if registry.len() >= MAX_SESSIONS {
        drop(registry);
        session.kill_failed_start();
        return Err("Too many Neovim sessions.".into());
    }
    registry.insert(session.id.clone(), session.clone());
    Ok(session.descriptor())
}
fn number(args: &Value, key: &str, default: u64) -> Result<u64, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(v) => v
            .as_u64()
            .ok_or_else(|| format!("{key} must be an unsigned integer.")),
    }
}
fn text<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    let s = args
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("Missing {key}."))?;
    if s.len() > MAX_INPUT {
        Err(format!("{key} is too large."))
    } else {
        Ok(s)
    }
}
fn session(args: &Value) -> Result<Arc<Session>, String> {
    let id = text(args, "id")?;
    lock(&SESSIONS)
        .get(id)
        .cloned()
        .ok_or_else(|| "Neovim session no longer exists.".into())
}
// Listings are owner-local, bounded, and produced off the GPUI render thread.
fn list_directory(path: &std::path::Path) -> Result<Value, String> {
    let path = path
        .canonicalize()
        .map_err(|e| format!("Cannot open folder: {e}"))?;
    let mut entries = Vec::new();
    let mut truncated = false;
    for entry in std::fs::read_dir(&path).map_err(|e| format!("Cannot read folder: {e}"))? {
        if entries.len() == 4096 {
            truncated = true;
            break;
        }
        let entry = entry.map_err(|e| format!("Cannot read folder entry: {e}"))?;
        let kind = entry
            .file_type()
            .map_err(|e| format!("Cannot inspect folder entry: {e}"))?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| "Folder contains a filename that is not valid UTF-8.".to_owned())?;
        let is_directory = kind.is_dir() || (kind.is_symlink() && entry.path().is_dir());
        entries.push(json!({"name":name,"path":entry.path(),"isDirectory":is_directory,"isSymlink":kind.is_symlink()}));
    }
    entries.sort_by(|a, b| {
        b["isDirectory"]
            .as_bool()
            .cmp(&a["isDirectory"].as_bool())
            .then_with(|| {
                a["name"]
                    .as_str()
                    .unwrap_or("")
                    .to_lowercase()
                    .cmp(&b["name"].as_str().unwrap_or("").to_lowercase())
            })
            .then_with(|| a["name"].as_str().cmp(&b["name"].as_str()))
    });
    Ok(json!({"path":path,"parent":path.parent(),"entries":entries,"truncated":truncated}))
}
pub(crate) const COMMANDS: &[&str] = &[
    "neovim_list_directory",
    "neovim_open_file",
    "neovim_available",
    "open_neovim",
    "read_neovim",
    "neovim_input",
    "neovim_paste",
    "neovim_focus",
    "neovim_resize",
    "neovim_close_window",
    "neovim_command",
    "neovim_mouse",
    "list_neovim",
];
pub(crate) fn invoke(command: &str, args: Value) -> Result<Value, String> {
    match command {
        "neovim_available" => {
            return Ok(match discover() {
                Ok((executable, version)) => {
                    json!({"available":true,"executable":executable,"version":version})
                }
                Err(error) => {
                    json!({"available":false,"executable":null,"version":null,"error":error})
                }
            });
        }
        "open_neovim" => return open(&args),
        "list_neovim" => {
            return Ok(Value::Array(
                lock(&SESSIONS).values().map(|s| s.descriptor()).collect(),
            ));
        }
        _ => {}
    }
    let s = session(&args)?;
    match command {
        "neovim_list_directory" => {
            let path = args
                .get("path")
                .and_then(Value::as_str)
                .filter(|p| !p.is_empty())
                .unwrap_or(&s.cwd);
            let path = PathBuf::from(path);
            if !path.is_absolute() {
                return Err("Folder path must be absolute.".into());
            }
            list_directory(&path)
        }
        "neovim_open_file" => {
            let path = PathBuf::from(text(&args, "path")?);
            if !path.is_absolute() {
                return Err("File path must be absolute.".into());
            }
            let path = path
                .canonicalize()
                .map_err(|e| format!("Cannot open file: {e}"))?;
            if !path.is_file() {
                return Err("Select a regular file to open in Neovim.".into());
            }
            let path = path.to_str().ok_or("File path is not valid UTF-8.")?;
            let win = number(&args, "windowId", 0)?;
            if win == 0 {
                return Err("Missing Neovim window.".into());
            }
            // Structured command arguments keep spaces, bars, quotes and Vim
            // filename expansion characters literal. Never force modified buffers.
            s.request("nvim_exec_lua", vec![
                "local w,p=...; vim.api.nvim_set_current_win(w); vim.api.nvim_cmd({cmd='edit',args={p},magic={file=false,bar=false}},{}); return true".into(),
                array(vec![win.into(), path.into()]),
            ])?;
            s.refresh_metadata();
            Ok(Value::Null)
        }
        "read_neovim" => {
            let after = number(&args, "after", 0)?;
            let gap = lock(&s.replay).gap(after);
            let mut baseline = after;
            if gap && !s.exited.load(Ordering::Acquire) {
                let _recovery = lock(&s.recovery);
                baseline = lock(&s.replay).seq;
                s.request("nvim_ui_detach", vec![])?;
                let (cols, rows) = *lock(&s.size);
                s.request("nvim_ui_attach", vec![cols.into(), rows.into(), options()])?;
            }
            // Redraw activity invalidates the layout projection. An idle read
            // must not manufacture another Lua RPC (and worker thread).
            if gap || lock(&s.replay).seq > after {
                s.refresh_metadata();
            }
            let metadata = lock(&s.metadata).clone();
            let replay = lock(&s.replay);
            let mut events = Vec::new();
            let mut bytes = 0usize;
            let mut seq = baseline.min(replay.seq);
            for batch in replay.batches.iter().filter(|b| b.seq > baseline) {
                if !events.is_empty() && bytes + batch.bytes > 2 * 1024 * 1024 {
                    break;
                }
                bytes += batch.bytes;
                seq = batch.seq;
                events.push(json!({"seq":batch.seq,"events":batch.value}));
            }
            Ok(
                json!({"id":s.id,"seq":seq,"latestSeq":replay.seq,"events":events,"gap":gap,"cwd":s.cwd,"layout":metadata["layout"],"windows":metadata["windows"],"currentWindow":metadata["currentWindow"],"mode":metadata["mode"],"exited":s.exited.load(Ordering::Acquire),"error":lock(&s.error).clone()}),
            )
        }
        "neovim_input" => {
            let keys = text(&args, "keys")?;
            let mut sent = 0usize;
            let deadline = Instant::now() + REQUEST_TIMEOUT;
            while sent < keys.len() {
                let count = s
                    .request("nvim_input", vec![keys[sent..].into()])?
                    .as_u64()
                    .ok_or("Invalid Neovim input acknowledgement.")?
                    as usize;
                if count > keys.len() - sent || !keys.is_char_boundary(sent + count) {
                    return Err("Invalid Neovim input byte count.".into());
                }
                sent += count;
                if sent < keys.len() && Instant::now() > deadline {
                    return Err(
                        "Neovim input queue timed out; input was partially accepted.".into(),
                    );
                }
                if count == 0 {
                    thread::sleep(Duration::from_millis(5));
                }
            }
            Ok(json!({"accepted":sent}))
        }
        "neovim_paste" => {
            let result = s.request(
                "nvim_paste",
                vec![text(&args, "text")?.into(), false.into(), (-1).into()],
            )?;
            if result.as_bool() != Some(true) {
                return Err("Neovim cancelled paste.".into());
            }
            Ok(Value::Null)
        }
        "neovim_focus" => {
            let id = number(&args, "windowId", 0)?;
            if id == 0 {
                return Err("Missing windowId.".into());
            }
            s.request("nvim_set_current_win", vec![id.into()])?;
            Ok(Value::Null)
        }
        "neovim_resize" => {
            let cols = number(&args, "cols", 0)?;
            let rows = number(&args, "rows", 0)?;
            let grid = number(&args, "grid", 1)?;
            if !(2..=1000).contains(&cols) || !(2..=500).contains(&rows) {
                return Err("Neovim size must be 2–1000 columns and 2–500 rows.".into());
            }
            // Resize is deferred by Neovim during command-line/modal input.
            // A notification preserves wire ordering without waiting for an
            // acknowledgement that would prevent the GUI from sending input.
            // grid_resize confirms the applied size; nvim_error_event reports errors.
            if grid <= 1 {
                s.notify("nvim_ui_try_resize", vec![cols.into(), rows.into()])?;
                *lock(&s.size) = (cols, rows);
            } else {
                s.notify(
                    "nvim_ui_try_resize_grid",
                    vec![grid.into(), cols.into(), rows.into()],
                )?;
            }
            Ok(Value::Null)
        }
        "neovim_close_window" => {
            let id = number(&args, "windowId", 0)?;
            if id == 0 {
                return Err("Missing windowId.".into());
            }
            let force = args.get("force").and_then(Value::as_bool).unwrap_or(false);
            let script = r#"local w,force=...; if not vim.api.nvim_win_is_valid(w) then error('Neovim window no longer exists.') end; local ok,err=pcall(vim.api.nvim_win_close,w,force); if not ok then if not tostring(err):find('E444',1,true) then error(err) end; vim.api.nvim_set_current_win(w); if vim.api.nvim_get_current_win()~=w then error('Neovim focus changed during close.') end; vim.cmd.quit({bang=force}) end; return true"#;
            if let Err(error) = s.request(
                "nvim_exec_lua",
                vec![script.into(), array(vec![id.into(), force.into()])],
            ) {
                if !error.contains("stream closed") && !error.contains("has exited") {
                    return Err(error);
                }
                let deadline = Instant::now() + Duration::from_millis(250);
                loop {
                    if lock(&s.child)
                        .try_wait()
                        .ok()
                        .flatten()
                        .is_some_and(|status| status.success())
                    {
                        s.exited.store(true, Ordering::Release);
                        break;
                    }
                    if Instant::now() >= deadline {
                        return Err(error);
                    }
                    thread::sleep(Duration::from_millis(10));
                }
            }
            Ok(Value::Null)
        }
        "neovim_command" => {
            s.command(text(&args, "command")?)?;
            Ok(Value::Null)
        }
        "neovim_mouse" => {
            if let Some(id) = args.get("windowId").and_then(Value::as_u64) {
                s.request("nvim_set_current_win", vec![id.into()])?;
            }
            let button = text(&args, "button")?;
            let action = text(&args, "action")?;
            if !["left", "right", "middle", "wheel", "move"].contains(&button)
                || !["press", "drag", "release", "up", "down", "left", "right"].contains(&action)
            {
                return Err("Invalid Neovim mouse event.".into());
            }
            let modifier = args.get("modifier").and_then(Value::as_str).unwrap_or("");
            if modifier.len() > 32 {
                return Err("Invalid Neovim mouse modifier.".into());
            }
            s.request(
                "nvim_input_mouse",
                vec![
                    button.into(),
                    action.into(),
                    modifier.into(),
                    number(&args, "grid", 1)?.into(),
                    number(&args, "row", 0)?.into(),
                    number(&args, "col", 0)?.into(),
                ],
            )?;
            Ok(Value::Null)
        }
        _ => Err(format!("Unsupported native Neovim command: {command}")),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn replay_gap_and_bounds() {
        let mut r = Replay::default();
        for _ in 0..MAX_BATCHES + 3 {
            r.push(json!([]));
        }
        assert_eq!(r.batches.len(), MAX_BATCHES);
        assert!(r.gap(0));
        assert!(!r.gap(3));
        assert!(r.gap(r.seq + 1));
    }
    #[test]
    fn msgpack_handles_are_numeric() {
        let mut bytes = Vec::new();
        rmpv::encode::write_value(&mut bytes, &Mp::from(1000)).unwrap();
        assert_eq!(mp_json(&Mp::Ext(1, bytes)), json!(1000));
    }
    #[test]
    #[ignore = "requires installed Neovim and isolated temporary files"]
    fn native_browser_and_window_status_preserve_editor_semantics() {
        let dir = std::env::temp_dir().join(format!("monitter-browser-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(dir.join("nested")).unwrap();
        let dir = dir.canonicalize().unwrap();
        let path = dir.join("literal | % #.rs");
        std::fs::write(&path, "fn main() {}\n").unwrap();
        std::fs::write(dir.join("other.txt"), "other\n").unwrap();
        let opened = invoke("open_neovim", json!({"cwd":dir,"clean":true})).unwrap();
        let id = opened["id"].as_str().unwrap();
        let session = session(&json!({"id":id})).unwrap();
        struct Cleanup(Arc<Session>, PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                self.0.command("qa!").ok();
                std::fs::remove_dir_all(&self.1).ok();
            }
        }
        let _cleanup = Cleanup(session.clone(), dir.clone());
        let listing = invoke("neovim_list_directory", json!({"id":id,"path":dir})).unwrap();
        assert_eq!(listing["entries"][0]["name"], "nested");
        assert_eq!(listing["entries"][0]["isDirectory"], true);
        assert_eq!(listing["truncated"], false);
        let win = session.snapshot().unwrap()["currentWindow"]
            .as_u64()
            .unwrap();
        session.command("filetype on").unwrap();
        invoke(
            "neovim_open_file",
            json!({"id":id,"windowId":win,"path":path}),
        )
        .unwrap();
        let status = session.snapshot().unwrap();
        assert_eq!(status["windows"][0]["name"], path.to_str().unwrap());
        assert_eq!(status["windows"][0]["filetype"], "rust");
        assert_eq!(status["windows"][0]["cwd"], dir.to_str().unwrap());
        session.command("vsplit").unwrap();
        session.command("lcd nested").unwrap();
        let local = session.snapshot().unwrap();
        let active = local["currentWindow"].as_u64().unwrap();
        for row in local["windows"].as_array().unwrap() {
            let expected = if row["id"].as_u64() == Some(active) {
                dir.join("nested")
            } else {
                dir.clone()
            };
            assert_eq!(row["cwd"], expected.to_str().unwrap());
        }
        session.command("only").unwrap();
        session.command("set nohidden").unwrap();
        session
            .request(
                "nvim_buf_set_lines",
                vec![
                    0.into(),
                    0.into(),
                    (-1).into(),
                    false.into(),
                    array(vec!["unsaved".into()]),
                ],
            )
            .unwrap();
        assert!(
            invoke(
                "neovim_open_file",
                json!({"id":id,"windowId":active,"path":dir.join("other.txt")})
            )
            .unwrap_err()
            .contains("E37")
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "fn main() {}\n");
        assert!(
            invoke(
                "neovim_open_file",
                json!({"id":id,"windowId":active,"path":dir})
            )
            .is_err()
        );
    }
    #[test]
    fn rejects_invalid_arguments() {
        assert!(number(&json!({"cols":-1}), "cols", 100).is_err());
        assert!(session(&json!({"id":"missing"})).is_err());
    }
    #[test]
    #[ignore = "requires installed Neovim and writes an isolated temporary directory"]
    fn real_neovim_edit_split_and_protection() {
        let dir =
            std::env::temp_dir().join(format!("monitter-neovim-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("draft.txt");
        std::fs::write(&path, "original\n").unwrap();
        let opened = invoke(
            "open_neovim",
            json!({"cwd":dir,"path":path,"clean":true,"cols":80,"rows":24}),
        )
        .unwrap();
        let id = opened["id"].as_str().unwrap();
        let s = session(&json!({"id":id})).unwrap();
        s.request(
            "nvim_exec_lua",
            vec![
                "vim.api.nvim_buf_set_lines(0,0,-1,false,{'edited'})".into(),
                array(vec![]),
            ],
        )
        .unwrap();
        s.snapshot().unwrap();
        let first = invoke("read_neovim", json!({"id":id})).unwrap();
        assert_eq!(first["windows"][0]["modified"], true);
        let win = first["currentWindow"].as_u64().unwrap();
        assert!(
            invoke("neovim_close_window", json!({"id":id,"windowId":win}))
                .unwrap_err()
                .contains("E37")
        );
        invoke("neovim_command", json!({"id":id,"command":"write"})).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "edited\n");
        invoke("neovim_command", json!({"id":id,"command":"vsplit"})).unwrap();
        s.snapshot().unwrap();
        let split = invoke("read_neovim", json!({"id":id,"after":0})).unwrap();
        assert_eq!(split["layout"][0], "row");
        assert_eq!(split["windows"].as_array().unwrap().len(), 2);
        invoke("neovim_focus", json!({"id":id,"windowId":win})).unwrap();
        assert_eq!(s.snapshot().unwrap()["currentWindow"], win);
        invoke(
            "neovim_input",
            json!({"id":id,"keys":":echo 'native-commandline'"}),
        )
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let state = invoke("read_neovim", json!({"id":id,"after":0})).unwrap();
            if state["events"].to_string().contains("cmdline_show") {
                break;
            }
            assert!(Instant::now() < deadline, "No external commandline redraw");
            thread::sleep(Duration::from_millis(10));
        }
        invoke("neovim_input", json!({"id":id,"keys":"<Esc>"})).unwrap();
        s.command("qa!").ok();
        std::fs::remove_dir_all(dir).unwrap();
    }
}
