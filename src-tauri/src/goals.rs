use crate::{
    model::{Host, Task},
    runner::{add_ssh_options, remote_exec, resolve_local, ssh_target},
};
use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader, Write},
    process::{Child, Command, Stdio},
    sync::mpsc::{self, Receiver},
    thread,
    time::{Duration, Instant},
};

const READ_TIMEOUT: Duration = Duration::from_secs(5);

/// Read the persisted Codex `/goal` state without resuming the thread or
/// starting a turn. `None` means this thread has no active goal.
pub(crate) fn read_goal(host: &Host, task: &Task) -> Result<Option<Value>, String> {
    let thread_id = task
        .native_session_id
        .as_deref()
        .filter(|id| !id.trim().is_empty())
        .ok_or_else(|| "A Codex session ID is required to read its goal.".to_string())?;
    let mut child = app_server_command(host)?.spawn().map_err(|error| {
        format!("Could not start Codex app-server for read-only goal lookup: {error}")
    })?;
    let result = read_goal_from_child(&mut child, thread_id);
    stop_child(&mut child);
    result
}

/// Remove the persisted Codex `/goal` state without resuming the thread or
/// starting a turn. The native transcript and Monitter task remain intact.
pub(crate) fn clear_goal(host: &Host, task: &Task) -> Result<(), String> {
    let thread_id = task
        .native_session_id
        .as_deref()
        .filter(|id| !id.trim().is_empty())
        .ok_or_else(|| "A Codex session ID is required to clear its goal.".to_string())?;
    let mut child = app_server_command(host)?.spawn().map_err(|error| {
        format!("Could not start Codex app-server to clear the goal: {error}")
    })?;
    let result = clear_goal_from_child(&mut child, thread_id);
    stop_child(&mut child);
    result
}

fn app_server_command(host: &Host) -> Result<Command, String> {
    let mut command = if host.kind == "local" {
        let mut command = Command::new(resolve_local(&host.codex_path)?);
        command.arg("app-server");
        command
    } else if host.kind == "ssh" {
        let cli = if host.codex_path.trim().is_empty() {
            "codex"
        } else {
            host.codex_path.trim()
        };
        let mut command = Command::new("ssh");
        add_ssh_options(&mut command, host);
        command
            .arg(ssh_target(host)?)
            .arg(remote_exec(cli, ["app-server"]));
        command
    } else {
        return Err("Host kind must be local or ssh.".into());
    };
    isolate_process_group(&mut command);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    Ok(command)
}

fn read_goal_from_child(child: &mut Child, thread_id: &str) -> Result<Option<Value>, String> {
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| "Could not open Codex app-server stdin.".to_string())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "Could not open Codex app-server stdout.".to_string())?;
    let lines = spawn_reader(stdout);
    let deadline = Instant::now() + READ_TIMEOUT;
    let mut stdin = stdin;

    send(&mut stdin, initialize_request())?;
    response(&lines, 1, deadline, "goal lookup")
        .and_then(|value| ensure_success(&value, "Codex app-server initialization"))?;
    send(&mut stdin, initialized_notification())?;
    send(&mut stdin, goal_request(thread_id))?;
    let value = response(&lines, 2, deadline, "goal lookup").and_then(|value| {
        ensure_success(&value, "Codex app-server thread/goal/get")?;
        normalize_goal(&value)
    })?;
    Ok(value)
}

fn clear_goal_from_child(child: &mut Child, thread_id: &str) -> Result<(), String> {
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| "Could not open Codex app-server stdin.".to_string())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "Could not open Codex app-server stdout.".to_string())?;
    let lines = spawn_reader(stdout);
    let deadline = Instant::now() + READ_TIMEOUT;
    let mut stdin = stdin;

    send(&mut stdin, initialize_request())?;
    response(&lines, 1, deadline, "goal clear")
        .and_then(|value| ensure_success(&value, "Codex app-server initialization"))?;
    send(&mut stdin, initialized_notification())?;
    send(&mut stdin, clear_goal_request(thread_id))?;
    let value = response(&lines, 2, deadline, "goal clear")?;
    ensure_success(&value, "Codex app-server thread/goal/clear")?;
    // `cleared: false` is an idempotent success: there is already no
    // persisted goal for this thread. It must still be a well-formed native
    // response, never a locally assumed clear.
    let _ = clear_result(&value)?;
    Ok(())
}

fn initialize_request() -> Value {
    json!({
        "id": 1,
        "method": "initialize",
        "params": { "clientInfo": { "name": "Monitter", "version": "0.1" } }
    })
}

fn goal_request(thread_id: &str) -> Value {
    json!({
        "id": 2,
        "method": "thread/goal/get",
        "params": { "threadId": thread_id }
    })
}

fn clear_goal_request(thread_id: &str) -> Value {
    json!({
        "id": 2,
        "method": "thread/goal/clear",
        "params": { "threadId": thread_id }
    })
}

fn initialized_notification() -> Value {
    json!({ "method": "initialized" })
}

fn send(stdin: &mut impl Write, request: Value) -> Result<(), String> {
    serde_json::to_writer(&mut *stdin, &request)
        .map_err(|error| format!("Could not encode Codex app-server request: {error}"))?;
    stdin
        .write_all(b"\n")
        .and_then(|_| stdin.flush())
        .map_err(|error| format!("Could not write Codex app-server request: {error}"))
}

