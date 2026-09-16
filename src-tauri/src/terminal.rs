use crate::{
    model::Host,
    runner::{remote_path, ssh_target},
};
use portable_pty::{native_pty_system, ChildKiller, CommandBuilder, MasterPty, PtySize};
use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    io::{Read, Write},
    path::Path,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

const MAX_OUTPUT_BYTES: usize = 1024 * 1024;
const MAX_READ_BYTES: usize = 256 * 1024;
const MAX_INPUT_BYTES: usize = 64 * 1024;
const MAX_COMMAND_BYTES: usize = 4096;
const TITLE_SETTLE: Duration = Duration::from_secs(5);
const TITLE_POLL: Duration = Duration::from_secs(1);

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalTarget {
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub task_id: Option<String>,
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub host_id: Option<String>,
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub command: Option<String>,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalSession {
    pub id: String,
    pub title: String,
    /// The automatic title is retained even when an explicit user/Auto-name
    /// title is currently displayed.
    pub auto_title: Option<String>,
    /// A title set by Auto-name is an intentional opt-out from automatic
    /// process titles for the life of this terminal session.
    pub custom_title: Option<String>,
    pub host_id: String,
    pub cwd: String,
    pub status: String,
    pub exit_code: Option<i32>,
}
#[derive(Clone, Serialize)]
pub struct TerminalChunk {
    pub seq: u64,
    pub data: Vec<u8>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalRead {
    pub chunks: Vec<TerminalChunk>,
    pub next_seq: u64,
    pub status: String,
    pub exit_code: Option<i32>,
    pub truncated: bool,
    pub session: TerminalSession,
}
struct OutputRing {
    chunks: VecDeque<TerminalChunk>,
    bytes: usize,
    next: u64,
    dropped_before: u64,
    reader_eof: bool,
}
pub struct Session {
    info: Mutex<TerminalSession>,
    master: Mutex<Box<dyn MasterPty + Send>>,
    writer: Mutex<Box<dyn Write + Send>>,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
    output: Mutex<OutputRing>,
    local_pty: bool,
}

struct TitleTracker {
    observed: Option<ForegroundProcess>,
    since: Instant,
}

#[derive(Clone, Eq, PartialEq)]
struct ForegroundProcess {
    group: libc::pid_t,
    name: String,
}

impl TitleTracker {
    fn new(now: Instant) -> Self {
        Self {
            observed: None,
            since: now,
        }
    }

    /// Returns a new title only after the foreground process has been stable
    /// for the settle window. This deliberately avoids terminal-tab flicker.
    fn observe(&mut self, process: Option<ForegroundProcess>, now: Instant) -> Option<String> {
        let process = match process {
            Some(process) => process,
            None => {
                // An unknown interval must not count toward five continuous
                // seconds of stability.
                self.observed = None;
                self.since = now;
                return None;
            }
        };
        if self.observed.as_ref() != Some(&process) {
            self.observed = Some(process);
            self.since = now;
            return None;
        }
        (now.duration_since(self.since) >= TITLE_SETTLE).then(|| {
            format!(
                "Terminal: {}",
                self.observed
                    .as_ref()
                    .map(|process| process.name.as_str())
                    .unwrap_or_default()
            )
        })
    }
}

pub fn open(
    id: String,
    host: &Host,
    cwd: String,
    cols: u16,
    rows: u16,
    initial_command: Option<String>,
) -> Result<Arc<Session>, String> {
    validate_size(cols, rows)?;
    if let Some(command) = initial_command.as_deref() {
        if command.is_empty() || command.len() > MAX_COMMAND_BYTES || command.contains('\0') {
            return Err("Terminal command must be 1-4096 bytes without NUL characters.".into());
        }
    }
    if cwd.trim().is_empty() {
        return Err("Terminal needs a working directory.".into());
    }
    if host.kind == "local" && !Path::new(&cwd).is_dir() {
        return Err("Terminal working directory does not exist.".into());
    }
    let pty = native_pty_system()
        .openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| format!("Could not open terminal PTY: {e}"))?;
    let mut command = build_command(host, &cwd)?;
    command.env("TERM", "xterm-256color");
    let reader = pty
        .master
        .try_clone_reader()
        .map_err(|e| format!("Could not read terminal output: {e}"))?;
    let writer = pty
        .master
        .take_writer()
        .map_err(|e| format!("Could not open terminal input: {e}"))?;
    let mut child = pty
        .slave
        .spawn_command(command)
        .map_err(|e| format!("Could not start terminal: {e}"))?;
    let local_pty = host.kind == "local" && child.process_id().is_some();
    let session = Arc::new(Session {
        info: Mutex::new(TerminalSession {
            id,
            // Do not invent a shell/process title before it has actually been
            // stable for five seconds.
            title: "Terminal".into(),
            auto_title: None,
            custom_title: None,
            host_id: host.id.clone(),
            cwd,
            status: "running".into(),
            exit_code: None,
        }),
        master: Mutex::new(pty.master),
        writer: Mutex::new(writer),
        killer: Mutex::new(child.clone_killer()),
        output: Mutex::new(OutputRing {
            chunks: VecDeque::new(),
            bytes: 0,
            next: 1,
            dropped_before: 0,
            reader_eof: false,
        }),
        local_pty,
    });
    let reader_session = Arc::clone(&session);
    thread::spawn(move || read_output(reader, reader_session));
    let status_session = Arc::clone(&session);
    thread::spawn(move || {
        let status = child.wait().ok();
        if let Ok(mut info) = status_session.info.lock() {
            info.status = "exited".into();
            info.exit_code = status.map(|s| s.exit_code() as i32);
        }
    });
    if session.local_pty {
        let title_session = Arc::clone(&session);
        thread::spawn(move || monitor_title(title_session));
    }
    if let Some(command) = initial_command {
        let _ = session.write(format!("{command}\n").as_bytes());
    }
    Ok(session)
}

fn monitor_title(session: Arc<Session>) {
    let mut tracker = TitleTracker::new(Instant::now());
    loop {
        if session
            .snapshot()
            .map(|info| info.status != "running")
            .unwrap_or(true)
        {
            return;
        }
        if let Some(title) = tracker.observe(session.foreground_name(), Instant::now()) {
            let _ = session.set_auto_title(title);
        }
        thread::sleep(TITLE_POLL);
    }
}

fn build_command(host: &Host, cwd: &str) -> Result<CommandBuilder, String> {
    if host.kind == "local" {
        let shell = std::env::var("SHELL")
            .ok()
            .filter(|path| Path::new(path).is_file())
            .unwrap_or_else(|| "/bin/zsh".into());
        let mut command = CommandBuilder::new(shell);
        command.arg("-l");
        command.cwd(cwd);
        return Ok(command);
    }
    if host.kind != "ssh" {
        return Err("Host kind must be local or ssh.".into());
    }
    let mut command = CommandBuilder::new("ssh");
    command.args([
        "-o",
        "BatchMode=yes",
        "-o",
        "ConnectTimeout=8",
        "-o",
        "StrictHostKeyChecking=yes",
        "-tt",
    ]);
    if host.port != 0 {
        command.arg("-p");
        command.arg(host.port.to_string());
    }
    if !host.identity_file.trim().is_empty() {
        command.arg("-i");
        command.arg(expand_local_home(host.identity_file.trim()));
    }
    command.arg(ssh_target(host)?);
    command.arg(format!(
        "cd {} || exit; export TERM=xterm-256color; exec \"${{SHELL:-/bin/sh}}\" -l",
        remote_path(cwd)
    ));
    Ok(command)
}
fn expand_local_home(value: &str) -> String {
    if value == "~" {
        std::env::var("HOME").unwrap_or_else(|_| value.into())
    } else if let Some(rest) = value.strip_prefix("~/") {
        format!(
            "{}/{}",
            std::env::var("HOME").unwrap_or_else(|_| "~".into()),
            rest
        )
    } else {
        value.into()
    }
}
fn validate_size(cols: u16, rows: u16) -> Result<(), String> {
    if !(10..=500).contains(&cols) || !(4..=300).contains(&rows) {
        Err("Terminal size is outside the supported range.".into())
    } else {
        Ok(())
    }
}
fn read_output(mut reader: Box<dyn Read + Send>, session: Arc<Session>) {
    let mut buffer = [0; 8192];
    while let Ok(read) = reader.read(&mut buffer) {
        if read == 0 {
            break;
        }
        if let Ok(mut output) = session.output.lock() {
            let chunk = TerminalChunk {
                seq: output.next,
                data: buffer[..read].to_vec(),
            };
            output.next += 1;
            output.bytes += read;
            output.chunks.push_back(chunk);
            while output.bytes > MAX_OUTPUT_BYTES {
                if let Some(oldest) = output.chunks.pop_front() {
                    output.bytes -= oldest.data.len();
                    output.dropped_before = oldest.seq;
                } else {
                    break;
                }
            }
        }
    }
    if let Ok(mut output) = session.output.lock() {
        output.reader_eof = true;
    }
}

impl Session {
    pub fn rename(&self, title: String) -> Result<TerminalSession, String> {
        let mut info = self
            .info
            .lock()
            .map_err(|_| "Terminal session lock failed.".to_string())?;
        // Auto-name is a user-requested title, not a transient process label.
        // Keep it separate so the monitor can continue tracking truth without
        // ever overwriting the user's chosen display title.
        info.custom_title = Some(title.clone());
        info.title = title;
        Ok(info.clone())
    }

    fn set_auto_title(&self, title: String) -> Result<(), String> {
        let mut info = self
            .info
            .lock()
            .map_err(|_| "Terminal session lock failed.".to_string())?;
        info.auto_title = Some(title);
        info.title = display_title(info.custom_title.as_deref(), info.auto_title.as_deref());
        Ok(())
    }

    /// The terminal driver reports the foreground process group. That lets us
    /// ignore background jobs without enumerating unrelated system processes.
    fn foreground_name(&self) -> Option<ForegroundProcess> {
        #[cfg(unix)]
        {
            if !self.local_pty {
                return None;
            }
            let fd = self.master.lock().ok()?.as_raw_fd()?;
            // SAFETY: fd is borrowed from the live native PTY master for this call.
            let foreground_group = unsafe { libc::tcgetpgrp(fd) };
            if foreground_group <= 0 {
                return None;
            }
            // Query even when the shell owns the foreground group: `exec vim` or
            // an exec'd nested shell can retain that PID while changing executable.
            // We never infer "idle" solely from a matching process group.
            Some(ForegroundProcess {
                group: foreground_group,
                name: process_name(foreground_group)?,
            })
        }
        #[cfg(not(unix))]
        {
            // No portable foreground-process-group API exists here; retain
            // the default title instead of inventing a process state.
            None
        }
    }
    pub fn snapshot(&self) -> Result<TerminalSession, String> {
        self.info
            .lock()
            .map(|info| info.clone())
            .map_err(|_| "Terminal status lock failed.".into())
    }
    pub fn write(&self, data: &[u8]) -> Result<(), String> {
        if data.is_empty() || data.len() > MAX_INPUT_BYTES {
            return Err("Terminal input must be between 1 and 65536 bytes.".into());
        }
        if self.snapshot()?.status != "running" {
            return Err("Terminal session has exited.".into());
        }
        let mut writer = self
            .writer
            .lock()
            .map_err(|_| "Terminal input lock failed.".to_string())?;
        writer
            .write_all(data)
            .and_then(|_| writer.flush())
            .map_err(|e| format!("Could not write terminal input: {e}"))
    }
    pub fn resize(&self, cols: u16, rows: u16) -> Result<(), String> {
        validate_size(cols, rows)?;
        self.master
            .lock()
            .map_err(|_| "Terminal resize lock failed.".to_string())?
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| format!("Could not resize terminal: {e}"))
    }
    pub fn read(&self, after: u64) -> Result<TerminalRead, String> {
        let output = self
            .output
            .lock()
            .map_err(|_| "Terminal output lock failed.".to_string())?;
        let mut bytes = 0;
        let mut chunks = Vec::new();
        let truncated = after < output.dropped_before;
        for chunk in output.chunks.iter().filter(|chunk| chunk.seq > after) {
            if bytes + chunk.data.len() > MAX_READ_BYTES {
                break;
            }
            bytes += chunk.data.len();
            chunks.push(chunk.clone());
        }
        let next_seq = chunks.last().map(|chunk| chunk.seq).unwrap_or(after);
        let delivered = output
            .chunks
            .back()
            .map(|chunk| next_seq >= chunk.seq)
            .unwrap_or(true);
        let info = self.snapshot()?;
        let finished = info.status == "exited" && output.reader_eof && delivered;
        Ok(TerminalRead {
            chunks,
            next_seq,
            status: if finished {
                info.status.clone()
            } else {
                "running".into()
            },
            exit_code: if finished { info.exit_code } else { None },
            truncated,
            session: info,
        })
    }
    pub fn close(&self) -> Result<(), String> {
        if self.snapshot()?.status != "exited" {
            self.killer
                .lock()
                .map_err(|_| "Terminal close lock failed.".to_string())?
                .kill()
                .map_err(|error| format!("Could not close terminal: {error}"))?;
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
            while std::time::Instant::now() < deadline {
                if self.snapshot()?.status == "exited" {
                    break;
                }
                thread::sleep(std::time::Duration::from_millis(20));
            }
            if self.snapshot()?.status != "exited" {
                return Err("Terminal is still stopping; try closing it again.".into());
            }
        }
        if let Ok(mut output) = self.output.lock() {
            output.chunks.clear();
            output.bytes = 0;
        }
        Ok(())
    }
    #[cfg(test)]
    fn size(&self) -> (u16, u16) {
        let size = self.master.lock().unwrap().get_size().unwrap();
        (size.cols, size.rows)
    }
}

