//! Resident, local-only Codex app-server adapter.
//!
//! App-server is JSON-RPC over the owned child stdio pipes.  It deliberately
//! never exposes a listener and never substitutes `exec resume` when a live
//! transport is lost: doing so could replay an uncertain user turn.

use crate::{
    model::{InputOption, InputQuestion, InteractionInput, Task},
    runner::{resolve_local, RunControl},
    ApprovalDecision, CreateApprovalRequest, Parsed, Service,
};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    io::{BufRead, BufReader},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc, Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

const INITIALIZE_ID: i64 = 1;
const THREAD_ID: i64 = 2;
const FIRST_TURN_ID: i64 = 3;
const MAX_SERVER_REQUESTS: usize = 16;
const MAX_RPC_LINE: usize = 2 * 1024 * 1024;
const MAX_DELTA_TEXT: usize = 2 * 1024 * 1024;
const INITIALIZE_TIMEOUT: Duration = Duration::from_secs(20);
// Resuming a thread can require every configured MCP server to initialize.
// Keep this bounded, but accommodate the documented 120-second MCP timeout.
const THREAD_TIMEOUT: Duration = Duration::from_secs(150);
const TURN_START_TIMEOUT: Duration = Duration::from_secs(20);
const DELTA_FLUSH_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RequestStage {
    Initialize,
    Thread,
    InitialTurn,
    SubsequentTurn,
}

impl RequestStage {
    fn timeout(self) -> Duration {
        match self {
            Self::Initialize => INITIALIZE_TIMEOUT,
            Self::Thread => THREAD_TIMEOUT,
            Self::InitialTurn | Self::SubsequentTurn => TURN_START_TIMEOUT,
        }
    }

