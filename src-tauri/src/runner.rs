use crate::{
    adapters,
    collaboration_transport::SessionGrant,
    model::{valid_sandbox_for_provider, Host, Task, UsageContext, UsageTokens},
    ApprovalDecision, CreateApprovalRequest, Service,
};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    fs,
    io::{BufRead, BufReader, Read, Write},
    path::PathBuf,
    process::{Child, ChildStdin, Command, ExitStatus, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc, Arc, Condvar, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

#[cfg(test)]
static TEST_SSH: std::sync::OnceLock<Mutex<std::collections::HashMap<String, PathBuf>>> =
    std::sync::OnceLock::new();

pub(crate) fn ssh_command(host: &Host) -> Command {
    #[cfg(test)]
    if let Some(path) = TEST_SSH
        .get()
        .and_then(|map| map.lock().ok().and_then(|map| map.get(&host.id).cloned()))
    {
        return Command::new(path);
    }
    Command::new("ssh")
}

#[cfg(test)]
pub(crate) struct TestSshOverride {
    host_id: String,
}
#[cfg(test)]
impl Drop for TestSshOverride {
    fn drop(&mut self) {
        if let Some(map) = TEST_SSH.get() {
            if let Ok(mut map) = map.lock() {
                map.remove(&self.host_id);
            }
        }
    }
}
#[cfg(test)]
pub(crate) fn override_ssh_for_test(
    host_id: &str,
    executable: &std::path::Path,
) -> TestSshOverride {
    TEST_SSH
        .get_or_init(|| Mutex::new(std::collections::HashMap::new()))
        .lock()
        .unwrap()
        .insert(host_id.into(), executable.into());
    TestSshOverride {
        host_id: host_id.into(),
    }
}

pub fn posix_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn local_home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

pub fn resolve_local(configured: &str) -> Result<PathBuf, String> {
    resolve_local_provider("codex", configured)
}

pub fn resolve_local_provider(provider: &str, configured: &str) -> Result<PathBuf, String> {
    if !configured.trim().is_empty() {
        let configured = configured.trim();
        let path = if configured == "~" {
            local_home()
        } else if let Some(rest) = configured.strip_prefix("~/") {
            local_home().join(rest)
        } else {
            PathBuf::from(configured)
        };
        if path.is_file() {
            return Ok(path);
        }
        return Err(format!(
            "Configured {provider} executable does not exist: {}",
            path.display()
        ));
    }
    let home = local_home();
    let candidates = match provider {
        "codex" => vec![
            home.join(".local/bin/codex"),
            home.join(".npm-global/bin/codex"),
            PathBuf::from("/opt/homebrew/bin/codex"),
            PathBuf::from("/usr/local/bin/codex"),
            PathBuf::from("/usr/bin/codex"),
        ],
        "claude" => vec![
            home.join(".local/bin/claude"),
            home.join(".npm-global/bin/claude"),
            PathBuf::from("/opt/homebrew/bin/claude"),
            PathBuf::from("/usr/local/bin/claude"),
        ],
        "hermes" => vec![
            home.join(".local/bin/hermes"),
            home.join(".npm-global/bin/hermes"),
            PathBuf::from("/opt/homebrew/bin/hermes"),
            PathBuf::from("/usr/local/bin/hermes"),
        ],
        "opencode" => vec![
            home.join(".opencode/bin/opencode"),
            home.join(".local/bin/opencode"),
            home.join(".npm-global/bin/opencode"),
            PathBuf::from("/opt/homebrew/bin/opencode"),
            PathBuf::from("/usr/local/bin/opencode"),
        ],
        _ => return Err(format!("Provider '{provider}' has no local CLI adapter.")),
    };
    candidates
        .into_iter()
        .find(|path| path.is_file())
        .ok_or_else(|| {
            format!(
                "{} CLI was not found. Set its path on the local host.",
                provider.to_uppercase()
            )
        })
}

fn configured_path<'a>(host: &'a Host, provider: &str) -> Result<&'a str, String> {
    match provider {
        "codex" => Ok(&host.codex_path),
        "claude" => Ok(&host.claude_path),
        "opencode" => Ok(&host.opencode_path),
        "hermes" => Ok(&host.hermes_path),
        _ => Err(format!(
            "Provider '{provider}' is not implemented in Monitter."
        )),
    }
}

fn remote_cli<'a>(host: &'a Host, provider: &'a str) -> Result<&'a str, String> {
    let configured = configured_path(host, provider)?;
    Ok(if configured.trim().is_empty() {
        provider
    } else {
        configured.trim()
    })
}

const MONITTER_TOOLS: &[&str] = &[
    "list_agents",
    "delegate_task",
    "send_message",
    "get_task_result",
    "wait_for_task",
    "list_messages",
    "cancel_delegation",
    "terminal_run",
];

fn claude_tool_names() -> String {
    MONITTER_TOOLS
        .iter()
        .map(|tool| format!("mcp__monitter__{tool}"))
        .collect::<Vec<_>>()
        .join(",")
}

fn claude_mcp_config(helper: &str) -> String {
    serde_json::json!({
        "mcpServers": {
            "monitter": {
                "command": "python3",
                "args": [helper],
                "env": {
                    "MONITTER_ENDPOINT": "${MONITTER_ENDPOINT}",
                    "MONITTER_TOKEN": "${MONITTER_TOKEN}"
                }
            }
        }
    })
    .to_string()
}

fn merge_opencode_mcp_config(existing: Option<&str>, helper: &str) -> Result<String, String> {
    let mut config = match existing.filter(|value| !value.trim().is_empty()) {
        Some(value) => serde_json::from_str::<Value>(value)
            .map_err(|_| "Existing OpenCode inline configuration is invalid.".to_string())?,
        None => serde_json::json!({}),
    };
    let root = config
        .as_object_mut()
        .ok_or("Existing OpenCode inline configuration must be an object.")?;
    let mcp = root
        .entry("mcp")
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
        .ok_or("Existing OpenCode MCP configuration must be an object.")?;
    mcp.insert(
        "monitter".into(),
        serde_json::json!({
            // OpenCode 1.18's installed schema uses server names directly
            // below `mcp`; newer documentation describes a v2 `servers`
            // nesting that this local CLI rejects.
            "type": "local",
            "command": ["python3", helper],
            "environment": {
                "MONITTER_ENDPOINT": "{env:MONITTER_ENDPOINT}",
                "MONITTER_TOKEN": "{env:MONITTER_TOKEN}"
            },
            "enabled": true
        }),
    );
    Ok(config.to_string())
}

fn codex_args(task: &Task, collaboration_helper: Option<&str>) -> Vec<String> {
    // These are exec options and must precede the optional resume subcommand.
    let mut args = vec!["exec".into()];
    if task.sandbox == "yolo" {
        // `yolo` is an app-level choice, not a Codex sandbox value. This is
        // Codex CLI's documented all-approvals-and-sandbox bypass.
        args.push("--dangerously-bypass-approvals-and-sandbox".into());
    } else {
        args.extend(["-s".into(), task.sandbox.clone()]);
    }
    args.push("--skip-git-repo-check".into());
    if !task.model.trim().is_empty() {
        args.extend(["-m".into(), task.model.clone()]);
    }
    if let Some(settings) = &task.model_settings {
        if let Some(effort) = settings.reasoning_effort.as_deref() {
            args.extend([
                "-c".into(),
                format!(
                    "model_reasoning_effort={}",
                    serde_json::to_string(effort).unwrap_or_else(|_| "\"medium\"".into())
                ),
            ]);
        }
        // The live 0.154 app-server catalog calls this tier `priority`; `fast`
        // is a deprecated display alias. Explicit default clears inherited Fast.
        if let Some(fast) = settings.fast_mode {
            let tier = if fast { "priority" } else { "default" };
            args.extend(["-c".into(), format!("service_tier={tier:?}")]);
        }
    }
    if let Some(helper) = collaboration_helper {
        // This only layers the Monitter server over the user's ordinary Codex
        // configuration. Credentials stay in the child environment, never in
        // an argument, event, or shell fragment.
        let helper_args = serde_json::to_string(&vec![helper]).unwrap_or_else(|_| "[]".into());
        let tools = serde_json::to_string(MONITTER_TOOLS).unwrap_or_else(|_| "[]".into());
        args.extend([
            "-c".into(),
            "mcp_servers.monitter.command=\"python3\"".into(),
            "-c".into(),
            format!("mcp_servers.monitter.args={helper_args}"),
            "-c".into(),
            "mcp_servers.monitter.env_vars=[\"MONITTER_ENDPOINT\",\"MONITTER_TOKEN\"]".into(),
            "-c".into(),
            "mcp_servers.monitter.required=true".into(),
            "-c".into(),
            "mcp_servers.monitter.startup_timeout_sec=20".into(),
            "-c".into(),
            format!("mcp_servers.monitter.enabled_tools={tools}"),
            "-c".into(),
            "mcp_servers.monitter.default_tools_approval_mode=\"approve\"".into(),
        ]);
    }
    if let Some(native) = &task.native_session_id {
        args.extend(["resume".into(), "--json".into(), native.clone(), "-".into()]);
    } else {
        args.extend(["--json".into(), "-".into()]);
    }
    args
}

pub(crate) fn remote_path(value: &str) -> String {
    if value == "~" {
        "\"$HOME\"".into()
    } else if let Some(rest) = value.strip_prefix("~/") {
        format!("\"$HOME\"/{}", posix_quote(rest))
    } else {
        posix_quote(value)
    }
}

fn remote_executable_directory(executable: &str) -> Option<&str> {
    let executable = executable.trim();
    let (directory, filename) = executable.rsplit_once('/')?;
    if filename.is_empty() {
        return None;
    }
    Some(if directory.is_empty() { "/" } else { directory })
}

/// Build an argument-safe remote exec command. When a configured executable is
/// an npm/NVM-style wrapper, its shebang commonly uses `/usr/bin/env node`.
/// Non-interactive SSH does not load the user's shell startup files, so expose
/// the executable's own directory to that shebang without invoking a shell or
/// copying the user's authentication/configuration.
pub(crate) fn remote_exec<'a>(executable: &str, args: impl IntoIterator<Item = &'a str>) -> String {
    let executable = executable.trim();
    let command = std::iter::once("exec".to_string())
        .chain(std::iter::once(remote_path(executable)))
        .chain(args.into_iter().map(posix_quote))
        .collect::<Vec<_>>()
        .join(" ");
    if let Some(directory) = remote_executable_directory(executable) {
        format!(
            "PATH={}:\"$PATH\"; export PATH; {command}",
            remote_path(directory)
        )
    } else {
        command
    }
}

pub(crate) fn ssh_target(host: &Host) -> Result<String, String> {
    if host.address.trim().is_empty() {
        return Err("SSH host needs an address or configured alias.".into());
    }
    Ok(if host.user.trim().is_empty() {
        host.address.trim().into()
    } else {
        format!("{}@{}", host.user.trim(), host.address.trim())
    })
}

pub(crate) fn add_ssh_options(command: &mut Command, host: &Host) {
    command.args([
        "-o",
        "BatchMode=yes",
        "-o",
        "ConnectTimeout=8",
        "-o",
        "StrictHostKeyChecking=yes",
    ]);
    if host.port != 0 {
        command.arg("-p").arg(host.port.to_string());
    }
    if !host.identity_file.trim().is_empty() {
        let identity = if host.identity_file.trim() == "~" {
            local_home()
        } else if let Some(rest) = host.identity_file.trim().strip_prefix("~/") {
            local_home().join(rest)
        } else {
            PathBuf::from(host.identity_file.trim())
        };
        command.arg("-i").arg(identity);
    }
}

const REMOTE_SUPERVISOR: &str = r#"import os
import select
import signal
import subprocess
import sys

PREFIX = b"MONITTER/1 "
COLLAB_PREFIX = b"MONITTER/COLLAB/1 "

header = sys.stdin.buffer.readline()
if header.startswith(COLLAB_PREFIX):
    try:
        config_size = int(header[len(COLLAB_PREFIX):].strip())
    except ValueError:
        raise SystemExit("invalid Monitter collaboration frame")
    if config_size < 2 or config_size > 64 * 1024:
        raise SystemExit("invalid Monitter collaboration configuration")
    import json
    config_bytes = sys.stdin.buffer.read(config_size)
    if len(config_bytes) != config_size:
        raise SystemExit("incomplete Monitter collaboration configuration")
    try:
        config = json.loads(config_bytes)
        endpoint = config["endpoint"]
        token = config["token"]
    except (ValueError, KeyError, TypeError):
        raise SystemExit("invalid Monitter collaboration configuration")
    if not isinstance(endpoint, str) or not isinstance(token, str) or not endpoint or not token:
        raise SystemExit("invalid Monitter collaboration credentials")
    os.environ["MONITTER_ENDPOINT"] = endpoint
    os.environ["MONITTER_TOKEN"] = token
    opencode_helper = config.get("opencodeHelper")
    if opencode_helper is not None:
        if not isinstance(opencode_helper, str) or not opencode_helper:
            raise SystemExit("invalid Monitter OpenCode configuration")
        try:
            existing = os.environ.get("OPENCODE_CONFIG_CONTENT", "")
            opencode = json.loads(existing) if existing.strip() else {}
            if not isinstance(opencode, dict):
                raise ValueError()
            mcp = opencode.setdefault("mcp", {})
            if not isinstance(mcp, dict):
                raise ValueError()
            mcp["monitter"] = {
                "type": "local", "command": ["python3", opencode_helper],
                "environment": {"MONITTER_ENDPOINT": "{env:MONITTER_ENDPOINT}", "MONITTER_TOKEN": "{env:MONITTER_TOKEN}"},
                "enabled": True,
            }
            os.environ["OPENCODE_CONFIG_CONTENT"] = json.dumps(opencode, separators=(",", ":"))
        except (ValueError, TypeError):
            raise SystemExit("invalid existing OpenCode inline configuration")
    header = sys.stdin.buffer.readline()

if not header.startswith(PREFIX):
    raise SystemExit("invalid Monitter prompt frame")
try:
    size = int(header[len(PREFIX):].strip())
except ValueError:
    raise SystemExit("invalid Monitter prompt length")
if size < 0 or size > 16 * 1024 * 1024:
    raise SystemExit("Monitter prompt is too large")
prompt = sys.stdin.buffer.read(size)
if len(prompt) != size:
    raise SystemExit("incomplete Monitter prompt frame")

environment = os.environ.copy()
environment.pop("CODEX_THREAD_ID", None)
environment.pop("CODEX_SESSION_ID", None)
program = os.path.expanduser(sys.argv[2])
cwd = os.path.expanduser(sys.argv[1])
program_directory = os.path.dirname(program)
if program_directory:
    current_path = environment.get("PATH", "")
    environment["PATH"] = program_directory + (os.pathsep + current_path if current_path else "")
process = subprocess.Popen(
    [program, *sys.argv[3:]],
    cwd=cwd,
    env=environment,
    stdin=subprocess.PIPE,
    stdout=sys.stdout.buffer,
    stderr=sys.stderr.buffer,
    start_new_session=True,
)
assert process.stdin is not None
process.stdin.write(prompt)
# Hermes receives a nested length frame because its bridge stays resident for
# approval replies. Adding a newline would become a stray approval input.
if not prompt.startswith(b"MONITTER/HERMES/1 "):
    process.stdin.write(b"\n")
process.stdin.close()

while process.poll() is None:
    readable, _, _ = select.select([sys.stdin.fileno()], [], [], 0.05)
    if readable:
        value = os.read(sys.stdin.fileno(), 1)
        if value and value != b"C":
            continue
        try:
            os.killpg(process.pid, signal.SIGINT)
        except ProcessLookupError:
            pass
        try:
            process.wait(timeout=1)
        except subprocess.TimeoutExpired:
            try:
                os.killpg(process.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
            try:
                process.wait(timeout=1)
            except subprocess.TimeoutExpired:
                try:
                    os.killpg(process.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                process.wait()
        raise SystemExit(130)
raise SystemExit(process.returncode)
"#;

const HERMES_PROMPT_PREFIX: &str = "MONITTER/HERMES/1 ";

fn hermes_prompt_frame(prompt: &str) -> String {
    format!("{HERMES_PROMPT_PREFIX}{}\n{prompt}", prompt.len())
}

fn remote_runner(cli: &str, cwd: &str, args: &[String]) -> String {
    std::iter::once(posix_quote("python3"))
        .chain(std::iter::once(posix_quote("-c")))
        .chain(std::iter::once(posix_quote(REMOTE_SUPERVISOR)))
        .chain(std::iter::once(posix_quote(cwd)))
        .chain(std::iter::once(posix_quote(cli)))
        .chain(args.iter().map(|arg| posix_quote(arg)))
        .collect::<Vec<_>>()
        .join(" ")
}

pub(crate) fn isolate_child(command: &mut Command) {
    command
        .env_remove("CODEX_THREAD_ID")
        .env_remove("CODEX_SESSION_ID")
        .env_remove("MONITTER_ENDPOINT")
        .env_remove("MONITTER_TOKEN");
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
}

/// Provider stderr is a transport diagnostic channel, not a user activity feed.
/// CLI harnesses routinely write startup, shutdown, and MCP warnings there even
/// when a turn succeeds. Keep those out of the conversation, while retaining a
/// single concise error that can help an operator diagnose a failed run.
pub(crate) fn provider_stderr_diagnostic(line: &str) -> Option<String> {
    let text = line.trim();
    if text.is_empty() {
        return None;
    }
    let upper = text.to_ascii_uppercase();
    if ["TRACE", "DEBUG", "INFO", "WARN", "WARNING"]
        .iter()
        .any(|level| {
            upper == *level
                || upper.starts_with(&format!("{level} "))
                || upper.contains(&format!(" {level} "))
        })
    {
        return None;
    }

    let lower = text.to_ascii_lowercase();
    let actionable = [
        "error",
        "fatal",
        "panic",
        "permission denied",
        "unauthorized",
        "authentication",
        "not found",
        "could not",
        "failed",
    ]
    .iter()
    .any(|needle| lower.contains(needle));
    if !actionable {
        return None;
    }

    let compact = text.split_whitespace().collect::<Vec<_>>().join(" ");
    const LIMIT: usize = 600;
    if compact.chars().count() > LIMIT {
        Some(format!(
            "{}…",
            compact.chars().take(LIMIT).collect::<String>()
        ))
    } else {
        Some(compact)
    }
}

/// SSH writes connection and trust failures to stderr. Keep only familiar
/// operational diagnostics, rather than echoing arbitrary remote stderr into
/// the app's error surface.
pub(crate) fn ssh_stderr_diagnostic(stderr: &[u8]) -> Option<String> {
    const MARKERS: &[&str] = &[
        "host key",
        "permission denied",
        "authentication",
        "connection",
        "could not resolve",
        "name or service",
        "no route",
        "timed out",
        "refused",
        "identity file",
        "kex",
        "remote host",
        "invalid format",
        "error in libcrypto",
    ];
    const LIMIT: usize = 600;

    let details = String::from_utf8_lossy(stderr)
        .lines()
        .filter_map(|line| {
            let compact = line
                .chars()
                .filter(|character| !character.is_control() || *character == '\t')
                .collect::<String>()
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            (!compact.is_empty()
                && MARKERS
                    .iter()
                    .any(|marker| compact.to_ascii_lowercase().contains(marker)))
            .then_some(compact)
        })
        .collect::<Vec<_>>()
        .join(" ");
    if details.is_empty() {
        return None;
    }
    if details.chars().count() > LIMIT {
        Some(format!(
            "{}…",
            details.chars().take(LIMIT).collect::<String>()
        ))
    } else {
        Some(details)
    }
}

pub(crate) fn with_ssh_diagnostic(message: impl Into<String>, stderr: &[u8]) -> String {
    match ssh_stderr_diagnostic(stderr) {
        Some(diagnostic) => format!("{} SSH diagnostic: {diagnostic}", message.into()),
        None => message.into(),
    }
}

#[cfg(test)]
pub(crate) fn build_command(host: &Host, task: &Task) -> Result<Command, String> {
    build_command_with_collaboration(host, task, None)
}

/// Reads up to `limit` bytes from `pipe` on a worker thread while continuing
/// to drain the pipe once the cap is reached. Used by the SSH helper
/// staging path to keep verbose diagnostics from blocking a full
/// stdout/stderr pipe. The historical `title_` prefix is retained to avoid
/// renaming a shared reader; the naming subprocess path was retired in
/// favour of the resident Monitter Admin broker.
fn title_reader<R: Read + Send + 'static>(
    mut pipe: R,
    limit: usize,
) -> mpsc::Receiver<Result<Vec<u8>, String>> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let result = (|| {
            let mut bytes = Vec::new();
            let mut buffer = [0u8; 8192];
            loop {
                let count = pipe.read(&mut buffer).map_err(|error| error.to_string())?;
                if count == 0 {
                    break;
                }
                let keep = count.min(limit.saturating_sub(bytes.len()));
                bytes.extend_from_slice(&buffer[..keep]);
                // Continue draining after the capture limit so verbose diagnostics
                // cannot block the child on a full stdout/stderr pipe.
            }
            Ok(bytes)
        })();
        let _ = sender.send(result);
    });
    receiver
}

