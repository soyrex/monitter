//! Command construction for generic ACP local/SSH stdio. No ACP-specific agent
//! flags or authentication/configuration changes are ever added here.
use crate::{
    acp_discovery,
    model::{AcpLaunch, Host},
    runner,
};
use std::process::{Command, Stdio};

pub const REMOTE_READY_METHOD: &str = "_monitter/acp_transport_ready";
pub const REMOTE_SUPERVISOR: &str = include_str!("acp_remote.py");

pub fn command(host: &Host, launch: &AcpLaunch, cwd: &str) -> Result<Command, String> {
    if !crate::model::valid_acp_launch(launch) || cwd.contains('\0') || cwd.is_empty() {
        return Err("Invalid ACP executable, arguments or working folder.".into());
    }
    let mut command = match host.kind.as_str() {
        "local" => {
            let mut command = Command::new(acp_discovery::resolve_command(&launch.command)?);
            command.args(&launch.args).current_dir(cwd);
            command
        }
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
    }
}