    fn timeout_error(self) -> &'static str {
        match self {
            Self::Initialize => "Codex app-server initialize request timed out.",
            Self::Thread => "Codex app-server thread start/resume request timed out.",
            Self::InitialTurn => "Codex app-server initial turn/start request timed out.",
            Self::SubsequentTurn => "Codex app-server subsequent turn/start request timed out.",
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct StageClock {
    stage: RequestStage,
    started_at: Instant,
}

impl StageClock {
    fn start(stage: RequestStage) -> Self {
        Self {
            stage,
            started_at: Instant::now(),
        }
    }

    fn deadline(self) -> Instant {
        self.started_at + self.stage.timeout()
    }

    fn expired_at(self, now: Instant) -> bool {
        now >= self.deadline()
    }
}

fn observe_subsequent_turn_clock(
    started: bool,
    control: &RunControl,
    stage_clock: &mut Option<StageClock>,
) {
    if started && stage_clock.is_none() && control.has_app_server_turn_request() {
        *stage_clock = Some(StageClock::start(RequestStage::SubsequentTurn));
    }
}

struct BufferedMessage {
    text: String,
    last_flush: Instant,
}

#[derive(Default)]
struct RequestLifecycle {
    approvals: HashMap<String, String>,
    received: HashSet<String>,
    resolved: HashSet<String>,
}

struct OwnedRun {
    service: Arc<Service>,
    task_id: String,
    control: Arc<RunControl>,
}

impl Drop for OwnedRun {
    fn drop(&mut self) {
        self.control.terminate_owned();
        self.service
            .release_app_server_run(&self.task_id, &self.control);
    }
}

pub fn start(service: Arc<Service>, task_id: String, prompt: String, control: Arc<RunControl>) {
    thread::spawn(move || run(service, task_id, prompt, control));
}

fn run(service: Arc<Service>, task_id: String, prompt: String, control: Arc<RunControl>) {
    let _owned = OwnedRun {
        service: service.clone(),
        task_id: task_id.clone(),
        control: control.clone(),
    };
    let (task, host) = match service.task_and_host(&task_id) {
        Ok(value) => value,
        Err(error) => {
            service.complete_app_server_turn(&task_id, &control, None, "error", Some(error));
            return;
        }
    };
    if host.kind != "local" {
        service.complete_app_server_turn(&task_id, &control, None, "error", Some("Codex app-server interactive sessions currently require a local desktop host. Monitter will not fall back to an uncertain exec resume turn on an SSH host.".into()));
        return;
    }
    let mut extensions = match service.extension_config().map(|config| {
        crate::extensions_runtime::RuntimeExtensions::for_agent(&config, &task.agent_id)
    }) {
        Ok(extensions) => extensions,
        Err(error) => {
            service.complete_app_server_turn(&task_id, &control, None, "error", Some(error));
            return;
        }
    };
    if let Err(error) = extensions.validate_for("codex", &host.kind) {
        service.complete_app_server_turn(&task_id, &control, None, "error", Some(error));
        return;
    }
    control.set_mcp_fingerprint(extensions.mcp_fingerprint());
    let prompt = extensions.prompt(&prompt);
    let executable = match resolve_local(&host.codex_path) {
        Ok(value) => value,
        Err(error) => {
            service.complete_app_server_turn(&task_id, &control, None, "error", Some(error));
            return;
        }
    };
    let grant = match service.collaboration_grant(&task_id) {
        Ok(value) => value,
        Err(error) => {
            service.complete_app_server_turn(&task_id, &control, None, "error", Some(error));
            return;
        }
    };
    let helper = match grant.as_ref() {
        Some(_) => match service.collaboration_helper() {
            Ok(value) => value,
            Err(error) => {
                service.complete_app_server_turn(&task_id, &control, None, "error", Some(error));
                return;
            }
        },
        None => std::path::PathBuf::new(),
    };
    let mut command = Command::new(executable);
    command
        .arg("app-server")
        .current_dir(&task.cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    crate::runner::isolate_child(&mut command);
    if let Some(grant) = &grant {
        command
            .env("MONITTER_ENDPOINT", &grant.endpoint)
            .env("MONITTER_TOKEN", &grant.token);
    }
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            service.complete_app_server_turn(
                &task_id,
                &control,
                None,
                "error",
                Some(format!("Could not start Codex app-server: {error}")),
            );
            return;
        }
    };
    let Some(stdin) = child.stdin.take() else {
        crate::runner::terminate_bounded(&mut child);
        service.complete_app_server_turn(
            &task_id,
            &control,
            None,
            "error",
            Some("Could not open Codex app-server stdin.".into()),
        );
        return;
    };
    let Some(stdout) = child.stdout.take() else {
        crate::runner::terminate_bounded(&mut child);
        service.complete_app_server_turn(
            &task_id,
            &control,
            None,
            "error",
            Some("Could not open Codex app-server stdout.".into()),
        );
        return;
    };
    let stderr = child.stderr.take();
    if let Err((mut child, _)) = control.install(child, Some(stdin)) {
        super::runner::terminate_bounded(&mut child);
        service.complete_app_server_turn(&task_id, &control, None, "interrupted", None);
        return;
    }
    control.mark_resident();
    if let Some(stderr) = stderr {
        let service = service.clone();
        let task = task_id.clone();
        thread::spawn(move || {
            let mut emitted = false;
            let mut reader = BufReader::new(stderr);
            while let Ok(Some(line)) = read_bounded_line(&mut reader, 64 * 1024) {
                if !emitted {
                    if let Some(line) = crate::runner::provider_stderr_diagnostic(&line) {
                        emitted = true;
                        service.record(&task, "error", "Codex app-server diagnostic", line);
                    }
                }
            }
        });
    }
    let mut stage_clock = Some(StageClock::start(RequestStage::Initialize));
    if let Err(error) = send(&control, initialize()) {
        service.complete_app_server_turn(&task_id, &control, None, "error", Some(error));
        return;
    }
    let mut started = false;
    let mut terminal = false;
    let mut message_text = HashMap::<String, BufferedMessage>::new();
    let outstanding_requests = Arc::new(AtomicUsize::new(0));
    let seen_server_requests = Arc::new(Mutex::new(HashSet::<String>::new()));
    let turn_items = Arc::new(Mutex::new(HashMap::<String, Value>::new()));
    let request_lifecycle = Arc::new(Mutex::new(RequestLifecycle::default()));
    let (output_tx, output_rx) = mpsc::sync_channel(64);
    thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        loop {
            match read_bounded_line(&mut reader, MAX_RPC_LINE) {
                Ok(Some(line)) => {
                    if output_tx.send(Ok(line)).is_err() {
                        break;
                    }
                }
                Ok(None) => break,
                Err(error) => {
                    let _ = output_tx.send(Err(error));
                    break;
                }
            }
        }
    });
    loop {
        if control.is_cancelled() {
            terminal = true;
            break;
        }
        // `recv_timeout` returns immediately while the pipe has queued
        // notifications. Check absolute expiry before accepting another frame
        // so notification noise cannot starve a request-stage timeout.
        if stage_clock.is_some_and(|clock| clock.expired_at(Instant::now())) {
            service.complete_app_server_turn(
                &task_id,
                &control,
                None,
                "error",
                Some(
                    stage_clock
                        .map(|clock| clock.stage.timeout_error())
                        .unwrap_or("Codex app-server request timed out.")
                        .into(),
                ),
            );
            terminal = true;
            break;
        }
        // A resident connection starts later turns outside this reader.  Begin
        // their own clock when the outstanding request is observed; only its
        // matching response clears it. Notifications never extend a clock.
        observe_subsequent_turn_clock(started, &control, &mut stage_clock);
        let deadline = stage_clock.map(StageClock::deadline);
        let poll = deadline
            .map(|deadline| deadline.saturating_duration_since(Instant::now()))
            .unwrap_or(Duration::from_millis(250))
            .min(Duration::from_millis(250));
        let received = match output_rx.recv_timeout(poll) {
            Ok(value) => Ok(value),
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                Err("Codex app-server output closed.".into())
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if stage_clock.is_some_and(|clock| clock.expired_at(Instant::now())) {
                    Err(stage_clock
                        .map(|clock| clock.stage.timeout_error())
                        .unwrap_or("Codex app-server request timed out.")
                        .into())
                } else {
                    continue;
                }
            }
        };
        let line = match received {
            Ok(Ok(line)) => line,
            Ok(Err(error)) | Err(error) => {
                service.complete_app_server_turn(
                    &task_id,
                    &control,
                    None,
                    "error",
                    Some(format!("Could not read Codex app-server output: {error}")),
                );
                terminal = true;
                break;
            }
        };
        let value: Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(error) => {
                service.complete_app_server_turn(
                    &task_id,
                    &control,
                    None,
                    "error",
                    Some(format!("Codex app-server emitted invalid JSON: {error}")),
                );
                terminal = true;
                break;
            }
        };
        // A later turn can be marked and responded to while this reader is in
        // recv_timeout. Observe again after waking, before correlating the
        // frame, so an immediate valid response cannot look out of order.
        observe_subsequent_turn_clock(started, &control, &mut stage_clock);
        // Server-to-client requests also have JSON-RPC ids.  Dispatch them
        // before considering client response correlation; never mistake one
        // for a late turn/start acknowledgement.
        if value.get("method").is_some() && value.get("id").is_some() {
            handle_server_request(
                service.clone(),
                task_id.clone(),
                control.clone(),
                value,
                outstanding_requests.clone(),
                seen_server_requests.clone(),
                turn_items.clone(),
                request_lifecycle.clone(),
            );
            continue;
        }
        if let Some(id) = value.get("id").and_then(Value::as_i64) {
            let is_turn_request = control.take_app_server_turn_request(id);
            let steer_request = control.take_app_server_steer_request(id);
            let known_request = id == INITIALIZE_ID
                || id == THREAD_ID
                || is_turn_request
                || steer_request.is_some();
            if !known_request {
                service.complete_app_server_turn(
                    &task_id,
                    &control,
                    None,
                    "error",
                    Some(format!(
                        "Codex app-server returned an unknown response ID {id}."
                    )),
                );
                terminal = true;
                break;
            }
            if let Some(steer) = steer_request {
                if let Some(error) = value.pointer("/error/message").and_then(Value::as_str) {
                    service.app_server_steer_rejected(
                        &task_id,
                        &control,
                        &steer.queued_message_id,
                        error,
                    );
                    continue;
                }
                let accepted_turn = value.pointer("/result/turnId").and_then(Value::as_str);
                if accepted_turn != Some(steer.expected_turn_id.as_str()) {
                    service.app_server_steer_rejected(
                        &task_id,
                        &control,
                        &steer.queued_message_id,
                        "Codex returned a steering acknowledgement without the expected active turn ID.",
                    );
                    continue;
                }
                service.app_server_steer_accepted(
                    &task_id,
                    &control,
                    &steer.expected_turn_id,
                    &steer.queued_message_id,
                );
                continue;
            }
            if let Some(error) = value.pointer("/error/message").and_then(Value::as_str) {
                service.complete_app_server_turn(
                    &task_id,
                    &control,
                    None,
                    "error",
                    Some(format!("Codex app-server request {id} failed: {error}")),
                );
                terminal = true;
                break;
            }
            if id == INITIALIZE_ID {
                if stage_clock.map(|clock| clock.stage) != Some(RequestStage::Initialize) {
                    service.complete_app_server_turn(
                        &task_id,
                        &control,
                        None,
                        "error",
                        Some("Codex app-server returned initialize out of order.".into()),
                    );
                    terminal = true;
                    break;
                }
                if send(&control, json!({"method":"initialized"}))
                    .and_then(|_| {
                        send(
                            &control,
                            thread_request(&task, helper.to_str(), grant.is_some(), &extensions),
                        )
                    })
                    .is_err()
                {
                    service.complete_app_server_turn(
                        &task_id,
                        &control,
                        None,
                        "error",
                        Some("Could not initialize Codex app-server thread.".into()),
                    );
                    terminal = true;
                    break;
                }
                stage_clock = Some(StageClock::start(RequestStage::Thread));
            } else if id == THREAD_ID {
                if stage_clock.map(|clock| clock.stage) != Some(RequestStage::Thread) {
                    service.complete_app_server_turn(
                        &task_id,
                        &control,
                        None,
                        "error",
                        Some("Codex app-server returned thread start/resume out of order.".into()),
                    );
                    terminal = true;
                    break;
                }
                let Some(thread_id) = value.pointer("/result/thread/id").and_then(Value::as_str)
                else {
                    service.complete_app_server_turn(
                        &task_id,
                        &control,
                        None,
                        "error",
                        Some("Codex app-server thread response omitted its ID.".into()),
                    );
                    terminal = true;
                    break;
                };
                control.set_app_server_thread(thread_id.into());
                if let Err(error) = service.app_server_event(
                    &task_id,
                    &control,
                    None,
                    Parsed {
                        native_session_id: Some(thread_id.into()),
                        assistant: None,
                        event: None,
                        failed: false,
                    },
                ) {
                    service.complete_app_server_turn(
                        &task_id,
                        &control,
                        None,
                        "error",
                        Some(error),
                    );
                    terminal = true;
                    break;
                }
                if let Err(error) = control
                    .mark_app_server_turn_request(FIRST_TURN_ID)
                    .and_then(|_| send(&control, turn_request(thread_id, &prompt, &task)))
                {
                    service.complete_app_server_turn(
                        &task_id,
                        &control,
                        None,
                        "error",
                        Some(error),
                    );
                    terminal = true;
                    break;
                }
                stage_clock = Some(StageClock::start(RequestStage::InitialTurn));
            } else if is_turn_request {
                let expected = if started {
                    RequestStage::SubsequentTurn
                } else {
                    RequestStage::InitialTurn
                };
                if stage_clock.map(|clock| clock.stage) != Some(expected) {
                    service.complete_app_server_turn(
                        &task_id,
                        &control,
                        None,
                        "error",
                        Some("Codex app-server returned turn/start out of order.".into()),
                    );
                    terminal = true;
                    break;
                }
                if let Some(turn_id) = value.pointer("/result/turn/id").and_then(Value::as_str) {
                    control.set_app_server_turn(turn_id.into());
                    started = true;
                } else {
                    service.complete_app_server_turn(
                        &task_id,
                        &control,
                        None,
                        "error",
                        Some("Codex app-server turn response omitted its ID.".into()),
                    );
                    terminal = true;
                    break;
                }
                stage_clock = None;
            }
            continue;
        }
        handle_notification(
            &service,
            &task_id,
            &control,
            &value,
            &mut started,
            &mut terminal,
            &mut message_text,
            &turn_items,
            &seen_server_requests,
            &request_lifecycle,
        );
        if terminal {
            break;
        }
    }
    if !terminal && !control.is_cancelled() {
        service.complete_app_server_turn(
            &task_id,
            &control,
            None,
            "error",
            Some(
                "Codex app-server ended before confirming this turn; Monitter did not replay it."
                    .into(),
            ),
        );
    }
    // The reader owns this child.  Once its pipe is closed, a resident handle
    // is no longer usable; bounded teardown and pointer-checked release keep
    // a cancelled/EOF transport from stranding the run registry.
}