fn build_command_with_collaboration(
    host: &Host,
    task: &Task,
    collaboration: Option<(&SessionGrant, &str)>,
) -> Result<Command, String> {
    build_command_with_options(host, task, collaboration, None)
}

fn build_command_with_options(
    host: &Host,
    task: &Task,
    collaboration: Option<(&SessionGrant, &str)>,
    claude_mcp_config_path: Option<&std::path::Path>,
) -> Result<Command, String> {
    if !valid_sandbox_for_provider(&task.provider, &task.sandbox) {
        return Err("Task has an invalid provider sandbox setting.".into());
    }
    if task.cwd.trim().is_empty() {
        return Err("Task folder cannot be empty.".into());
    }
    let local_opencode_config = if host.kind == "local" && task.provider == "opencode" {
        collaboration
            .map(|(_, helper)| {
                merge_opencode_mcp_config(
                    std::env::var("OPENCODE_CONFIG_CONTENT").ok().as_deref(),
                    helper,
                )
            })
            .transpose()?
    } else {
        None
    };
    let args = match task.provider.as_str() {
        "codex" => codex_args(task, collaboration.map(|(_, helper)| helper)),
        "claude" => {
            let mut args = adapters::claude::args(task);
            if let Some(path) = claude_mcp_config_path {
                args.extend(["--mcp-config".into(), path.to_string_lossy().into_owned()]);
                if collaboration.is_some() {
                    args.extend(["--allowedTools".into(), claude_tool_names()]);
                }
            } else if let Some((_, helper)) = collaboration {
                // --mcp-config layers this config over normal Claude settings.
                // --strict-mcp-config is intentionally absent so user servers remain.
                args.extend([
                    "--mcp-config".into(),
                    claude_mcp_config(helper),
                    "--allowedTools".into(),
                    claude_tool_names(),
                ]);
            }
            args
        }
        "opencode" => adapters::opencode::args(task),
        "hermes" => adapters::hermes::args(task, &host.hermes_path, host.kind == "local"),
        _ => {
            return Err(format!(
                "Provider '{}' is not implemented in Monitter.",
                task.provider
            ))
        }
    };
    let cli = configured_path(host, &task.provider)?;
    let program = if task.provider == "hermes" {
        adapters::hermes::BRIDGE_PROGRAM
    } else {
        cli
    };
    let mut command = if host.kind == "local" {
        let executable = if task.provider == "hermes" {
            PathBuf::from(program)
        } else {
            resolve_local_provider(&task.provider, cli)?
        };
        let mut command = Command::new(executable);
        command.args(&args).current_dir(&task.cwd);
        command
    } else if host.kind == "ssh" {
        let mut command = Command::new("ssh");
        add_ssh_options(&mut command, host);
        command.arg(ssh_target(host)?).arg(remote_runner(
            if task.provider == "hermes" {
                adapters::hermes::BRIDGE_PROGRAM
            } else {
                remote_cli(host, &task.provider)?
            },
            &task.cwd,
            &args,
        ));
        command
    } else {
        return Err("Host kind must be local or ssh.".into());
    };
    isolate_child(&mut command);
    if host.kind == "local" {
        if let Some((grant, _helper)) = collaboration {
            command.env("MONITTER_ENDPOINT", &grant.endpoint);
            command.env("MONITTER_TOKEN", &grant.token);
            if task.provider == "opencode" {
                // The documented inline config layer preserves normal config
                // sources. OpenCode exposes no verified server-only approval
                // CLI setting, so its existing permission policy is retained.
                command.env(
                    "OPENCODE_CONFIG_CONTENT",
                    local_opencode_config.as_deref().unwrap_or_default(),
                );
            }
        }
    }
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    Ok(command)
}

const OPENCODE_EXPORT_TIMEOUT: Duration = Duration::from_secs(8);
const OPENCODE_EXPORT_LIMIT: usize = 1024 * 1024;

fn opencode_export_command(host: &Host, task: &Task, session_id: &str) -> Result<Command, String> {
    let cli = configured_path(host, "opencode")?;
    let mut command = if host.kind == "local" {
        let mut command = Command::new(resolve_local_provider("opencode", cli)?);
        command
            .args(["export", session_id])
            .current_dir(&task.cwd)
            .env("PWD", &task.cwd);
        command
    } else if host.kind == "ssh" {
        let mut command = Command::new("ssh");
        add_ssh_options(&mut command, host);
        command.arg(ssh_target(host)?).arg(format!(
            "cd {} && {}",
            remote_path(&task.cwd),
            remote_exec(remote_cli(host, "opencode")?, ["export", session_id]),
        ));
        command
    } else {
        return Err("Host kind must be local or ssh.".into());
    };
    isolate_child(&mut command);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    Ok(command)
}

fn read_bounded<R: Read + Send + 'static>(
    mut pipe: R,
    limit: usize,
) -> mpsc::Receiver<Result<Vec<u8>, String>> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let result = (|| {
            let mut bytes = Vec::new();
            let mut buffer = [0_u8; 8192];
            loop {
                let count = pipe.read(&mut buffer).map_err(|error| error.to_string())?;
                if count == 0 {
                    break;
                }
                if bytes.len().saturating_add(count) > limit {
                    // Keep draining the process pipe, but reject an oversized
                    // transcript rather than retaining raw conversation data.
                    while pipe.read(&mut buffer).map_err(|error| error.to_string())? != 0 {}
                    return Err("OpenCode session metadata was too large to inspect safely.".into());
                }
                bytes.extend_from_slice(&buffer[..count]);
            }
            Ok(bytes)
        })();
        let _ = sender.send(result);
    });
    receiver
}

fn opencode_export_directory(
    host: &Host,
    task: &Task,
    session_id: &str,
    control: &RunControl,
) -> Result<String, String> {
    opencode_export_directory_with_timeout(host, task, session_id, control, OPENCODE_EXPORT_TIMEOUT)
}

fn opencode_export_directory_with_timeout(
    host: &Host,
    task: &Task,
    session_id: &str,
    control: &RunControl,
    timeout: Duration,
) -> Result<String, String> {
    let mut command = opencode_export_command(host, task, session_id)?;
    let mut child = command
        .spawn()
        .map_err(|error| format!("Could not inspect OpenCode session: {error}"))?;
    let stdout = read_bounded(
        child
            .stdout
            .take()
            .ok_or("Could not read OpenCode session metadata.")?,
        OPENCODE_EXPORT_LIMIT,
    );
    let stderr = read_bounded(
        child
            .stderr
            .take()
            .ok_or("Could not read OpenCode session diagnostics.")?,
        64 * 1024,
    );
    let deadline = Instant::now() + timeout;
    let status = loop {
        if control.cancelled.load(Ordering::SeqCst) {
            terminate_bounded(&mut child);
            return Err("OpenCode session inspection was cancelled.".into());
        }
        if let Some(status) = child
            .try_wait()
            .map_err(|error| format!("Could not inspect OpenCode session: {error}"))?
        {
            break status;
        }
        if Instant::now() >= deadline {
            terminate_bounded(&mut child);
            return Err("OpenCode session inspection timed out.".into());
        }
        thread::sleep(Duration::from_millis(25));
    };
    // The process has exited; give its reader threads a small independent
    // grace period to observe EOF instead of racing a just-expired deadline.
    let drain_timeout = deadline
        .saturating_duration_since(Instant::now())
        .max(Duration::from_millis(250))
        .min(Duration::from_secs(1));
    let output = match stdout.recv_timeout(drain_timeout) {
        Ok(output) => output?,
        Err(_) => {
            // A descendant retaining an inherited pipe must not keep this
            // metadata probe alive after its direct child has exited.
            signal_child(&mut child, libc::SIGTERM);
            return Err("OpenCode session inspection timed out.".into());
        }
    };
    let stderr_timeout = deadline
        .saturating_duration_since(Instant::now())
        .max(Duration::from_millis(250))
        .min(Duration::from_secs(1));
    let diagnostics = match stderr.recv_timeout(stderr_timeout) {
        Ok(diagnostics) => diagnostics?,
        Err(_) => {
            signal_child(&mut child, libc::SIGTERM);
            return Err("OpenCode session inspection timed out.".into());
        }
    };
    if !status.success() {
        let _ = diagnostics;
        return Err("Could not inspect OpenCode session.".into());
    }
    let value = parse_opencode_export_metadata(&output)?;
    let info = value
        .get("info")
        .ok_or("OpenCode session metadata was invalid.")?;
    if info.get("id").and_then(Value::as_str) != Some(session_id) {
        return Err("OpenCode session metadata did not match the requested session.".into());
    }
    let directory = info
        .get("directory")
        .and_then(Value::as_str)
        .filter(|directory| {
            !directory.trim().is_empty()
                && !directory.contains('\0')
                && std::path::Path::new(directory).is_absolute()
        })
        .ok_or("OpenCode session metadata did not contain an absolute folder.")?;
    Ok(directory.into())
}

/// OpenCode 1.18 writes a human-readable export banner before the JSON document.
/// Keep accepting the older JSON-only output, but only strip this known banner so
/// diagnostics or malformed output cannot be mistaken for session metadata.
fn parse_opencode_export_metadata(output: &[u8]) -> Result<Value, String> {
    if let Ok(value) = serde_json::from_slice(output) {
        return Ok(value);
    }
    let output = std::str::from_utf8(output)
        .map_err(|_| "OpenCode session metadata was invalid.".to_string())?;
    let Some((banner, json)) = output.split_once('\n') else {
        return Err("OpenCode session metadata was invalid.".into());
    };
    if !banner.trim().starts_with("Exporting session:") {
        return Err("OpenCode session metadata was invalid.".into());
    }
    serde_json::from_str(json).map_err(|_| "OpenCode session metadata was invalid.".to_string())
}

pub fn build_probe_command(host: &Host, provider: &str) -> Result<Command, String> {
    let cli = configured_path(host, provider)?;
    let mut command = if host.kind == "local" {
        let mut command = Command::new(resolve_local_provider(provider, cli)?);
        command.arg("--version");
        command
    } else if host.kind == "ssh" {
        let mut command = Command::new("ssh");
        add_ssh_options(&mut command, host);
        command
            .arg(ssh_target(host)?)
            .arg(remote_exec(remote_cli(host, provider)?, ["--version"]));
        command
    } else {
        return Err("Host kind must be local or ssh.".into());
    };
    isolate_child(&mut command);
    Ok(command)
}

pub fn resume_command(host: &Host, task: &Task, native: &str) -> Result<String, String> {
    let cli = if host.kind == "local" {
        resolve_local_provider(&task.provider, configured_path(host, &task.provider)?)?
            .display()
            .to_string()
    } else {
        remote_cli(host, &task.provider)?.into()
    };
    let resume = match task.provider.as_str() {
        "codex" => format!("resume {}", posix_quote(native)),
        "claude" => format!("--resume {}", posix_quote(native)),
        "opencode" => format!("--session {}", posix_quote(native)),
        "hermes" => format!("--resume {}", posix_quote(native)),
        _ => {
            return Err(format!(
                "Provider '{}' has no terminal resume command.",
                task.provider
            ))
        }
    };
    let inner = format!(
        "cd {} && {} {}",
        if host.kind == "ssh" {
            remote_path(&task.cwd)
        } else {
            posix_quote(&task.cwd)
        },
        if host.kind == "ssh" {
            remote_path(&cli)
        } else {
            posix_quote(&cli)
        },
        resume
    );
    if host.kind == "local" {
        return Ok(inner);
    }
    if host.kind != "ssh" {
        return Err("Host kind must be local or ssh.".into());
    }
    let mut args = vec![
        "ssh".to_string(),
        "-o".into(),
        "BatchMode=yes".into(),
        "-o".into(),
        "ConnectTimeout=8".into(),
        "-o".into(),
        "StrictHostKeyChecking=yes".into(),
    ];
    if host.port != 0 {
        args.extend(["-p".into(), host.port.to_string()]);
    }
    if !host.identity_file.trim().is_empty() {
        let identity = if let Some(rest) = host.identity_file.trim().strip_prefix("~/") {
            local_home().join(rest)
        } else {
            PathBuf::from(host.identity_file.trim())
        };
        args.extend(["-i".into(), identity.display().to_string()]);
    }
    args.push(ssh_target(host)?);
    args.push(inner);
    Ok(args
        .iter()
        .map(|arg| posix_quote(arg))
        .collect::<Vec<_>>()
        .join(" "))
}

#[derive(Default)]
pub struct Parsed {
    pub native_session_id: Option<String>,
    pub assistant: Option<String>,
    pub event: Option<(String, String, String)>,
    pub failed: bool,
}

#[derive(Clone, Default)]
pub struct NormalizedUsage {
    pub classification: String,
    pub provider_turn_id: Option<String>,
    pub tokens: UsageTokens,
    pub cost_usd: Option<f64>,
    pub duration_ms: Option<i64>,
    pub api_duration_ms: Option<i64>,
    pub provider_turns: Option<i64>,
    pub context: Option<UsageContext>,
}

fn integer(value: Option<&Value>) -> Option<i64> {
    value.and_then(Value::as_i64).filter(|v| *v >= 0)
}
fn number(value: Option<&Value>) -> Option<f64> {
    value
        .and_then(Value::as_f64)
        .filter(|v| v.is_finite() && *v >= 0.0)
}
/// The small adapter boundary used by every harness. It deliberately accepts
/// only documented numeric fields and leaves unknown provider payload intact
/// only in the diagnostic RunEvent.
pub fn normalize_usage(
    value: &Value,
    classification: &str,
    provider_turn_id: Option<String>,
) -> Option<NormalizedUsage> {
    let usage = value.get("usage").unwrap_or(value);
    let tokens = usage.get("tokens").unwrap_or(usage);
    let result = NormalizedUsage {
        classification: classification.into(),
        provider_turn_id,
        tokens: UsageTokens {
            input: integer(tokens.get("input_tokens").or_else(|| tokens.get("input"))),
            output: integer(tokens.get("output_tokens").or_else(|| tokens.get("output"))),
            cache_read: integer(
                tokens
                    .get("cache_read_input_tokens")
                    .or_else(|| tokens.get("cacheRead")),
            ),
            cache_write: integer(
                tokens
                    .get("cache_creation_input_tokens")
                    .or_else(|| tokens.get("cacheWrite")),
            ),
            reasoning: integer(
                tokens
                    .get("reasoning_tokens")
                    .or_else(|| tokens.get("reasoning")),
            ),
            total: integer(tokens.get("total_tokens").or_else(|| tokens.get("total"))),
        },
        cost_usd: number(
            usage
                .get("total_cost_usd")
                .or_else(|| usage.get("cost"))
                .or_else(|| usage.get("cost_usd")),
        ),
        duration_ms: integer(usage.get("duration_ms")),
        api_duration_ms: integer(usage.get("duration_api_ms")),
        provider_turns: integer(usage.get("num_turns")),
        context: match (integer(usage.get("used")), integer(usage.get("size"))) {
            (Some(used), Some(size)) => Some(UsageContext { used, size }),
            _ => None,
        },
    };
    (result.tokens.input.is_some()
        || result.tokens.output.is_some()
        || result.tokens.cache_read.is_some()
        || result.tokens.cache_write.is_some()
        || result.tokens.reasoning.is_some()
        || result.tokens.total.is_some()
        || result.cost_usd.is_some()
        || result.duration_ms.is_some()
        || result.api_duration_ms.is_some()
        || result.provider_turns.is_some()
        || result.context.is_some())
    .then_some(result)
}

/// Hermes' gateway exposes a request ID that must be echoed on its dedicated
/// control pipe. Other adapters have their own transport work; do not infer a
/// response protocol from a generic tool event.
fn hermes_approval(parsed: &Parsed) -> Option<(String, String, String, String)> {
    let (kind, _, detail) = parsed.event.as_ref()?;
    if kind != "approval" {
        return None;
    }
    let value: Value = serde_json::from_str(detail).ok()?;
    let request_id = value.get("requestId")?.as_str()?.trim();
    if request_id.is_empty() {
        return None;
    }
    let tool = value
        .get("tool")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("Hermes tool")
        .to_owned();
    let summary = value
        .get("summary")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("Hermes requests approval")
        .to_owned();
    let detail = value
        .get("detail")
        .cloned()
        .unwrap_or(Value::Null)
        .to_string();
    Some((request_id.into(), tool, summary, detail))
}

/// Claude Code's headless SDK transport emits permission prompts as a control
/// request rather than as an assistant tool/result message. Preserve the exact
/// input that Claude supplied: an approval response may carry it back as
/// `updatedInput`, and rewriting it would authorize a different operation.
fn claude_approval(value: &Value) -> Option<(String, String, Value, String, String, String)> {
    if value.get("type").and_then(Value::as_str) != Some("control_request") {
        return None;
    }
    let request_id = value.get("request_id")?.as_str()?.trim();
    let request = value.get("request")?;
    if request.get("subtype").and_then(Value::as_str) != Some("can_use_tool")
        || request_id.is_empty()
    {
        return None;
    }
    let tool = request
        .get("tool_name")
        .and_then(Value::as_str)
        .filter(|name| !name.trim().is_empty())
        .unwrap_or("Claude tool")
        .to_owned();
    let input = request.get("input").cloned().unwrap_or(Value::Null);
    let summary = request
        .get("title")
        .or_else(|| request.get("display_name"))
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| format!("Claude requests {tool}"));
    let detail = serde_json::json!({
        "input": input.clone(),
        "description": request.get("description"),
        "decisionReason": request.get("decision_reason"),
        "decisionReasonType": request.get("decision_reason_type"),
        "toolUseId": request.get("tool_use_id"),
        "requiresUserInteraction": request.get("requires_user_interaction"),
    })
    .to_string();
    let risk = match tool.as_str() {
        "Bash" | "PowerShell" => "high",
        "Write" | "Edit" | "NotebookEdit" => "medium",
        _ => "unknown",
    }
    .to_owned();
    Some((request_id.into(), tool, input, summary, detail, risk))
}

fn json_detail(value: Option<&Value>) -> String {
    value
        .map(|value| match value {
            Value::String(text) => text.clone(),
            other => other.to_string(),
        })
        .unwrap_or_default()
}

