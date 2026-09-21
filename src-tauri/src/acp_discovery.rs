//! A reviewed catalog plus non-executing executable discovery. Catalog entries
//! never trigger package-manager commands, downloads, authentication or sessions.
use crate::{
    model::{AcpLaunch, Host},
    runner,
};
use serde::Serialize;
use std::{
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc,
    thread,
    time::Duration,
};

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AcpCandidate {
    pub id: String,
    pub name: String,
    pub description: String,
    pub source_url: String,
    pub integration: String,
    pub launch: AcpLaunch,
    pub detected: bool,
}

pub fn catalog() -> Vec<AcpCandidate> {
    // Reviewed against upstream documentation/package bin fields, 2026-09-13.
    // In particular the Qwen npm package is qwen-code, but its binary is qwen.
    [
        (
            "mona-acp",
            "mona-acp (Monitter harness)",
            "First-party Monitter harness with per-turn Jev routing across codex, claude, and minimax",
            "https://github.com/soyrex/mona",
            "native",
            "mona-acp",
            vec![],
        ),
        (
            "gemini",
            "Gemini CLI",
            "Google's native ACP integration",
            "https://geminicli.com/docs/cli/acp-mode/",
            "native",
            "gemini",
            vec!["--acp"],
        ),
        (
            "opencode",
            "OpenCode",
            "Native ACP; existing OpenCode chats keep their legacy transport",
            "https://opencode.ai/v2/docs/cli/acp/",
            "native",
            "opencode",
            vec!["acp"],
        ),
        (
            "goose",
            "Goose",
            "Block's native ACP integration",
            "https://github.com/block/goose",
            "native",
            "goose",
            vec!["acp"],
        ),
        (
            "kilo",
            "Kilo",
            "Native ACP integration",
            "https://github.com/Kilo-Org/kilocode",
            "native",
            "kilo",
            vec!["acp"],
        ),
        (
            "junie",
            "Junie",
            "JetBrains ACP integration",
            "https://github.com/JetBrains/junie-acp-release",
            "native",
            "junie",
            vec!["--acp=true"],
        ),
        (
            "vibe",
            "Mistral Vibe",
            "Dedicated ACP executable included with Vibe",
            "https://github.com/mistralai/mistral-vibe",
            "native",
            "vibe-acp",
            vec![],
        ),
        (
            "qwen",
            "Qwen Code",
            "Native ACP integration",
            "https://github.com/QwenLM/qwen-code",
            "native",
            "qwen",
            vec!["--acp"],
        ),
        (
            "pi",
            "Pi (ACP bridge)",
            "Requires pi-acp; Pi's native RPC is a different protocol",
            "https://github.com/victor-software-house/pi-acp",
            "bridge",
            "pi-acp",
            vec![],
        ),
        (
            "claude-acp",
            "Claude Agent (ACP bridge)",
            "Requires claude-agent-acp, separate from the native Claude integration",
            "https://github.com/agentclientprotocol/claude-agent-acp",
            "bridge",
            "claude-agent-acp",
            vec![],
        ),
        (
            "codex-acp",
            "Codex (ACP bridge)",
            "Requires codex-acp, separate from the native Codex app-server integration",
            "https://github.com/agentclientprotocol/codex-acp",
            "bridge",
            "codex-acp",
            vec![],
        ),
    ]
    .into_iter()
    .map(
        |(id, name, description, source, integration, command, args)| AcpCandidate {
            id: id.into(),
            name: name.into(),
            description: description.into(),
            source_url: source.into(),
            integration: integration.into(),
            launch: AcpLaunch {
                command: command.into(),
                args: args.into_iter().map(str::to_string).collect(),
            },
            detected: false,
        },
    )
    .collect()
}

fn executable(path: &Path) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn local_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        for suffix in [
            ".local/bin",
            ".npm-global/bin",
            ".opencode/bin",
            ".bun/bin",
            ".cargo/bin",
        ] {
            dirs.push(home.join(suffix));
        }
    }
    if let Some(path) = std::env::var_os("PATH") {
        // Inspect only exact candidate names in absolute PATH directories. Never
        // recurse, interpret shell startup files, or select from the task cwd.
        dirs.extend(
            std::env::split_paths(&path)
                .filter(|p| p.is_absolute())
                .take(128),
        );
    }
    dirs.extend(["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin"].map(PathBuf::from));
    dirs
}