fn read_bounded_line<R: std::io::Read>(
    reader: &mut BufReader<R>,
    max: usize,
) -> Result<Option<String>, String> {
    let mut bytes = Vec::new();
    loop {
        let available = reader.fill_buf().map_err(|e| e.to_string())?;
        if available.is_empty() {
            if bytes.is_empty() {
                return Ok(None);
            }
            break;
        }
        let take = available
            .iter()
            .position(|byte| *byte == b'\n')
            .map(|index| index + 1)
            .unwrap_or(available.len());
        if bytes.len().saturating_add(take) > max {
            return Err(format!(
                "Codex app-server emitted a line larger than {max} bytes."
            ));
        }
        bytes.extend_from_slice(&available[..take]);
        reader.consume(take);
        if bytes.last() == Some(&b'\n') {
            break;
        }
    }
    while matches!(bytes.last(), Some(b'\n' | b'\r')) {
        bytes.pop();
    }
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|_| "Codex app-server emitted non-UTF-8 output.".into())
}

fn remembered_file_change_item(item: &Value) -> Option<Value> {
    let has_change = item
        .get("changes")
        .and_then(Value::as_array)
        .is_some_and(|changes| !changes.is_empty())
        || item
            .get("patch")
            .and_then(Value::as_str)
            .is_some_and(|patch| !patch.trim().is_empty());
    if !has_change {
        return None;
    }
    let mut item = crate::strip_known_approval_envelope(item);
    if let Some(object) = item.as_object_mut() {
        // Item lifecycle labels identify the streamed envelope, not the
        // requested patch. Do not recurse into changes/tool arguments.
        for key in ["id", "status", "phase", "completedAtMs"] {
            object.remove(key);
        }
    }
    Some(item)
}

