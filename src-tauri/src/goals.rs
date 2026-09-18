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
    let mut child = app_server_command(host, task.codex_home.as_deref())?
        .spawn()
        .map_err(|error| {
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
    let mut child = app_server_command(host, task.codex_home.as_deref())?.spawn().map_err(|error| {
        format!("Could not start Codex app-server to clear the goal: {error}")
    })?;
    let result = clear_goal_from_child(&mut child, thread_id);
    stop_child(&mut child);
    result
}

/// Set or update the persisted Codex goal without starting a model turn.
pub(crate) fn set_goal(
    host: &Host,
    task: &Task,
    objective: Option<&str>,
    status: &str,
) -> Result<Value, String> {
    let thread_id = task
        .native_session_id
        .as_deref()
        .filter(|id| !id.trim().is_empty())
        .ok_or_else(|| "A Codex session ID is required to update its goal.".to_string())?;
    if let Some(objective) = objective {
        let length = objective.chars().count();
        if objective.trim().is_empty() || length > 4_000 {
            return Err("Goal objective must contain 1 to 4,000 characters.".into());
        }
    }
    if !matches!(status, "active" | "paused") {
        return Err("Goal status must be active or paused.".into());
    }
    let mut child = app_server_command(host, task.codex_home.as_deref())?.spawn().map_err(|error| {
        format!("Could not start Codex app-server to update the goal: {error}")
    })?;
    let result = set_goal_from_child(&mut child, thread_id, objective, status);
    stop_child(&mut child);
    result
}

/// Read a native Codex child thread without resuming it or starting a turn.
/// The app-server payload is intentionally reduced to the same transcript
/// vocabulary used by Monitter-owned delegated tasks.
pub(crate) fn read_subagent_transcript(
    host: &Host,
    codex_home: Option<&str>,
    thread_id: &str,
) -> Result<Vec<crate::model::SubagentTranscriptEntry>, String> {
    if thread_id.trim().is_empty() {
        return Err("A Codex subagent thread ID is required.".into());
    }
    let mut child = app_server_command(host, codex_home)?.spawn().map_err(|error| {
        format!("Could not start Codex app-server for subagent transcript lookup: {error}")
    })?;
    let result = read_subagent_transcript_from_child(&mut child, thread_id);
    stop_child(&mut child);
    result
}

fn app_server_command(host: &Host, codex_home: Option<&str>) -> Result<Command, String> {
    if host.kind != "local" && codex_home.is_some() {
        return Err("A selected Codex account home can only run on this Mac, not over SSH.".into());
    }
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
    if host.kind == "local" {
        crate::codex_accounts::configure_command(&mut command, codex_home)?;
    }
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

fn set_goal_from_child(
    child: &mut Child,
    thread_id: &str,
    objective: Option<&str>,
    status: &str,
) -> Result<Value, String> {
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
    response(&lines, 1, deadline, "goal update")
        .and_then(|value| ensure_success(&value, "Codex app-server initialization"))?;
    send(&mut stdin, initialized_notification())?;
    let mut request = json!({
        "id": 2,
        "method": "thread/goal/set",
        "params": { "threadId": thread_id, "status": status }
    });
    if let Some(objective) = objective {
        request["params"]["objective"] = Value::String(objective.trim().into());
    }
    send(&mut stdin, request)?;
    let value = response(&lines, 2, deadline, "goal update")?;
    ensure_success(&value, "Codex app-server thread/goal/set")?;
    value
        .pointer("/result/goal")
        .cloned()
        .ok_or_else(|| "Codex app-server goal update omitted its goal.".to_string())
}

fn read_subagent_transcript_from_child(
    child: &mut Child,
    thread_id: &str,
) -> Result<Vec<crate::model::SubagentTranscriptEntry>, String> {
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
    response(&lines, 1, deadline, "subagent transcript lookup")
        .and_then(|value| ensure_success(&value, "Codex app-server initialization"))?;
    send(&mut stdin, initialized_notification())?;
    send(&mut stdin, thread_read_request(thread_id))?;
    let value = response(&lines, 2, deadline, "subagent transcript lookup")?;
    ensure_success(&value, "Codex app-server thread/read")?;
    normalize_transcript(&value)
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

fn thread_read_request(thread_id: &str) -> Value {
    json!({
        "id": 2,
        "method": "thread/read",
        "params": { "threadId": thread_id, "includeTurns": true }
    })
}

fn bounded_text(value: &str) -> String {
    const LIMIT: usize = 12_000;
    if value.chars().count() <= LIMIT {
        value.to_string()
    } else {
        format!("{}\n…", value.chars().take(LIMIT).collect::<String>())
    }
}

fn item_text(item: &Value) -> Option<(&'static str, String)> {
    match item.get("type").and_then(Value::as_str)? {
        "userMessage" => {
            let text = item
                .get("content")?
                .as_array()?
                .iter()
                .filter(|part| part.get("type").and_then(Value::as_str) == Some("text"))
                .filter_map(|part| part.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n");
            (!text.trim().is_empty()).then_some(("user", text))
        }
        "agentMessage" => item
            .get("text")
            .and_then(Value::as_str)
            .filter(|text| !text.trim().is_empty())
            .map(|text| ("assistant", text.to_string())),
        "reasoning" => {
            let text = item
                .get("summary")?
                .as_array()?
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join("\n");
            (!text.trim().is_empty()).then_some(("reasoning", text))
        }
        "plan" => item
            .get("text")
            .and_then(Value::as_str)
            .filter(|text| !text.trim().is_empty())
            .map(|text| ("activity", format!("Plan\n{text}"))),
        "commandExecution" => {
            let command = item.get("command").and_then(Value::as_str).unwrap_or("Command");
            let status = item.get("status").and_then(Value::as_str).unwrap_or("running");
            let output = item
                .get("aggregatedOutput")
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .map(|value| format!("\n{}", bounded_text(value)))
                .unwrap_or_default();
            Some(("activity", format!("{status}: {command}{output}")))
        }
        "fileChange" => {
            let count = item.get("changes").and_then(Value::as_array).map(Vec::len).unwrap_or(0);
            let status = item.get("status").and_then(Value::as_str).unwrap_or("updated");
            Some(("activity", format!("{status}: changed {count} {}", if count == 1 { "file" } else { "files" })))
        }
        "mcpToolCall" => {
            let server = item.get("server").and_then(Value::as_str).unwrap_or("tool");
            let tool = item.get("tool").and_then(Value::as_str).unwrap_or("call");
            let status = item.get("status").and_then(Value::as_str).unwrap_or("running");
            Some(("activity", format!("{status}: {server}.{tool}")))
        }
        "dynamicToolCall" => {
            let tool = item.get("tool").and_then(Value::as_str).unwrap_or("tool");
            let status = item.get("status").and_then(Value::as_str).unwrap_or("running");
            Some(("activity", format!("{status}: {tool}")))
        }
        "collabAgentToolCall" => {
            let tool = item.get("tool").and_then(Value::as_str).unwrap_or("delegation");
            let status = item.get("status").and_then(Value::as_str).unwrap_or("running");
            Some(("activity", format!("{status}: {tool}")))
        }
        "webSearch" => Some(("activity", "Searching the web".into())),
        "imageView" => Some(("activity", "Inspecting an image".into())),
        "sleep" => Some(("activity", "Waiting".into())),
        _ => None,
    }
}

fn normalize_transcript(value: &Value) -> Result<Vec<crate::model::SubagentTranscriptEntry>, String> {
    let turns = value
        .pointer("/result/thread/turns")
        .and_then(Value::as_array)
        .ok_or_else(|| "Codex app-server thread/read omitted result.thread.turns.".to_string())?;
    let mut entries = Vec::new();
    for turn in turns {
        let created_at = turn
            .get("startedAt")
            .and_then(Value::as_i64)
            .unwrap_or_default()
            .saturating_mul(1000);
        let turn_id = turn.get("id").and_then(Value::as_str).unwrap_or("turn");
        for (index, item) in turn
            .get("items")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .enumerate()
        {
            let Some((role, text)) = item_text(item) else { continue };
            entries.push(crate::model::SubagentTranscriptEntry {
                id: item
                    .get("id")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("{turn_id}:{index}")),
                role: role.into(),
                text: bounded_text(&text),
                created_at,
            });
        }
    }
    Ok(entries)
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
        assert_eq!(thread_read_request("child-1")["method"], "thread/read");
        assert_eq!(thread_read_request("child-1")["params"]["threadId"], "child-1");
        assert_eq!(thread_read_request("child-1")["params"]["includeTurns"], true);
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

    #[test]
    fn normalizes_subagent_messages_and_activity() {
        let entries = normalize_transcript(&json!({
            "result": { "thread": { "turns": [{
                "id": "turn-1", "startedAt": 42,
                "items": [
                    {"type":"userMessage","id":"u1","content":[{"type":"text","text":"Check it","text_elements":[]}]},
                    {"type":"reasoning","id":"r1","summary":["Inspecting files"],"content":[]},
                    {"type":"commandExecution","id":"c1","command":"rg TODO","status":"completed","aggregatedOutput":"one"},
                    {"type":"agentMessage","id":"a1","text":"Done"}
                ]
            }]}}
        })).unwrap();
        assert_eq!(entries.len(), 4);
        assert_eq!(entries[0].role, "user");
        assert_eq!(entries[1].role, "reasoning");
        assert_eq!(entries[2].text, "completed: rg TODO\none");
        assert_eq!(entries[3].text, "Done");
        assert_eq!(entries[3].created_at, 42_000);
    }

    #[test]
    fn local_goal_probe_receives_selected_codex_home() {
        let host = Host {
            id: "local".into(),
            name: "local".into(),
            kind: "local".into(),
            address: "localhost".into(),
            user: String::new(),
            port: 0,
            identity_file: String::new(),
            default_cwd: "/tmp".into(),
            codex_path: "/bin/echo".into(),
            claude_path: String::new(),
            opencode_path: String::new(),
            hermes_path: String::new(),
        };
        let command = app_server_command(&host, Some("/tmp")).unwrap();
        assert!(command.get_envs().any(|(key, value)| {
            key == std::ffi::OsStr::new("CODEX_HOME")
                && (value == Some(std::ffi::OsStr::new("/private/tmp"))
                    || value == Some(std::ffi::OsStr::new("/tmp")))
        }));
    }
}