pub fn parse_codex_event(value: &Value) -> Parsed {
    let ty = value.get("type").and_then(Value::as_str).unwrap_or("");
    let native_session_id = ["thread_id", "threadId", "session_id", "sessionId"]
        .iter()
        .find_map(|key| value.get(key).and_then(Value::as_str).map(str::to_owned))
        .or_else(|| {
            value
                .pointer("/thread/id")
                .and_then(Value::as_str)
                .map(str::to_owned)
        });
    let item = value.get("item").unwrap_or(value);
    let item_type = item.get("type").and_then(Value::as_str).unwrap_or("");
    let text = item
        .get("text")
        .and_then(Value::as_str)
        .or_else(|| item.get("content").and_then(Value::as_str))
        .or_else(|| value.get("text").and_then(Value::as_str))
        .map(str::to_owned);

    // Computer Use returns screenshots as a real MCP result image. This is
    // distinct from a UserMessage local_image and is safe to associate with
    // the following assistant response from this same native run.
    if matches!(item_type, "McpToolCall" | "mcp_tool_call")
        && matches!(ty, "item.completed" | "item_completed")
        && matches!(
            item.get("server").and_then(Value::as_str),
            Some("cua_repl" | "mcp__cua_repl")
        )
        && item.get("tool").and_then(Value::as_str) == Some("js")
    {
        if let Some(data) = item
            .pointer("/result/content")
            .and_then(Value::as_array)
            .and_then(|content| {
                content.iter().find_map(|entry| {
                    (entry.get("type").and_then(Value::as_str) == Some("image"))
                        .then(|| entry.get("data").and_then(Value::as_str))
                        .flatten()
                })
            })
        {
            return Parsed {
                native_session_id,
                assistant: None,
                event: Some((
                    "computer_image".into(),
                    "Computer screenshot".into(),
                    data.into(),
                )),
                failed: false,
            };
        }
    }

    if matches!(item_type, "agent_message" | "assistant_message") && ty == "item.completed" {
        return Parsed {
            native_session_id,
            assistant: text.clone(),
            event: text.map(|text| ("output".into(), "Assistant response".into(), text)),
            failed: false,
        };
    }
    if item_type == "reasoning" {
        return Parsed {
            native_session_id,
            assistant: None,
            event: Some((
                "reasoning".into(),
                "Reasoning".into(),
                text.unwrap_or_default(),
            )),
            failed: false,
        };
    }
    if matches!(
        item_type,
        "command_execution" | "mcp_tool_call" | "file_change" | "web_search"
    ) || item_type.contains("tool")
        || ty.contains("tool")
    {
        let title = item
            .get("command")
            .and_then(Value::as_str)
            .or_else(|| item.get("name").and_then(Value::as_str))
            .or_else(|| item.get("tool").and_then(Value::as_str))
            .unwrap_or(item_type);
        let detail = json_detail(
            item.get("aggregated_output")
                .or_else(|| item.get("output"))
                .or_else(|| item.get("result"))
                .or_else(|| item.get("error"))
                .or_else(|| item.get("arguments"))
                .or_else(|| item.get("changes"))
                .or_else(|| item.get("query"))
                .or_else(|| item.get("status")),
        );
        return Parsed {
            native_session_id,
            assistant: None,
            event: Some((
                "tool".into(),
                if title.is_empty() {
                    "Tool activity".into()
                } else {
                    title.into()
                },
                detail,
            )),
            failed: false,
        };
    }
    if ty.contains("usage") || value.get("usage").is_some() {
        return Parsed {
            native_session_id,
            assistant: None,
            event: Some((
                "usage".into(),
                "Usage updated".into(),
                json_detail(value.get("usage")),
            )),
            failed: false,
        };
    }
    if ty.contains("error") || ty.contains("failed") {
        let detail = value
            .pointer("/error/message")
            .and_then(Value::as_str)
            .or_else(|| value.get("message").and_then(Value::as_str))
            .or_else(|| value.get("error").and_then(Value::as_str))
            .unwrap_or("Codex reported an error");
        return Parsed {
            native_session_id,
            assistant: None,
            event: Some(("error".into(), "Codex error".into(), detail.into())),
            failed: true,
        };
    }
    if ty.ends_with("started") || ty.ends_with("completed") {
        return Parsed {
            native_session_id,
            assistant: None,
            event: Some(("status".into(), ty.into(), String::new())),
            failed: false,
        };
    }
    Parsed {
        native_session_id,
        assistant: None,
        event: None,
        failed: false,
    }
}

pub fn parse_events(provider: &str, value: &Value) -> Vec<Parsed> {
    match provider {
        "codex" => parse_codex_events(value),
        "claude" => adapters::claude::parse_event(value),
        "opencode" => adapters::opencode::parse_event(value),
        "hermes" => adapters::hermes::parse_event(value),
        _ => Vec::new(),
    }
}

fn parse_codex_events(value: &Value) -> Vec<Parsed> {
    let mut events = vec![parse_codex_event(value)];
    let ty = value.get("type").and_then(Value::as_str).unwrap_or("");
    let item = value.get("item").unwrap_or(value);
    let server = item.get("server").and_then(Value::as_str).unwrap_or("");
    let tool = item
        .get("tool")
        .and_then(Value::as_str)
        .or_else(|| item.get("name").and_then(Value::as_str))
        .unwrap_or("");
    let is_computer = item.get("type").and_then(Value::as_str) == Some("mcp_tool_call")
        && (matches!(server, "computer-use" | "cua_repl")
            || server.contains("computer")
            || (server.contains("browser") && tool.contains("computer"))
            || tool == "computer_use");
    if is_computer && matches!(ty, "item.started" | "item.completed") {
        let phase = if ty == "item.started" {
            "started"
        } else {
            "completed"
        };
        let detail = serde_json::json!({"id": item.get("id").cloned().unwrap_or(Value::Null), "phase": phase, "tool": tool, "summary": json_detail(item.get("arguments").or_else(|| item.get("result")).or_else(|| item.get("error"))) }).to_string();
        events.push(Parsed {
            native_session_id: None,
            assistant: None,
            event: Some(("computer".into(), "Computer activity".into(), detail)),
            failed: false,
        });
    }
    events
}

pub struct RunControl {
    cancelled: AtomicBool,
    // Process lifecycle is deliberately independent from the task's durable
    // status. A completed task may retain a healthy resident transport, while
    // a failed task may have no transport at all.
    lifecycle: Mutex<RuntimeLifecycle>,
    lifecycle_changed: Condvar,
    // This is a stop reason, not cancellation. Readers use it to suppress
    // automatic transport repair after the collector intentionally closes a
    // resident process.
    planned_retirement: AtomicBool,
    teardown_started: AtomicBool,
    owned_process_group: std::sync::atomic::AtomicI32,
    // A runtime with work Monitter cannot prove it owns must remain resident.
    // Adapters may hold this pin around owned background/tool work.
    acp_permission_waits: AtomicUsize,
    // Captured once the resident session is ready, before a model turn. A
    // later descendant is conservatively treated as unowned background work.
    runtime_process_baseline: Mutex<Option<HashSet<(libc::pid_t, i64)>>>,
    // Once provider activity has shown an actual tool invocation, descendants
    // must retain the original session-ready baseline. This prevents a tool
    // from making its own background child look like infrastructure.
    tool_work_observed: AtomicBool,
    // A Claude stream-json process survives between user turns. It remains in
    // the registry while its task is completed so a later message can use the
    // same native context rather than creating a --resume subprocess.
    resident: AtomicBool,
    /// Monitter-owned identity for the current provider turn. It is minted on
    /// every send, including later turns on resident transports.
    run_id: Mutex<String>,
    run_started_at: Mutex<i64>,
    child: Mutex<Option<Child>>,
    control_stdin: Mutex<Option<ChildStdin>>,
    // Codex app-server is a resident JSON-RPC transport.  Keep its thread and
    // current turn separate from the generic child handle so stale
    // notifications cannot mutate a later task turn.
    app_server_thread: Mutex<Option<String>>,
    app_server_turn: Mutex<Option<String>>,
    app_server_next_request: Mutex<i64>,
    app_server_turn_requests: Mutex<HashSet<i64>>,
    // Steers share the current turn instead of beginning a new one. Keep the
    // durable queue record and its expected native turn together so the
    // reader can either confirm the send or safely return it to the queue.
    app_server_steer_requests: Mutex<HashMap<i64, AppServerSteerRequest>>,
    acp_config_requests: Mutex<HashSet<i64>>,
    // A later ACP turn is reserved before its optional model configuration is
    // sent. This makes the config acknowledgement a real ordering barrier and
    // prevents two UI sends from racing into the same resident transport.
    acp_turn_reserved: Mutex<bool>,
    acp_prompt_after_config: Mutex<Option<String>>,
    mcp_fingerprint: Mutex<Option<String>>,
    acp_session_result: Mutex<Option<Value>>,
    app_server_instance_id: String,
    acp_transport: AtomicBool,
    acp_control: Mutex<Option<mpsc::SyncSender<AcpControlFrame>>>,
    auxiliary: Mutex<Vec<Child>>,
    remote_helper_cleanups: Mutex<Vec<(Host, String)>>,
    remote_supervised: bool,
}

