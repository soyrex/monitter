//! Ignored compatibility probe for the installed Codex app-server MCP client.

#[cfg(test)]
mod tests {
    use crate::collaboration_transport::Broker;
    use serde_json::{json, Value};
    use std::{
        collections::BTreeSet,
        io::{BufRead, BufReader, Write},
        path::PathBuf,
        process::{Child, Command, Stdio},
        sync::{mpsc, Arc},
        thread,
        time::{Duration, Instant},
    };

    fn send(stdin: &mut impl Write, value: Value) {
        writeln!(stdin, "{value}").expect("write Codex app-server request");
        stdin.flush().expect("flush Codex app-server request");
    }

    fn response_for(rx: &mpsc::Receiver<Value>, id: i64, stage: &str) -> Value {
        let deadline = Instant::now() + Duration::from_secs(45);
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(
                !remaining.is_zero(),
                "Codex app-server timed out during {stage} (request {id})"
            );
            let value = rx.recv_timeout(remaining).unwrap_or_else(|_| {
                panic!("Codex app-server output ended during {stage} (request {id})")
            });
            if value.get("id").and_then(Value::as_i64) == Some(id) {
                assert!(
                    value.get("error").is_none(),
                    "Codex app-server request {id} failed during {stage}"
                );
                return value;
            }
        }
    }

    struct TempCodexHome(PathBuf);
    impl TempCodexHome {
        fn create() -> Self {
            let path = std::env::temp_dir()
                .join(format!("monitter-codex-mcp-probe-{}", crate::model::id()));
            std::fs::create_dir(&path).expect("create isolated Codex home");
            Self(path)
        }
    }
    impl Drop for TempCodexHome {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    struct ChildGuard(Option<Child>);
    impl Drop for ChildGuard {
        fn drop(&mut self) {
            if let Some(mut child) = self.0.take() {
                #[cfg(unix)]
                unsafe {
                    libc::kill(-(child.id() as i32), libc::SIGTERM);
                }
                let deadline = Instant::now() + Duration::from_secs(2);
                while Instant::now() < deadline {
                    if child.try_wait().ok().flatten().is_some() {
                        return;
                    }
                    thread::sleep(Duration::from_millis(25));
                }
                #[cfg(unix)]
                unsafe {
                    libc::kill(-(child.id() as i32), libc::SIGKILL);
                }
                let _ = child.wait();
            }
        }
    }

    #[test]
    #[ignore = "requires an explicitly selected installed Codex binary and performs a local compatibility probe"]
    fn installed_codex_app_server_loads_http_monitter_mcp() {
        let codex = std::env::var("MONITTER_TEST_CODEX")
            .expect("set MONITTER_TEST_CODEX to the installed Codex binary");
        let broker = Broker::start(Arc::new(|_, _, _| Ok(json!({}))))
            .expect("start real Monitter MCP broker");
        let grant = broker.session("compatibility-probe");
        let codex_home = TempCodexHome::create();

        let mut command = Command::new(codex);
        command.args([
            "-c",
            &format!("mcp_servers.monitter.url={:?}", grant.endpoint),
            "-c",
            "mcp_servers.monitter.bearer_token_env_var=\"MONITTER_TOKEN\"",
            "-c",
            "mcp_servers.monitter.required=true",
            "app-server",
        ]);
        crate::runner::isolate_child(&mut command);
        command
            .env("MONITTER_TOKEN", &grant.token)
            .env("CODEX_HOME", &codex_home.0)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = ChildGuard(Some(command.spawn().expect("start Codex app-server")));
        let process = child.0.as_mut().expect("owned Codex process");
        let stdout = process.stdout.take().expect("Codex stdout");
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if line.len() <= 1024 * 1024 {
                    if let Ok(value) = serde_json::from_str(&line) {
                        if tx.send(value).is_err() {
                            break;
                        }
                    }
                }
            }
        });
        let stdin = process.stdin.as_mut().expect("Codex stdin");
        send(
            stdin,
            json!({"id":1,"method":"initialize","params":{"clientInfo":{"name":"monitter-compatibility-probe","version":"1"},"capabilities":{"experimentalApi":false}}}),
        );
        assert!(response_for(&rx, 1, "initialize")["result"].is_object());
        send(stdin, json!({"method":"initialized"}));
        send(
            stdin,
            json!({"id":2,"method":"thread/start","params":{"cwd":codex_home.0,"model":null,"approvalPolicy":"never","sandbox":"read-only","ephemeral":true}}),
        );
        let thread = response_for(&rx, 2, "thread start")["result"]["thread"]["id"]
            .as_str()
            .expect("Codex thread/start omitted its thread id")
            .to_owned();
        send(
            stdin,
            json!({"id":3,"method":"mcpServerStatus/list","params":{"threadId":thread}}),
        );
        let status = response_for(&rx, 3, "MCP status discovery");
        let monitter = status["result"]["data"]
            .as_array()
            .and_then(|servers| {
                servers
                    .iter()
                    .find(|server| server["name"].as_str() == Some("monitter"))
            })
            .expect("Codex did not report the Monitter MCP server");
        let names = monitter["tools"]
            .as_object()
            .expect("Monitter MCP status omitted its tool map")
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>();
        let expected = crate::collaboration_mcp::tool_names()
            .iter()
            .map(|name| (*name).to_owned())
            .collect::<BTreeSet<_>>();
        assert_eq!(
            names, expected,
            "Codex reported an unexpected MCP tool catalogue"
        );
    }
}