fn handle_server_request(
    service: Arc<Service>,
    task_id: String,
    control: Arc<RunControl>,
    request: Value,
    outstanding: Arc<AtomicUsize>,
    seen: Arc<Mutex<HashSet<String>>>,
    turn_items: Arc<Mutex<HashMap<String, Value>>>,
    lifecycle: Arc<Mutex<RequestLifecycle>>,
) {
    let valid_id = matches!(
        request.get("id"),
        Some(Value::Number(_)) | Some(Value::String(_))
    );
    if !valid_id {
        return;
    }
    let params = request.get("params").unwrap_or(&Value::Null);
    if params.to_string().len() > 64 * 1024 {
        let _ = send(
            &control,
            json!({"id":request["id"],"error":{"code":-32602,"message":"Codex request is too large."}}),
        );
        return;
    }
    let Some(turn_id) = params
        .get("turnId")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
    else {
        let _ = send(
            &control,
            json!({"id":request["id"],"error":{"code":-32602,"message":"Codex request omitted a turn ID."}}),
        );
        return;
    };
    let Some(thread_id) = params
        .get("threadId")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
    else {
        let _ = send(
            &control,
            json!({"id":request["id"],"error":{"code":-32602,"message":"Codex request omitted a thread ID."}}),
        );
        return;
    };
    if !control.app_server_turn_is_current(thread_id, turn_id) {
        let _ = send(
            &control,
            json!({"id":request["id"],"error":{"code":-32000,"message":"Codex request is no longer current."}}),
        );
        return;
    }
    let rpc_id = request
        .get("id")
        .cloned()
        .unwrap_or(Value::Null)
        .to_string();
    let accepted = accept_received_request(&lifecycle, &rpc_id);
    if !accepted {
        let _ = send(
            &control,
            json!({"id":request["id"],"error":{"code":-32000,"message":"Duplicate or excessive Codex request."}}),
        );
        return;
    }
    if outstanding.fetch_add(1, Ordering::SeqCst) >= MAX_SERVER_REQUESTS {
        outstanding.fetch_sub(1, Ordering::SeqCst);
        let _ = finish_request(&lifecycle, &rpc_id);
        let _ = send(
            &control,
            json!({"id":request["id"],"error":{"code":-32000,"message":"Too many outstanding Codex requests."}}),
        );
        return;
    }
    thread::spawn(move || {
        struct RequestSlot(Arc<AtomicUsize>);
        impl Drop for RequestSlot {
            fn drop(&mut self) {
                self.0.fetch_sub(1, Ordering::SeqCst);
            }
        }
        let _slot = RequestSlot(outstanding);
        let id = request.get("id").cloned().unwrap_or(Value::Null);
        let method = request.get("method").and_then(Value::as_str).unwrap_or("");
        let params = request.get("params").cloned().unwrap_or(Value::Null);
        if params.to_string().len() > 64 * 1024 {
            let _ = send(
                &control,
                json!({"id":id,"error":{"code":-32602,"message":"Codex request is too large."}}),
            );
            return;
        }
        let Some(turn_id) = params
            .get("turnId")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
        else {
            let _ = send(
                &control,
                json!({"id":id,"error":{"code":-32602,"message":"Codex request omitted a turn ID."}}),
            );
            return;
        };
        let Some(thread_id) = params
            .get("threadId")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
        else {
            let _ = send(
                &control,
                json!({"id":id,"error":{"code":-32602,"message":"Codex request omitted a thread ID."}}),
            );
            return;
        };
        if !control.app_server_turn_is_current(thread_id, turn_id) {
            let _ = send(
                &control,
                json!({"id":id,"error":{"code":-32000,"message":"Codex request is no longer current."}}),
            );
            return;
        }
        let request_key = control.app_server_request_key(turn_id, &rpc_id);
        let is_new = seen
            .lock()
            .map(|mut requests| {
                if requests.len() >= 256 || !requests.insert(request_key.clone()) {
                    false
                } else {
                    true
                }
            })
            .unwrap_or(false);
        if !is_new {
            let _ = send(
                &control,
                json!({"id":id,"error":{"code":-32000,"message":"Duplicate or excessive Codex request."}}),
            );
            return;
        }
        if method == "item/tool/requestUserInput" {
            let questions = params
                .get("questions")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|question| {
                    let id = question.get("id")?.as_str()?.to_owned();
                    let question_text = question
                        .get("question")
                        .or_else(|| question.get("prompt"))?
                        .as_str()?
                        .to_owned();
                    Some(InputQuestion {
                        id,
                        header: question
                            .get("header")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .into(),
                        question: question_text,
                        is_secret: question
                            .get("isSecret")
                            .and_then(Value::as_bool)
                            .unwrap_or(false),
                        options: question
                            .get("options")
                            .and_then(Value::as_array)
                            .into_iter()
                            .flatten()
                            .filter_map(|option| {
                                Some(InputOption {
                                    label: option.get("label")?.as_str()?.into(),
                                    description: option
                                        .get("description")
                                        .and_then(Value::as_str)
                                        .unwrap_or_default()
                                        .into(),
                                })
                            })
                            .collect(),
                    })
                })
                .collect::<Vec<_>>();
            if questions.is_empty() || questions.len() > 8 {
                let _ = send(
                    &control,
                    json!({"id":id,"error":{"code":-32602,"message":"Unsupported Codex input request."}}),
                );
                let _ = finish_request(&lifecycle, &rpc_id);
                return;
            }
            let approval = service.create_app_server_approval(
                &control,
                turn_id,
                CreateApprovalRequest {
                    task_id: task_id.clone(),
                    provider: "codex".into(),
                    run_id: request_key.clone(),
                    tool: "User input".into(),
                    summary: "Codex needs your input".into(),
                    detail: params.to_string(),
                    risk: "unknown".into(),
                    raw_input: None,
                },
                Some(InteractionInput {
                    kind: "questions".into(),
                    questions,
                    schema: None,
                    url: None,
                }),
            );
            let approval = approval.and_then(|request| {
                if register_request(&lifecycle, &rpc_id, &request.id) {
                    let _ = service.expire_approval_request(&request.id);
                    Err("Codex resolved this request upstream.".into())
                } else {
                    service.wait_for_input(&request.id, || !control.is_cancelled())
                }
            });
            match approval {
                Ok(response) => {
                    let response_live = finish_request(&lifecycle, &rpc_id);
                    if response_live && control.app_server_turn_is_current(thread_id, turn_id) {
                        let _ = send(&control, json!({"id":id,"result":response}));
                    }
                }
                Err(error) => {
                    let response_live = finish_request(&lifecycle, &rpc_id);
                    if response_live && control.app_server_turn_is_current(thread_id, turn_id) {
                        let _ = send(
                            &control,
                            json!({"id":id,"error":{"code":-32000,"message":error}}),
                        );
                    }
                }
            }
            return;
        }
        if method == "mcpServer/elicitation/request" {
            let mode = params.get("mode").and_then(Value::as_str).unwrap_or("");
            let interaction = match mode {
                "form" | "openai/form" | "openaiForm" => Some(InteractionInput {
                    kind: "form".into(),
                    questions: vec![],
                    schema: params.get("requestedSchema").cloned(),
                    url: None,
                }),
                "url" => params
                    .get("url")
                    .and_then(Value::as_str)
                    .map(|url| InteractionInput {
                        kind: "url".into(),
                        questions: vec![],
                        schema: None,
                        url: Some(url.into()),
                    }),
                _ => None,
            };
            let Some(interaction) = interaction else {
                let _ = send(
                    &control,
                    json!({"id":id,"result":{"action":"decline","content":null,"_meta":null}}),
                );
                let _ = finish_request(&lifecycle, &rpc_id);
                return;
            };
            let request = service.create_app_server_approval(
                &control,
                turn_id,
                CreateApprovalRequest {
                    task_id: task_id.clone(),
                    provider: "codex".into(),
                    run_id: request_key,
                    tool: "MCP elicitation".into(),
                    summary: params
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("An MCP server needs your input")
                        .into(),
                    detail: params.to_string(),
                    risk: "unknown".into(),
                    raw_input: None,
                },
                Some(interaction),
            );
            let result = match request.and_then(|request| {
                if register_request(&lifecycle, &rpc_id, &request.id) {
                    let _ = service.expire_approval_request(&request.id);
                    Err("Codex resolved this request upstream.".into())
                } else {
                    service.wait_for_input(&request.id, || !control.is_cancelled())
                }
            }) {
                Ok(_response) if mode == "url" => {
                    json!({"action":"accept","content":null,"_meta":params.get("_meta").cloned().unwrap_or(Value::Null)})
                }
                Ok(response) => {
                    json!({"action":"accept","content":response,"_meta":params.get("_meta").cloned().unwrap_or(Value::Null)})
                }
                Err(_) => json!({"action":"decline","content":null,"_meta":null}),
            };
            let response_live = finish_request(&lifecycle, &rpc_id);
            if response_live && control.app_server_turn_is_current(thread_id, turn_id) {
                let _ = send(&control, json!({"id":id,"result":result}));
            }
            return;
        }
        let (tool, summary, detail, risk, raw_input) = match method {
            "item/commandExecution/requestApproval" | "execCommandApproval" => (
                "Command execution".into(),
                params
                    .get("command")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .filter(|text| !text.is_empty())
                    .unwrap_or_else(|| "Codex requests command approval".into()),
                params.to_string(),
                "high".into(),
                params
                    .get("command")
                    .and_then(Value::as_str)
                    .filter(|value| !value.trim().is_empty())
                    .map(|_| params.clone()),
            ),
            "item/fileChange/requestApproval" | "applyPatchApproval" => {
                let item = params
                    .get("itemId")
                    .and_then(Value::as_str)
                    .and_then(|item_id| turn_items.lock().ok()?.get(item_id).cloned());
                (
                    "File change".into(),
                    "Codex requests file-change approval".into(),
                    json!({"request":params,"item":item.clone()}).to_string(),
                    "medium".into(),
                    item.as_ref().and_then(remembered_file_change_item).map(|item| json!({"request":crate::strip_known_approval_envelope(&params),"item":item})),
                )
            }
            "item/permissions/requestApproval" => (
                "Additional permissions".into(),
                "Codex requests additional permissions".into(),
                params.to_string(),
                "high".into(),
                params
                    .get("permissions")
                    .filter(|value| {
                        !value.is_null()
                            && (!value.is_object()
                                || !value.as_object().is_some_and(|object| object.is_empty()))
                    })
                    .map(|_| params.clone()),
            ),
            _ => {
                let _ = send(
                    &control,
                    json!({"id":id,"error":{"code":-32601,"message":"Monitter does not support this interactive Codex request."}}),
                );
                let _ = finish_request(&lifecycle, &rpc_id);
                return;
            }
        };
        let request = match service.create_app_server_approval(
            &control,
            turn_id,
            CreateApprovalRequest {
                task_id: task_id.clone(),
                provider: "codex".into(),
                run_id: request_key,
                tool,
                summary,
                detail,
                risk,
                raw_input,
            },
            None,
        ) {
            Ok(request) => request,
            Err(error) => {
                let _ = send(
                    &control,
                    json!({"id":id,"error":{"code":-32000,"message":error}}),
                );
                let _ = finish_request(&lifecycle, &rpc_id);
                return;
            }
        };
        if register_request(&lifecycle, &rpc_id, &request.id) {
            let _ = service.expire_approval_request(&request.id);
            return;
        }
        let decision = service
            .wait_for_approval(&request.id, || !control.is_cancelled())
            .unwrap_or(ApprovalDecision::Deny);
        if control.is_cancelled() {
            let _ = service.expire_approval_request(&request.id);
        }
        let result = match method {
            "item/commandExecution/requestApproval" | "execCommandApproval" => {
                json!({"decision": if matches!(decision, ApprovalDecision::ApproveOnce | ApprovalDecision::ApproveSession | ApprovalDecision::ApproveAlways) {"accept"} else {"decline"}})
            }
            "item/fileChange/requestApproval" | "applyPatchApproval" => {
                json!({"decision": if matches!(decision, ApprovalDecision::ApproveOnce | ApprovalDecision::ApproveSession | ApprovalDecision::ApproveAlways) {"accept"} else {"decline"}})
            }
            "item/permissions/requestApproval" => {
                json!({"permissions": if matches!(decision, ApprovalDecision::ApproveOnce | ApprovalDecision::ApproveSession | ApprovalDecision::ApproveAlways) {params.get("permissions").cloned().unwrap_or_else(|| json!({}))} else {json!({})}, "scope":"turn"})
            }
            _ => json!({"decision":"decline"}),
        };
        let response_live = finish_request(&lifecycle, &rpc_id);
        if response_live && control.app_server_turn_is_current(thread_id, turn_id) {
            let _ = send(&control, json!({"id":id,"result":result}));
        }
    });
}