/// A control frame for the ACP-owned stdin writer.  Most frames only need
/// queueing; cancellation can request a bounded flush acknowledgement before
/// the reader tears down the transport.
pub(crate) struct AcpControlFrame {
    pub(crate) frame: String,
    pub(crate) flushed: Option<mpsc::SyncSender<Result<(), String>>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RuntimePhase {
    Dormant,
    Active,
    Idle { since: Instant },
    Retiring,
}

#[derive(Debug)]
struct RuntimeLifecycle {
    phase: RuntimePhase,
    resume_supported: bool,
    retiring_idle_since: Option<Instant>,
    inflight_events: usize,
}

/// Keeps one reader record inside its ownership fence until all durable event
/// writes for that record have returned. Retirement cannot claim in between.
pub(crate) struct RuntimeEventPermit {
    control: Arc<RunControl>,
}

impl Drop for RuntimeEventPermit {
    fn drop(&mut self) {
        if let Ok(mut lifecycle) = self.control.lifecycle.lock() {
            lifecycle.inflight_events = lifecycle.inflight_events.saturating_sub(1);
            self.control.lifecycle_changed.notify_all();
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct AppServerSteerRequest {
    pub queued_message_id: String,
    pub expected_turn_id: String,
}

impl RunControl {
    pub fn new(remote_supervised: bool) -> Arc<Self> {
        Arc::new(Self {
            cancelled: AtomicBool::new(false),
            lifecycle: Mutex::new(RuntimeLifecycle {
                phase: RuntimePhase::Dormant,
                resume_supported: false,
                retiring_idle_since: None,
                inflight_events: 0,
            }),
            lifecycle_changed: Condvar::new(),
            planned_retirement: AtomicBool::new(false),
            teardown_started: AtomicBool::new(false),
            owned_process_group: std::sync::atomic::AtomicI32::new(0),
            acp_permission_waits: AtomicUsize::new(0),
            runtime_process_baseline: Mutex::new(None),
            tool_work_observed: AtomicBool::new(false),
            resident: AtomicBool::new(false),
            run_id: Mutex::new(crate::model::id()),
            run_started_at: Mutex::new(crate::model::now()),
            child: Mutex::new(None),
            control_stdin: Mutex::new(None),
            app_server_thread: Mutex::new(None),
            app_server_turn: Mutex::new(None),
            app_server_next_request: Mutex::new(10),
            app_server_turn_requests: Mutex::new(HashSet::new()),
            app_server_steer_requests: Mutex::new(HashMap::new()),
            acp_config_requests: Mutex::new(HashSet::new()),
            acp_turn_reserved: Mutex::new(false),
            acp_prompt_after_config: Mutex::new(None),
            mcp_fingerprint: Mutex::new(None),
            acp_session_result: Mutex::new(None),
            app_server_instance_id: crate::model::id(),
            acp_transport: AtomicBool::new(false),
            acp_control: Mutex::new(None),
            auxiliary: Mutex::new(Vec::new()),
            remote_helper_cleanups: Mutex::new(Vec::new()),
            remote_supervised,
        })
    }

    pub(crate) fn set_mcp_fingerprint(&self, fingerprint: Option<String>) {
        if let Ok(mut value) = self.mcp_fingerprint.lock() {
            *value = fingerprint;
        }
    }

    pub(crate) fn mcp_fingerprint(&self) -> Option<String> {
        self.mcp_fingerprint
            .lock()
            .ok()
            .and_then(|value| value.clone())
    }

    pub fn cancel(&self) {
        self.reserve_cancellation();
        if self.acp_transport.load(Ordering::SeqCst) {
            // ACP's reader owns session/cancel. It waits for outstanding
            // permission workers to write their mandatory cancelled response
            // before it emits that native cancellation and tears down.
            return;
        }
        // Never hold a service/run lock behind a potentially blocked pipe
        // write. Cancellation is invoked from UI-facing paths, so detach the
        // owned stdin and let a short-lived writer attempt the advisory
        // interrupt while process-group signalling proceeds immediately.
        let thread_id = self
            .app_server_thread
            .try_lock()
            .ok()
            .and_then(|v| v.clone());
        let turn_id = self.app_server_turn.try_lock().ok().and_then(|v| v.clone());
        let stdin = self
            .control_stdin
            .try_lock()
            .ok()
            .and_then(|mut slot| slot.take());
        let acp_transport = self.acp_transport.load(Ordering::SeqCst);
        if let Some(mut stdin) = stdin {
            thread::spawn(move || {
                if !acp_transport {
                    if let (Some(thread_id), Some(turn_id)) = (thread_id, turn_id) {
                        let frame = serde_json::json!({
                            "id": 0,
                            "method": "turn/interrupt",
                            "params": {"threadId": thread_id, "turnId": turn_id}
                        });
                        let _ = stdin
                            .write_all(frame.to_string().as_bytes())
                            .and_then(|_| stdin.write_all(b"\n"))
                            .and_then(|_| stdin.flush());
                    }
                }
                // EOF is intentional: cancellation ends this resident
                // transport and its reader performs bounded owned teardown.
            });
        }
        if self.remote_supervised {
            self.cleanup_auxiliary();
            return;
        }
        if let Ok(mut slot) = self.child.lock() {
            if let Some(child) = slot.as_mut() {
                signal_child(child, libc::SIGINT);
            }
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    /// Fence this exact runtime from later turns without performing I/O.
    /// Service mutations use this while holding data -> runs, then invoke
    /// `cancel` only after their durable write and waiter notifications.
    pub(crate) fn reserve_cancellation(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    /// Reserve this owner for a new provider turn. Existing startup paths may
    /// call this while already active; the important atomic boundary is that a
    /// collector which has claimed retirement cannot be overtaken by a send.
    pub(crate) fn begin_run(&self) -> Result<(String, i64), String> {
        if self.is_cancelled() {
            return Err("This runtime has been cancelled.".into());
        }
        {
            let mut lifecycle = self
                .lifecycle
                .lock()
                .map_err(|_| "Monitter runtime lifecycle lock failed.".to_string())?;
            if lifecycle.phase == RuntimePhase::Retiring {
                return Err(
                    "This runtime is retiring; wait for its owned teardown before sending again."
                        .into(),
                );
            }
            if self.is_cancelled() {
                return Err("This runtime has been cancelled.".into());
            }
            lifecycle.phase = RuntimePhase::Active;
            lifecycle.retiring_idle_since = None;
        }
        let started_at = crate::model::now();
        *self
            .run_id
            .lock()
            .map_err(|_| "Monitter run identity lock failed.".to_string())? = crate::model::id();
        *self
            .run_started_at
            .lock()
            .map_err(|_| "Monitter run identity lock failed.".to_string())? = started_at;
        Ok((self.current_run_id()?, started_at))
    }

    /// Record idleness after the caller has durably completed the turn. The
    /// first timestamp wins so read-only polling cannot extend collection.
    pub(crate) fn mark_idle(&self) {
        if let Ok(mut lifecycle) = self.lifecycle.lock() {
            if lifecycle.phase == RuntimePhase::Active {
                lifecycle.phase = RuntimePhase::Idle {
                    since: Instant::now(),
                };
            }
        }
    }

    pub(crate) fn set_resume_supported(&self, supported: bool) {
        if let Ok(mut lifecycle) = self.lifecycle.lock() {
            lifecycle.resume_supported = supported;
        }
    }

    pub(crate) fn begin_event_processing(self: &Arc<Self>) -> Option<RuntimeEventPermit> {
        let mut lifecycle = self.lifecycle.lock().ok()?;
        if lifecycle.phase == RuntimePhase::Retiring || self.is_planned_retirement() {
            return None;
        }
        lifecycle.inflight_events = lifecycle.inflight_events.saturating_add(1);
        Some(RuntimeEventPermit {
            control: Arc::clone(self),
        })
    }

    pub(crate) fn acp_permission_wait_started(&self) {
        self.acp_permission_waits.fetch_add(1, Ordering::SeqCst);
    }

    pub(crate) fn acp_permission_wait_finished(&self) {
        let _ =
            self.acp_permission_waits
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |value| {
                    value.checked_sub(1)
                });
    }

    /// Lets the ACP reader preserve required permission responses before it
    /// sends session/cancel. This is intentionally bounded and uses no
    /// service locks; cancellation itself remains fenced immediately.
    pub(crate) fn wait_for_acp_permission_waits(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            if self.acp_permission_waits.load(Ordering::SeqCst) == 0 {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    /// Capture the exact owned process tree while the harness is session-ready
    /// and before its first prompt. Sampling failures fail closed: a runtime
    /// without a verified baseline is never idle-collected.
    pub(crate) fn capture_runtime_process_baseline(&self) -> Result<(), String> {
        let root = self
            .child
            .lock()
            .map_err(|_| "Monitter provider child lock failed.".to_string())?
            .as_ref()
            .map(Child::id)
            .map(|pid| pid as libc::pid_t)
            .ok_or("Resident provider process is not available for ownership capture.")?;
        self.capture_runtime_process_baseline_for_pid(root)
    }

    fn capture_runtime_process_baseline_for_pid(&self, root: libc::pid_t) -> Result<(), String> {
        let baseline = crate::process_metrics::retirement_process_tree(root)?
            .into_iter()
            .collect::<HashSet<_>>();
        *self
            .runtime_process_baseline
            .lock()
            .map_err(|_| "Monitter runtime ownership lock failed.".to_string())? = Some(baseline);
        Ok(())
    }

    /// Freeze descendant ownership after the provider reports tool work. The
    /// baseline lock serializes this with a no-tool infrastructure refresh.
    pub(crate) fn mark_tool_work_observed(&self) {
        let Ok(_baseline) = self.runtime_process_baseline.lock() else {
            // A poisoned ownership lock already fails retirement closed.
            return;
        };
        self.tool_work_observed.store(true, Ordering::SeqCst);
    }

    /// Some configured MCP/provider helpers start lazily after session setup.
    /// A completed no-tool turn may refresh that infrastructure baseline. Once
    /// any tool activity is observed this permanently refuses to refresh, so a
    /// tool-created descendant continues to pin the resident runtime.
    ///
    /// This performs process inspection and callers must invoke it outside
    /// service/data/run-registry locks.
    pub(crate) fn refresh_runtime_process_baseline_if_no_tool_work(&self) -> Result<bool, String> {
        let root = self
            .child
            .lock()
            .map_err(|_| "Monitter provider child lock failed.".to_string())?
            .as_ref()
            .map(Child::id)
            .map(|pid| pid as libc::pid_t)
            .ok_or("Resident provider process is not available for ownership capture.")?;
        let mut baseline = self
            .runtime_process_baseline
            .lock()
            .map_err(|_| "Monitter runtime ownership lock failed.".to_string())?;
        if self.tool_work_observed.load(Ordering::SeqCst) {
            return Ok(false);
        }
        let refreshed = crate::process_metrics::retirement_process_tree(root)?
            .into_iter()
            .collect::<HashSet<_>>();
        // Keep earlier identities as well: a verified startup helper may
        // have detached since capture, but teardown still owns its lifetime.
        baseline.get_or_insert_with(HashSet::new).extend(refreshed);
        Ok(true)
    }

    /// Adopt lazily initialized provider infrastructure only while this owner
    /// remains conclusively idle. Holding the lifecycle lock across the
    /// sample prevents a later send from changing the owner to Active between
    /// the no-tool check and ownership capture. This is deliberately stricter
    /// than the completed-turn refresh above: collectors run concurrently
    /// with new accepted sends.
    pub(crate) fn refresh_runtime_process_baseline_while_idle_if_no_tool_work(
        &self,
    ) -> Result<bool, String> {
        if self.is_cancelled()
            || self.acp_permission_waits.load(Ordering::SeqCst) != 0
            || self.has_app_server_turn_request()
            || self.has_pending_runtime_request()
        {
            return Ok(false);
        }
        let lifecycle = self
            .lifecycle
            .lock()
            .map_err(|_| "Monitter runtime lifecycle lock failed.".to_string())?;
        if !matches!(lifecycle.phase, RuntimePhase::Idle { .. })
            || lifecycle.inflight_events != 0
            || self.tool_work_observed.load(Ordering::SeqCst)
        {
            return Ok(false);
        }
        let root = self
            .child
            .lock()
            .map_err(|_| "Monitter provider child lock failed.".to_string())?
            .as_ref()
            .map(Child::id)
            .map(|pid| pid as libc::pid_t)
            .ok_or("Resident provider process is not available for ownership capture.")?;
        let mut baseline = self
            .runtime_process_baseline
            .lock()
            .map_err(|_| "Monitter runtime ownership lock failed.".to_string())?;
        // `mark_tool_work_observed` serializes on this same lock. Checking
        // again immediately before extending prevents an observed tool from
        // being adopted if it arrived while this method waited for ownership.
        if self.tool_work_observed.load(Ordering::SeqCst) {
            return Ok(false);
        }
        let refreshed = crate::process_metrics::retirement_process_tree(root)?
            .into_iter()
            .collect::<HashSet<_>>();
        baseline.get_or_insert_with(HashSet::new).extend(refreshed);
        drop(baseline);
        drop(lifecycle);
        Ok(true)
    }

    /// This deliberately does OS I/O and must be called outside service/data
    /// locks. New descendants or a failed sample mean retirement is unsafe.
    pub(crate) fn retirement_process_tree_is_safe(&self) -> bool {
        let root = match self.child.lock() {
            Ok(child) => child.as_ref().map(Child::id).map(|pid| pid as libc::pid_t),
            Err(_) => return false,
        };
        let Some(root) = root else {
            return cfg!(test);
        };
        let baseline = match self.runtime_process_baseline.lock() {
            Ok(value) => value.clone(),
            Err(_) => return false,
        };
        let Some(baseline) = baseline else {
            return false;
        };
        match crate::process_metrics::retirement_process_tree(root) {
            Ok(current) => current
                .into_iter()
                .all(|identity| baseline.contains(&identity)),
            Err(_) => false,
        }
    }

    #[cfg(test)]
    pub(crate) fn retirement_diagnostic(&self) -> String {
        let root = self.child.lock().unwrap().as_ref().map(Child::id);
        format!(
            "phase={:?} cancelled={} permissions={} pending={} tool_work={} baseline={:?} current={:?}",
            self.lifecycle.lock().unwrap(),
            self.is_cancelled(),
            self.acp_permission_waits.load(Ordering::SeqCst),
            self.has_pending_runtime_request(),
            self.tool_work_observed.load(Ordering::SeqCst),
            self.runtime_process_baseline.lock().unwrap(),
            root.map(|pid| crate::process_metrics::retirement_process_tree(pid as libc::pid_t)),
        )
    }

    /// Atomically claim an eligible idle runtime for planned retirement. The
    /// service still performs durable queue/request checks under its
    /// data-then-runs lock; these local checks fence in-flight protocol work.
    pub(crate) fn try_retire_idle(&self, now: Instant, timeout: Duration) -> bool {
        if self.is_cancelled()
            || self.acp_permission_waits.load(Ordering::SeqCst) != 0
            || self.has_app_server_turn_request()
            || self.has_pending_runtime_request()
        {
            return false;
        }
        let Ok(mut lifecycle) = self.lifecycle.lock() else {
            return false;
        };
        let RuntimePhase::Idle { since } = lifecycle.phase else {
            return false;
        };
        if lifecycle.inflight_events != 0
            || !lifecycle.resume_supported
            || now.saturating_duration_since(since) <= timeout
        {
            return false;
        }
        lifecycle.phase = RuntimePhase::Retiring;
        lifecycle.retiring_idle_since = Some(since);
        self.teardown_started.store(false, Ordering::SeqCst);
        self.planned_retirement.store(true, Ordering::SeqCst);
        true
    }

    fn has_pending_runtime_request(&self) -> bool {
        self.current_app_server_turn().is_some()
            || self
                .acp_turn_reserved
                .lock()
                .map(|value| *value)
                .unwrap_or(true)
            || self
                .acp_prompt_after_config
                .lock()
                .map(|value| value.is_some())
                .unwrap_or(true)
            || self
                .acp_config_requests
                .lock()
                .map(|value| !value.is_empty())
                .unwrap_or(true)
            || self
                .app_server_steer_requests
                .lock()
                .map(|value| !value.is_empty())
                .unwrap_or(true)
    }

    pub(crate) fn is_retiring(&self) -> bool {
        self.lifecycle
            .lock()
            .map(|lifecycle| lifecycle.phase == RuntimePhase::Retiring)
            .unwrap_or(true)
    }

    pub(crate) fn is_idle(&self) -> bool {
        self.lifecycle
            .lock()
            .map(|lifecycle| matches!(lifecycle.phase, RuntimePhase::Idle { .. }))
            .unwrap_or(false)
    }

    /// Cheap prefilter before process inspection. Retirement still rechecks
    /// the clock and all lifecycle guards when it claims this owner.
    pub(crate) fn idle_timeout_elapsed(&self, now: Instant, timeout: Duration) -> bool {
        self.lifecycle
            .lock()
            .map(|lifecycle| match lifecycle.phase {
                RuntimePhase::Idle { since } => now.saturating_duration_since(since) > timeout,
                _ => false,
            })
            .unwrap_or(false)
    }

    /// A resident transport may be retained after an interrupted durable task
    /// while its replacement session/resume handshake is still active.
    pub(crate) fn is_active(&self) -> bool {
        self.lifecycle
            .lock()
            .map(|lifecycle| lifecycle.phase == RuntimePhase::Active)
            .unwrap_or(false)
    }

    pub(crate) fn is_planned_retirement(&self) -> bool {
        self.planned_retirement.load(Ordering::SeqCst)
    }

    /// Wait outside service locks for a collector-owned teardown/release.
    pub(crate) fn wait_for_teardown(&self, timeout: Duration) -> Result<(), String> {
        let lifecycle = self
            .lifecycle
            .lock()
            .map_err(|_| "Monitter runtime lifecycle lock failed.".to_string())?;
        let (lifecycle, _) = self
            .lifecycle_changed
            .wait_timeout_while(lifecycle, timeout, |value| {
                value.phase == RuntimePhase::Retiring
            })
            .map_err(|_| "Monitter runtime lifecycle wait failed.".to_string())?;
        if lifecycle.phase == RuntimePhase::Retiring {
            return Err("Runtime teardown is still in progress.".into());
        }
        Ok(())
    }

    /// Must be called only after the service has pointer-checked registry
    /// release and `retire_owned` has reaped all owned resources. A teardown
    /// failure intentionally leaves the runtime fenced in Retiring.
    pub(crate) fn finish_retirement(&self) {
        if let Ok(mut lifecycle) = self.lifecycle.lock() {
            if lifecycle.phase == RuntimePhase::Retiring {
                lifecycle.phase = RuntimePhase::Dormant;
                lifecycle.retiring_idle_since = None;
                self.lifecycle_changed.notify_all();
            }
        }
    }

    /// Undo a reservation only before pipes/processes are touched. This keeps
    /// an ownership-inspection race conservative without fencing a healthy
    /// runtime forever; its original continuous-idle timestamp is retained.
    pub(crate) fn abort_retirement(&self) -> bool {
        if self.teardown_started.load(Ordering::SeqCst) {
            return false;
        }
        let Ok(mut lifecycle) = self.lifecycle.lock() else {
            return false;
        };
        if lifecycle.phase != RuntimePhase::Retiring {
            return false;
        }
        let since = lifecycle
            .retiring_idle_since
            .take()
            .unwrap_or_else(Instant::now);
        lifecycle.phase = RuntimePhase::Idle { since };
        self.planned_retirement.store(false, Ordering::SeqCst);
        self.lifecycle_changed.notify_all();
        true
    }
    pub(crate) fn current_run_id(&self) -> Result<String, String> {
        self.run_id
            .lock()
            .map(|id| id.clone())
            .map_err(|_| "Monitter run identity lock failed.".to_string())
    }
    pub(crate) fn run_started_at(&self) -> Result<i64, String> {
        self.run_started_at
            .lock()
            .map(|at| *at)
            .map_err(|_| "Monitter run identity lock failed.".to_string())
    }

    /// Sends a provider control frame only while this run still owns an
    /// intentionally persistent stdin channel. This is never used to inject
    /// another user prompt into a completed or unrelated process.
    pub(crate) fn send_control(&self, frame: &str) -> Result<(), String> {
        if self.acp_transport.load(Ordering::SeqCst) {
            let sender = self
                .acp_control
                .lock()
                .map_err(|_| "ACP control queue unavailable.".to_string())?
                .clone()
                .ok_or("ACP control channel is closed.")?;
            return sender
                .try_send(AcpControlFrame {
                    frame: frame.into(),
                    flushed: None,
                })
                .map_err(|_| "ACP control channel is busy or closed.".into());
        }
        if self.cancelled.load(Ordering::SeqCst) {
            return Err("Task was stopped before the approval response could be sent.".into());
        }
        let mut stdin = self
            .control_stdin
            .lock()
            .map_err(|_| "Monitter provider control lock failed.".to_string())?;
        let stdin = stdin
            .as_mut()
            .ok_or("This provider run has no interactive approval channel.")?;
        stdin
            .write_all(frame.as_bytes())
            .and_then(|_| stdin.write_all(b"\n"))
            .and_then(|_| stdin.flush())
            .map_err(|error| format!("Could not send approval response to provider: {error}"))
    }

    /// Queue an ACP control frame and wait only for its writer to flush it.
    /// This is used for the final session/cancel after pending permission
    /// outcomes have been emitted.
    pub(crate) fn send_acp_control_flushed(
        &self,
        frame: &str,
        timeout: Duration,
    ) -> Result<(), String> {
        let sender = self
            .acp_control
            .lock()
            .map_err(|_| "ACP control queue unavailable.".to_string())?
            .clone()
            .ok_or("ACP control channel is closed.")?;
        let (flushed, receiver) = mpsc::sync_channel(1);
        sender
            .try_send(AcpControlFrame {
                frame: frame.into(),
                flushed: Some(flushed),
            })
            .map_err(|_| "ACP control channel is busy or closed.".to_string())?;
        receiver
            .recv_timeout(timeout)
            .map_err(|_| "ACP cancellation frame was not flushed in time.".to_string())?
    }

    /// Start another turn on an already initialized app-server transport.  The
    /// caller only uses this after `is_resident`; it never falls back to an
    /// uncertain one-shot `exec resume` invocation.
    pub(crate) fn send_user_turn(&self, prompt: &str) -> Result<(), String> {
        self.send_user_turn_with_task(prompt, None)
    }

    /// Starts a resident Codex turn with the task's latest saved settings.
    /// Callers that have refreshed the task snapshot should use this variant;
    /// Claude continues to use `send_user_turn` and its native stream frame.
    pub(crate) fn send_user_turn_with_task(
        &self,
        prompt: &str,
        task: Option<&Task>,
    ) -> Result<(), String> {
        let thread_id = self
            .app_server_thread
            .lock()
            .map_err(|_| "Codex app-server state lock failed.".to_string())?
            .clone();
        // Claude's pre-existing stream-json resident transport shares this
        // method. It has no app-server thread ID and must retain its native
        // frame instead of receiving a JSON-RPC turn/start request.
        let Some(thread_id) = thread_id else {
            return self.send_control(&adapters::claude::user_frame(prompt).to_string());
        };
        let request_id = {
            let mut next = self
                .app_server_next_request
                .lock()
                .map_err(|_| "Codex app-server request lock failed.".to_string())?;
            let id = *next;
            *next = next.saturating_add(1);
            id
        };
        let mut params = serde_json::json!({
            "threadId": thread_id,
            "input": [{"type":"text", "text":prompt, "text_elements": []}]
        });
        if let Some(task) = task {
            params["cwd"] = Value::String(task.cwd.clone());
            params["approvalPolicy"] = Value::String(
                if task.sandbox == "yolo" {
                    "never"
                } else {
                    "on-request"
                }
                .into(),
            );
            params["sandboxPolicy"] = match task.sandbox.as_str() {
                "workspace-write" => {
                    serde_json::json!({"type":"workspaceWrite","writableRoots":[task.cwd],"networkAccess":false,"excludeTmpdirEnvVar":false,"excludeSlashTmp":false})
                }
                "yolo" => serde_json::json!({"type":"dangerFullAccess"}),
                _ => serde_json::json!({"type":"readOnly","networkAccess":false}),
            };
            params["model"] = if task.model.trim().is_empty() {
                Value::Null
            } else {
                Value::String(task.model.clone())
            };
            if let Some(settings) = &task.model_settings {
                if let Some(effort) = &settings.reasoning_effort {
                    params["effort"] = Value::String(effort.clone());
                }
                if let Some(fast) = settings.fast_mode {
                    params["serviceTierForTurn"] =
                        Value::String(if fast { "priority" } else { "default" }.into());
                }
            }
        }
        let frame = serde_json::json!({
            "id": request_id,
            "method": "turn/start",
            "params": params
        });
        self.app_server_turn_requests
            .lock()
            .map_err(|_| "Codex app-server request lock failed.".to_string())?
            .insert(request_id);
        if let Err(error) = self.send_control(&frame.to_string()) {
            let _ = self.take_app_server_turn_request(request_id);
            return Err(error);
        }
        Ok(())
    }

    /// Append a user follow-up to the current Codex app-server turn. Unlike a
    /// normal resident send this must not call `begin_run`, alter task
    /// settings, or clear the current turn identity.
    pub(crate) fn send_app_server_steer(
        &self,
        prompt: &str,
        queued_message_id: String,
    ) -> Result<(), String> {
        if self.is_cancelled() {
            return Err("Task was stopped before the follow-up could be steered.".into());
        }
        let thread_id = self
            .current_app_server_thread()
            .ok_or("Codex is still starting; the follow-up cannot be steered yet.")?;
        let expected_turn_id = self
            .current_app_server_turn()
            .ok_or("Codex has no active turn to steer.")?;
        let request_id = {
            let mut next = self
                .app_server_next_request
                .lock()
                .map_err(|_| "Codex app-server request lock failed.".to_string())?;
            let id = *next;
            *next = next.saturating_add(1);
            id
        };
        let frame = serde_json::json!({
            "id": request_id,
            "method": "turn/steer",
            "params": {
                "threadId": thread_id,
                "expectedTurnId": expected_turn_id,
                "input": [{"type": "text", "text": prompt, "text_elements": []}]
            }
        });
        let request = AppServerSteerRequest {
            queued_message_id,
            expected_turn_id,
        };
        self.app_server_steer_requests
            .lock()
            .map_err(|_| "Codex app-server steering lock failed.".to_string())?
            .insert(request_id, request);
        if let Err(error) = self.send_control(&frame.to_string()) {
            let _ = self.take_app_server_steer_request(request_id);
            return Err(error);
        }
        Ok(())
    }

    /// Start a later ACP prompt on the owned resident stdio transport. ACP has
    /// no universal resume command; the session id is established by its
    /// initialize/session handshake and all prompt data stays on stdin.
    pub(crate) fn send_acp_turn(&self, prompt: &str, task: &Task) -> Result<(), String> {
        {
            let mut reserved = self
                .acp_turn_reserved
                .lock()
                .map_err(|_| "ACP turn reservation lock failed.".to_string())?;
            if *reserved {
                return Err("An ACP turn is already pending for this task.".into());
            }
            *reserved = true;
        }
        if let Err(error) = self.send_acp_turn_reserved(prompt, task) {
            self.clear_acp_turn_reservation();
            return Err(error);
        }
        Ok(())
    }

    fn send_acp_turn_reserved(&self, prompt: &str, task: &Task) -> Result<(), String> {
        // Recovery owns the same resident slot while it reloads the saved
        // session. A newly accepted explicit user send waits through the
        // bounded handshake rather than falling through to a competing run or
        // writing to the retired stdin queue.
        // Recovery can consume a cold provider initialization plus saved
        // session loading. Share ACP's phase bounds instead of encoding a
        // shorter loop count that rejects a healthy 60-second initialize.
        let deadline = Instant::now()
            + crate::acp_runtime::INITIALIZE_TIMEOUT
            + crate::acp_runtime::SESSION_TIMEOUT
            + Duration::from_secs(5);
        let session_id = loop {
            let session_id = self
                .app_server_thread
                .lock()
                .map_err(|_| "ACP session state lock failed.".to_string())?
                .clone();
            if self.is_cancelled() || session_id.is_some() || Instant::now() >= deadline {
                break session_id;
            }
            thread::sleep(Duration::from_millis(25));
        };
        if self.is_cancelled() {
            return Err("ACP session recovery was cancelled before it became ready.".into());
        }
        let session_id = session_id.ok_or("ACP session recovery did not become ready in time.")?;
        if task
            .model_settings
            .as_ref()
            .is_some_and(|s| s.fast_mode.is_some() || s.reasoning_effort.is_some())
        {
            return Err(
                "ACP has not advertised support for saved fast mode or reasoning effort settings."
                    .into(),
            );
        }
        let config = self
            .acp_session_result
            .lock()
            .map_err(|_| "ACP session configuration lock failed.".to_string())?
            .clone()
            .unwrap_or(Value::Null);
        let model_request =
            crate::acp_session_config::configured_model_request(&config, &session_id, &task.model)?;
        let request_id = {
            let mut next = self
                .app_server_next_request
                .lock()
                .map_err(|_| "ACP request lock failed.".to_string())?;
            let id = *next;
            *next = next.saturating_add(1);
            id
        };
        self.mark_app_server_turn_request(request_id)?;
        self.set_app_server_turn(format!("acp:{request_id}"));
        let prompt_frame = crate::acp_protocol::request(
            serde_json::json!(request_id),
            "session/prompt",
            serde_json::json!({
                "sessionId": session_id,
                "prompt": [{"type":"text", "text": prompt}]
            }),
        )
        .to_string();
        if let Some((method, params)) = model_request {
            let config_id = {
                let mut next = self
                    .app_server_next_request
                    .lock()
                    .map_err(|_| "ACP request lock failed.".to_string())?;
                let id = *next;
                *next = next.saturating_add(1);
                id
            };
            self.acp_config_requests
                .lock()
                .map_err(|_| "ACP config request lock failed.".to_string())?
                .insert(config_id);
            *self
                .acp_prompt_after_config
                .lock()
                .map_err(|_| "ACP pending prompt lock failed.".to_string())? = Some(prompt_frame);
            if let Err(error) = self.send_control(
                &crate::acp_protocol::request(serde_json::json!(config_id), method, params)
                    .to_string(),
            ) {
                let _ = self.take_acp_config_request(config_id);
                let _ = self.take_app_server_turn_request(request_id);
                let _ = self.take_acp_prompt_after_config();
                return Err(error);
            }
            return Ok(());
        }
        if let Err(error) = self.send_control(&prompt_frame) {
            let _ = self.take_app_server_turn_request(request_id);
            return Err(error);
        }
        Ok(())
    }

    pub(crate) fn set_acp_session_result(&self, value: Value) {
        if let Ok(mut slot) = self.acp_session_result.lock() {
            *slot = Some(value);
        }
    }

    pub(crate) fn acp_model_catalog(&self) -> Result<crate::model::ModelCatalog, String> {
        let result = self
            .acp_session_result
            .lock()
            .map_err(|_| "ACP session configuration lock failed.")?;
        crate::acp_session_config::model_catalog(result.as_ref().unwrap_or(&Value::Null))
    }

    pub(crate) fn update_acp_config_options(&self, options: &Value) -> Result<(), String> {
        crate::acp_session_config::parse_options(options)?;
        let mut result = self
            .acp_session_result
            .lock()
            .map_err(|_| "ACP session configuration lock failed.")?;
        let result = result.get_or_insert_with(|| serde_json::json!({}));
        if !result.is_object() {
            *result = serde_json::json!({});
        }
        result["configOptions"] = options.clone();
        Ok(())
    }
    pub(crate) fn take_acp_config_request(&self, id: i64) -> bool {
        self.acp_config_requests
            .lock()
            .ok()
            .is_some_and(|mut ids| ids.remove(&id))
    }

    pub(crate) fn take_acp_prompt_after_config(&self) -> Option<String> {
        self.acp_prompt_after_config.lock().ok()?.take()
    }

    pub(crate) fn clear_acp_turn_reservation(&self) {
        if let Ok(mut reserved) = self.acp_turn_reserved.lock() {
            *reserved = false;
        }
        if let Ok(mut pending) = self.acp_prompt_after_config.lock() {
            *pending = None;
        }
    }

    pub(crate) fn mark_acp_transport(&self) {
        self.acp_transport.store(true, Ordering::SeqCst);
    }

    pub(crate) fn set_acp_control(&self, sender: mpsc::SyncSender<AcpControlFrame>) {
        if let Ok(mut slot) = self.acp_control.lock() {
            *slot = Some(sender);
        }
    }

    pub(crate) fn set_app_server_thread(&self, thread_id: String) {
        if let Ok(mut thread) = self.app_server_thread.lock() {
            *thread = Some(thread_id);
        }
    }

    pub(crate) fn set_app_server_turn(&self, turn_id: String) {
        if let Ok(mut turn) = self.app_server_turn.lock() {
            *turn = Some(turn_id);
        }
    }

    pub(crate) fn clear_app_server_turn(&self) {
        if let Ok(mut turn) = self.app_server_turn.lock() {
            *turn = None;
        }
    }

    pub(crate) fn app_server_turn_is_current(&self, thread_id: &str, turn_id: &str) -> bool {
        self.app_server_thread
            .lock()
            .ok()
            .and_then(|value| value.clone())
            .as_deref()
            == Some(thread_id)
            && self
                .app_server_turn
                .lock()
                .ok()
                .and_then(|value| value.clone())
                .as_deref()
                == Some(turn_id)
            && !self.is_cancelled()
    }

    pub(crate) fn matches_app_server_turn(&self, turn_id: &str) -> bool {
        !turn_id.is_empty()
            && self
                .app_server_turn
                .lock()
                .ok()
                .and_then(|value| value.clone())
                .as_deref()
                == Some(turn_id)
            && !self.is_cancelled()
    }

    pub(crate) fn current_app_server_turn(&self) -> Option<String> {
        self.app_server_turn
            .lock()
            .ok()
            .and_then(|turn| turn.clone())
    }

    pub(crate) fn current_app_server_thread(&self) -> Option<String> {
        self.app_server_thread
            .lock()
            .ok()
            .and_then(|thread| thread.clone())
    }

    pub(crate) fn matches_app_server_thread(&self, thread_id: &str) -> bool {
        !thread_id.is_empty()
            && self
                .app_server_thread
                .lock()
                .ok()
                .and_then(|value| value.clone())
                .as_deref()
                == Some(thread_id)
            && !self.is_cancelled()
    }

    pub(crate) fn mark_app_server_turn_request(&self, id: i64) -> Result<(), String> {
        self.app_server_turn_requests
            .lock()
            .map_err(|_| "Codex app-server request lock failed.".to_string())?
            .insert(id);
        Ok(())
    }

    pub(crate) fn take_app_server_turn_request(&self, id: i64) -> bool {
        self.app_server_turn_requests
            .lock()
            .map(|mut requests| requests.remove(&id))
            .unwrap_or(false)
    }

    pub(crate) fn take_app_server_steer_request(&self, id: i64) -> Option<AppServerSteerRequest> {
        self.app_server_steer_requests
            .lock()
            .ok()
            .and_then(|mut requests| requests.remove(&id))
    }

    pub(crate) fn has_app_server_turn_request(&self) -> bool {
        self.app_server_turn_requests
            .lock()
            .map(|requests| !requests.is_empty())
            .unwrap_or(true)
    }

    pub(crate) fn app_server_request_key(&self, turn_id: &str, rpc_id: &str) -> String {
        format!("{}:{turn_id}:{rpc_id}", self.app_server_instance_id)
    }

    pub(crate) fn mark_resident(&self) {
        self.resident.store(true, Ordering::SeqCst);
    }

    /// An ACP reader observed that its stdout has closed.  Do not leave this
    /// control selectable for another prompt while the replacement handshake
    /// is running; a queued stdin writer is not proof that the agent received
    /// a prompt.
    pub(crate) fn retire_acp_transport(&self) {
        self.resident.store(false, Ordering::SeqCst);
        if let Ok(mut sender) = self.acp_control.lock() {
            *sender = None;
        }
    }

    /// Best-effort, deliberately small diagnostic for a closed ACP pipe.  Do
    /// not expose argv, environment, or provider JSON in the activity log.
    pub(crate) fn acp_exit_diagnostic(&self) -> Option<String> {
        let mut child = self.child.lock().ok()?;
        let status = child.as_mut()?.try_wait().ok().flatten()?;
        let code = status
            .code()
            .map(|code| code.to_string())
            .unwrap_or_else(|| "signal".into());
        *child = None;
        Some(format!("ACP process exited ({code})."))
    }

    pub(crate) fn is_resident(&self) -> bool {
        self.resident.load(Ordering::SeqCst)
            && !self.cancelled.load(Ordering::SeqCst)
            && !self.is_planned_retirement()
    }

    fn add_auxiliary(&self, mut child: Child) {
        if self.cancelled.load(Ordering::SeqCst) {
            terminate_bounded(&mut child);
            return;
        }
        if let Ok(mut children) = self.auxiliary.lock() {
            children.push(child);
        } else {
            terminate_bounded(&mut child);
        }
    }

    fn cleanup_auxiliary(&self) {
        if let Ok(mut children) = self.auxiliary.lock() {
            for child in children.iter_mut() {
                terminate_bounded(child);
            }
            children.clear();
        }
        let cleanups = self
            .remote_helper_cleanups
            .lock()
            .map(|mut entries| std::mem::take(&mut *entries))
            .unwrap_or_default();
        for (host, directory) in cleanups {
            cleanup_remote_helper(&host, &directory);
        }
    }

    pub(crate) fn install(
        &self,
        child: Child,
        control_stdin: Option<ChildStdin>,
    ) -> Result<(), (Child, Option<ChildStdin>)> {
        let mut slot = match self.child.lock() {
            Ok(slot) => slot,
            Err(_) => return Err((child, control_stdin)),
        };
        let mut stdin_slot = match self.control_stdin.lock() {
            Ok(slot) => slot,
            Err(_) => return Err((child, control_stdin)),
        };
        self.owned_process_group
            .store(child.id() as i32, Ordering::SeqCst);
        *slot = Some(child);
        *stdin_slot = control_stdin;
        drop(stdin_slot);
        drop(slot);
        if self.cancelled.load(Ordering::SeqCst) {
            self.cancel();
        }
        Ok(())
    }

    fn wait(&self) -> Result<ExitStatus, String> {
        let mut cancelled_at = None;
        loop {
            let cancelled = self.cancelled.load(Ordering::SeqCst);
            let mut slot = self
                .child
                .lock()
                .map_err(|_| "Codex child lock failed.".to_string())?;
            let child = slot
                .as_mut()
                .ok_or_else(|| "Codex child process disappeared.".to_string())?;
            if cancelled && cancelled_at.is_none() {
                if !self.remote_supervised {
                    signal_child(child, libc::SIGINT);
                }
                cancelled_at = Some(Instant::now());
            }
            if cancelled_at
                .map(|at| {
                    !self.remote_supervised
                        && at.elapsed() >= Duration::from_secs(1)
                        && at.elapsed() < Duration::from_secs(3)
                })
                .unwrap_or(false)
            {
                signal_child(child, libc::SIGTERM);
            }
            if cancelled_at
                .map(|at| at.elapsed() >= Duration::from_secs(3))
                .unwrap_or(false)
            {
                // A supervised SSH run normally exits after control EOF. This
                // is a bounded transport fallback if the remote host vanished.
                signal_child(child, libc::SIGKILL);
            }
            if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
                slot.take();
                drop(slot);
                self.cleanup_auxiliary();
                return Ok(status);
            }
            drop(slot);
            thread::sleep(Duration::from_millis(40));
        }
    }

    pub(crate) fn terminate_owned(&self) {
        self.cancel();
        let _ = self.wait();
    }

    /// Close only this runtime's owned streams and process group for an idle
    /// collector. This never calls `cancel()`: planned retirement is distinct
    /// from a user interrupt and ACP readers must not repair it as a crash.
    /// The service retains the registry fence until this has succeeded.
    pub(crate) fn retire_owned(&self) -> Result<(), String> {
        if !self.is_retiring() || !self.is_planned_retirement() {
            return Err("Runtime retirement was not reserved.".into());
        }
        if !self.teardown_started.load(Ordering::SeqCst) {
            if !self.retirement_process_tree_is_safe() {
                self.abort_retirement();
                return Err(
                    "Idle retirement skipped because owned process descendants changed.".into(),
                );
            }
            self.teardown_started.store(true, Ordering::SeqCst);
        }
        let baseline = self
            .runtime_process_baseline
            .lock()
            .map_err(|_| "Monitter runtime ownership lock failed.".to_string())?
            .clone()
            .ok_or("Idle runtime has no verified owned process baseline.")?;

        // EOF is the graceful shutdown request for the persistent stdio
        // transports. Dropping all control senders closes ACP's writer queue.
        self.control_stdin
            .lock()
            .map_err(|_| "Monitter provider control lock failed.".to_string())?
            .take();
        self.acp_control
            .lock()
            .map_err(|_| "ACP control queue unavailable.".to_string())?
            .take();

        let graceful_deadline = Instant::now() + Duration::from_secs(1);
        if self.wait_for_owned_exit_until(graceful_deadline, &baseline, false)? {
            self.cleanup_auxiliary();
            return Ok(());
        }
        self.signal_owned(libc::SIGTERM)?;
        self.signal_verified_baseline(&baseline, libc::SIGTERM)?;
        if self.wait_for_owned_exit_until(
            Instant::now() + Duration::from_secs(2),
            &baseline,
            false,
        )? {
            self.cleanup_auxiliary();
            return Ok(());
        }
        self.signal_owned(libc::SIGKILL)?;
        self.signal_verified_baseline(&baseline, libc::SIGKILL)?;
        if self.wait_for_owned_exit_until(
            Instant::now() + Duration::from_secs(1),
            &baseline,
            true,
        )? {
            self.cleanup_auxiliary();
            return Ok(());
        }
        Err("Idle runtime did not exit within the owned teardown deadline.".into())
    }

    fn signal_owned(&self, signal: i32) -> Result<(), String> {
        let process_group = self.owned_process_group.load(Ordering::SeqCst);
        #[cfg(unix)]
        if process_group > 0 {
            // Every launched harness is isolated into this process group.
            // Signalling it covers a helper which survived after its direct
            // parent consumed EOF, without reaching any unrelated terminal.
            unsafe {
                libc::kill(-process_group, signal);
            }
            return Ok(());
        }
        let mut child = self
            .child
            .lock()
            .map_err(|_| "Codex child lock failed.".to_string())?;
        if let Some(child) = child.as_mut() {
            signal_child(child, signal);
        }
        Ok(())
    }

    fn signal_verified_baseline(
        &self,
        baseline: &HashSet<(libc::pid_t, i64)>,
        signal: i32,
    ) -> Result<(), String> {
        for &(pid, started_at) in baseline {
            match crate::process_metrics::retirement_process_identity_alive(pid, started_at) {
                Ok(true) => {
                    // The identity check above prevents signalling a reused
                    // PID. These are captured provider helpers, which may
                    // have called setsid or reparented after their owned root
                    // exited.
                    unsafe {
                        libc::kill(pid, signal);
                    }
                }
                Ok(false) | Err(_) => {
                    // An incomplete macOS process lookup must never broaden
                    // a signal to a PID we cannot still prove we own. The
                    // bounded exit wait retries this identity shortly.
                }
            }
        }
        Ok(())
    }

    fn wait_for_owned_exit_until(
        &self,
        deadline: Instant,
        baseline: &HashSet<(libc::pid_t, i64)>,
        report_final_identity_error: bool,
    ) -> Result<bool, String> {
        let mut last_identity_error = None;
        loop {
            let mut slot = self
                .child
                .lock()
                .map_err(|_| "Codex child lock failed.".to_string())?;
            let root_exited = match slot.as_mut() {
                Some(child) => child
                    .try_wait()
                    .map_err(|error| error.to_string())?
                    .is_some(),
                None => true,
            };
            if root_exited {
                slot.take();
            }
            drop(slot);
            if root_exited && self.owned_process_group_exited() {
                match self.verified_baseline_exited(baseline) {
                    Ok(true) => return Ok(true),
                    Ok(false) => {}
                    Err(error) => last_identity_error = Some(error),
                }
            }
            if Instant::now() >= deadline {
                if report_final_identity_error {
                    if let Some(error) = last_identity_error {
                        return Err(error);
                    }
                }
                return Ok(false);
            }
            thread::sleep(Duration::from_millis(25));
        }
    }

    fn verified_baseline_exited(
        &self,
        baseline: &HashSet<(libc::pid_t, i64)>,
    ) -> Result<bool, String> {
        for &(pid, started_at) in baseline {
            if crate::process_metrics::retirement_process_identity_alive(pid, started_at)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn owned_process_group_exited(&self) -> bool {
        let process_group = self.owned_process_group.load(Ordering::SeqCst);
        if process_group <= 0 {
            return true;
        }
        #[cfg(unix)]
        {
            // kill(pid, 0) checks existence without changing process state.
            // A surviving process group remains our owned teardown target.
            let result = unsafe { libc::kill(-process_group, 0) };
            return result != 0
                && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH);
        }
        #[cfg(not(unix))]
        {
            true
        }
    }
}

fn signal_child(child: &mut Child, signal: i32) {
    #[cfg(unix)]
    unsafe {
        if libc::kill(-(child.id() as i32), signal) == 0 {
            return;
        }
    }
    let _ = child.kill();
}

pub(crate) fn terminate_bounded(child: &mut Child) {
    signal_child(child, libc::SIGTERM);
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        if child.try_wait().ok().flatten().is_some() {
            return;
        }
        thread::sleep(Duration::from_millis(25));
    }
    signal_child(child, libc::SIGKILL);
    let deadline = Instant::now() + Duration::from_secs(1);
    while Instant::now() < deadline {
        if child.try_wait().ok().flatten().is_some() {
            return;
        }
        thread::sleep(Duration::from_millis(25));
    }
}

pub(crate) struct RemoteCollaboration {
    host: Host,
    helper_dir: String,
    pub(crate) helper_path: String,
    pub(crate) endpoint: String,
    tunnel: Child,
}

fn broker_port(endpoint: &str) -> Result<u16, String> {
    let address = endpoint
        .strip_prefix("http://127.0.0.1:")
        .ok_or("Collaboration broker must use a loopback endpoint.")?;
    let port = address
        .split('/')
        .next()
        .ok_or("Collaboration broker endpoint is invalid.")?
        .parse::<u16>()
        .map_err(|_| "Collaboration broker endpoint is invalid.".to_string())?;
    if port == 0 {
        return Err("Collaboration broker endpoint is invalid.".into());
    }
    Ok(port)
}

fn stage_remote_helper(
    host: &Host,
    helper: &PathBuf,
    control: &RunControl,
) -> Result<(String, String), String> {
    let source =
        fs::read(helper).map_err(|error| format!("Cannot read collaboration helper: {error}"))?;
    let mut command = ssh_command(host);
    add_ssh_options(&mut command, host);
    command
        .arg(ssh_target(host)?)
        .arg("umask 077; d=$(mktemp -d /tmp/monitter-mcp.XXXXXXXX) || exit; cat > \"$d/monitter_mcp.py\" && chmod 700 \"$d/monitter_mcp.py\" && printf '__MONITTER_HELPER_DIR__%s\\n' \"$d\"")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    isolate_child(&mut command);
    let mut child = command
        .spawn()
        .map_err(|error| format!("Could not stage collaboration helper on SSH host: {error}"))?;
    let Some(mut stdin) = child.stdin.take() else {
        terminate_bounded(&mut child);
        return Err("Could not open SSH helper staging input.".into());
    };
    let Some(mut stdout) = child.stdout.take() else {
        terminate_bounded(&mut child);
        return Err("Could not read SSH helper staging output.".into());
    };
    let Some(stderr) = child.stderr.take() else {
        terminate_bounded(&mut child);
        return Err("Could not read SSH helper staging diagnostics.".into());
    };
    let (write_sender, write_receiver) = mpsc::channel();
    thread::spawn(move || {
        let result = stdin
            .write_all(&source)
            .and_then(|_| stdin.flush())
            .map_err(|error| error.to_string());
        drop(stdin);
        let _ = write_sender.send(result);
    });
    let (stdout_sender, stdout_receiver) = mpsc::channel();
    thread::spawn(move || {
        let mut output = String::new();
        let mut buffer = [0_u8; 8192];
        let result = (|| {
            loop {
                let count = stdout
                    .read(&mut buffer)
                    .map_err(|error| error.to_string())?;
                if count == 0 {
                    break;
                }
                if output.len().saturating_add(count) <= 4096 {
                    output.push_str(&String::from_utf8_lossy(&buffer[..count]));
                } else {
                    // Keep draining so a noisy remote process cannot block,
                    // but do not accept a valid-looking prefix as staging.
                    while stdout
                        .read(&mut buffer)
                        .map_err(|error| error.to_string())?
                        != 0
                    {}
                    return Err("SSH helper staging output exceeded its limit.".into());
                }
            }
            Ok::<_, String>(output)
        })();
        let _ = stdout_sender.send(result);
    });
    let stderr = title_reader(stderr, 16 * 1024);
    let deadline = Instant::now() + Duration::from_secs(20);
    let status = loop {
        if control.cancelled.load(Ordering::SeqCst) {
            terminate_bounded(&mut child);
            return Err("Collaboration helper staging was cancelled.".into());
        }
        if Instant::now() >= deadline {
            terminate_bounded(&mut child);
            return Err("Timed out staging collaboration helper on SSH host.".into());
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => thread::sleep(Duration::from_millis(25)),
            Err(error) => {
                terminate_bounded(&mut child);
                return Err(format!("Could not read SSH helper staging status: {error}"));
            }
        }
    };
    // Reader and writer threads must not retain staging after the SSH child exits.
    let drain_timeout = deadline
        .saturating_duration_since(Instant::now())
        .max(Duration::from_millis(250))
        .min(Duration::from_secs(1));
    let diagnostics = stderr
        .recv_timeout(drain_timeout)
        .unwrap_or_else(|_| Ok(Vec::new()))?;
    if !status.success() {
        return Err(with_ssh_diagnostic(
            format!(
                "SSH helper staging exited with {}.",
                status
                    .code()
                    .map_or("a signal".into(), |code| format!("status {code}"))
            ),
            &diagnostics,
        ));
    }
    let output = stdout_receiver
        .recv_timeout(drain_timeout)
        .map_err(|_| "Could not read SSH helper staging output.".to_string())??;
    write_receiver
        .recv_timeout(drain_timeout)
        .map_err(|_| "Could not send collaboration helper to SSH host.".to_string())?
        .map_err(|error| format!("Could not send collaboration helper to SSH host: {error}"))?;
    let directory = output
        .strip_prefix("__MONITTER_HELPER_DIR__")
        .and_then(|value| value.lines().next())
        .unwrap_or_default();
    let suffix = directory
        .strip_prefix("/tmp/monitter-mcp.")
        .unwrap_or_default();
    if suffix.len() < 8 || !suffix.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
        return Err("SSH host returned an invalid collaboration helper path.".into());
    }
    if output != format!("__MONITTER_HELPER_DIR__{directory}\n") {
        cleanup_remote_helper(host, directory);
        return Err("SSH host returned unexpected collaboration helper output.".into());
    }
    Ok((directory.into(), format!("{directory}/monitter_mcp.py")))
}

fn start_reverse_tunnel(
    host: &Host,
    endpoint: &str,
    control: &RunControl,
) -> Result<(Child, u16), String> {
    let local_port = broker_port(endpoint)?;
    let mut command = ssh_command(host);
    add_ssh_options(&mut command, host);
    command
        .args(["-N", "-v", "-o", "ExitOnForwardFailure=yes", "-R"])
        .arg(format!("127.0.0.1:0:127.0.0.1:{local_port}"))
        .arg(ssh_target(host)?)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    isolate_child(&mut command);
    let mut child = command
        .spawn()
        .map_err(|error| format!("Could not start SSH collaboration tunnel: {error}"))?;
    let Some(stderr) = child.stderr.take() else {
        terminate_bounded(&mut child);
        return Err("Could not read SSH collaboration tunnel status.".into());
    };
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let mut allocated = false;
        for line in BufReader::new(stderr).lines() {
            if let Ok(line) = line {
                if !allocated {
                    if let Some(port) = line
                        .split("Allocated port ")
                        .nth(1)
                        .and_then(|rest| rest.split_whitespace().next())
                        .and_then(|value| value.parse::<u16>().ok())
                    {
                        allocated = true;
                        let _ = sender.send(Some(port));
                    }
                }
            }
        }
        if !allocated {
            let _ = sender.send(None);
        }
    });
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if control.cancelled.load(Ordering::SeqCst) {
            terminate_bounded(&mut child);
            return Err("Collaboration tunnel setup was cancelled.".into());
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            terminate_bounded(&mut child);
            return Err("Could not establish a loopback-only SSH collaboration tunnel.".into());
        }
        match receiver.recv_timeout(remaining.min(Duration::from_millis(100))) {
            Ok(Some(port)) if port != 0 => return Ok((child, port)),
            Ok(_) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                terminate_bounded(&mut child);
                return Err("Could not establish a loopback-only SSH collaboration tunnel.".into());
            }
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
        }
    }
}

pub(crate) fn prepare_remote_collaboration(
    host: &Host,
    endpoint: &str,
    helper: &PathBuf,
    control: &RunControl,
) -> Result<RemoteCollaboration, String> {
    let (helper_dir, helper_path) = stage_remote_helper(host, helper, control)?;
    match start_reverse_tunnel(host, endpoint, control) {
        Ok((tunnel, port)) => Ok(RemoteCollaboration {
            host: host.clone(),
            helper_dir,
            helper_path,
            endpoint: format!("http://127.0.0.1:{port}/rpc"),
            tunnel,
        }),
        Err(error) => {
            cleanup_remote_helper(host, &helper_dir);
            Err(error)
        }
    }
}

/// Transfers SSH-only collaboration resources to the same owned run as the
/// provider process. `RunControl` stops the reverse tunnel before removing the
/// remote helper, and does both during its normal bounded teardown.
pub(crate) fn attach_remote_collaboration(remote: RemoteCollaboration, control: &RunControl) {
    let RemoteCollaboration {
        host,
        helper_dir,
        tunnel,
        ..
    } = remote;
    if let Ok(mut cleanups) = control.remote_helper_cleanups.lock() {
        cleanups.push((host, helper_dir));
        drop(cleanups);
        // `add_auxiliary` checks cancellation itself. If cancellation won the
        // race it terminates the tunnel, and the registered helper is removed
        // by the same owned teardown path.
        control.add_auxiliary(tunnel);
    } else {
        let mut tunnel = tunnel;
        terminate_bounded(&mut tunnel);
        cleanup_remote_helper(&host, &helper_dir);
    }
}

pub(crate) fn abort_remote_collaboration(mut remote: RemoteCollaboration) {
    terminate_bounded(&mut remote.tunnel);
    cleanup_remote_helper(&remote.host, &remote.helper_dir);
}

fn cleanup_remote_helper(host: &Host, helper_dir: &str) {
    let Ok(target) = ssh_target(host) else {
        return;
    };
    let mut command = ssh_command(host);
    add_ssh_options(&mut command, host);
    command
        .arg(target)
        .arg(format!(
            "rm -f -- {}/monitter_mcp.py && rmdir -- {}",
            posix_quote(helper_dir),
            posix_quote(helper_dir)
        ))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Ok(mut child) = command.spawn() {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if child.try_wait().ok().flatten().is_some() {
                return;
            }
            thread::sleep(Duration::from_millis(25));
        }
        terminate_bounded(&mut child);
    }
}

pub fn start(service: Arc<Service>, task_id: String, prompt: String, control: Arc<RunControl>) {
    thread::spawn(move || {
        let (mut task, host) = match service.task_and_host(&task_id) {
            Ok(value) => value,
            Err(error) => {
                service.finish(&task_id, "error", Some(error));
                return;
            }
        };
        // Codex uses one owned, resident app-server connection per Monitter
        // task.  Do not route it through exec's one-shot JSONL protocol.
        if task.provider == "codex" && host.kind == "local" {
            crate::codex_app_server::start(service, task_id, prompt, control);
            return;
        }
        // ACP is a resident JSON-RPC transport. Never send it through the
        // one-shot provider runner: doing so would lose its session and make
        // a later prompt/recovery ambiguous.
        if task.provider == "acp" {
            crate::acp_runtime::start(service, task_id, prompt, control);
            return;
        }
        let mut extensions = match service.extension_config().map(|config| {
            crate::extensions_runtime::RuntimeExtensions::for_agent(&config, &task.agent_id)
        }) {
            Ok(extensions) => extensions,
            Err(error) => {
                service.finish(&task_id, "error", Some(error));
                return;
            }
        };
        if let Err(error) = extensions.validate_for(&task.provider, &host.kind) {
            service.finish(&task_id, "error", Some(error));
            return;
        }
        control.set_mcp_fingerprint(extensions.mcp_fingerprint());
        let prompt = extensions.prompt(&prompt);
        if task.provider == "claude" && host.kind != "local" {
            service.finish(
                &task_id,
                "error",
                Some(
                    "Claude interactive sessions currently require a local desktop host. The SSH supervisor is not yet a full-duplex JSON transport, so Monitter will not fall back to a one-shot Claude run.".into(),
                ),
            );
            return;
        }
        if task.provider == "opencode" {
            if let Some(session_id) = task.native_session_id.clone() {
                let directory = match opencode_export_directory(&host, &task, &session_id, &control)
                {
                    Ok(directory) => directory,
                    Err(error) => {
                        if control.cancelled.load(Ordering::SeqCst) {
                            service.finish(&task_id, "interrupted", None);
                        } else {
                            service.finish(&task_id, "error", Some(error));
                        }
                        return;
                    }
                };
                match service.restore_opencode_task_directory(&task_id, &session_id, &directory) {
                    Ok(updated) => task = updated,
                    Err(error) => {
                        service.finish(&task_id, "error", Some(error));
                        return;
                    }
                }
            }
        }
        let grant = if matches!(task.provider.as_str(), "codex" | "claude" | "opencode") {
            match service.collaboration_grant(&task_id) {
                Ok(grant) => grant,
                Err(error) => {
                    service.finish(&task_id, "error", Some(error));
                    return;
                }
            }
        } else {
            None
        };
        let helper = match grant.as_ref() {
            Some(_) => match service.collaboration_helper() {
                Ok(path) => Some(path),
                Err(error) => {
                    service.finish(&task_id, "error", Some(error));
                    return;
                }
            },
            None => None,
        };
        let mut remote_collaboration = match (grant.as_ref(), helper.as_ref(), host.kind.as_str()) {
            (Some(grant), Some(helper), "ssh") => {
                match prepare_remote_collaboration(&host, &grant.endpoint, helper, &control) {
                    Ok(remote) => Some(remote),
                    Err(error) => {
                        if control.cancelled.load(Ordering::SeqCst) {
                            service.finish(&task_id, "interrupted", None);
                        } else {
                            service.finish(&task_id, "error", Some(error));
                        }
                        return;
                    }
                }
            }
            _ => None,
        };
        if control.cancelled.load(Ordering::SeqCst) {
            if let Some(mut remote) = remote_collaboration.take() {
                terminate_bounded(&mut remote.tunnel);
                cleanup_remote_helper(&host, &remote.helper_dir);
            }
            service.finish(&task_id, "interrupted", None);
            return;
        }
        let helper_for_command = remote_collaboration
            .as_ref()
            .map(|remote| remote.helper_path.as_str())
            .or_else(|| helper.as_ref().and_then(|path| path.to_str()));
        let grant_for_command = grant
            .as_ref()
            .zip(helper_for_command)
            .map(|(grant, helper)| (grant, helper));
        let mut claude_config_value = extensions.claude_config();
        if task.provider == "claude" {
            if let Some((_, helper)) = grant_for_command {
                if let (Some(target), Some(builtin)) = (
                    claude_config_value
                        .pointer_mut("/mcpServers")
                        .and_then(Value::as_object_mut),
                    serde_json::from_str::<Value>(&claude_mcp_config(helper))
                        .ok()
                        .and_then(|value| value.pointer("/mcpServers/monitter").cloned()),
                ) {
                    target.insert("monitter".into(), builtin);
                }
            }
        }
        let claude_config = if task.provider == "claude"
            && claude_config_value["mcpServers"]
                .as_object()
                .is_some_and(|servers| !servers.is_empty())
        {
            match crate::extensions_runtime::write_private_claude_config(&claude_config_value) {
                Ok(file) => Some(file),
                Err(error) => {
                    service.finish(&task_id, "error", Some(error));
                    return;
                }
            }
        } else {
            None
        };
        let mut command = match build_command_with_options(
            &host,
            &task,
            grant_for_command,
            claude_config.as_ref().map(|file| file.path()),
        ) {
            Ok(command) => command,
            Err(error) => {
                if let Some(mut remote) = remote_collaboration.take() {
                    terminate_bounded(&mut remote.tunnel);
                    cleanup_remote_helper(&host, &remote.helper_dir);
                }
                service.finish(&task_id, "error", Some(error));
                return;
            }
        };
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(error) => {
                if let Some(mut remote) = remote_collaboration.take() {
                    terminate_bounded(&mut remote.tunnel);
                    cleanup_remote_helper(&host, &remote.helper_dir);
                }
                service.finish(
                    &task_id,
                    "error",
                    Some(format!("Could not start {}: {error}", task.provider)),
                );
                return;
            }
        };
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        // Capture Claude's owned tree before its first prompt. Later unknown
        // descendants conservatively pin the runtime rather than being killed
        // by idle GC. An unavailable sample merely disables GC for this owner.
        if task.provider == "claude" {
            let _ = control.capture_runtime_process_baseline_for_pid(child.id() as libc::pid_t);
        }
        let remote_supervised = host.kind == "ssh";
        let remote_helper_dir = remote_collaboration
            .as_ref()
            .map(|remote| remote.helper_dir.clone());
        let remote_endpoint = remote_collaboration
            .as_ref()
            .map(|remote| remote.endpoint.clone());
        let control_stdin = if let Some(mut stdin) = child.stdin.take() {
            let write_result = if remote_supervised {
                let prompt_frame = if task.provider == "hermes" {
                    hermes_prompt_frame(&prompt)
                } else {
                    prompt.clone()
                };
                let collaboration_frame = grant.as_ref().map(|grant| {
                    let mut frame = serde_json::json!({
                        "endpoint": remote_endpoint.as_deref().unwrap_or(&grant.endpoint),
                        "token": &grant.token,
                    });
                    if task.provider == "opencode" {
                        if let Some(helper) = helper_for_command {
                            frame["opencodeHelper"] = Value::String(helper.into());
                        }
                    }
                    frame.to_string()
                });
                if let Some(frame) = collaboration_frame {
                    stdin
                        .write_all(format!("MONITTER/COLLAB/1 {}\n", frame.len()).as_bytes())
                        .and_then(|_| stdin.write_all(frame.as_bytes()))
                        .and_then(|_| {
                            stdin.write_all(
                                format!("MONITTER/1 {}\n", prompt_frame.len()).as_bytes(),
                            )
                        })
                        .and_then(|_| stdin.write_all(prompt_frame.as_bytes()))
                        .and_then(|_| stdin.flush())
                } else {
                    stdin
                        .write_all(format!("MONITTER/1 {}\n", prompt_frame.len()).as_bytes())
                        .and_then(|_| stdin.write_all(prompt_frame.as_bytes()))
                        .and_then(|_| stdin.flush())
                }
            } else if task.provider == "claude" {
                let frame = adapters::claude::user_frame(&prompt).to_string();
                stdin
                    .write_all(frame.as_bytes())
                    .and_then(|_| stdin.write_all(b"\n"))
                    .and_then(|_| stdin.flush())
            } else if task.provider == "hermes" {
                stdin
                    .write_all(hermes_prompt_frame(&prompt).as_bytes())
                    .and_then(|_| stdin.flush())
            } else {
                stdin
                    .write_all(prompt.as_bytes())
                    .and_then(|_| stdin.write_all(b"\n"))
            };
            if let Err(error) = write_result {
                terminate_bounded(&mut child);
                if let Some(mut remote) = remote_collaboration.take() {
                    terminate_bounded(&mut remote.tunnel);
                    cleanup_remote_helper(&host, &remote.helper_dir);
                }
                service.finish(
                    &task_id,
                    "error",
                    Some(format!("Could not send prompt to provider: {error}")),
                );
                return;
            }
            if remote_supervised || matches!(task.provider.as_str(), "hermes" | "claude") {
                Some(stdin)
            } else {
                None
            }
        } else {
            terminate_bounded(&mut child);
            if let Some(mut remote) = remote_collaboration.take() {
                terminate_bounded(&mut remote.tunnel);
                cleanup_remote_helper(&host, &remote.helper_dir);
            }
            service.finish(
                &task_id,
                "error",
                Some("Could not open provider stdin.".into()),
            );
            return;
        };

        if let Some(remote) = remote_collaboration.take() {
            control.add_auxiliary(remote.tunnel);
        }

        if let Err((mut child, _stdin)) = control.install(child, control_stdin) {
            terminate_bounded(&mut child);
            control.cleanup_auxiliary();
            if let Some(directory) = remote_helper_dir.as_deref() {
                cleanup_remote_helper(&host, directory);
            }
            service.finish(&task_id, "interrupted", None);
            return;
        }
        if task.provider == "claude" {
            control.mark_resident();
            // Claude's stream-json session can cold-resume through its saved
            // native session ID. The collector additionally requires that ID
            // to be durable before it can claim this resident owner.
            control.set_resume_supported(true);
        }

        let failed_event = Arc::new(AtomicBool::new(false));
        let assistant_seen = Arc::new(AtomicBool::new(false));
        let task_provider = task.provider.clone();
        let stdout_thread = stdout.map(|stdout| {
            let service = service.clone();
            let task = task_id.clone();
            let provider = task_provider.clone();
            let control = control.clone();
            let failed_event = failed_event.clone();
            let assistant_seen = assistant_seen.clone();
            thread::spawn(move || {
                for line in BufReader::new(stdout).lines() {
                    // `retire_owned` can reap the process before this reader
                    // has drained its final pipe buffer. Those late records
                    // belong to the retired owner and must never be applied
                    // after the registry releases a cold-resumed successor.
                    if control.is_planned_retirement() {
                        break;
                    }
                    let Some(_event_permit) = control.begin_event_processing() else {
                        break;
                    };
                    match line {
                        Ok(line) => match serde_json::from_str::<Value>(&line) {
                            Ok(value) => {
                                let complete_claude_turn = provider == "claude"
                                    && value.get("type").and_then(Value::as_str) == Some("result");
                                if provider == "claude" {
                                    if let Some((request_id, tool, input, summary, detail, risk)) =
                                        claude_approval(&value)
                                    {
                                        control.mark_tool_work_observed();
                                        let request = service.create_approval_request(
                                            CreateApprovalRequest {
                                                task_id: task.clone(),
                                                provider: provider.clone(),
                                                run_id: request_id.clone(),
                                                tool,
                                                summary,
                                                detail,
                                                risk,
                                                raw_input: Some(
                                                    serde_json::json!({"rawInput": input}),
                                                ),
                                            },
                                        );
                                        if let Ok(request) = &request {
                                            if let Ok(mut owners) =
                                                service.app_server_approvals.lock()
                                            {
                                                owners.insert(
                                                    request.id.clone(),
                                                    Arc::downgrade(&control),
                                                );
                                            }
                                        }
                                        let allow = match request.and_then(|request| {
                                            let decision = service
                                                .wait_for_approval(&request.id, || {
                                                    !control.cancelled.load(Ordering::SeqCst)
                                                });
                                            if decision.is_err() {
                                                let _ =
                                                    service.expire_approval_request(&request.id);
                                            }
                                            decision
                                        }) {
                                            Ok(ApprovalDecision::ApproveOnce)
                                            | Ok(ApprovalDecision::ApproveSession)
                                            | Ok(ApprovalDecision::ApproveAlways) => true,
                                            Ok(ApprovalDecision::Deny) | Err(_) => false,
                                        };
                                        let frame = adapters::claude::permission_response(
                                            &request_id,
                                            allow,
                                            &input,
                                        );
                                        if let Err(error) = control.send_control(&frame.to_string())
                                        {
                                            if !control.cancelled.load(Ordering::SeqCst) {
                                                failed_event.store(true, Ordering::SeqCst);
                                                service.record(
                                                    &task,
                                                    "error",
                                                    "Could not apply Claude approval",
                                                    error,
                                                );
                                            }
                                        }
                                    }
                                }
                                for parsed in parse_events(&provider, &value) {
                                    if parsed.event.as_ref().is_some_and(|(kind, _, _)| {
                                        matches!(kind.as_str(), "tool" | "computer")
                                    }) {
                                        control.mark_tool_work_observed();
                                    }
                                    if provider == "hermes" {
                                        if let Some((request_id, tool, summary, detail)) =
                                            hermes_approval(&parsed)
                                        {
                                            let request = service.create_approval_request(
                                                CreateApprovalRequest {
                                                    task_id: task.clone(),
                                                    provider: provider.clone(),
                                                    run_id: request_id.clone(),
                                                    tool,
                                                    summary,
                                                    detail,
                                                    risk: "unknown".into(),
                                                    raw_input: None,
                                                },
                                            );
                                            match request.and_then(|request| {
                                                let decision = service
                                                    .wait_for_approval(&request.id, || {
                                                        !control.cancelled.load(Ordering::SeqCst)
                                                    });
                                                if decision.is_err() {
                                                    let _ = service
                                                        .expire_approval_request(&request.id);
                                                }
                                                decision
                                            }) {
                                                Ok(ApprovalDecision::ApproveOnce)
                                                | Ok(ApprovalDecision::ApproveSession)
                                                | Ok(ApprovalDecision::ApproveAlways) => {
                                                    let frame = serde_json::json!({
                                                        "type": "approval_response",
                                                        "request_id": request_id,
                                                        "decision": "approve_once",
                                                    });
                                                    if let Err(error) =
                                                        control.send_control(&frame.to_string())
                                                    {
                                                        failed_event.store(true, Ordering::SeqCst);
                                                        service.record(
                                                            &task,
                                                            "error",
                                                            "Could not apply approval",
                                                            error,
                                                        );
                                                    }
                                                }
                                                Ok(ApprovalDecision::Deny) | Err(_) => {
                                                    let frame = serde_json::json!({
                                                        "type": "approval_response",
                                                        "request_id": request_id,
                                                        "decision": "deny",
                                                    });
                                                    if let Err(error) =
                                                        control.send_control(&frame.to_string())
                                                    {
                                                        if !control.cancelled.load(Ordering::SeqCst)
                                                        {
                                                            failed_event
                                                                .store(true, Ordering::SeqCst);
                                                            service.record(
                                                                &task,
                                                                "error",
                                                                "Could not deny approval",
                                                                error,
                                                            );
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    if parsed.failed {
                                        failed_event.store(true, Ordering::SeqCst);
                                    }
                                    if parsed.assistant.is_some() {
                                        assistant_seen.store(true, Ordering::SeqCst);
                                    }
                                    if let Err(error) = service.apply_event(&task, parsed) {
                                        failed_event.store(true, Ordering::SeqCst);
                                        service.record(
                                            &task,
                                            "error",
                                            "Provider event rejected",
                                            error,
                                        );
                                    }
                                }
                                // Attribute the final result usage to this
                                // current run before making the resident
                                // transport eligible for its next prompt.
                                if complete_claude_turn {
                                    service.complete_resident_turn(&task, &control);
                                }
                            }
                            Err(error) => {
                                failed_event.store(true, Ordering::SeqCst);
                                service.record(
                                    &task,
                                    "error",
                                    "Invalid provider JSON event",
                                    format!("{error}: {line}"),
                                )
                            }
                        },
                        Err(error) => {
                            failed_event.store(true, Ordering::SeqCst);
                            service.record(
                                &task,
                                "error",
                                "Could not read provider output",
                                error.to_string(),
                            )
                        }
                    }
                }
            })
        });
        let stderr_thread = stderr.map(|stderr| {
            let service = service.clone();
            let task = task_id.clone();
            let control = control.clone();
            thread::spawn(move || {
                let mut diagnostic_recorded = false;
                for line in BufReader::new(stderr).lines() {
                    if control.is_planned_retirement() {
                        break;
                    }
                    let Some(_event_permit) = control.begin_event_processing() else {
                        break;
                    };
                    match line {
                        Ok(line) => {
                            if !diagnostic_recorded {
                                if let Some(detail) = provider_stderr_diagnostic(&line) {
                                    diagnostic_recorded = true;
                                    service.record(&task, "error", "Provider diagnostic", detail);
                                }
                            }
                        }
                        Err(error) => service.record(
                            &task,
                            "error",
                            "Could not read provider stderr",
                            error.to_string(),
                        ),
                    }
                }
            })
        });

        let result = control.wait();
        if let Some(handle) = stdout_thread {
            let _ = handle.join();
        }
        if let Some(handle) = stderr_thread {
            let _ = handle.join();
        }
        if let Some(directory) = remote_helper_dir.as_deref() {
            cleanup_remote_helper(&host, directory);
        }
        // The collector owns planned retirement and performs the pointer-
        // checked registry release once all resources have been reaped. Do
        // not turn its intentional EOF into a task interruption or error.
        if control.is_planned_retirement() {
            return;
        }
        match result {
            Ok(_status) if control.cancelled.load(Ordering::SeqCst) => {
                service.finish(&task_id, "interrupted", None)
            }
            Ok(status) if status.success() && failed_event.load(Ordering::SeqCst) => service
                .finish(
                    &task_id,
                    "error",
                    Some(format!(
                        "{} reported an error in its event stream.",
                        task.provider
                    )),
                ),
            Ok(status) if status.success() && !assistant_seen.load(Ordering::SeqCst) => service
                .finish(
                    &task_id,
                    "error",
                    Some(format!(
                        "{} completed without an assistant response.",
                        task.provider
                    )),
                ),
            Ok(status) if status.success() => service.finish(&task_id, "completed", None),
            Ok(status) => service.finish(
                &task_id,
                "error",
                Some(format!("{} exited with {status}.", task.provider)),
            ),
            Err(error) => service.finish(&task_id, "error", Some(error)),
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{id, now};
    use std::ffi::OsString;

    fn task(native: Option<&str>, model: &str) -> Task {
        Task {
            id: id(),
            agent_id: id(),
            title: "test".into(),
            native_session_id: native.map(str::to_owned),
            status: "idle".into(),
            archived: false,
            created_at: now(),
            updated_at: now(),
            parent_task_id: None,
            channel_id: None,
            host_id: id(),
            cwd: "/tmp/work folder; touch /tmp/no".into(),
            provider: "codex".into(),
            model: model.into(),
            model_settings: None,
            sandbox: "read-only".into(),
            project_id: None,
            acp: None,
        }
    }

    #[test]
    fn retirement_requires_strict_timeout_and_no_reader_in_flight() {
        let control = RunControl::new(false);
        control.set_resume_supported(true);
        control.begin_run().unwrap();
        control.mark_idle();
        let since = match control.lifecycle.lock().unwrap().phase {
            RuntimePhase::Idle { since } => since,
            _ => panic!("expected idle"),
        };
        let timeout = Duration::from_secs(300);
        assert!(!control.idle_timeout_elapsed(since, timeout));
        assert!(!control.idle_timeout_elapsed(since + timeout, timeout));
        assert!(control.idle_timeout_elapsed(since + timeout + Duration::from_nanos(1), timeout));
        assert!(!control.try_retire_idle(since + timeout, timeout));
        let permit = control.begin_event_processing().unwrap();
        assert!(!control.try_retire_idle(since + timeout + Duration::from_nanos(1), timeout));
        drop(permit);
        assert!(control.try_retire_idle(since + timeout + Duration::from_nanos(1), timeout));
        assert!(!control.idle_timeout_elapsed(since + timeout + Duration::from_nanos(1), timeout));
        assert!(control.begin_event_processing().is_none());
        control.finish_retirement();
        // Buffered output remains stale even after the registry barrier opens.
        assert!(control.begin_event_processing().is_none());
    }

    #[test]
    fn idle_retirement_is_atomic_against_a_later_turn_reservation() {
        let control = RunControl::new(false);
        control.set_resume_supported(true);
        control.begin_run().unwrap();
        control.mark_idle();

        assert!(control.try_retire_idle(
            Instant::now() + Duration::from_secs(6),
            Duration::from_secs(5),
        ));
        assert!(control.is_retiring());
        assert!(control.is_planned_retirement());
        assert!(control.begin_run().is_err());

        control.finish_retirement();
        assert!(control.wait_for_teardown(Duration::ZERO).is_ok());
        assert!(control.begin_run().is_ok());
    }

    #[test]
    fn idle_timestamp_is_not_extended_and_pending_permission_blocks_retirement() {
        let control = RunControl::new(false);
        control.set_resume_supported(true);
        control.begin_run().unwrap();
        control.mark_idle();
        // A second completion/read-only observation must retain the original
        // monotonic timestamp rather than extending the GC deadline.
        control.mark_idle();
        control.acp_permission_wait_started();
        assert!(!control.try_retire_idle(
            Instant::now() + Duration::from_secs(6),
            Duration::from_secs(5),
        ));
        control.acp_permission_wait_finished();
        assert!(control.try_retire_idle(
            Instant::now() + Duration::from_secs(6),
            Duration::from_secs(5),
        ));
    }

    #[test]
    fn active_app_server_turn_blocks_idle_retirement() {
        let control = RunControl::new(false);
        control.set_resume_supported(true);
        control.begin_run().unwrap();
        control.mark_resident();
        control.set_app_server_turn("turn-1".into());
        control.mark_idle();
        assert!(!control.try_retire_idle(
            Instant::now() + Duration::from_secs(6),
            Duration::from_secs(5),
        ));
        control.clear_app_server_turn();
        assert!(control.try_retire_idle(
            Instant::now() + Duration::from_secs(6),
            Duration::from_secs(5),
        ));
        assert!(!control.is_resident());
        assert!(control.abort_retirement());
        assert!(control.is_idle());
    }

    #[test]
    fn cancellation_does_not_unfence_a_planned_retirement() {
        let control = RunControl::new(false);
        control.set_resume_supported(true);
        control.begin_run().unwrap();
        control.mark_resident();
        control.mark_idle();
        assert!(control.try_retire_idle(
            Instant::now() + Duration::from_secs(6),
            Duration::from_secs(5),
        ));
        control.cancel();
        assert!(control.is_cancelled());
        assert!(control.is_retiring());
        assert!(control.is_planned_retirement());
        assert!(!control.is_resident());
        assert!(control.begin_run().is_err());
    }

    #[test]
    fn cancellation_reservation_blocks_a_later_turn_before_teardown() {
        let control = RunControl::new(false);
        control.reserve_cancellation();
        assert!(control.is_cancelled());
        assert!(control.begin_run().is_err());
    }

    #[test]
    fn planned_retirement_reaps_an_isolated_owned_process_group() {
        let mut command = Command::new("sh");
        command
            // The shell is the owned root captured in the baseline. EOF makes
            // it leave a background helper behind, exercising the exact race
            // where waiting only for the root would leak a process.
            .args(["-c", "read ignored; sleep 30 & exit 0"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        isolate_child(&mut command);
        let mut child = command.spawn().unwrap();
        let stdin = child.stdin.take();
        let control = RunControl::new(false);
        control.install(child, stdin).unwrap();
        control.capture_runtime_process_baseline().unwrap();
        control.set_resume_supported(true);
        control.begin_run().unwrap();
        control.mark_idle();
        assert!(control.try_retire_idle(
            Instant::now() + Duration::from_secs(6),
            Duration::from_secs(5),
        ));
        control.retire_owned().unwrap();
        assert!(control.owned_process_group_exited());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn planned_retirement_reaps_a_verified_helper_that_left_its_process_group() {
        let mut command = Command::new("sh");
        command
            .args([
                "-c",
                "python3 -c 'import os,time; os.setsid(); time.sleep(30)' & echo $!; read ignored; exit 0",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        isolate_child(&mut command);
        let mut child = command.spawn().unwrap();
        let stdin = child.stdin.take();
        let helper_pid = {
            let mut line = String::new();
            BufReader::new(child.stdout.take().unwrap())
                .read_line(&mut line)
                .unwrap();
            line.trim().parse::<libc::pid_t>().unwrap()
        };
        let unrelated = Command::new("sleep").arg("30").spawn().unwrap();
        let unrelated_identity =
            crate::process_metrics::retirement_process_tree(unsafe { libc::getpid() })
                .unwrap()
                .into_iter()
                .find(|(pid, _)| *pid == unrelated.id() as libc::pid_t)
                .unwrap();
        let control = RunControl::new(false);
        control.install(child, stdin).unwrap();
        control.capture_runtime_process_baseline().unwrap();
        let helper_identity = control
            .runtime_process_baseline
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .iter()
            .find(|(pid, _)| *pid == helper_pid)
            .copied()
            .expect("helper must be captured before it re-parents");
        control.set_resume_supported(true);
        control.begin_run().unwrap();
        control.mark_idle();
        assert!(control.try_retire_idle(
            Instant::now() + Duration::from_secs(6),
            Duration::from_secs(5),
        ));
        control.retire_owned().unwrap();
        assert!(!crate::process_metrics::retirement_process_identity_alive(
            helper_identity.0,
            helper_identity.1,
        )
        .unwrap());
        assert!(crate::process_metrics::retirement_process_identity_alive(
            unrelated_identity.0,
            unrelated_identity.1,
        )
        .unwrap());
        let _ = unsafe { libc::kill(unrelated.id() as libc::pid_t, libc::SIGKILL) };
        let _ = unrelated.wait_with_output();
    }

    fn lazy_helper_control() -> Arc<RunControl> {
        let mut command = Command::new("sh");
        command
            .args(["-c", "read ignored; sleep 30 & wait"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        isolate_child(&mut command);
        let mut child = command.spawn().unwrap();
        let stdin = child.stdin.take();
        let control = RunControl::new(false);
        control.install(child, stdin).unwrap();
        control.capture_runtime_process_baseline().unwrap();
        control
    }

    #[test]
    fn no_tool_turn_can_adopt_a_delayed_provider_helper() {
        let control = lazy_helper_control();
        control.begin_run().unwrap();
        control.mark_idle();
        let mut stdin = control.control_stdin.lock().unwrap().take().unwrap();
        stdin.write_all(b"ready\n").unwrap();
        drop(stdin);
        thread::sleep(Duration::from_millis(60));

        assert!(control
            .refresh_runtime_process_baseline_while_idle_if_no_tool_work()
            .unwrap());
        assert!(control.retirement_process_tree_is_safe());
        control.cancel();
        let _ = control.wait();
    }

    #[test]
    fn active_turn_cannot_adopt_a_delayed_provider_helper() {
        let control = lazy_helper_control();
        control.begin_run().unwrap();
        let mut stdin = control.control_stdin.lock().unwrap().take().unwrap();
        stdin.write_all(b"ready\n").unwrap();
        drop(stdin);
        thread::sleep(Duration::from_millis(60));

        assert!(!control
            .refresh_runtime_process_baseline_while_idle_if_no_tool_work()
            .unwrap());
        assert!(!control.retirement_process_tree_is_safe());
        control.cancel();
        let _ = control.wait();
    }

    #[test]
    fn tool_work_keeps_a_later_unknown_child_pinned() {
        let control = lazy_helper_control();
        let mut stdin = control.control_stdin.lock().unwrap().take().unwrap();
        stdin.write_all(b"tool\n").unwrap();
        drop(stdin);
        thread::sleep(Duration::from_millis(60));

        control.mark_tool_work_observed();
        assert!(!control
            .refresh_runtime_process_baseline_if_no_tool_work()
            .unwrap());
        assert!(!control.retirement_process_tree_is_safe());
        control.cancel();
        let _ = control.wait();
    }

    #[test]
    fn reads_claude_permission_control_request_without_rewriting_input() {
        let input = serde_json::json!({"command": "git status --short"});
        let frame = serde_json::json!({
            "type": "control_request",
            "request_id": "permission-1",
            "request": {
                "subtype": "can_use_tool",
                "tool_name": "Bash",
                "tool_use_id": "tool-1",
                "input": input,
                "decision_reason": "Command needs approval"
            }
        });
        let parsed = claude_approval(&frame).expect("Claude permission request");
        assert_eq!(parsed.0, "permission-1");
        assert_eq!(parsed.1, "Bash");
        assert_eq!(
            parsed.2,
            serde_json::json!({"command": "git status --short"})
        );
        assert_eq!(parsed.5, "high");
        assert!(parsed.4.contains("Command needs approval"));
    }

    fn host(kind: &str) -> Host {
        Host {
            id: id(),
            name: "test".into(),
            kind: kind.into(),
            address: "mira".into(),
            user: String::new(),
            port: 0,
            identity_file: String::new(),
            default_cwd: "/tmp".into(),
            codex_path: "~/bin/codex'; echo unsafe".into(),
            claude_path: String::new(),
            opencode_path: String::new(),
            hermes_path: String::new(),
        }
    }

    #[test]
    fn quote_cannot_break_remote_argument() {
        assert_eq!(posix_quote("a';rm -rf /"), "'a'\\'';rm -rf /'");
    }

    #[test]
    fn remote_exec_exposes_an_explicit_executables_sibling_runtime() {
        assert_eq!(
            remote_exec(
                "/Users/alex/.nvm/versions/node/v22.17.0/bin/codex",
                ["--version"]
            ),
            "PATH='/Users/alex/.nvm/versions/node/v22.17.0/bin':\"$PATH\"; export PATH; exec '/Users/alex/.nvm/versions/node/v22.17.0/bin/codex' '--version'"
        );
        assert_eq!(
            remote_exec("codex", ["--version"]),
            "exec 'codex' '--version'"
        );
        assert_eq!(
            remote_exec("~/bin/codex'; echo unsafe", ["arg'; echo unsafe"]),
            "PATH=\"$HOME\"/'bin':\"$PATH\"; export PATH; exec \"$HOME\"/'bin/codex'\\''; echo unsafe' 'arg'\\''; echo unsafe'"
        );
    }

    #[test]
    fn remote_command_quotes_every_dynamic_value_and_omits_default_options() {
        let task = task(Some("id'; echo pwned"), "model'; echo pwned");
        let command = build_command(&host("ssh"), &task).unwrap();
        let args: Vec<OsString> = command.get_args().map(OsString::from).collect();
        let rendered = args.last().unwrap().to_string_lossy();
        assert!(rendered.contains("'~/bin/codex'\\''; echo unsafe'"));
        assert!(rendered.contains("'model'\\''; echo pwned'"));
        assert!(rendered.contains("'id'\\''; echo pwned'"));
        assert!(rendered.contains("start_new_session=True"));
        assert!(rendered.contains("os.killpg(process.pid"));
        assert!(!args.iter().any(|arg| arg == "-p"));
        assert!(!rendered.contains("dangerously-bypass"));
    }

    #[test]
    fn local_exec_options_precede_resume_and_clear_parent_ids() {
        let mut host = host("local");
        host.codex_path = std::env::current_exe().unwrap().display().to_string();
        let command = build_command(&host, &task(Some("native"), "gpt-test")).unwrap();
        let args = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert_eq!(args[0], "exec");
        assert!(args.windows(2).any(|pair| pair == ["-s", "read-only"]));
        let resume = args.iter().position(|arg| arg == "resume").unwrap();
        let sandbox = args.iter().position(|arg| arg == "-s").unwrap();
        assert!(sandbox < resume);
        assert_eq!(&args[resume..], ["resume", "--json", "native", "-"]);
        assert!(command
            .get_envs()
            .any(|(key, value)| key == "CODEX_THREAD_ID" && value.is_none()));
        assert!(command
            .get_envs()
            .any(|(key, value)| key == "CODEX_SESSION_ID" && value.is_none()));
    }

    #[test]
    fn yolo_uses_codex_bypass_flag_not_an_invalid_sandbox_value() {
        let mut host = host("local");
        host.codex_path = std::env::current_exe().unwrap().display().to_string();
        let mut task = task(None, "gpt-test");
        task.sandbox = "yolo".into();
        let args = build_command(&host, &task)
            .unwrap()
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert!(args
            .iter()
            .any(|arg| arg == "--dangerously-bypass-approvals-and-sandbox"));
        assert!(!args.windows(2).any(|pair| pair == ["-s", "yolo"]));
        assert!(!args.iter().any(|arg| arg == "-s"));
    }

    #[test]
    fn yolo_bypass_is_preserved_for_ssh_commands() {
        let mut task = task(None, "gpt-test");
        task.sandbox = "yolo".into();
        let command = build_command(&host("ssh"), &task).unwrap();
        let rendered = command.get_args().last().unwrap().to_string_lossy();
        assert!(rendered.contains("--dangerously-bypass-approvals-and-sandbox"));
        assert!(!rendered.contains("-s yolo"));
    }

    #[test]
    fn codex_model_settings_use_invocation_only_effort_and_priority_tier() {
        let mut host = host("local");
        host.codex_path = std::env::current_exe().unwrap().display().to_string();
        let mut task = task(None, "gpt-test");
        task.model_settings = Some(crate::model::ModelSettings {
            model: "gpt-test".into(),
            reasoning_effort: Some("high".into()),
            fast_mode: Some(true),
        });
        let args = build_command(&host, &task)
            .unwrap()
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert!(args
            .windows(2)
            .any(|pair| pair == ["-c", "model_reasoning_effort=\"high\""]));
        assert!(args
            .windows(2)
            .any(|pair| pair == ["-c", "service_tier=\"priority\""]));
    }

    #[test]
    fn codex_null_fast_override_does_not_set_a_service_tier() {
        let mut host = host("local");
        host.codex_path = std::env::current_exe().unwrap().display().to_string();
        let mut task = task(None, "gpt-test");
        task.model_settings = Some(crate::model::ModelSettings {
            model: "gpt-test".into(),
            reasoning_effort: None,
            fast_mode: None,
        });
        let args = build_command(&host, &task)
            .unwrap()
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert!(!args.iter().any(|arg| arg.starts_with("service_tier=")));
    }

    #[test]
    fn codex_collaboration_is_scoped_to_the_monitter_server() {
        let args = codex_args(&task(None, ""), Some("/private/runtime/monitter-mcp.py"));
        assert!(args
            .windows(2)
            .any(|pair| pair == ["-c", "mcp_servers.monitter.required=true"]));
        assert!(args.windows(2).any(|pair| pair
            == [
                "-c",
                "mcp_servers.monitter.default_tools_approval_mode=\"approve\""
            ]));
        assert!(args
            .iter()
            .any(|arg| arg.contains("mcp_servers.monitter.enabled_tools")));
        assert!(args.iter().any(|arg| arg.contains("list_agents")));
        assert!(args.iter().any(|arg| arg.contains("cancel_delegation")));
        assert!(!args.iter().any(|arg| arg.contains("ignore-user-config")));
        assert!(args
            .iter()
            .any(|arg| arg
                == "mcp_servers.monitter.env_vars=[\"MONITTER_ENDPOINT\",\"MONITTER_TOKEN\"]"));
        assert!(!args.iter().any(|arg| arg.contains("not-in-argv")));
    }

    #[test]
    fn local_collaboration_credentials_are_environment_only() {
        let mut host = host("local");
        host.codex_path = std::env::current_exe().unwrap().display().to_string();
        let grant = SessionGrant {
            endpoint: "http://127.0.0.1:4444/rpc".into(),
            token: "not-in-argv".into(),
        };
        let command = build_command_with_collaboration(
            &host,
            &task(None, ""),
            Some((&grant, "/private/runtime/monitter-mcp.py")),
        )
        .unwrap();
        let args = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert!(!args.iter().any(|arg| arg.contains("not-in-argv")));
        let environment = command
            .get_envs()
            .map(|(key, value)| {
                (
                    key.to_string_lossy().into_owned(),
                    value.map(|value| value.to_string_lossy().into_owned()),
                )
            })
            .collect::<Vec<_>>();
        assert!(
            environment
                .iter()
                .any(|(key, value)| key == "MONITTER_TOKEN"
                    && value.as_deref() == Some("not-in-argv"))
        );
        assert!(environment
            .iter()
            .any(|(key, value)| key == "MONITTER_ENDPOINT"
                && value.as_deref() == Some("http://127.0.0.1:4444/rpc")));
    }

    #[test]
    fn claude_injects_only_inline_monitter_mcp_and_its_tool_allowlist() {
        let mut host = host("local");
        host.claude_path = std::env::current_exe().unwrap().display().to_string();
        let mut task = task(None, "");
        task.provider = "claude".into();
        task.sandbox = "harness-configured".into();
        let grant = SessionGrant {
            endpoint: "http://127.0.0.1:4444/rpc".into(),
            token: "not-in-argv".into(),
        };
        let command = build_command_with_collaboration(
            &host,
            &task,
            Some((&grant, "/private/runtime/monitter-mcp.py")),
        )
        .unwrap();
        let args = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        let mcp = args.iter().position(|arg| arg == "--mcp-config").unwrap();
        assert_eq!(
            args[mcp + 1],
            claude_mcp_config("/private/runtime/monitter-mcp.py")
        );
        let allowed = args.iter().position(|arg| arg == "--allowedTools").unwrap();
        assert_eq!(args[allowed + 1], claude_tool_names());
        assert!(!args.iter().any(|arg| arg == "--strict-mcp-config"));
        assert!(!args.iter().any(|arg| arg.contains("not-in-argv")));
    }

    #[test]
    fn opencode_uses_inline_config_without_approval_override() {
        let mut host = host("local");
        host.opencode_path = std::env::current_exe().unwrap().display().to_string();
        let mut task = task(None, "");
        task.provider = "opencode".into();
        task.sandbox = "harness-configured".into();
        let grant = SessionGrant {
            endpoint: "http://127.0.0.1:4444/rpc".into(),
            token: "not-in-argv".into(),
        };
        let command = build_command_with_collaboration(
            &host,
            &task,
            Some((&grant, "/private/runtime/monitter-mcp.py")),
        )
        .unwrap();
        let environment = command
            .get_envs()
            .map(|(key, value)| {
                (
                    key.to_string_lossy().into_owned(),
                    value.map(|value| value.to_string_lossy().into_owned()),
                )
            })
            .collect::<Vec<_>>();
        let expected = merge_opencode_mcp_config(None, "/private/runtime/monitter-mcp.py").unwrap();
        assert!(environment
            .iter()
            .any(|(key, value)| key == "OPENCODE_CONFIG_CONTENT"
                && value.as_deref() == Some(expected.as_str())));
        assert!(!environment
            .iter()
            .any(|(key, value)| key == "OPENCODE_CONFIG_CONTENT"
                && value.as_deref().unwrap_or_default().contains("approval")));
    }

    #[test]
    fn opencode_inline_config_preserves_existing_settings_and_servers() {
        let existing = r#"{"model":"provider/model","nested":{"keep":true},"mcp":{"context7":{"type":"remote","url":"https://example.test/mcp","enabled":true}}}"#;
        let merged: Value = serde_json::from_str(
            &merge_opencode_mcp_config(Some(existing), "/private/runtime/monitter-mcp.py").unwrap(),
        )
        .unwrap();
        assert_eq!(merged["model"], "provider/model");
        assert_eq!(merged["nested"]["keep"], true);
        assert_eq!(merged["mcp"]["context7"]["url"], "https://example.test/mcp");
        assert_eq!(
            merged["mcp"]["monitter"]["command"][1],
            "/private/runtime/monitter-mcp.py"
        );
        assert!(merge_opencode_mcp_config(Some("not-json"), "helper").is_err());
    }

    #[test]
    fn provider_stderr_ignores_routine_log_levels() {
        assert_eq!(
            provider_stderr_diagnostic(
                "2026-09-11T14:56:46.955384Z WARN codex_rmcp::rmcp_client: shutdown failed"
            ),
            None
        );
        assert_eq!(provider_stderr_diagnostic("DEBUG connection retry"), None);
        assert_eq!(provider_stderr_diagnostic("INFO ready"), None);
    }

    #[test]
    fn provider_stderr_keeps_a_concise_actionable_error() {
        assert_eq!(
            provider_stderr_diagnostic("ERROR authentication failed for configured provider"),
            Some("ERROR authentication failed for configured provider".into())
        );
        assert_eq!(provider_stderr_diagnostic("ordinary progress line"), None);
    }

    #[test]
    fn remote_supervisor_accepts_secret_collaboration_frame_before_prompt() {
        let mut command = Command::new("python3");
        command
            .args([
                "-c",
                REMOTE_SUPERVISOR,
                "/tmp",
                "/bin/sh",
                "-c",
                "printf '%s:%s' \"$MONITTER_ENDPOINT\" \"$MONITTER_TOKEN\"; cat >/dev/null",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = command.spawn().unwrap();
        let frame =
            serde_json::json!({"endpoint":"http://127.0.0.1:4444/rpc","token":"not-in-argv"})
                .to_string();
        let mut stdin = child.stdin.take().unwrap();
        stdin
            .write_all(format!("MONITTER/COLLAB/1 {}\n", frame.len()).as_bytes())
            .unwrap();
        stdin.write_all(frame.as_bytes()).unwrap();
        stdin.write_all(b"MONITTER/1 2\nok").unwrap();
        let mut output = String::new();
        child
            .stdout
            .take()
            .unwrap()
            .read_to_string(&mut output)
            .unwrap();
        drop(stdin);
        assert!(child.wait().unwrap().success());
        assert_eq!(output, "http://127.0.0.1:4444/rpc:not-in-argv");
    }

    #[test]
    fn hermes_prompt_frame_keeps_multiline_peer_context_intact() {
        let prompt = "Monitter agent: Hermes\n\nPeer context from Rafa:\nYou are a bungleflop!";
        assert_eq!(
            hermes_prompt_frame(prompt),
            format!("MONITTER/HERMES/1 {}\n{prompt}", prompt.len())
        );
    }

    #[test]
    fn remote_supervisor_forwards_hermes_frame_without_an_extra_newline() {
        let prompt = "First line\nPeer context from Rafa:\nYou are a bungleflop!";
        let frame = hermes_prompt_frame(prompt);
        let mut command = Command::new("python3");
        command
            .args([
                "-c",
                REMOTE_SUPERVISOR,
                "/tmp",
                "python3",
                "-c",
                "import sys; sys.stdout.buffer.write(sys.stdin.buffer.read())",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = command.spawn().unwrap();
        let mut stdin = child.stdin.take().unwrap();
        stdin
            .write_all(format!("MONITTER/1 {}\n", frame.len()).as_bytes())
            .unwrap();
        stdin.write_all(frame.as_bytes()).unwrap();
        // Keep the control pipe open: an EOF is intentionally interpreted as
        // cancellation by the remote supervisor. The real runner likewise
        // retains it for cancellation/approval control.
        let mut output = String::new();
        child
            .stdout
            .take()
            .unwrap()
            .read_to_string(&mut output)
            .unwrap();
        assert!(child.wait().unwrap().success());
        assert_eq!(output, frame);
    }

    #[test]
    fn remote_supervisor_exposes_the_launchers_sibling_runtime() {
        use std::os::unix::fs::PermissionsExt;

        let directory = std::env::temp_dir().join(format!("monitter-remote-runtime-path-{}", id()));
        std::fs::create_dir_all(&directory).unwrap();
        let runtime = directory.join("monitter-node");
        let launcher = directory.join("codex");
        std::fs::write(
            &runtime,
            "#!/bin/sh\ncat >/dev/null\nprintf 'sibling-runtime-found'\n",
        )
        .unwrap();
        std::fs::write(&launcher, "#!/usr/bin/env monitter-node\n").unwrap();
        std::fs::set_permissions(&runtime, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::set_permissions(&launcher, std::fs::Permissions::from_mode(0o700)).unwrap();

        let mut command = Command::new("python3");
        command
            .args(["-c", REMOTE_SUPERVISOR])
            .arg(&directory)
            .arg(&launcher)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn().unwrap();
        let mut stdin = child.stdin.take().unwrap();
        stdin.write_all(b"MONITTER/1 2\nok").unwrap();
        let mut output = String::new();
        child
            .stdout
            .take()
            .unwrap()
            .read_to_string(&mut output)
            .unwrap();
        drop(stdin);
        let result = child.wait_with_output().unwrap();
        let _ = std::fs::remove_dir_all(&directory);
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(output, "sibling-runtime-found");
    }

    #[test]
    fn remote_supervisor_merges_remote_opencode_inline_config() {
        let existing = r#"{"model":"remote/model","nested":{"keep":"value"},"mcp":{"existing":{"type":"remote","url":"https://example.test/mcp"}}}"#;
        let mut command = Command::new("python3");
        command
            .args([
                "-c",
                REMOTE_SUPERVISOR,
                "/tmp",
                "/bin/sh",
                "-c",
                "cat >/dev/null; /usr/bin/env",
            ])
            .env("OPENCODE_CONFIG_CONTENT", existing)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = command.spawn().unwrap();
        let frame = serde_json::json!({
            "endpoint":"http://127.0.0.1:4444/rpc",
            "token":"not-in-argv",
            "opencodeHelper":"/tmp/monitter-mcp.example/monitter_mcp.py",
        })
        .to_string();
        let mut stdin = child.stdin.take().unwrap();
        stdin
            .write_all(format!("MONITTER/COLLAB/1 {}\n", frame.len()).as_bytes())
            .unwrap();
        stdin.write_all(frame.as_bytes()).unwrap();
        stdin.write_all(b"MONITTER/1 2\nok").unwrap();
        let mut output = String::new();
        child
            .stdout
            .take()
            .unwrap()
            .read_to_string(&mut output)
            .unwrap();
        drop(stdin);
        assert!(child.wait().unwrap().success());
        let config = output
            .lines()
            .find_map(|line| line.strip_prefix("OPENCODE_CONFIG_CONTENT="))
            .unwrap();
        let merged: Value = serde_json::from_str(config).unwrap();
        assert_eq!(merged["model"], "remote/model");
        assert_eq!(merged["nested"]["keep"], "value");
        assert_eq!(merged["mcp"]["existing"]["url"], "https://example.test/mcp");
        assert_eq!(
            merged["mcp"]["monitter"]["command"][1],
            "/tmp/monitter-mcp.example/monitter_mcp.py"
        );
    }

    #[test]
    fn parses_outputs_tools_usage_and_nested_errors() {
        let output = parse_codex_event(&serde_json::json!({
            "type":"item.completed",
            "item":{"type":"agent_message","text":"ok"}
        }));
        assert_eq!(output.assistant.as_deref(), Some("ok"));
        assert_eq!(output.event.as_ref().unwrap().0, "output");
        let tool = parse_codex_event(&serde_json::json!({
            "type":"item.completed",
            "item":{"type":"command_execution","command":"pwd","aggregated_output":"/tmp"}
        }));
        assert_eq!(
            tool.event.unwrap(),
            ("tool".into(), "pwd".into(), "/tmp".into())
        );
        let usage = parse_codex_event(&serde_json::json!({
            "type":"turn.completed","usage":{"input_tokens":12,"output_tokens":3}
        }));
        assert_eq!(usage.event.unwrap().0, "usage");
        let failed = parse_codex_event(&serde_json::json!({
            "type":"turn.failed","error":{"message":"capacity"}
        }));
        assert_eq!(failed.event.unwrap().2, "capacity");
    }

    #[test]
    fn computer_mcp_items_emit_separate_lifecycle_metadata() {
        let events = parse_events(
            "codex",
            &serde_json::json!({
                "type":"item.started", "thread_id":"thread-1", "item":{
                    "id":"item-1", "type":"mcp_tool_call", "server":"computer-use", "tool":"computer_use", "arguments":{"action":"click"}
                }
            }),
        );
        assert_eq!(events.len(), 2);
        let computer = events
            .iter()
            .find(|event| event.event.as_ref().map(|item| item.0.as_str()) == Some("computer"))
            .unwrap();
        let detail: Value = serde_json::from_str(&computer.event.as_ref().unwrap().2).unwrap();
        assert_eq!(detail["id"], "item-1");
        assert_eq!(detail["phase"], "started");
        assert_eq!(detail["tool"], "computer_use");
    }

    #[test]
    fn only_agent_items_become_output() {
        assert!(parse_codex_event(&serde_json::json!({
            "type":"item.completed","item":{"type":"user_message","text":"no"}
        }))
        .assistant
        .is_none());
    }

    #[test]
    fn cua_result_image_is_a_generated_computer_image_event() {
        let parsed = parse_codex_event(&serde_json::json!({
            "type":"item.completed", "thread_id":"session-1", "item":{
                "type":"McpToolCall", "server":"cua_repl", "tool":"js",
                "result":{"content":[{"type":"image","data":"/9j/2Q=="}]}
            }
        }));
        assert_eq!(parsed.native_session_id.as_deref(), Some("session-1"));
        assert_eq!(parsed.event.unwrap().0, "computer_image");
    }

    #[test]
    fn cancellation_terminates_the_owned_process_group() {
        let mut command = Command::new("sh");
        command.args(["-c", "sleep 30"]);
        isolate_child(&mut command);
        let child = command.spawn().unwrap();
        let control = RunControl::new(false);
        control.install(child, None).unwrap();
        let started = Instant::now();
        control.cancel();
        let status = control.wait().unwrap();
        assert!(!status.success());
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn remote_supervisor_kills_its_exact_child_group_on_control_eof() {
        let mut command = Command::new("python3");
        command
            .args([
                "-c",
                REMOTE_SUPERVISOR,
                "/tmp",
                "/bin/sh",
                "-c",
                "cat >/dev/null; sleep 30",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        isolate_child(&mut command);
        let mut child = command.spawn().unwrap();
        let prompt = b"safe prompt";
        let mut stdin = child.stdin.take().unwrap();
        stdin
            .write_all(format!("MONITTER/1 {}\n", prompt.len()).as_bytes())
            .unwrap();
        stdin.write_all(prompt).unwrap();
        stdin.flush().unwrap();
        let control = RunControl::new(true);
        control.install(child, Some(stdin)).unwrap();
        let started = Instant::now();
        control.cancel();
        let status = control.wait().unwrap();
        assert!(!status.success());
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    fn fake_opencode(script: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = std::env::temp_dir().join(format!("monitter-opencode-export-{}", id()));
        std::fs::write(&path, format!("#!/bin/sh\n{script}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        path
    }

    #[test]
    fn helper_staging_reports_ssh_host_key_failure_before_stdin_error() {
        use std::os::unix::fs::PermissionsExt;

        let shim = std::env::temp_dir().join(format!("monitter-ssh-stage-{}", id()));
        let helper = std::env::temp_dir().join(format!("monitter-helper-{}", id()));
        std::fs::write(
            &shim,
            "#!/bin/sh\nprintf '%s\\n' 'Host key verification failed.' >&2\nexit 255\n",
        )
        .unwrap();
        std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::write(&helper, "# helper fixture\n").unwrap();
        let host = host("ssh");
        let _ssh = override_ssh_for_test(&host.id, &shim);
        let error = stage_remote_helper(&host, &helper, &RunControl::new(false)).unwrap_err();
        assert!(error.contains("status 255"));
        assert!(error.contains("Host key verification failed."));
        let _ = std::fs::remove_file(shim);
        let _ = std::fs::remove_file(helper);
    }

    #[test]
    fn helper_staging_cancellation_bounds_a_writer_blocked_on_ssh_stdin() {
        use std::os::unix::fs::PermissionsExt;

        let shim = std::env::temp_dir().join(format!("monitter-ssh-stage-blocked-{}", id()));
        let helper = std::env::temp_dir().join(format!("monitter-helper-blocked-{}", id()));
        std::fs::write(&shim, "#!/bin/sh\nsleep 30\n").unwrap();
        std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::write(&helper, vec![b'x'; 256 * 1024]).unwrap();
        let host = host("ssh");
        let _ssh = override_ssh_for_test(&host.id, &shim);
        let control = RunControl::new(false);
        let started = Instant::now();
        let error = thread::scope(|scope| {
            scope.spawn(|| {
                thread::sleep(Duration::from_millis(75));
                control.cancelled.store(true, Ordering::SeqCst);
            });
            stage_remote_helper(&host, &helper, &control).unwrap_err()
        });
        assert!(error.contains("cancelled"));
        assert!(started.elapsed() < Duration::from_secs(5));
        let _ = std::fs::remove_file(shim);
        let _ = std::fs::remove_file(helper);
    }

    #[test]
    fn helper_staging_rejects_oversized_output_instead_of_accepting_a_prefix() {
        use std::os::unix::fs::PermissionsExt;

        let shim = std::env::temp_dir().join(format!("monitter-ssh-stage-output-{}", id()));
        let helper = std::env::temp_dir().join(format!("monitter-helper-output-{}", id()));
        std::fs::write(&shim, "#!/bin/sh\ncat >/dev/null\nhead -c 5000 /dev/zero\n").unwrap();
        std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::write(&helper, "# helper fixture\n").unwrap();
        let host = host("ssh");
        let _ssh = override_ssh_for_test(&host.id, &shim);
        let error = stage_remote_helper(&host, &helper, &RunControl::new(false)).unwrap_err();
        assert!(error.contains("output exceeded its limit"));
        let _ = std::fs::remove_file(shim);
        let _ = std::fs::remove_file(helper);
    }

    fn opencode_task(session: &str) -> Task {
        let mut value = task(Some(session), "");
        value.provider = "opencode".into();
        value.sandbox = "harness-configured".into();
        value.cwd = "/tmp".into();
        value
    }

    #[test]
    fn opencode_export_restores_only_matching_absolute_session_directory() {
        let executable = fake_opencode("printf '%s\\n' '{\"info\":{\"id\":\"ses_123\",\"directory\":\"/original/project\"},\"messages\":[]}'");
        let mut local = host("local");
        local.opencode_path = executable.display().to_string();
        let control = RunControl::new(false);
        let result = opencode_export_directory_with_timeout(
            &local,
            &opencode_task("ses_123"),
            "ses_123",
            &control,
            Duration::from_secs(1),
        );
        assert_eq!(result.unwrap(), "/original/project");
        let _ = std::fs::remove_file(executable);
    }

    #[test]
    fn opencode_export_accepts_the_cli_banner_before_metadata() {
        let executable = fake_opencode("printf '%s\\n%s\\n' 'Exporting session: ses_123' '{\"info\":{\"id\":\"ses_123\",\"directory\":\"/original/project\"},\"messages\":[]}'");
        let mut local = host("local");
        local.opencode_path = executable.display().to_string();
        let control = RunControl::new(false);
        let result = opencode_export_directory_with_timeout(
            &local,
            &opencode_task("ses_123"),
            "ses_123",
            &control,
            Duration::from_secs(3),
        );
        assert_eq!(result.unwrap(), "/original/project");
        let _ = std::fs::remove_file(executable);
    }

    #[test]
    fn opencode_export_rejects_mismatched_or_relative_metadata() {
        for payload in [
            "{\"info\":{\"id\":\"other\",\"directory\":\"/original/project\"},\"messages\":[]}",
            "{\"info\":{\"id\":\"ses_123\",\"directory\":\"relative\"},\"messages\":[]}",
        ] {
            let executable = fake_opencode(&format!("printf '%s\\n' '{}'", payload));
            let mut local = host("local");
            local.opencode_path = executable.display().to_string();
            let control = RunControl::new(false);
            assert!(opencode_export_directory_with_timeout(
                &local,
                &opencode_task("ses_123"),
                "ses_123",
                &control,
                Duration::from_secs(1)
            )
            .is_err());
            let _ = std::fs::remove_file(executable);
        }
    }

    #[test]
    fn opencode_export_is_bounded() {
        let executable = fake_opencode("sleep 30");
        let mut local = host("local");
        local.opencode_path = executable.display().to_string();
        let control = RunControl::new(false);
        let started = Instant::now();
        let result = opencode_export_directory_with_timeout(
            &local,
            &opencode_task("ses_123"),
            "ses_123",
            &control,
            Duration::from_millis(100),
        );
        assert!(result.unwrap_err().contains("timed out"));
        assert!(started.elapsed() < Duration::from_secs(5));
        let _ = std::fs::remove_file(executable);
    }
}