fn spawn_reader(stdout: impl std::io::Read + Send + 'static) -> Receiver<Result<Value, String>> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let value = line
                .map_err(|error| format!("Could not read Codex app-server output: {error}"))
                .and_then(|line| {
                    serde_json::from_str(&line)
                        .map_err(|error| format!("Invalid Codex app-server JSON: {error}"))
                });
            if tx.send(value).is_err() {
                return;
            }
        }
    });
    rx
}

fn response(
    lines: &Receiver<Result<Value, String>>,
    id: i64,
    deadline: Instant,
    operation: &str,
) -> Result<Value, String> {
    loop {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or_else(|| {
                format!("Timed out waiting for Codex app-server {operation} response.")
            })?;
        let value = lines
            .recv_timeout(remaining)
            .map_err(|_| {
                format!(
                    "Codex app-server exited or timed out before responding to {operation}; confirm this Codex version supports app-server."
                )
            })??;
        if value.get("id").and_then(Value::as_i64) == Some(id) {
            return Ok(value);
        }
    }
}

fn ensure_success(value: &Value, operation: &str) -> Result<(), String> {
    if let Some(error) = value.get("error") {
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("unknown protocol error");
        return Err(format!("{operation} failed: {message}"));
    }
    if value.get("result").is_none() {
        return Err(format!("{operation} returned an unsupported response."));
    }
    Ok(())
}

fn clear_result(value: &Value) -> Result<bool, String> {
    value
        .pointer("/result/cleared")
        .and_then(Value::as_bool)
        .ok_or_else(|| "Codex app-server goal clear response omitted result.cleared.".to_string())
}

fn normalize_goal(value: &Value) -> Result<Option<Value>, String> {
    let goal = value
        .pointer("/result/goal")
        .ok_or_else(|| "Codex app-server goal response omitted result.goal.".to_string())?;
    if goal.is_null() {
        return Ok(None);
    }
    let object = goal
        .as_object()
        .ok_or_else(|| "Codex app-server returned an invalid goal.".to_string())?;
    for key in ["objective", "status", "tokensUsed", "timeUsedSeconds"] {
        if !object.contains_key(key) {
            return Err(format!("Codex app-server goal response omitted {key}."));
        }
    }
    let mut normalized = serde_json::Map::new();
    for key in [
        "objective",
        "status",
        "tokenBudget",
        "tokensUsed",
        "timeUsedSeconds",
    ] {
        if let Some(value) = object.get(key) {
            normalized.insert(key.into(), value.clone());
        }
    }
    Ok(Some(Value::Object(normalized)))
}

fn isolate_process_group(command: &mut Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
}

fn stop_child(child: &mut Child) {
    #[cfg(unix)]
    unsafe {
        let _ = libc::kill(-(child.id() as i32), libc::SIGTERM);
    }
    let until = Instant::now() + Duration::from_millis(500);
    while Instant::now() < until {
        if child.try_wait().ok().flatten().is_some() {
            return;
        }
        thread::sleep(Duration::from_millis(25));
    }
    #[cfg(unix)]
    unsafe {
        let _ = libc::kill(-(child.id() as i32), libc::SIGKILL);
    }
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emits_goal_get_and_clear_requests() {
        assert_eq!(initialize_request()["method"], "initialize");
        assert_eq!(
            initialized_notification(),
            json!({ "method": "initialized" })
        );
        assert_eq!(goal_request("thread-1")["method"], "thread/goal/get");
        assert_eq!(goal_request("thread-1")["params"]["threadId"], "thread-1");
        assert_eq!(clear_goal_request("thread-1")["method"], "thread/goal/clear");
        assert_eq!(
            clear_goal_request("thread-1")["params"]["threadId"],
            "thread-1"
        );
    }

    #[test]
    fn normalizes_a_real_goal_without_extra_fields() {
        let goal = normalize_goal(&json!({
            "result": { "goal": {
                "threadId": "thread-1", "objective": "Ship", "status": "active",
                "tokenBudget": 1000, "tokensUsed": 12, "timeUsedSeconds": 3, "updatedAt": 4
            }}
        }))
        .unwrap()
        .unwrap();
        assert_eq!(
            goal,
            json!({
                "objective": "Ship", "status": "active", "tokenBudget": 1000,
                "tokensUsed": 12, "timeUsedSeconds": 3
            })
        );
    }

    #[test]
    fn returns_none_for_a_thread_without_a_goal() {
        assert_eq!(
            normalize_goal(&json!({ "result": { "goal": null } })).unwrap(),
            None
        );
    }

    #[test]
    fn does_not_fabricate_an_optional_token_budget() {
        let goal = normalize_goal(&json!({
            "result": { "goal": {
                "objective": "Ship", "status": "active", "tokensUsed": 12, "timeUsedSeconds": 3
            }}
        }))
        .unwrap()
        .unwrap();
        assert!(goal.get("tokenBudget").is_none());
    }

    #[test]
    fn accepts_a_native_idempotent_goal_clear_response() {
        assert_eq!(clear_result(&json!({ "result": { "cleared": true } })).unwrap(), true);
        assert_eq!(clear_result(&json!({ "result": { "cleared": false } })).unwrap(), false);
        assert!(clear_result(&json!({ "result": {} })).is_err());
    }
}