fn thread_context_usage_detail(params: &Value, fallback_turn_id: &str) -> String {
    let usage = params.get("tokenUsage");
    json!({
        "providerTurnId": params.get("turnId").and_then(Value::as_str).unwrap_or(fallback_turn_id),
        "usage": {
            "used": usage.and_then(|value| value.pointer("/last/totalTokens")),
            "size": usage.and_then(|value| value.get("modelContextWindow")),
        },
    }).to_string()
}

fn handle_notification(
    service: &Arc<Service>,
    task_id: &str,
    control: &Arc<RunControl>,
    value: &Value,
    started: &mut bool,
    _terminal: &mut bool,
    message_text: &mut HashMap<String, BufferedMessage>,
    turn_items: &Arc<Mutex<HashMap<String, Value>>>,
    seen: &Arc<Mutex<HashSet<String>>>,
    lifecycle: &Arc<Mutex<RequestLifecycle>>,
) {
    let method = value.get("method").and_then(Value::as_str).unwrap_or("");
    let params = value.get("params").unwrap_or(&Value::Null);
    let thread_id = params.get("threadId").and_then(Value::as_str).unwrap_or("");
    let turn_id = params
        .get("turnId")
        .and_then(Value::as_str)
        .or_else(|| params.pointer("/turn/id").and_then(Value::as_str))
        .unwrap_or("");
    if method == "serverRequest/resolved" {
        let Some(request_id) = params.get("requestId") else {
            return;
        };
        if !control.matches_app_server_thread(thread_id) {
            return;
        }
        let rpc_id = request_id.to_string();
        let approval_id = lifecycle.lock().ok().and_then(|mut state| {
            if !state.received.contains(&rpc_id) && !state.approvals.contains_key(&rpc_id) {
                return None;
            }
            state.resolved.insert(rpc_id.clone());
            state.approvals.get(&rpc_id).cloned()
        });
        if let Some(approval_id) = approval_id {
            let _ = service.expire_approval_request(&approval_id);
        }
        return;
    }
    if method == "turn/started" && !thread_id.is_empty() && !turn_id.is_empty() {
        // turn/start's correlated response installs the exact IDs first.
        // A delayed notification from an older turn must never replace them.
        if control.app_server_turn_is_current(thread_id, turn_id) {
            *started = true;
        } else {
            service.record(
                task_id,
                "error",
                "Invalid Codex app-server event",
                "Ignoring turn/started for a turn this run did not start.".into(),
            );
        }
        return;
    }
    if matches!(
        method,
        "item/agentMessage/delta" | "item/completed" | "turn/completed" | "thread/tokenUsage/updated"
    ) && (thread_id.is_empty() || turn_id.is_empty())
    {
        service.record(
            task_id,
            "error",
            "Invalid Codex app-server event",
            "Turn-scoped event omitted thread or turn ID.".into(),
        );
        return;
    }
    if !thread_id.is_empty()
        && !turn_id.is_empty()
        && !control.app_server_turn_is_current(thread_id, turn_id)
    {
        return;
    }
    match method {
        "item/started" => {
            if let Some(item) = params.get("item") {
                if let Some(item_id) = item.get("id").and_then(Value::as_str) {
                    if let Ok(mut items) = turn_items.lock() {
                        if items.len() < 256 {
                            items.insert(item_id.into(), item.clone());
                        }
                    }
                }
                if let Err(error) = service.app_server_event(
                    task_id,
                    control,
                    Some(turn_id),
                    parse_item(item, true),
                ) {
                    service.complete_app_server_turn(
                        task_id,
                        control,
                        Some(turn_id),
                        "error",
                        Some(error),
                    );
                    *_terminal = true;
                }
            }
        }
        "item/agentMessage/delta" => {
            if let (Some(item), Some(delta)) = (
                params.get("itemId").and_then(Value::as_str),
                params.get("delta").and_then(Value::as_str),
            ) {
                let phase = turn_items.lock().ok().and_then(|items| {
                    items
                        .get(item)
                        .and_then(|started| started.get("phase"))
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                });
                let stable = format!("{turn_id}:{item}");
                if !message_text.contains_key(&stable) && message_text.len() >= 256 {
                    service.complete_app_server_turn(
                        task_id,
                        control,
                        Some(turn_id),
                        "error",
                        Some("Codex emitted too many partial message items.".into()),
                    );
                    *_terminal = true;
                    return;
                }
                let buffered =
                    message_text
                        .entry(stable.clone())
                        .or_insert_with(|| BufferedMessage {
                            text: String::new(),
                            last_flush: Instant::now() - DELTA_FLUSH_INTERVAL,
                        });
                if buffered.text.len().saturating_add(delta.len()) > MAX_DELTA_TEXT {
                    service.complete_app_server_turn(
                        task_id,
                        control,
                        Some(turn_id),
                        "error",
                        Some("Codex reply exceeded the 2 MiB safety limit.".into()),
                    );
                    *_terminal = true;
                    return;
                }
                buffered.text.push_str(delta);
                if buffered.last_flush.elapsed() >= DELTA_FLUSH_INTERVAL {
                    if let Err(error) = service.app_server_message(
                        task_id,
                        control,
                        turn_id,
                        &stable,
                        &buffered.text,
                        phase.as_deref(),
                        false,
                    ) {
                        service.complete_app_server_turn(
                            task_id,
                            control,
                            Some(turn_id),
                            "error",
                            Some(error),
                        );
                        *_terminal = true;
                        return;
                    }
                    buffered.last_flush = Instant::now();
                }
            }
        }
        "item/completed" => {
            if let Some(item) = params.get("item") {
                if item.get("type").and_then(Value::as_str) == Some("agentMessage") {
                    if let (Some(item_id), Some(text)) = (
                        item.get("id").and_then(Value::as_str),
                        item.get("text").and_then(Value::as_str),
                    ) {
                        let stable = format!("{turn_id}:{item_id}");
                        message_text.insert(
                            stable.clone(),
                            BufferedMessage {
                                text: text.into(),
                                last_flush: Instant::now(),
                            },
                        );
                        if let Err(error) = service
                            .app_server_message(
                                task_id,
                                control,
                                turn_id,
                                &stable,
                                text,
                                item.get("phase").and_then(Value::as_str),
                                true,
                            )
                        {
                            service.complete_app_server_turn(
                                task_id,
                                control,
                                Some(turn_id),
                                "error",
                                Some(error),
                            );
                            *_terminal = true;
                            return;
                        }
                    }
                } else {
                    if let Some(image) = mcp_image(item) {
                        if let Err(error) = service.app_server_event(
                            task_id,
                            control,
                            Some(turn_id),
                            Parsed {
                                native_session_id: None,
                                assistant: None,
                                event: Some((
                                    "computer_image".into(),
                                    "Computer screenshot".into(),
                                    image.into(),
                                )),
                                failed: false,
                            },
                        ) {
                            service.complete_app_server_turn(
                                task_id,
                                control,
                                Some(turn_id),
                                "error",
                                Some(error),
                            );
                            *_terminal = true;
                            return;
                        }
                    }
                    let parsed = parse_item(item, false);
                    if let Err(error) =
                        service.app_server_event(task_id, control, Some(turn_id), parsed)
                    {
                        service.complete_app_server_turn(
                            task_id,
                            control,
                            Some(turn_id),
                            "error",
                            Some(error),
                        );
                        *_terminal = true;
                        return;
                    }
                }
            }
        }
        "thread/tokenUsage/updated" => {
            // Codex reports the live context window separately from turn
            // completion. `last.totalTokens` is the current context used;
            // `total` is cumulative session accounting and must not become a
            // context reading. The shared usage ledger rejects the sample
            // when either reported value is absent or invalid.
            let detail = thread_context_usage_detail(params, turn_id);
            if let Err(error) = service.app_server_event(
                task_id,
                control,
                Some(turn_id),
                Parsed {
                    native_session_id: None,
                    assistant: None,
                    event: Some(("usage".into(), "Context usage updated".into(), detail)),
                    failed: false,
                },
            ) {
                service.record(task_id, "error", "Usage capture warning", error);
            }
        }
        "turn/completed" => {
            // Revoke response rights before durable completion wakes waiters.
            // This ordering prevents an error result racing upstream's own
            // terminal notification.
            if let Ok(mut state) = lifecycle.lock() {
                let rpc_ids = state.received.iter().cloned().collect::<Vec<_>>();
                let mapped = state.approvals.keys().cloned().collect::<HashSet<_>>();
                state.resolved.retain(|rpc_id| mapped.contains(rpc_id));
                state.resolved.extend(rpc_ids);
                state.received.clear();
            }
            let status = params
                .pointer("/turn/status")
                .and_then(Value::as_str)
                .unwrap_or("failed");
            let error = params
                .pointer("/turn/error/message")
                .and_then(Value::as_str)
                .map(str::to_owned);
            let mapped = if status == "completed" {
                "completed"
            } else if status == "interrupted" {
                "interrupted"
            } else {
                "error"
            };
            // Codex 0.154 places turn accounting on the completion params as
            // `usage`. Keep the native turn ID for cumulative-snapshot
            // replacement and emit the same normalized detail to the timeline.
            if let Some(usage) = params.get("usage").or_else(|| params.pointer("/turn/usage")) {
                let detail = json!({"providerTurnId": turn_id, "usage": usage}).to_string();
                if let Err(error) = service.app_server_event(task_id, control, Some(turn_id), Parsed {
                    native_session_id: None, assistant: None,
                    event: Some(("usage".into(), "Usage updated".into(), detail)), failed: false,
                }) { service.record(task_id, "error", "Usage capture warning", error); }
            }
            service.complete_app_server_turn(task_id, control, Some(turn_id), mapped, error);
            let prefix = format!("{turn_id}:");
            message_text.retain(|key, _| !key.starts_with(&prefix));
            if let Ok(mut items) = turn_items.lock() {
                items.clear();
            }
            if let Ok(mut requests) = seen.lock() {
                let marker = format!(":{turn_id}:");
                requests.retain(|key| !key.contains(&marker));
            }
        }
        "error" => {
            service.record(task_id, "error", "Codex app-server", params.to_string());
        }
        _ => {}
    }
}

