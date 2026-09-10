use crate::{
    adapters,
    collaboration_transport::SessionGrant,
    model::{valid_sandbox_for_provider, Host, Task},
    Service,
};
use serde_json::Value;
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    path::PathBuf,
    process::{Child, ChildStdin, Command, ExitStatus, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

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
    let mut args = vec![
        "exec".into(),
        "-s".into(),
        task.sandbox.clone(),
        "--skip-git-repo-check".into(),
    ];
    if !task.model.trim().is_empty() {
        args.extend(["-m".into(), task.model.clone()]);
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

fn isolate_child(command: &mut Command) {
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

#[cfg(test)]
pub fn build_command(host: &Host, task: &Task) -> Result<Command, String> {
    build_command_with_collaboration(host, task, None)
}

fn build_command_with_collaboration(
    host: &Host,
    task: &Task,
    collaboration: Option<(&SessionGrant, &str)>,
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
            if let Some((_, helper)) = collaboration {
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
        "hermes" => adapters::hermes::args(task, &host.hermes_path),
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

pub fn build_probe_command(host: &Host, provider: &str) -> Result<Command, String> {
    let cli = configured_path(host, provider)?;
    let mut command = if host.kind == "local" {
        let mut command = Command::new(resolve_local_provider(provider, cli)?);
        command.arg("--version");
        command
    } else if host.kind == "ssh" {
        let mut command = Command::new("ssh");
        add_ssh_options(&mut command, host);
        command.arg(ssh_target(host)?).arg(format!(
            "exec {} --version",
            remote_path(remote_cli(host, provider)?)
        ));
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
    child: Mutex<Option<Child>>,
    control_stdin: Mutex<Option<ChildStdin>>,
    auxiliary: Mutex<Vec<Child>>,
    remote_supervised: bool,
}

impl RunControl {
    pub fn new(remote_supervised: bool) -> Arc<Self> {
        Arc::new(Self {
            cancelled: AtomicBool::new(false),
            child: Mutex::new(None),
            control_stdin: Mutex::new(None),
            auxiliary: Mutex::new(Vec::new()),
            remote_supervised,
        })
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        if let Ok(mut stdin) = self.control_stdin.lock() {
            stdin.take();
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
    }

    fn install(
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

fn terminate_bounded(child: &mut Child) {
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

struct RemoteCollaboration {
    helper_dir: String,
    helper_path: String,
    endpoint: String,
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
    let mut command = Command::new("ssh");
    add_ssh_options(&mut command, host);
    command
        .arg(ssh_target(host)?)
        .arg("umask 077; d=$(mktemp -d /tmp/monitter-mcp.XXXXXXXX) || exit; cat > \"$d/monitter_mcp.py\" && chmod 700 \"$d/monitter_mcp.py\" && printf '__MONITTER_HELPER_DIR__%s\\n' \"$d\"")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    isolate_child(&mut command);
    let mut child = command
        .spawn()
        .map_err(|error| format!("Could not stage collaboration helper on SSH host: {error}"))?;
    let Some(mut stdin) = child.stdin.take() else {
        terminate_bounded(&mut child);
        return Err("Could not open SSH helper staging input.".into());
    };
    if let Err(error) = stdin.write_all(&source) {
        terminate_bounded(&mut child);
        return Err(format!(
            "Could not send collaboration helper to SSH host: {error}"
        ));
    }
    drop(stdin);
    let mut output = String::new();
    let Some(mut stdout) = child.stdout.take() else {
        terminate_bounded(&mut child);
        return Err("Could not read SSH helper staging output.".into());
    };
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let result = stdout.read_to_string(&mut output).map(|_| output);
        let _ = sender.send(result);
    });
    let deadline = Instant::now() + Duration::from_secs(20);
    let output = loop {
        if control.cancelled.load(Ordering::SeqCst) {
            terminate_bounded(&mut child);
            return Err("Collaboration helper staging was cancelled.".into());
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            terminate_bounded(&mut child);
            return Err("Timed out staging collaboration helper on SSH host.".into());
        }
        match receiver.recv_timeout(remaining.min(Duration::from_millis(100))) {
            Ok(Ok(output)) => break output,
            Ok(Err(_)) => {
                terminate_bounded(&mut child);
                return Err("Could not read SSH helper staging output.".into());
            }
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                terminate_bounded(&mut child);
                return Err("Could not read SSH helper staging output.".into());
            }
        }
    };
    // EOF on SSH stdout can arrive just before the local SSH process is reaped.
    // Wait only for the remainder of the bounded staging window instead of treating
    // that normal race as a staging failure.
    let status = loop {
        if control.cancelled.load(Ordering::SeqCst) {
            terminate_bounded(&mut child);
            return Err("Collaboration helper staging was cancelled.".into());
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(25)),
            Ok(None) => {
                terminate_bounded(&mut child);
                return Err("Timed out waiting for SSH helper staging to finish.".into());
            }
            Err(_) => {
                terminate_bounded(&mut child);
                return Err("Could not read SSH helper staging status.".into());
            }
        }
    };
    if !status.success() {
        return Err(format!(
            "SSH helper staging exited with {}.",
            status
                .code()
                .map_or("a signal".into(), |code| format!("status {code}"))
        ));
    }
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
    let mut command = Command::new("ssh");
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

fn prepare_remote_collaboration(
    host: &Host,
    endpoint: &str,
    helper: &PathBuf,
    control: &RunControl,
) -> Result<RemoteCollaboration, String> {
    let (helper_dir, helper_path) = stage_remote_helper(host, helper, control)?;
    match start_reverse_tunnel(host, endpoint, control) {
        Ok((tunnel, port)) => Ok(RemoteCollaboration {
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

fn cleanup_remote_helper(host: &Host, helper_dir: &str) {
    let Ok(target) = ssh_target(host) else {
        return;
    };
    let mut command = Command::new("ssh");
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
        let (task, host) = match service.task_and_host(&task_id) {
            Ok(value) => value,
            Err(error) => {
                service.finish(&task_id, "error", Some(error));
                return;
            }
        };
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
        let mut command = match build_command_with_collaboration(&host, &task, grant_for_command) {
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
        let remote_supervised = host.kind == "ssh";
        let remote_helper_dir = remote_collaboration
            .as_ref()
            .map(|remote| remote.helper_dir.clone());
        let remote_endpoint = remote_collaboration
            .as_ref()
            .map(|remote| remote.endpoint.clone());
        let control_stdin = if let Some(mut stdin) = child.stdin.take() {
            let write_result = if remote_supervised {
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
                            stdin.write_all(format!("MONITTER/1 {}\n", prompt.len()).as_bytes())
                        })
                        .and_then(|_| stdin.write_all(prompt.as_bytes()))
                        .and_then(|_| stdin.flush())
                } else {
                    stdin
                        .write_all(format!("MONITTER/1 {}\n", prompt.len()).as_bytes())
                        .and_then(|_| stdin.write_all(prompt.as_bytes()))
                        .and_then(|_| stdin.flush())
                }
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
            if remote_supervised {
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

        let failed_event = Arc::new(AtomicBool::new(false));
        let assistant_seen = Arc::new(AtomicBool::new(false));
        let task_provider = task.provider.clone();
        let stdout_thread = stdout.map(|stdout| {
            let service = service.clone();
            let task = task_id.clone();
            let provider = task_provider.clone();
            let failed_event = failed_event.clone();
            let assistant_seen = assistant_seen.clone();
            thread::spawn(move || {
                for line in BufReader::new(stdout).lines() {
                    match line {
                        Ok(line) => match serde_json::from_str(&line) {
                            Ok(value) => {
                                for parsed in parse_events(&provider, &value) {
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
            thread::spawn(move || {
                for line in BufReader::new(stderr).lines() {
                    match line {
                        Ok(line) if !line.trim().is_empty() => {
                            service.record(&task, "error", "Provider stderr", line)
                        }
                        Ok(_) => {}
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
            sandbox: "read-only".into(),
            project_id: None,
        }
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
    fn remote_supervisor_merges_remote_opencode_inline_config() {
        let existing = r#"{"model":"remote/model","nested":{"keep":"value"},"mcp":{"existing":{"type":"remote","url":"https://example.test/mcp"}}}"#;
        let mut command = Command::new("python3");
        command
            .args(["-c", REMOTE_SUPERVISOR, "/tmp", "/usr/bin/env"])
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
}