pub fn resolve_command(command: &str) -> Result<PathBuf, String> {
    if command.trim().is_empty() || command.chars().any(char::is_control) {
        return Err(
            "ACP executable must be a name or an absolute path without control characters.".into(),
        );
    }
    let path = if let Some(rest) = command.strip_prefix("~/") {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or("Cannot resolve the ACP executable home directory.")?
            .join(rest)
    } else {
        PathBuf::from(command)
    };
    if path.is_absolute() {
        return executable(&path)
            .then_some(path)
            .ok_or("ACP executable was not found or is not executable.".into());
    }
    if command.contains('/') || command.contains('\\') {
        return Err("Use an absolute ACP executable path, or a name from PATH.".into());
    }
    local_dirs()
        .into_iter()
        .map(|dir| dir.join(command))
        .find(|path| executable(path))
        .ok_or(
            "ACP executable was not found. Install it separately or choose its full path.".into(),
        )
}

pub fn discover(host: &Host) -> Result<Vec<AcpCandidate>, String> {
    let mut candidates = catalog();
    match host.kind.as_str() {
        "local" => {
            for candidate in &mut candidates {
                // Respect the user's existing OpenCode path without modifying it.
                let command = if candidate.id == "opencode" && !host.opencode_path.is_empty() {
                    host.opencode_path.as_str()
                } else {
                    candidate.launch.command.as_str()
                };
                if let Ok(path) = resolve_command(command) {
                    candidate.launch.command = path.to_string_lossy().into_owned();
                    candidate.detected = true;
                }
            }
        }
        "ssh" => {
            let target = runner::ssh_target(host)?;
            if target.starts_with('-') || target.chars().any(char::is_control) {
                return Err("Invalid SSH target for ACP discovery.".into());
            }
            let script = discovery_script(&candidates, host);
            let mut command = runner::ssh_command(host);
            runner::add_ssh_options(&mut command, host);
            command
                .arg("--")
                .arg(target)
                .arg(format!("sh -c {}", runner::posix_quote(&script)));
            let output = bounded_discovery(command)?;
            for line in output.lines() {
                let Some((id, path)) = line.split_once('\t') else {
                    continue;
                };
                if !path.starts_with('/') || path.chars().any(char::is_control) {
                    continue;
                }
                if let Some(candidate) = candidates.iter_mut().find(|c| c.id == id) {
                    candidate.launch.command = path.into();
                    candidate.detected = true;
                }
            }
        }
        _ => return Err("Unknown host kind for ACP discovery.".into()),
    }
    candidates.sort_by(|a, b| {
        b.detected
            .cmp(&a.detected)
            .then_with(|| a.name.cmp(&b.name))
    });
    Ok(candidates)
}

fn discovery_script(candidates: &[AcpCandidate], host: &Host) -> String {
    let mut script = String::new();
    for candidate in candidates {
        let bin = runner::posix_quote(&candidate.launch.command);
        script.push_str(&format!(
            "monitter_acp_path=$(command -v {bin} 2>/dev/null || true)\n"
        ));
        if candidate.id == "opencode" && !host.opencode_path.is_empty() {
            let configured = if let Some(rest) = host.opencode_path.strip_prefix("~/") {
                format!("\"$HOME\"/{}", runner::posix_quote(rest))
            } else {
                runner::posix_quote(&host.opencode_path)
            };
            script.push_str(&format!("if [ -f {configured} ] && [ -x {configured} ]; then monitter_acp_path={configured}; fi\n"));
        }
        script.push_str(&format!(
            "if [ ! -f \"$monitter_acp_path\" ] || [ ! -x \"$monitter_acp_path\" ]; then\n\
             for monitter_acp_dir in \"$HOME/.local/bin\" \"$HOME/.npm-global/bin\" \"$HOME/.opencode/bin\" \"$HOME/.bun/bin\" \"$HOME/.cargo/bin\" /opt/homebrew/bin /usr/local/bin /usr/bin /bin; do\n\
             if [ -f \"$monitter_acp_dir\"/{bin} ] && [ -x \"$monitter_acp_dir\"/{bin} ]; then monitter_acp_path=\"$monitter_acp_dir\"/{bin}; break; fi\n\
             done\nfi\n\
             case \"$monitter_acp_path\" in /*) if [ -f \"$monitter_acp_path\" ] && [ -x \"$monitter_acp_path\" ]; then printf '%s\\t%s\\n' {} \"$monitter_acp_path\"; fi;; esac\n",
            runner::posix_quote(&candidate.id)));
    }
    script
}

fn bounded_discovery(command: Command) -> Result<String, String> {
    bounded_discovery_with_timeout(command, Duration::from_secs(12))
}