/// Returns true when app-server had already resolved the request before the
/// durable approval mapping became visible.
fn register_request(lifecycle: &Mutex<RequestLifecycle>, rpc_id: &str, approval_id: &str) -> bool {
    lifecycle
        .lock()
        .map(|mut state| {
            state.approvals.insert(rpc_id.into(), approval_id.into());
            state.resolved.contains(rpc_id)
        })
        .unwrap_or(true)
}

fn accept_received_request(lifecycle: &Mutex<RequestLifecycle>, rpc_id: &str) -> bool {
    lifecycle
        .lock()
        .map(|mut state| {
            if state.received.len() >= MAX_SERVER_REQUESTS
                || state.received.contains(rpc_id)
                || state.approvals.contains_key(rpc_id)
            {
                false
            } else {
                state.received.insert(rpc_id.into());
                true
            }
        })
        .unwrap_or(false)
}

/// Atomically consumes request state. False suppresses a late response after
/// serverRequest/resolved or turn completion won the race.
fn finish_request(lifecycle: &Mutex<RequestLifecycle>, rpc_id: &str) -> bool {
    lifecycle
        .lock()
        .map(|mut state| {
            state.approvals.remove(rpc_id);
            state.received.remove(rpc_id);
            !state.resolved.remove(rpc_id)
        })
        .unwrap_or(false)
}

