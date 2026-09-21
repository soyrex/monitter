//! Explicit initialize-only connection verification. No session creation,
//! authentication, prompt, filesystem/terminal callback or package installation.
use crate::{
    acp_protocol, acp_session_config, acp_transport,
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
    /// Per-harness extension flags. mona-acp sets `monitter.auth_loader`,
    /// `monitter.jev_routing`, and `monitter.reasoning_effort` to true.
    /// Other harnesses leave these false or omit them.
    #[serde(default)]
    pub jev_routing: bool,
    #[serde(default)]
    pub auth_loader: bool,
    #[serde(default)]
    pub reasoning_effort: bool,
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

    #[test]
    fn catalog_probe_creates_a_prompt_free_session_and_reads_advertised_models() {
        let log = std::env::temp_dir().join(format!("monitter-catalog-log-{}", crate::id()));
        let path = std::env::temp_dir().join(format!("monitter-catalog-{}", crate::id()));
        let source = format!(
            r#"#!/usr/bin/env node
import readline from 'node:readline'; import fs from 'node:fs'; const log={log:?};
readline.createInterface({{input:process.stdin}}).on('line', line=>{{
 const frame=JSON.parse(line); fs.appendFileSync(log,(frame.method||'response')+'\n');
 if(frame.method==='initialize') console.log(JSON.stringify({{jsonrpc:'2.0',id:frame.id,result:{{protocolVersion:1,agentCapabilities:{{}}}}}}));
 else if(frame.method==='session/new') console.log(JSON.stringify({{jsonrpc:'2.0',id:frame.id,result:{{sessionId:'catalog-session',configOptions:[{{id:'opaque-model',name:'Model',category:'model',type:'select',currentValue:'vendor/one',options:[{{value:'vendor/one',name:'One'}},{{value:'vendor/two',name:'Two'}}]}}]}}}}));
}});"#
        );
        fs::write(&path, source).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let catalog = model_catalog(
            &host(),
            &AcpLaunch {
                command: path.to_string_lossy().into_owned(),
                args: vec![],
            },
            std::env::temp_dir().to_string_lossy().as_ref(),
        )
        .unwrap();
        assert_eq!(catalog.current.model, "vendor/one");
        assert_eq!(
            catalog
                .models
                .iter()
                .map(|model| model.id.as_str())
                .collect::<Vec<_>>(),
            ["vendor/one", "vendor/two"]
        );
        assert_eq!(
            fs::read_to_string(&log).unwrap(),
            "initialize\nsession/new\n"
        );
        let _ = fs::remove_file(path);
        let _ = fs::remove_file(log);
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
        // mona-acp (and any harness that opts in to Monitter extensions)
        // returns `agentCapabilities.extensions.monitter = { auth_loader,
        // jev_routing, reasoning_effort }`. Other harnesses leave the
        // block out entirely; we default to false.
        let ext = &result["agentCapabilities"]["extensions"]["monitter"];
        return Ok(ProbeResult {
            protocol_version: acp_protocol::PROTOCOL_VERSION,
            agent_name: label(&result["agentInfo"]["name"]),
            agent_version: label(&result["agentInfo"]["version"]),
            load_session: caps.load_session,
            resume_session: caps.resume_session,
            image: caps.image,
            audio: caps.audio,
            embedded_context: caps.embedded_context,
            jev_routing: ext["jev_routing"].as_bool().unwrap_or(false),
            auth_loader: ext["auth_loader"].as_bool().unwrap_or(false),
            reasoning_effort: ext["reasoning_effort"].as_bool().unwrap_or(false),
        });
    }
    Err("Agent sent too many messages before ACP initialization finished.".into())
}

/// Read the model selector advertised by a fresh ACP session. The probe has
/// no prompt, MCP servers, or approval path: session creation only lets an ACP
/// harness materialize the configuration it normally advertises before turn
/// one. The transport is always terminated before returning.
pub fn model_catalog(
    host: &Host,
    launch: &AcpLaunch,
    cwd: &str,
) -> Result<crate::model::ModelCatalog, String> {
    let child = acp_transport::command(host, launch, cwd)?
        .spawn()
        .map_err(|e| format!("Could not start ACP model lookup: {e}"))?;
    let mut owned = Stop(child);
    let mut stdin = owned
        .0
        .stdin
        .take()
        .ok_or("ACP model lookup input unavailable.")?;
    let stdout = owned
        .0
        .stdout
        .take()
        .ok_or("ACP model lookup output unavailable.")?;
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
    let (tx, rx) = mpsc::sync_channel(16);
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
    write_frame(
        &mut stdin,
        acp_protocol::request(json!(1), "initialize", acp_protocol::initialize_params()),
    )?;
    let initialize = read_response(&rx, &mut stdin, 1, Instant::now() + Duration::from_secs(60))?;
    acp_protocol::Capabilities::from_initialize(&initialize["result"])?;
    write_frame(
        &mut stdin,
        acp_protocol::request(
            json!(2),
            "session/new",
            json!({"cwd": cwd, "mcpServers": []}),
        ),
    )?;
    let session = read_response(&rx, &mut stdin, 2, Instant::now() + Duration::from_secs(20))?;
    let result = session
        .get("result")
        .ok_or("ACP session/new returned an unsupported response.")?;
    // Require an actual session rather than treating a partial reply as an
    // advertisement; its value is deliberately never persisted or reused.
    result["sessionId"]
        .as_str()
        .filter(|id| !id.is_empty() && id.len() <= 512)
        .ok_or("ACP session/new omitted sessionId during model lookup.")?;
    acp_session_config::model_catalog(result)
}

fn write_frame(stdin: &mut impl Write, frame: Value) -> Result<(), String> {
    serde_json::to_writer(&mut *stdin, &frame)
        .map_err(|_| "Could not write ACP model lookup request.".to_string())?;
    stdin
        .write_all(b"\n")
        .and_then(|_| stdin.flush())
        .map_err(|_| "Could not send ACP model lookup request.".to_string())
}

fn read_response(
    rx: &mpsc::Receiver<Result<Option<Value>, String>>,
    stdin: &mut impl Write,
    expected_id: i64,
    deadline: Instant,
) -> Result<Value, String> {
    for _ in 0..128 {
        let wait = deadline
            .checked_duration_since(Instant::now())
            .ok_or("ACP model lookup timed out.")?;
        let frame = rx
            .recv_timeout(wait)
            .map_err(|_| "ACP model lookup timed out or the agent exited.")??
            .ok_or("ACP agent closed before model lookup completed.")?;
        if let Some(method) = frame.get("method") {
            if frame.get("id").is_some() {
                // ACP callbacks have no authority in a catalog probe. Reply
                // explicitly so a harness cannot wait forever, but never
                // grant permissions, expose tools, or create another session.
                write_frame(
                    stdin,
                    acp_protocol::error_response(
                        frame["id"].clone(),
                        -32601,
                        "Monitter model lookup does not support ACP callbacks.",
                    ),
                )?;
            }
            if method.as_str().is_none() {
                return Err("ACP model lookup received a malformed notification.".into());
            }
            continue;
        }
        if frame["id"].as_i64() != Some(expected_id) {
            return Err("ACP model lookup received an unexpected response ID.".into());
        }
        if frame.get("error").is_some() {
            let message = frame
                .pointer("/error/message")
                .and_then(Value::as_str)
                .ok_or("ACP model lookup received an invalid error response.")?;
            return Err(format!("ACP model lookup request failed: {message}"));
        }
        return Ok(frame);
    }
    Err("ACP sent too many messages during model lookup.".into())
}