fn bounded_discovery_with_timeout(
    mut command: Command,
    timeout: Duration,
) -> Result<String, String> {
    let deadline = std::time::Instant::now() + timeout;
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    runner::isolate_child(&mut command);
    let mut child = command
        .spawn()
        .map_err(|e| format!("Could not start SSH discovery: {e}"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or("SSH discovery stdout unavailable.")?;
    let stderr = child
        .stderr
        .take()
        .ok_or("SSH discovery diagnostics unavailable.")?;
    let (tx, rx) = mpsc::sync_channel(1);
    thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = stdout
            .take(32 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map(|_| bytes);
        let _ = tx.send(result);
    });
    let (diagnostic_tx, diagnostic_rx) = mpsc::sync_channel(1);
    thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = stderr
            .take(16 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map(|_| bytes);
        let _ = diagnostic_tx.send(result);
    });
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if std::time::Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(10));
            }
            Ok(None) => break None,
            Err(_) => break None,
        }
    };
    // Also signal an exited SSH parent: a remote descendant may still retain
    // stdout/stderr and otherwise keep the reader threads alive.
    runner::terminate_bounded(&mut child);
    let drain_timeout = deadline
        .saturating_duration_since(std::time::Instant::now())
        .max(Duration::from_millis(250))
        .min(Duration::from_secs(1));
    let diagnostics = diagnostic_rx
        .recv_timeout(drain_timeout)
        .unwrap_or_else(|_| Ok(Vec::new()))
        .map_err(|_| "Could not read SSH agent discovery diagnostics.")?;
    if status.is_none() {
        return Err(runner::with_ssh_diagnostic(
            "SSH agent discovery timed out. Check the saved host connection.",
            &diagnostics,
        ));
    }
    let bytes = rx
        .recv_timeout(drain_timeout)
        .map_err(|_| "Could not read SSH agent discovery output.".to_string())?
        .map_err(|_| "Could not read SSH agent discovery output.")?;
    if !status.is_some_and(|s| s.success()) {
        return Err(runner::with_ssh_diagnostic(
            "SSH agent discovery failed. Check host keys, authentication and the saved host connection.",
            &diagnostics,
        ));
    }
    if bytes.len() > 32 * 1024 {
        return Err("SSH discovery output exceeded its limit.".into());
    }
    String::from_utf8(bytes).map_err(|_| "SSH discovery returned invalid text.".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovery_requires_successful_process_exit_not_just_stdout_eof() {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "printf 'candidate\\t/bin/sh\\n'; exit 7"]);
        assert!(bounded_discovery_with_timeout(command, Duration::from_secs(1)).is_err());
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "exec 1>&-; sleep 2"]);
        assert!(bounded_discovery_with_timeout(command, Duration::from_millis(100)).is_err());
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "printf 'candidate\\t/bin/sh\\n'"]);
        assert_eq!(
            bounded_discovery_with_timeout(command, Duration::from_secs(1)).unwrap(),
            "candidate\t/bin/sh\n"
        );
    }

    #[test]
    fn discovery_reports_sanitized_ssh_failure_diagnostic() {
        let mut command = Command::new("/bin/sh");
        command.args([
            "-c",
            "printf 'WARNING: ignored startup noise\\nHost key verification failed.\\n' >&2; exit 255",
        ]);
        let error = bounded_discovery_with_timeout(command, Duration::from_secs(1)).unwrap_err();
        assert!(error.contains("Host key verification failed."));
        assert!(!error.contains("ignored startup noise"));
    }

    #[test]
    fn catalog_has_unique_ids_and_never_auto_installs() {
        let candidates = catalog();
        assert!(candidates.len() >= 10);
        let mut ids = std::collections::HashSet::new();
        for candidate in &candidates {
            assert!(ids.insert(&candidate.id));
            assert!(
                !["npx", "npm", "uvx", "curl", "sh"].contains(&candidate.launch.command.as_str())
            );
            assert!(!candidate.detected);
            assert!(candidate.source_url.starts_with("https://"));
        }
        assert_eq!(
            candidates
                .iter()
                .find(|c| c.id == "qwen")
                .unwrap()
                .launch
                .command,
            "qwen"
        );
        assert_eq!(
            candidates
                .iter()
                .find(|c| c.id == "pi")
                .unwrap()
                .launch
                .command,
            "pi-acp"
        );
    }

    #[test]
    fn custom_resolution_does_not_invoke_shell_or_accept_relative_paths() {
        assert!(resolve_command("./agent").is_err());
        assert!(resolve_command("../agent").is_err());
        assert!(resolve_command("agent\nother").is_err());
        assert!(resolve_command("$(touch /tmp/monitter-not-created)").is_err());
    }
}