fn parse_item(item: &Value, started: bool) -> Parsed {
    let ty = item.get("type").and_then(Value::as_str).unwrap_or("");
    // Conversation lifecycle envelopes are transport metadata, not tool work.
    // Their authoritative content is persisted through app_server_message (or
    // already exists as the user-authored transcript entry), so recording the
    // empty started envelope only leaves a misleading Timeline row behind.
    if ty.eq_ignore_ascii_case("userMessage") || ty.eq_ignore_ascii_case("agentMessage") {
        return Parsed {
            native_session_id: None,
            assistant: None,
            event: None,
            failed: false,
        };
    }
    if ty == "mcpToolCall" {
        let server = item.get("server").and_then(Value::as_str).unwrap_or("");
        let tool = item
            .get("tool")
            .and_then(Value::as_str)
            .unwrap_or("MCP tool");
        if server.contains("computer")
            || server.contains("cua")
            || tool.contains("computer")
            || tool == "js"
        {
            return Parsed {
                native_session_id: None,
                assistant: None,
                event: Some((
                    "computer".into(),
                    "Computer activity".into(),
                    json!({"id":item.get("id"),"phase":if started {"started"} else {"completed"},"tool":tool,"summary":item.get("arguments").or_else(|| item.get("error"))}).to_string(),
                )),
                failed: false,
            };
        }
    }
    let title = item
        .get("tool")
        .or_else(|| item.get("command"))
        .and_then(Value::as_str)
        .unwrap_or(ty)
        .to_owned();
    // Context compaction arrives as separate lifecycle items. Preserve that
    // phase so the UI never has to infer success from a task becoming idle.
    let detail = if ty.eq_ignore_ascii_case("ContextCompaction") {
        let mut detail = item.clone();
        if let Some(object) = detail.as_object_mut() {
            object.insert(
                "monitterPhase".into(),
                Value::String(if started { "started" } else { "completed" }.into()),
            );
        }
        detail.to_string()
    } else {
        item.to_string()
    };
    Parsed {
        native_session_id: None,
        assistant: None,
        event: Some((
            if ty == "reasoning" {
                "reasoning"
            } else {
                "tool"
            }
            .into(),
            title,
            detail,
        )),
        failed: false,
    }
}

fn mcp_image(item: &Value) -> Option<&str> {
    (item.get("type").and_then(Value::as_str) == Some("mcpToolCall"))
        .then(|| item.pointer("/result/content").and_then(Value::as_array))
        .flatten()?
        .iter()
        .find_map(|entry| {
            (entry.get("type").and_then(Value::as_str) == Some("image"))
                .then(|| entry.get("data").and_then(Value::as_str))
                .flatten()
        })
}