fn display_title(custom: Option<&str>, automatic: Option<&str>) -> String {
    custom
        .or(automatic)
        .filter(|title| !title.is_empty())
        .unwrap_or("Terminal")
        .into()
}

fn process_name(pid: libc::pid_t) -> Option<String> {
    // `proc_name` is a bounded, single-PID macOS lookup. It does not launch a
    // command, enumerate processes, or observe background jobs.
    #[cfg(target_os = "macos")]
    {
        let mut name = [0u8; libc::MAXCOMLEN as usize + 1];
        let length = unsafe {
            // SAFETY: the buffer is valid for MAXCOMLEN+1 bytes as required by proc_name.
            libc::proc_name(pid, name.as_mut_ptr().cast(), name.len() as u32)
        };
        if length <= 0 {
            return None;
        }
        return std::str::from_utf8(&name[..length as usize])
            .ok()
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(str::to_owned);
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = pid;
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn foreground(group: libc::pid_t, name: &str) -> Option<ForegroundProcess> {
        Some(ForegroundProcess {
            group,
            name: name.into(),
        })
    }
    fn local_host() -> Host {
        Host {
            id: "local".into(),
            name: "This Mac".into(),
            kind: "local".into(),
            address: "localhost".into(),
            user: String::new(),
            port: 0,
            identity_file: String::new(),
            default_cwd: "/tmp".into(),
            codex_path: String::new(),
            claude_path: String::new(),
            opencode_path: String::new(),
            hermes_path: String::new(),
        }
    }
    #[test]
    fn ssh_command_forces_pty_and_quotes_cwd() {
        let host = Host {
            id: "h".into(),
            name: "Mira".into(),
            kind: "ssh".into(),
            address: "mira".into(),
            user: "alex".into(),
            port: 22,
            identity_file: "~/.ssh/id_test".into(),
            default_cwd: "~/work".into(),
            codex_path: String::new(),
            claude_path: String::new(),
            opencode_path: String::new(),
            hermes_path: String::new(),
        };
        let command = build_command(&host, "~/work/a b").unwrap();
        let args = command
            .get_argv()
            .iter()
            .map(|value| value.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert!(args.contains(&"-tt".into()));
        assert!(args
            .iter()
            .any(|arg| arg.contains("cd \"$HOME\"/'work/a b'")));
        assert!(args.iter().any(|arg| arg == "alex@mira"));
    }
    #[test]
    fn local_pty_rejects_missing_working_directory() {
        let missing = format!("/tmp/monitter-terminal-missing-{}", crate::model::id());
        let error = match open("test".into(), &local_host(), missing, 80, 24, None) {
            Ok(_) => panic!("missing cwd opened"),
            Err(error) => error,
        };
        assert!(error.contains("working directory"));
    }

    #[test]
    fn title_waits_five_seconds_and_does_not_flicker() {
        let start = Instant::now();
        let mut tracker = TitleTracker::new(start);
        assert_eq!(tracker.observe(foreground(10, "zsh"), start), None);
        assert_eq!(
            tracker.observe(foreground(10, "zsh"), start + Duration::from_secs(4)),
            None
        );
        assert_eq!(
            tracker.observe(foreground(10, "zsh"), start + TITLE_SETTLE),
            Some("Terminal: zsh".into())
        );
        // A foreground command has to settle independently; the last title
        // remains in place during that window instead of flickering.
        assert_eq!(
            tracker.observe(foreground(20, "sleep"), start + Duration::from_secs(6)),
            None
        );
        assert_eq!(
            tracker.observe(foreground(20, "sleep"), start + Duration::from_secs(10)),
            None
        );
        assert_eq!(
            tracker.observe(foreground(20, "sleep"), start + Duration::from_secs(11)),
            Some("Terminal: sleep".into())
        );
    }

    #[test]
    fn title_returns_to_shell_after_foreground_command_and_custom_wins() {
        let start = Instant::now();
        let mut tracker = TitleTracker::new(start);
        tracker.observe(foreground(20, "sleep"), start);
        assert_eq!(
            tracker.observe(foreground(20, "sleep"), start + TITLE_SETTLE),
            Some("Terminal: sleep".into())
        );
        assert_eq!(
            tracker.observe(foreground(10, "zsh"), start + Duration::from_secs(6)),
            None
        );
        assert_eq!(
            tracker.observe(foreground(10, "zsh"), start + Duration::from_secs(11)),
            Some("Terminal: zsh".into())
        );
        assert_eq!(
            display_title(Some("Investigating logs"), Some("Terminal: zsh")),
            "Investigating logs"
        );
        assert_eq!(display_title(None, Some("Terminal: zsh")), "Terminal: zsh");
    }

    #[test]
    fn unknown_gap_and_same_name_new_group_restart_stability() {
        let start = Instant::now();
        let mut tracker = TitleTracker::new(start);
        tracker.observe(foreground(20, "node"), start);
        assert_eq!(tracker.observe(None, start + Duration::from_secs(3)), None);
        assert_eq!(
            tracker.observe(foreground(20, "node"), start + Duration::from_secs(5)),
            None
        );
        assert_eq!(
            tracker.observe(foreground(20, "node"), start + Duration::from_secs(10)),
            Some("Terminal: node".into())
        );
        // A new foreground group with the same executable is a new command.
        assert_eq!(
            tracker.observe(foreground(21, "node"), start + Duration::from_secs(11)),
            None
        );
    }

    #[test]
    fn native_pty_projects_custom_and_auto_titles_through_exit() {
        let session = open("test".into(), &local_host(), "/tmp".into(), 80, 24, None).unwrap();
        assert_eq!(session.snapshot().unwrap().title, "Terminal");
        session.set_auto_title("Terminal: zsh".into()).unwrap();
        session.rename("Build logs".into()).unwrap();
        let before_exit = session.snapshot().unwrap();
        assert_eq!(before_exit.title, "Build logs");
        assert_eq!(before_exit.auto_title.as_deref(), Some("Terminal: zsh"));
        assert_eq!(before_exit.custom_title.as_deref(), Some("Build logs"));
        session.write(b"exit\n").unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        while session.snapshot().unwrap().status == "running" && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(session.snapshot().unwrap().status, "exited");
        let read = session.read(0).unwrap();
        assert_eq!(read.session.status, "exited");
        assert_eq!(read.session.title, "Build logs");
    }

    #[test]
    fn read_batches_large_output_and_drains_tail_before_exit() {
        let session = open("test".into(), &local_host(), "/tmp".into(), 80, 24, None).unwrap();
        session.write(b"python3 -c 'import sys; sys.stdout.write(\"x\" * 300000 + \"TAIL_\" + \"MARKER\\n\")'\nexit\n").unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let mut after = 0;
        let mut all = Vec::new();
        let mut saw_batched_read = false;
        loop {
            let read = session.read(after).unwrap();
            let bytes = read
                .chunks
                .iter()
                .map(|chunk| chunk.data.len())
                .sum::<usize>();
            saw_batched_read |= bytes > 100 * 1024 && bytes <= 256 * 1024;
            assert!(!read.truncated);
            all.extend(
                read.chunks
                    .iter()
                    .flat_map(|chunk| chunk.data.iter().copied()),
            );
            after = read.next_seq;
            if read.status == "exited" && read.chunks.is_empty() {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "terminal did not drain and exit"
            );
            thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(saw_batched_read);
        assert!(String::from_utf8_lossy(&all).contains("TAIL_MARKER"));
    }

    #[test]
    fn close_reaps_a_started_foreground_child() {
        let session = open("test".into(), &local_host(), "/tmp".into(), 80, 24, None).unwrap();
        session
            .write(b"sleep 10 & child=$!; printf '%s%s\\n' CLOSE_ PID=$child; wait $child\n")
            .unwrap();
        let output = wait_for(&session, "CLOSE_PID=");
        let pid = output
            .split("CLOSE_PID=")
            .nth(1)
            .and_then(|value| {
                value
                    .chars()
                    .take_while(|character| character.is_ascii_digit())
                    .collect::<String>()
                    .parse::<u32>()
                    .ok()
            })
            .expect("foreground child PID");
        session.close().unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while std::time::Instant::now() < deadline {
            if session.snapshot().unwrap().status == "exited" {
                break;
            }
            thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(session.snapshot().unwrap().status, "exited");
        assert!(!std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .status()
            .unwrap()
            .success());
    }

    #[test]
    fn local_pty_accepts_input_resize_and_interrupt() {
        let session = open("test".into(), &local_host(), "/tmp".into(), 80, 24, None).unwrap();
        session
            .write(b"printf '%s%s\\n' MONITTER_PTY_ MARKER\n")
            .unwrap();
        wait_for(&session, "MONITTER_PTY_MARKER");
        session.resize(101, 41).unwrap();
        assert_eq!(session.size(), (101, 41));
        session
            .write(b"printf '%s%s\\n' SLEEP_ STARTED; sleep 10\n")
            .unwrap();
        wait_for(&session, "SLEEP_STARTED");
        session.write(&[3]).unwrap();
        session.write(b"printf '%s%s\\n' SHELL_ ALIVE\n").unwrap();
        wait_for(&session, "SHELL_ALIVE");
        session.close().unwrap();
    }

    #[test]
    #[ignore = "read-only saved Mira terminal PTY proof"]
    fn live_mira_pty_executes_after_readiness_output() {
        let state_path = std::path::PathBuf::from(std::env::var("HOME").unwrap())
            .join("Library/Application Support/com.monitter.desktop/state.json");
        let state: crate::model::Snapshot =
            serde_json::from_slice(&std::fs::read(state_path).unwrap()).unwrap();
        let host = state
            .hosts
            .iter()
            .find(|host| host.kind == "ssh" && host.name.eq_ignore_ascii_case("Mira"))
            .unwrap();
        let session = open(
            "mira-proof".into(),
            host,
            host.default_cwd.clone(),
            80,
            24,
            None,
        )
        .unwrap();
        wait_for_any_output(&session, std::time::Duration::from_secs(20));
        let initial = session.read(0).unwrap();
        eprintln!(
            "Mira readiness: {}",
            String::from_utf8_lossy(
                &initial
                    .chunks
                    .iter()
                    .flat_map(|chunk| chunk.data.iter().copied())
                    .collect::<Vec<_>>()
            )
        );
        session
            .write(b"printf '%s%s\\n' MONITTER_MIRA_ PTY_OK\r")
            .unwrap();
        wait_for_timeout(
            &session,
            "MONITTER_MIRA_PTY_OK",
            std::time::Duration::from_secs(20),
        );
        session.close().unwrap();
    }

    fn wait_for_any_output(session: &Session, timeout: std::time::Duration) {
        let deadline = std::time::Instant::now() + timeout;
        while std::time::Instant::now() < deadline {
            if !session.read(0).unwrap().chunks.is_empty() {
                return;
            }
            thread::sleep(std::time::Duration::from_millis(50));
        }
        panic!("Mira terminal did not produce readiness output");
    }

    fn wait_for(session: &Session, needle: &str) -> String {
        wait_for_timeout(session, needle, std::time::Duration::from_secs(3))
    }
    fn wait_for_timeout(session: &Session, needle: &str, timeout: std::time::Duration) -> String {
        let mut after = 0;
        let mut output = Vec::new();
        let deadline = std::time::Instant::now() + timeout;
        while std::time::Instant::now() < deadline {
            let read = session.read(after).unwrap();
            after = read.next_seq;
            output.extend(read.chunks.into_iter().flat_map(|chunk| chunk.data));
            let text = String::from_utf8_lossy(&output).into_owned();
            if text.contains(needle) {
                return text;
            }
            thread::sleep(std::time::Duration::from_millis(20));
        }
        panic!(
            "terminal did not emit {needle}; output={}",
            String::from_utf8_lossy(&output)
        );
    }
}
