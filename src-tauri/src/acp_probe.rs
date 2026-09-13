//! Explicit initialize-only connection verification. No session creation,
//! authentication, prompt, filesystem/terminal callback or package installation.
use crate::{
    acp_protocol, acp_transport,
    model::{AcpLaunch, Host},
    runner,
};
use serde::Serialize;
use serde_json::{json, Value};
use std::{
    io::{BufReader, Read, Write},
    process::Child,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeResult {
    pub protocol_version: u64,
    pub agent_name: Option<String>,
    pub agent_version: Option<String>,
    pub load_session: bool,
    pub resume_session: bool,
    pub image: bool,
    pub audio: bool,
    pub embedded_context: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, path::PathBuf};
    fn host() -> Host {
        Host {
            id: "local".into(),
            name: "Local".into(),
            kind: "local".into(),
            address: "localhost".into(),
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
    fn script(mode: &str, log: &PathBuf) -> PathBuf {
        let path = std::env::temp_dir().join(format!("monitter-probe-{}", crate::id()));
        let source=format!("#!/usr/bin/env node\nimport readline from 'node:readline';import fs from 'node:fs';const m={mode:?},l={log:?};readline.createInterface({{input:process.stdin}}).on('line',x=>{{let v=JSON.parse(x);fs.appendFileSync(l,(v.method||'response')+'\\n');if(v.method==='initialize'){{if(m==='bad')console.log('bad');else if(m==='callback')console.log(JSON.stringify({{jsonrpc:'2.0',id:'callback',method:'tools/call',params:{{}}}}));else console.log(JSON.stringify({{jsonrpc:'2.0',id:v.id,result:{{protocolVersion:m==='version'?2:1,agentCapabilities:{{}}}}}}));}}}});\n");
        fs::write(&path, source).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        }
        path
    }
    #[test]
    fn initialize_only_probe_never_sends_session_or_prompt() {
        let log = std::env::temp_dir().join(format!("monitter-probe-log-{}", crate::id()));
        let path = script("ok", &log);
        assert!(verify(
            &host(),
            &AcpLaunch {
                command: path.to_string_lossy().into_owned(),
                args: vec![]
            }
        )
        .is_ok());
        assert_eq!(fs::read_to_string(&log).unwrap(), "initialize\n");
        let _ = fs::remove_file(path);
        let _ = fs::remove_file(log);
    }
    #[test]
    fn probe_rejects_wrong_version_malformed_stdout_and_callback() {
        for mode in ["version", "bad", "callback"] {
            let log = std::env::temp_dir().join(format!("monitter-probe-log-{}", crate::id()));
            let path = script(mode, &log);
            assert!(verify(
                &host(),
                &AcpLaunch {
                    command: path.to_string_lossy().into_owned(),
                    args: vec![]
                }
            )
            .is_err());
            assert_eq!(fs::read_to_string(&log).unwrap(), "initialize\n");
            let _ = fs::remove_file(path);
            let _ = fs::remove_file(log);
        }
    }
}

struct Stop(Child);
impl Drop for Stop {
    fn drop(&mut self) {
        runner::terminate_bounded(&mut self.0);
    }
}

fn label(value: &Value) -> Option<String> {
    value
        .as_str()
        .map(|s| s.chars().filter(|c| !c.is_control()).take(160).collect())
}

pub fn verify(host: &Host, launch: &AcpLaunch) -> Result<ProbeResult, String> {
    let directory = std::env::temp_dir();
    let cwd = if host.kind == "local" {
        directory.to_string_lossy().into_owned()
    } else if host.default_cwd.is_empty() {
        "~".into()
    } else {
        host.default_cwd.clone()
    };
    let child = acp_transport::command(host, launch, &cwd)?
        .spawn()
        .map_err(|e| format!("Could not start ACP verification: {e}"))?;
    let mut owned = Stop(child);
    let mut stdin = owned
        .0
        .stdin
        .take()
        .ok_or("ACP verification input unavailable.")?;
    let stdout = owned
        .0
        .stdout
        .take()
        .ok_or("ACP verification output unavailable.")?;
    if let Some(mut stderr) = owned.0.stderr.take() {
        thread::spawn(move || {
            let mut chunk = [0u8; 8192];
            while let Ok(n) = stderr.read(&mut chunk) {
                if n == 0 {
                    break;
                }
            }
        });
    }
    let (tx, rx) = mpsc::sync_channel(1);
    thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        loop {
            let frame = acp_protocol::read_frame(&mut reader);
            let done = !matches!(&frame, Ok(Some(_)));
            if tx.send(frame).is_err() || done {
                break;
            }
        }
    });
    // Only this small bounded frame is written; no callback responses can
    // block the probe behind an agent that doesn't read its stdin.
    let initialize =
        acp_protocol::request(json!(1), "initialize", acp_protocol::initialize_params());
    stdin
        .write_all(format!("{initialize}\n").as_bytes())
        .map_err(|_| "Could not send ACP initialize.")?;
    let deadline = Instant::now() + Duration::from_secs(20);
    for _ in 0..128 {
        let frame=rx.recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .map_err(|_|"ACP initialize verification timed out after 20 seconds.")??
            .ok_or("Agent closed before completing ACP initialize. Check its ACP launch arguments and sign-in.")?;
        if frame.get("method").is_some() {
            if frame.get("id").is_some() {
                // No session exists in this probe, so no interactive operation
                // can be granted. Close instead of exposing approval controls.
                return Err("Agent requested client operations before initialization finished. Verification stopped without granting access.".into());
            }
            continue;
        }
        if frame["id"] != 1 {
            return Err("ACP verification received an unexpected response ID.".into());
        }
        if frame.get("error").is_some() {
            return Err(
                "Agent rejected ACP initialize. Check its ACP mode, version and existing sign-in."
                    .into(),
            );
        }
        let result = &frame["result"];
        let caps = acp_protocol::Capabilities::from_initialize(result)?;
        return Ok(ProbeResult {
            protocol_version: acp_protocol::PROTOCOL_VERSION,
            agent_name: label(&result["agentInfo"]["name"]),
            agent_version: label(&result["agentInfo"]["version"]),
            load_session: caps.load_session,
            resume_session: caps.resume_session,
            image: caps.image,
            audio: caps.audio,
            embedded_context: caps.embedded_context,
        });
    }
    Err("Agent sent too many messages before ACP initialization finished.".into())
}
