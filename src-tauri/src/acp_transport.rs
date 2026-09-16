//! Command construction for generic ACP local/SSH stdio. No ACP-specific agent
//! flags or authentication/configuration changes are ever added here.
use crate::{
    acp_discovery,
    model::{AcpLaunch, Host},
    runner,
};
use std::{
    collections::HashSet,
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

pub const REMOTE_READY_METHOD: &str = "_monitter/acp_transport_ready";
pub const REMOTE_SUPERVISOR: &str = include_str!("acp_remote.py");

pub fn command(host: &Host, launch: &AcpLaunch, cwd: &str) -> Result<Command, String> {
    if !crate::model::valid_acp_launch(launch) || cwd.contains('\0') || cwd.is_empty() {
        return Err("Invalid ACP executable, arguments or working folder.".into());
    }
    let mut command = match host.kind.as_str() {
        "local" => local_command(launch, cwd, std::env::var_os("PATH").as_deref())?,
        "ssh" => {
            let target = runner::ssh_target(host)?;
            if target.starts_with('-') || target.chars().any(char::is_control) {
                return Err("Invalid SSH target for ACP.".into());
            }
            let mut command = runner::ssh_command(host);
            runner::add_ssh_options(&mut command, host);
            let mut remote = vec![
                "python3".into(),
                "-u".into(),
                "-c".into(),
                REMOTE_SUPERVISOR.into(),
                cwd.into(),
                launch.command.clone(),
            ];
            remote.extend(launch.args.iter().cloned());
            command.arg("-T").arg("--").arg(target).arg(
                remote
                    .iter()
                    .map(|arg| runner::posix_quote(arg))
                    .collect::<Vec<_>>()
                    .join(" "),
            );
            command
        }
        _ => return Err("Unknown host kind for ACP.".into()),
    };
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    runner::isolate_child(&mut command);
    Ok(command)
}

fn local_command(
    launch: &AcpLaunch,
    cwd: &str,
    inherited_path: Option<&OsStr>,
) -> Result<Command, String> {
    let executable = acp_discovery::resolve_command(&launch.command)?;
    let mut command = Command::new(&executable);
    command
        .args(&launch.args)
        .current_dir(cwd)
        .env("PATH", local_runtime_path(&executable, inherited_path)?);
    Ok(command)
}

/// Finder normally supplies only `/usr/bin:/bin`. ACP bridge launchers can be
/// `env node` shims, so their child PATH needs the resolved launcher directory
/// and reviewed local runtime directories. This remains an argv invocation;
/// neither a shell nor any provider configuration is involved.
fn local_runtime_path(
    executable: &Path,
    inherited_path: Option<&OsStr>,
) -> Result<OsString, String> {
    let mut paths = Vec::new();
    if let Some(parent) = executable.parent() {
        paths.push(parent.to_path_buf());
    }
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        paths.extend([home.join(".local/bin"), home.join(".npm-global/bin")]);
    }
    paths.extend([
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
        PathBuf::from("/usr/bin"),
        PathBuf::from("/bin"),
        PathBuf::from("/usr/sbin"),
        PathBuf::from("/sbin"),
    ]);
    if let Some(existing) = inherited_path {
        paths.extend(
            std::env::split_paths(existing)
                .filter(|path| path.is_absolute())
                .take(128),
        );
    }
    let mut seen = HashSet::new();
    paths.retain(|path| seen.insert(path.clone()));
    std::env::join_paths(paths)
        .map_err(|_| "The ACP runtime PATH contains an unsupported directory.".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_launch_quotes_each_argument_and_keeps_ssh_security() {
        let host = Host {
            id: "ssh".into(),
            name: "SSH".into(),
            kind: "ssh".into(),
            address: "fixture-host".into(),
            user: "".into(),
            port: 0,
            identity_file: "".into(),
            default_cwd: "~/work".into(),
            codex_path: "".into(),
            claude_path: "".into(),
            opencode_path: "".into(),
            hermes_path: "".into(),
        };
        let launch = AcpLaunch {
            command: "/path with spaces/agent".into(),
            args: vec!["literal $(not-run)".into(), "quote'arg".into()],
        };
        let built = command(&host, &launch, "~/project folder").unwrap();
        let args = built
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert!(args.iter().any(|a| a == "StrictHostKeyChecking=yes"));
        assert!(args.iter().any(|a| a == "BatchMode=yes"));
        assert!(!args.iter().any(|a| a == "-p"));
        let remote = args.last().unwrap();
        assert!(remote.ends_with(
            "'~/project folder' '/path with spaces/agent' 'literal $(not-run)' 'quote'\\''arg'"
        ));
        assert!(!remote.contains("--dangerously"));
        assert!(built
            .get_envs()
            .all(|(key, _)| key != std::ffi::OsStr::new("PATH")));
    }

    #[cfg(unix)]
    #[test]
    fn local_acp_shebang_finds_sibling_runtime_with_minimal_parent_path() {
        use std::{
            fs,
            os::unix::fs::PermissionsExt,
            time::{SystemTime, UNIX_EPOCH},
        };

        let root = std::env::temp_dir().join(format!(
            "monitter-acp-transport-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let bin = root.join("bin");
        fs::create_dir_all(&bin).unwrap();
        let helper = bin.join("agent");
        let runtime = bin.join("fixture-acp-runtime");
        fs::write(&helper, "#!/usr/bin/env fixture-acp-runtime\n").unwrap();
        fs::write(&runtime, "#!/bin/sh\nprintf 'runtime-found\\n'\n").unwrap();
        for script in [&helper, &runtime] {
            fs::set_permissions(script, fs::Permissions::from_mode(0o700)).unwrap();
        }

        let launch = AcpLaunch {
            command: helper.to_string_lossy().into_owned(),
            args: vec![],
        };
        let mut built = local_command(
            &launch,
            root.to_str().unwrap(),
            Some(OsStr::new("/usr/bin:/bin")),
        )
        .unwrap();
        let output = built.output().unwrap();
        let _ = fs::remove_dir_all(&root);

        assert!(output.status.success(), "{:?}", output);
        assert_eq!(output.stdout, b"runtime-found\n");
    }
}