fn send(control: &RunControl, value: Value) -> Result<(), String> {
    control.send_control(&value.to_string())
}
fn initialize() -> Value {
    json!({"id":INITIALIZE_ID,"method":"initialize","params":{"clientInfo":{"name":"Monitter","version":"0.1"},"capabilities":{"experimentalApi":false,"requestAttestation":false,"mcpServerOpenaiFormElicitation":true,"extensions":{"openai/form":{}}}}})
}
fn turn_request(thread_id: &str, prompt: &str, task: &Task) -> Value {
    let mut params = json!({
        "threadId":thread_id,
        "input":[{"type":"text","text":prompt,"text_elements":[]}],
        "cwd":task.cwd,
        "approvalPolicy":if task.sandbox == "yolo" {"never"} else {"on-request"},
        "sandboxPolicy":match task.sandbox.as_str() {
            "workspace-write" => json!({"type":"workspaceWrite","writableRoots":[task.cwd],"networkAccess":false,"excludeTmpdirEnvVar":false,"excludeSlashTmp":false}),
            "yolo" => json!({"type":"dangerFullAccess"}),
            _ => json!({"type":"readOnly","networkAccess":false}),
        },
        "model":if task.model.trim().is_empty(){Value::Null}else{Value::String(task.model.clone())}
    });
    if let Some(settings) = &task.model_settings {
        if let Some(effort) = &settings.reasoning_effort {
            params["effort"] = Value::String(effort.clone());
        }
        if let Some(fast) = settings.fast_mode {
            params["serviceTierForTurn"] =
                Value::String(if fast { "priority" } else { "default" }.into());
        }
    }
    json!({"id":FIRST_TURN_ID,"method":"turn/start","params":params})
}
fn thread_request(
    task: &Task,
    helper: Option<&str>,
    has_grant: bool,
    extensions: &crate::extensions_runtime::RuntimeExtensions,
) -> Value {
    let sandbox = if task.sandbox == "yolo" {
        "danger-full-access"
    } else {
        task.sandbox.as_str()
    };
    let mut params = json!({"cwd":task.cwd,"model":if task.model.trim().is_empty(){Value::Null}else{Value::String(task.model.clone())},"sandbox":sandbox});
    // Do not inherit an ambient approval default.  Yolo is the explicit
    // per-task bypass choice; every other Monitter Codex task asks through
    // this app-server connection so its decision is durable and correlated.
    params["approvalPolicy"] = Value::String(
        if task.sandbox == "yolo" {
            "never"
        } else {
            "on-request"
        }
        .into(),
    );
    if let Some(settings) = &task.model_settings {
        if let Some(fast) = settings.fast_mode {
            params["serviceTier"] = Value::String(if fast { "priority" } else { "default" }.into());
        }
    }
    let mut config = extensions.codex_config();
    if has_grant {
        if let Some(helper) = helper {
            config.extend(json!({"mcp_servers.monitter.command":"python3","mcp_servers.monitter.args":[helper],"mcp_servers.monitter.env_vars":["MONITTER_ENDPOINT","MONITTER_TOKEN"],"mcp_servers.monitter.required":true,"mcp_servers.monitter.enabled_tools":["list_agents","delegate_task","send_message","get_task_result","wait_for_task","list_messages","cancel_delegation"]}).as_object().cloned().unwrap_or_default());
        }
    }
    if !config.is_empty() {
        params["config"] = Value::Object(config);
    }
    if let Some(id) = task
        .native_session_id
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        let mut resume = params.as_object().cloned().unwrap_or_default();
        resume.insert("threadId".into(), Value::String(id.into()));
        // The desktop transcript is Monitter's durable UI projection. Avoid
        // receiving an unbounded native history during resume; new events are
        // still correlated to this owned connection and turn.
        resume.insert("excludeTurns".into(), Value::Bool(true));
        json!({"id":THREAD_ID,"method":"thread/resume","params":Value::Object(resume)})
    } else {
        json!({"id":THREAD_ID,"method":"thread/start","params":params})
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn bounded_line_rejects_before_accumulating_following_chunks() {
        let input = Cursor::new(vec![b'x'; 65]);
        let mut reader = BufReader::with_capacity(8, input);
        assert!(read_bounded_line(&mut reader, 32)
            .unwrap_err()
            .contains("larger than 32 bytes"));
    }

    #[test]
    fn bounded_line_accepts_eof_terminated_json() {
        let mut reader = BufReader::new(Cursor::new(br#"{"id":1}"#.to_vec()));
        assert_eq!(
            read_bounded_line(&mut reader, 32).unwrap().as_deref(),
            Some(r#"{"id":1}"#)
        );
        assert!(read_bounded_line(&mut reader, 32).unwrap().is_none());
    }

    #[test]
    fn resolved_request_suppresses_late_response() {
        let lifecycle = Mutex::new(RequestLifecycle::default());
        assert!(accept_received_request(&lifecycle, "7"));
        assert!(!accept_received_request(&lifecycle, "7"));
        assert!(!register_request(&lifecycle, "7", "approval-7"));
        lifecycle.lock().unwrap().resolved.insert("7".into());
        assert!(!finish_request(&lifecycle, "7"));
        assert!(lifecycle.lock().unwrap().approvals.is_empty());
    }

    #[test]
    fn completed_turn_cannot_send_a_waiter_response() {
        let control = RunControl::new(false);
        control.set_app_server_thread("thread-1".into());
        control.set_app_server_turn("turn-1".into());
        let lifecycle = Mutex::new(RequestLifecycle::default());
        assert!(accept_received_request(&lifecycle, "9"));
        assert!(!register_request(&lifecycle, "9", "approval-9"));
        control.clear_app_server_turn();
        let response_live = finish_request(&lifecycle, "9");
        assert!(response_live);
        assert!(!control.app_server_turn_is_current("thread-1", "turn-1"));
    }

    #[test]
    fn initialize_explicitly_declines_experimental_requests() {
        let value = initialize();
        assert_eq!(
            value.pointer("/params/capabilities/experimentalApi"),
            Some(&Value::Bool(false))
        );
        assert_eq!(
            value.pointer("/params/capabilities/requestAttestation"),
            Some(&Value::Bool(false))
        );
    }

    #[test]
    fn stage_clock_uses_the_bound_for_its_request_stage() {
        let now = Instant::now();
        let initialize = StageClock {
            stage: RequestStage::Initialize,
            started_at: now,
        };
        let thread = StageClock {
            stage: RequestStage::Thread,
            started_at: now,
        };
        assert_eq!(initialize.deadline(), now + INITIALIZE_TIMEOUT);
        assert_eq!(thread.deadline(), now + THREAD_TIMEOUT);
        assert_eq!(RequestStage::InitialTurn.timeout(), TURN_START_TIMEOUT);
        assert_eq!(RequestStage::SubsequentTurn.timeout(), TURN_START_TIMEOUT);
    }

    #[test]
    fn stage_clock_expires_only_at_its_own_deadline() {
        let now = Instant::now();
        let clock = StageClock {
            stage: RequestStage::InitialTurn,
            started_at: now,
        };
        assert!(!clock.expired_at(now + TURN_START_TIMEOUT - Duration::from_millis(1)));
        assert!(clock.expired_at(now + TURN_START_TIMEOUT));
    }

    #[test]
    fn notifications_do_not_extend_a_stage_clock() {
        let now = Instant::now();
        let clock = StageClock {
            stage: RequestStage::Thread,
            started_at: now,
        };
        let deadline_before_notification = clock.deadline();
        // Receiving a notification intentionally does not construct a new
        // clock; only a correlated response advances the request stage.
        let deadline_after_notification = clock.deadline();
        assert_eq!(deadline_after_notification, deadline_before_notification);
        assert!(clock.expired_at(deadline_before_notification));
    }

    #[test]
    fn expired_stage_stays_expired_when_notifications_arrive() {
        let now = Instant::now();
        let clock = StageClock {
            stage: RequestStage::Thread,
            started_at: now,
        };
        let notification_arrived_at = clock.deadline() + Duration::from_millis(1);
        assert!(clock.expired_at(notification_arrived_at));
        assert_eq!(
            clock.stage.timeout_error(),
            "Codex app-server thread start/resume request timed out."
        );
    }

    #[test]
    fn resume_request_excludes_stored_turns() {
        let task: Task = serde_json::from_value(json!({
            "id":"task", "agentId":"agent", "title":"task",
            "nativeSessionId":"thread", "status":"idle", "archived":false,
            "createdAt":0, "updatedAt":0, "parentTaskId":null, "channelId":null,
            "hostId":"host", "cwd":"/tmp", "provider":"codex", "model":"",
            "modelSettings":null, "sandbox":"read-only", "projectId":null
        }))
        .unwrap();
        let request = thread_request(
            &task,
            None,
            false,
            &crate::extensions_runtime::RuntimeExtensions::default(),
        );
        assert_eq!(request["method"], "thread/resume");
        assert_eq!(request["params"]["excludeTurns"], Value::Bool(true));
    }

    #[test]
    fn observes_consecutive_immediate_resident_turn_responses() {
        let control = RunControl::new(false);
        let mut stage_clock = None;

        control.mark_app_server_turn_request(10).unwrap();
        // This models a response waking the reader before its next loop-top
        // observation of the just-sent resident turn.
        observe_subsequent_turn_clock(true, &control, &mut stage_clock);
        assert_eq!(
            stage_clock.map(|clock| clock.stage),
            Some(RequestStage::SubsequentTurn)
        );
        assert!(control.take_app_server_turn_request(10));
        stage_clock = None;

        control.mark_app_server_turn_request(11).unwrap();
        observe_subsequent_turn_clock(true, &control, &mut stage_clock);
        assert_eq!(
            stage_clock.map(|clock| clock.stage),
            Some(RequestStage::SubsequentTurn)
        );
        assert!(control.take_app_server_turn_request(11));
    }

    #[test]
    fn context_compaction_keeps_its_native_id_and_lifecycle_phase() {
        let item = json!({"type":"ContextCompaction","id":"compact-1"});
        for (started, phase) in [(true, "started"), (false, "completed")] {
            let parsed = parse_item(&item, started);
            let event = parsed.event.expect("compaction is activity");
            assert_eq!(event.0, "tool");
            assert_eq!(event.1, "ContextCompaction");
            let detail: Value = serde_json::from_str(&event.2).expect("JSON detail");
            assert_eq!(detail["id"], "compact-1");
            assert_eq!(detail["monitterPhase"], phase);
        }
    }

    #[test]
    fn thread_token_usage_uses_last_total_for_current_context() {
        let detail: Value = serde_json::from_str(&thread_context_usage_detail(
            &json!({
                "turnId": "turn-1",
                "tokenUsage": {
                    "last": { "totalTokens": 12_345 },
                    "total": { "totalTokens": 98_765 },
                    "modelContextWindow": 114_688,
                },
            }),
            "fallback-turn",
        )).unwrap();
        assert_eq!(detail["providerTurnId"], "turn-1");
        assert_eq!(detail.pointer("/usage/used"), Some(&json!(12_345)));
        assert_eq!(detail.pointer("/usage/size"), Some(&json!(114_688)));
    }

    #[test]
    fn conversation_lifecycle_items_do_not_become_timeline_events() {
        for item_type in ["userMessage", "agentMessage"] {
            for started in [true, false] {
                let parsed = parse_item(
                    &json!({"type":item_type,"id":"message-1","text":""}),
                    started,
                );
                assert!(
                    parsed.event.is_none(),
                    "{item_type} is transcript transport"
                );
            }
        }
    }

    #[test]
    fn remembered_file_change_requires_content_and_keeps_change_ids() {
        assert!(
            remembered_file_change_item(&json!({"id":"envelope","status":"pending"})).is_none()
        );
        let item = remembered_file_change_item(&json!({
            "id":"envelope", "status":"pending", "changes":[{"path":"a.rs","toolArgumentId":"semantic-id"}]
        })).unwrap();
        assert!(item.get("id").is_none());
        assert!(item.get("status").is_none());
        assert_eq!(item["changes"][0]["toolArgumentId"], "semantic-id");
    }
}
