//! Resident Codex app-server adapter for local and SSH hosts.
//!
//! App-server is JSON-RPC over the owned child stdio pipes.  It deliberately
//! never exposes a listener and never substitutes `exec resume` when a live
//! transport is lost: doing so could replay an uncertain user turn.

use crate::{
    model::{InputOption, InputQuestion, InteractionInput, Task},
    runner::{parse_codex_subagent_updates, RunControl, SpawnedAppServer},
    ApprovalDecision, CreateApprovalRequest, Parsed, Service,
};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    io::{BufRead, BufReader},
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
const MCP_STARTUP_TIMEOUT: Duration = Duration::from_secs(30);
const DELTA_FLUSH_INTERVAL: Duration = Duration::from_millis(100);

// Enumerate the actual thread inventory, including home/plugin MCP servers.
// Querying happens on a worker; the sole stdout reader must stay responsive.
fn mcp_inventory_ready(
    mut query: impl FnMut(Value) -> Result<Value, String>,
    thread_id: &str,
    updates: &HashMap<String, String>,
) -> Result<bool, String> {
    let mut cursor: Option<String> = None;
    let mut cursors = HashSet::new();
    let mut ready = true;
    for _ in 0..64 {
        let page =
            query(json!({"threadId":thread_id,"detail":"full","limit":100,"cursor":cursor}))?;
        let rows = page
            .get("data")
            .and_then(Value::as_array)
            .ok_or("MCP startup inventory unavailable")?;
        for row in rows {
            let name = row
                .get("name")
                .and_then(Value::as_str)
                .ok_or("MCP inventory omitted a server name")?;
            let status = row
                .get("runtimeStatus")
                .and_then(Value::as_str)
                .or_else(|| updates.get(name).map(String::as_str));
            match status {
                Some(
                    "ready"
                    | "connected"
                    | "failed"
                    | "cancelled"
                    | "disabled"
                    | "authenticationRequired",
                ) => {}
                Some("starting" | "notStarted") => ready = false,
                _ => return Err("MCP startup readiness is not supported by this provider".into()),
            }
        }
        match page
            .get("nextCursor")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
        {
            Some(next) if cursors.insert(next.to_owned()) => cursor = Some(next.to_owned()),
            Some(_) => return Err("MCP startup inventory repeated its cursor".into()),
            None => return Ok(ready),
        }
    }
    Err("MCP startup inventory exceeded the bounded page limit".into())
}

fn record_mcp_startup_update(
    value: &Value,
    thread_id: Option<&str>,
    updates: &mut HashMap<String, String>,
) {
    let Some(thread_id) = thread_id else {
        return;
    };
    let params = &value["params"];
    if params
        .get("threadId")
        .and_then(Value::as_str)
        .is_some_and(|id| id != thread_id)
    {
        return;
    }
    let (Some(name), Some(status)) = (params["name"].as_str(), params["status"].as_str()) else {
        return;
    };
    if matches!(status, "starting" | "ready" | "failed" | "cancelled")
        && (updates.len() < 4096 || updates.contains_key(name))
    {
        updates.insert(name.into(), status.into());
    }
}

fn wait_for_mcp_startup(
    control: &RunControl,
    thread_id: &str,
    updates: &Mutex<HashMap<String, String>>,
    timeout: Duration,
) -> bool {
    let deadline = Instant::now() + timeout;
    while !control.is_cancelled() && !control.is_planned_retirement() {
        if Instant::now() >= deadline {
            return false;
        }
        let snapshot = match updates.lock() {
            Ok(value) => value.clone(),
            Err(_) => return false,
        };
        let result = mcp_inventory_ready(
            |params| {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Err("MCP startup timed out".into());
                }
                control.query_app_server(
                    "mcpServerStatus/list",
                    params,
                    remaining.min(Duration::from_secs(2)),
                )
            },
            thread_id,
            &snapshot,
        );
        match result {
            Ok(true) => return Instant::now() < deadline,
            Err(_) => return false,
            Ok(false) => {}
        }
        thread::sleep(
            deadline
                .saturating_duration_since(Instant::now())
                .min(Duration::from_millis(100)),
        );
    }
    false
}

fn take_mcp_startup_result(pending: &mut Option<mpsc::Receiver<bool>>) -> Option<bool> {
    match pending.as_ref()?.try_recv() {
        Ok(ready) => {
            pending.take();
            Some(ready)
        }
        Err(mpsc::TryRecvError::Disconnected) => {
            pending.take();
            Some(false)
        }
        Err(mpsc::TryRecvError::Empty) => None,
    }
}

// This trace is deliberately separate from provider stderr diagnostics and
// transcript events. It contains only fixed protocol labels and booleans.
#[cfg(feature = "experimental-gpui")]
type ProtocolTraceState = Option<ProtocolTrace>;
#[cfg(not(feature = "experimental-gpui"))]
type ProtocolTraceState = ();

#[cfg(feature = "experimental-gpui")]
struct ProtocolTrace {
    started_at: Instant,
}

#[cfg(feature = "experimental-gpui")]
fn protocol_trace_state() -> ProtocolTraceState {
    (std::env::var_os("MONITTER_EXPERIMENTAL_APP_SERVER_TRACE").as_deref()
        == Some(std::ffi::OsStr::new("1")))
    .then(|| ProtocolTrace {
        started_at: Instant::now(),
    })
}

#[cfg(not(feature = "experimental-gpui"))]
fn protocol_trace_state() -> ProtocolTraceState {}

fn protocol_trace(
    trace: &ProtocolTraceState,
    method: &'static str,
    stage: &'static str,
    milestone: &'static str,
    request_correlated: Option<bool>,
    scope_matched: Option<bool>,
) {
    #[cfg(feature = "experimental-gpui")]
    if let Some(trace) = trace {
        let yes_no = |value| match value {
            Some(true) => "yes",
            Some(false) => "no",
            None => "n/a",
        };
        eprintln!(
            "[gpui-app-server-trace] method={method} stage={stage} milestone={milestone} elapsed_ms={} request_correlated={} scope_matched={}",
            trace.started_at.elapsed().as_millis(),
            yes_no(request_correlated),
            yes_no(scope_matched),
        );
    }
    #[cfg(not(feature = "experimental-gpui"))]
    let _ = (
        trace,
        method,
        stage,
        milestone,
        request_correlated,
        scope_matched,
    );
}

fn traceable_notification_method(method: &str) -> Option<&'static str> {
    match method {
        "turn/started" => Some("turn/started"),
        "turn/completed" => Some("turn/completed"),
        "item/started" => Some("item/started"),
        "item/completed" => Some("item/completed"),
        "item/agentMessage/delta" => Some("item/agentMessage/delta"),
        "thread/tokenUsage/updated" => Some("thread/tokenUsage/updated"),
        _ => None,
    }
}

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
    trace: &ProtocolTraceState,
) {
    if started && stage_clock.is_none() {
        let ordinary_turn_pending = control.has_app_server_turn_request();
        if !ordinary_turn_pending && !control.has_app_server_native_turn_request() {
            return;
        }
        *stage_clock = Some(StageClock::start(RequestStage::SubsequentTurn));
        if ordinary_turn_pending {
            protocol_trace(
                trace,
                "turn/start",
                "subsequent_turn",
                "request_observed",
                None,
                None,
            );
        }
    }
}

struct BufferedMessage {
    text: String,
    last_flush: Instant,
    flushed_len: usize,
}

/// Persist every dirty assistant item for this turn before a server request can
/// block the reader. A Stop fences later service mutations, so waiting for the
/// next delta (the normal 100 ms coalescing path) could otherwise discard a
/// final buffered chunk while an approval is visible.
fn flush_buffered_messages(
    service: &Arc<Service>,
    task_id: &str,
    control: &Arc<RunControl>,
    turn_id: &str,
    message_text: &mut HashMap<String, BufferedMessage>,
    turn_items: &Arc<Mutex<HashMap<String, Value>>>,
    complete: bool,
) -> Result<(), String> {
    let prefix = format!("{turn_id}:");
    for (stable, buffered) in message_text.iter_mut() {
        if !stable.starts_with(&prefix)
            || (!complete && buffered.flushed_len == buffered.text.len())
        {
            continue;
        }
        let item_id = stable
            .strip_prefix(&prefix)
            .ok_or("Buffered Codex message has an invalid item key.")?;
        let phase = turn_items.lock().ok().and_then(|items| {
            items
                .get(item_id)
                .and_then(|started| started.get("phase"))
                .and_then(Value::as_str)
                .map(str::to_owned)
        });
        service.app_server_message(
            task_id,
            control,
            turn_id,
            stable,
            &buffered.text,
            phase.as_deref(),
            complete,
            None,
        )?;
        buffered.last_flush = Instant::now();
        buffered.flushed_len = buffered.text.len();
    }
    Ok(())
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
    let trace = protocol_trace_state();
    protocol_trace(&trace, "transport", "run", "started", None, None);
    let _owned = OwnedRun {
        service: service.clone(),
        task_id: task_id.clone(),
        control: control.clone(),
    };
    let (mut task, host) = match service.task_and_host(&task_id) {
        Ok(value) => value,
        Err(error) => {
            protocol_trace(&trace, "transport", "prepare", "failure", None, None);
            service.complete_app_server_turn(&task_id, &control, None, "error", Some(error));
            return;
        }
    };
    let mut extensions = match service.runtime_extensions_for_task(&task_id) {
        Ok(extensions) => extensions,
        Err(error) => {
            protocol_trace(&trace, "transport", "prepare", "failure", None, None);
            service.complete_app_server_turn(&task_id, &control, None, "error", Some(error));
            return;
        }
    };
    if let Err(error) = extensions.validate_for("codex", &host.kind) {
        protocol_trace(&trace, "transport", "prepare", "failure", None, None);
        service.complete_app_server_turn(&task_id, &control, None, "error", Some(error));
        return;
    }
    control.set_mcp_fingerprint(extensions.mcp_fingerprint());
    control.set_runtime_infrastructure_launches(extensions.stdio_launches());
    let grant = match service.collaboration_grant(&task_id) {
        Ok(value) => value,
        Err(error) => {
            protocol_trace(&trace, "transport", "prepare", "failure", None, None);
            service.complete_app_server_turn(&task_id, &control, None, "error", Some(error));
            return;
        }
    };
    let jev_decisions_grant = match service.jev_decisions_grant(&task_id) {
        Ok(value) => value,
        Err(error) => {
            protocol_trace(&trace, "transport", "prepare", "failure", None, None);
            service.complete_app_server_turn(&task_id, &control, None, "error", Some(error));
            return;
        }
    };
    if host.kind == "ssh" && jev_decisions_grant.is_some() {
        protocol_trace(&trace, "transport", "prepare", "unsupported", None, None);
        service.complete_app_server_turn(
            &task_id,
            &control,
            None,
            "error",
            Some("Jev Decisions MCP is not supported for SSH Codex sessions until a dedicated secure tunnel is available.".into()),
        );
        return;
    }
    let prompt = extensions.prompt(&prompt);
    let prompt = if jev_decisions_grant.is_some() {
        crate::runner::with_jev_decisions_guidance(&prompt)
    } else {
        prompt
    };
    let SpawnedAppServer {
        mut child,
        collaboration_endpoint,
        jev_decisions_endpoint,
        stderr,
        cwd,
        startup_diagnostics,
    } = match crate::runner::spawn_codex_app_server_with_jev_decisions(
        &host,
        &task,
        grant
            .as_ref()
            .map(|grant| (grant.endpoint.as_str(), grant.token.as_str())),
        jev_decisions_grant
            .as_ref()
            .map(|grant| (grant.endpoint.as_str(), grant.token.as_str())),
        &control,
        service.environment_secrets_for_local_user_task(&task_id, &host),
    ) {
        Ok(transport) => transport,
        Err(error) => {
            protocol_trace(&trace, "transport", "launch", "failure", None, None);
            service.complete_app_server_turn(&task_id, &control, None, "error", Some(error));
            return;
        }
    };
    // Use the remote process's expanded folder in protocol parameters only.
    // Attachment ownership and the saved task snapshot keep their original cwd.
    task.cwd = cwd;
    let Some(stdin) = child.stdin.take() else {
        protocol_trace(&trace, "transport", "stdin", "failure", None, None);
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
        protocol_trace(&trace, "transport", "stdout", "failure", None, None);
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
    if let Err(error) = control.install(child, Some(stdin)) {
        protocol_trace(&trace, "transport", "install", "failure", None, None);
        service.complete_app_server_turn(&task_id, &control, None, "error", Some(error));
        return;
    }
    control.mark_resident();
    if let Some(stderr) = stderr {
        let service = service.clone();
        let task = task_id.clone();
        let token = grant.as_ref().map(|grant| grant.token.clone());
        thread::spawn(move || {
            let mut emitted = false;
            let mut emit = |line: &str| {
                if !emitted {
                    if let Some(line) = crate::runner::provider_stderr_diagnostic(line) {
                        emitted = true;
                        let line = token
                            .as_deref()
                            .filter(|token| !token.is_empty())
                            .map_or_else(
                                || line.clone(),
                                |token| line.replace(token, "[redacted]"),
                            );
                        // The internal admin has no durable transcript or
                        // activity surface. Keep provider diagnostics inside
                        // its process-local caller path rather than leaking
                        // an invisible mini-turn into the task timeline.
                        if !service.is_internal_admin_task(&task) {
                            service.record(&task, "error", "Codex app-server diagnostic", line);
                        }
                    }
                }
            };
            for line in startup_diagnostics.lines() {
                emit(line);
            }
            let mut reader = BufReader::new(stderr);
            while let Ok(Some(line)) = read_bounded_line(&mut reader, 64 * 1024) {
                emit(&line);
            }
        });
    }
    let mut startup_pending: Option<mpsc::Receiver<bool>> = None;
    let mut startup_thread_id: Option<String> = None;
    let startup_updates = Arc::new(Mutex::new(HashMap::<String, String>::new()));
    let mut stage_clock = Some(StageClock::start(RequestStage::Initialize));
    protocol_trace(&trace, "initialize", "initialize", "request", None, None);
    if let Err(error) = send(&control, initialize()) {
        protocol_trace(&trace, "initialize", "initialize", "failure", None, None);
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
        if control.is_planned_retirement() {
            protocol_trace(&trace, "transport", "run", "retired", None, None);
            terminal = true;
            break;
        }
        if control.is_cancelled() {
            protocol_trace(&trace, "transport", "run", "cancelled", None, None);
            terminal = true;
            break;
        }
        if let Some(ready) = take_mcp_startup_result(&mut startup_pending) {
            let Some(thread_id) = startup_thread_id.take() else {
                break;
            };
            let owner_current = service
                .runs
                .lock()
                .map(|runs| {
                    runs.tasks
                        .get(&task_id)
                        .is_some_and(|owner| Arc::ptr_eq(owner, &control))
                })
                .unwrap_or(false);
            if !owner_current
                || control.is_cancelled()
                || control.is_planned_retirement()
                || control.current_app_server_thread().as_deref() != Some(thread_id.as_str())
            {
                control.disable_runtime_process_baseline();
                terminal = true;
                break;
            }
            if !ready || control.capture_runtime_process_baseline().is_err() {
                control.disable_runtime_process_baseline();
            }
            protocol_trace(&trace, "turn/start", "initial_turn", "request", None, None);
            if let Err(error) = control
                .mark_app_server_turn_request(FIRST_TURN_ID)
                .and_then(|_| send(&control, turn_request(&thread_id, &prompt, &task)))
            {
                service.complete_app_server_turn(&task_id, &control, None, "error", Some(error));
                terminal = true;
                break;
            }
            stage_clock = Some(StageClock::start(RequestStage::InitialTurn));
        }
        // `recv_timeout` returns immediately while the pipe has queued
        // notifications. Check absolute expiry before accepting another frame
        // so notification noise cannot starve a request-stage timeout.
        if stage_clock.is_some_and(|clock| clock.expired_at(Instant::now())) {
            let stage = stage_clock
                .map(|clock| match clock.stage {
                    RequestStage::Initialize => "initialize",
                    RequestStage::Thread => "thread",
                    RequestStage::InitialTurn => "initial_turn",
                    RequestStage::SubsequentTurn => "subsequent_turn",
                })
                .unwrap_or("request");
            protocol_trace(&trace, "request", stage, "timeout", None, None);
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
        observe_subsequent_turn_clock(started, &control, &mut stage_clock, &trace);
        let deadline = stage_clock.map(StageClock::deadline);
        let poll = deadline
            .map(|deadline| deadline.saturating_duration_since(Instant::now()))
            .unwrap_or(Duration::from_millis(250))
            .min(Duration::from_millis(250));
        let received = match output_rx.recv_timeout(poll) {
            Ok(value) => Ok(value),
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                protocol_trace(&trace, "transport", "stdout", "eof", None, None);
                Err("Codex app-server output closed.".into())
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if stage_clock.is_some_and(|clock| clock.expired_at(Instant::now())) {
                    protocol_trace(&trace, "request", "active", "timeout", None, None);
                    Err(stage_clock
                        .map(|clock| clock.stage.timeout_error())
                        .unwrap_or("Codex app-server request timed out.")
                        .into())
                } else {
                    continue;
                }
            }
        };
        let Some(_event) = control.begin_event_processing() else {
            protocol_trace(&trace, "transport", "run", "cancelled", None, None);
            terminal = true;
            break;
        };
        let line = match received {
            Ok(Ok(line)) => line,
            Ok(Err(error)) | Err(error) => {
                protocol_trace(&trace, "transport", "stdout", "read_failure", None, None);
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
                protocol_trace(&trace, "transport", "stdout", "invalid_json", None, None);
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
        observe_subsequent_turn_clock(started, &control, &mut stage_clock, &trace);
        if value.get("method").and_then(Value::as_str) == Some("mcpServer/startupStatus/updated") {
            if let Ok(mut updates) = startup_updates.lock() {
                record_mcp_startup_update(
                    &value,
                    control.current_app_server_thread().as_deref(),
                    &mut updates,
                );
            }
            continue;
        }
        // Server-to-client requests also have JSON-RPC ids.  Dispatch them
        // before considering client response correlation; never mistake one
        // for a late turn/start acknowledgement.
        if value.get("method").is_some() && value.get("id").is_some() {
            let request_thread = value
                .pointer("/params/threadId")
                .and_then(Value::as_str)
                .unwrap_or("");
            let request_turn = value
                .pointer("/params/turnId")
                .and_then(Value::as_str)
                .unwrap_or("");
            if !request_thread.is_empty()
                && !request_turn.is_empty()
                && control.app_server_turn_is_current(request_thread, request_turn)
            {
                if let Err(error) = flush_buffered_messages(
                    &service,
                    &task_id,
                    &control,
                    request_turn,
                    &mut message_text,
                    &turn_items,
                    false,
                ) {
                    service.complete_app_server_turn(
                        &task_id,
                        &control,
                        Some(request_turn),
                        "error",
                        Some(error),
                    );
                    terminal = true;
                    break;
                }
            }
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
            if let Some(waiter) = control.take_app_server_query(id) {
                let _ = waiter.send(value);
                continue;
            }
            let is_turn_request = control.take_app_server_turn_request(id);
            let native_turn_request = control.take_app_server_native_turn_request(id);
            let steer_request = control.take_app_server_steer_request(id);
            let known_request = id == INITIALIZE_ID
                || id == THREAD_ID
                || is_turn_request
                || native_turn_request.is_some()
                || steer_request.is_some();
            if !known_request {
                protocol_trace(
                    &trace,
                    "rpc/response",
                    "response",
                    "uncorrelated",
                    Some(false),
                    None,
                );
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
            let response_method = if id == INITIALIZE_ID {
                Some("initialize")
            } else if id == THREAD_ID {
                Some(
                    if task
                        .native_session_id
                        .as_deref()
                        .is_some_and(|value| !value.trim().is_empty())
                    {
                        "thread/resume"
                    } else {
                        "thread/start"
                    },
                )
            } else if is_turn_request {
                Some("turn/start")
            } else {
                None
            };
            if let Some(method) = response_method {
                let stage = if is_turn_request {
                    if started {
                        "subsequent_turn"
                    } else {
                        "initial_turn"
                    }
                } else if id == THREAD_ID {
                    "thread"
                } else {
                    "initialize"
                };
                protocol_trace(
                    &trace,
                    method,
                    stage,
                    if value.get("error").is_some() {
                        "failure_response"
                    } else {
                        "ack"
                    },
                    Some(true),
                    None,
                );
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
            if let Some(command) = native_turn_request {
                if let Some(error) = value.pointer("/error/message").and_then(Value::as_str) {
                    let _ = control.take_pending_native_turn();
                    service.complete_app_server_turn(
                        &task_id,
                        &control,
                        None,
                        "error",
                        Some(format!("Codex command failed: {error}")),
                    );
                    stage_clock = None;
                    continue;
                }
                let turn_already_observed = !control.has_pending_native_turn();
                if stage_clock.map(|clock| clock.stage) != Some(RequestStage::SubsequentTurn)
                    && !turn_already_observed
                {
                    let _ = control.take_pending_native_turn();
                    service.complete_app_server_turn(
                        &task_id,
                        &control,
                        None,
                        "error",
                        Some("Codex returned a native command response out of order.".into()),
                    );
                    stage_clock = None;
                    continue;
                }
                if command == crate::runner::NativeTurnCommand::Review {
                    let Some(turn_id) = value.pointer("/result/turn/id").and_then(Value::as_str)
                    else {
                        let _ = control.take_pending_native_turn();
                        service.complete_app_server_turn(
                            &task_id,
                            &control,
                            None,
                            "error",
                            Some("Codex review response omitted its turn ID.".into()),
                        );
                        stage_clock = None;
                        continue;
                    };
                    control.set_app_server_turn(turn_id.into());
                    let _ = control.take_pending_native_turn();
                }
                // Compaction returns `{}` before announcing its real turn
                // through `turn/started`. Keep both the pending marker and
                // absolute stage deadline until that notification arrives.
                if command == crate::runner::NativeTurnCommand::Review {
                    stage_clock = None;
                }
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
                if send(&control, json!({"method":"initialized"})).is_err() {
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
                let request = thread_request(
                    &task,
                    collaboration_endpoint.as_deref(),
                    jev_decisions_endpoint.as_deref(),
                    &extensions,
                );
                let method = if request["method"] == "thread/resume" {
                    "thread/resume"
                } else {
                    "thread/start"
                };
                protocol_trace(&trace, method, "thread", "request", None, None);
                if send(&control, request).is_err() {
                    protocol_trace(&trace, method, "thread", "failure", None, None);
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
                control.set_resume_supported(true);
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
                let (startup_tx, startup_rx) = mpsc::channel();
                startup_pending = Some(startup_rx);
                startup_thread_id = Some(thread_id.to_owned());
                stage_clock = None;
                let startup_control = control.clone();
                let startup_thread = thread_id.to_owned();
                let updates = startup_updates.clone();
                // No user prompt is sent until this worker finishes or its
                // bounded readiness fallback fires. The reader keeps pumping
                // list responses and per-server startup notifications.
                if thread::Builder::new()
                    .name("monitter-codex-mcp-ready".into())
                    .spawn(move || {
                        let _ = startup_tx.send(wait_for_mcp_startup(
                            &startup_control,
                            &startup_thread,
                            &updates,
                            MCP_STARTUP_TIMEOUT,
                        ));
                    })
                    .is_err()
                {
                    // Dropped sender resolves once, disables GC, and still
                    // starts the original prompt on the reader's next loop.
                    control.disable_runtime_process_baseline();
                }
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
        if value.get("method").and_then(Value::as_str) == Some("turn/started")
            && control.has_pending_native_turn()
        {
            let params = value.get("params").unwrap_or(&Value::Null);
            if let Some(turn_id) = params
                .get("turnId")
                .and_then(Value::as_str)
                .or_else(|| params.pointer("/turn/id").and_then(Value::as_str))
            {
                // Current app-server schemas omit `threadId` from this
                // notification. Older versions included it, so validate when
                // present and otherwise retain the already-owned thread.
                let thread_matches = params
                    .get("threadId")
                    .and_then(Value::as_str)
                    .map(|thread_id| control.matches_app_server_thread(thread_id))
                    .unwrap_or(true);
                if thread_matches && !turn_id.is_empty() {
                    control.set_app_server_turn(turn_id.into());
                    let _ = control.take_pending_native_turn();
                    stage_clock = None;
                    started = true;
                }
            }
        }
        handle_notification_with_trace(
            &service,
            &task_id,
            &control,
            &value,
            &trace,
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
    let response_turn_id = params
        .get("turnId")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty());
    let response_thread_id = params
        .get("threadId")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty());
    if params.to_string().len() > 64 * 1024 {
        send_protocol_response(
            &service,
            &task_id,
            &control,
            response_thread_id,
            response_turn_id,
            json!({"id":request["id"],"error":{"code":-32602,"message":"Codex request is too large."}}),
        );
        return;
    }
    let Some(turn_id) = params
        .get("turnId")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
    else {
        send_protocol_response(
            &service,
            &task_id,
            &control,
            response_thread_id,
            None,
            json!({"id":request["id"],"error":{"code":-32602,"message":"Codex request omitted a turn ID."}}),
        );
        return;
    };
    let Some(thread_id) = params
        .get("threadId")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
    else {
        send_protocol_response(
            &service,
            &task_id,
            &control,
            response_thread_id,
            response_turn_id,
            json!({"id":request["id"],"error":{"code":-32602,"message":"Codex request omitted a thread ID."}}),
        );
        return;
    };
    if !control.app_server_turn_is_current(thread_id, turn_id) {
        send_protocol_response(
            &service,
            &task_id,
            &control,
            Some(thread_id),
            Some(turn_id),
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
        send_protocol_response(
            &service,
            &task_id,
            &control,
            Some(thread_id),
            Some(turn_id),
            json!({"id":request["id"],"error":{"code":-32000,"message":"Duplicate or excessive Codex request."}}),
        );
        return;
    }
    if outstanding.fetch_add(1, Ordering::SeqCst) >= MAX_SERVER_REQUESTS {
        outstanding.fetch_sub(1, Ordering::SeqCst);
        let _ = finish_request(&lifecycle, &rpc_id);
        send_protocol_response(
            &service,
            &task_id,
            &control,
            Some(thread_id),
            Some(turn_id),
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
        let response_turn_id = params
            .get("turnId")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty());
        let response_thread_id = params
            .get("threadId")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty());
        if params.to_string().len() > 64 * 1024 {
            send_protocol_response(
                &service,
                &task_id,
                &control,
                response_thread_id,
                response_turn_id,
                json!({"id":id,"error":{"code":-32602,"message":"Codex request is too large."}}),
            );
            return;
        }
        let Some(turn_id) = params
            .get("turnId")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
        else {
            send_protocol_response(
                &service,
                &task_id,
                &control,
                response_thread_id,
                None,
                json!({"id":id,"error":{"code":-32602,"message":"Codex request omitted a turn ID."}}),
            );
            return;
        };
        let Some(thread_id) = params
            .get("threadId")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
        else {
            send_protocol_response(
                &service,
                &task_id,
                &control,
                response_thread_id,
                response_turn_id,
                json!({"id":id,"error":{"code":-32602,"message":"Codex request omitted a thread ID."}}),
            );
            return;
        };
        if !control.app_server_turn_is_current(thread_id, turn_id) {
            send_protocol_response(
                &service,
                &task_id,
                &control,
                Some(thread_id),
                Some(turn_id),
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
            send_protocol_response(
                &service,
                &task_id,
                &control,
                Some(thread_id),
                Some(turn_id),
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
                send_protocol_response(
                    &service,
                    &task_id,
                    &control,
                    Some(thread_id),
                    Some(turn_id),
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
                        send_protocol_response(
                            &service,
                            &task_id,
                            &control,
                            Some(thread_id),
                            Some(turn_id),
                            json!({"id":id,"result":response}),
                        );
                    }
                }
                Err(error) => {
                    let response_live = finish_request(&lifecycle, &rpc_id);
                    if response_live && control.app_server_turn_is_current(thread_id, turn_id) {
                        send_protocol_response(
                            &service,
                            &task_id,
                            &control,
                            Some(thread_id),
                            Some(turn_id),
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
                send_protocol_response(
                    &service,
                    &task_id,
                    &control,
                    Some(thread_id),
                    Some(turn_id),
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
                Ok(response) if mcp_tool_consent(&params) => {
                    mcp_tool_consent_wire_response(&response)
                },
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
                send_protocol_response(
                    &service,
                    &task_id,
                    &control,
                    Some(thread_id),
                    Some(turn_id),
                    json!({"id":id,"result":result}),
                );
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
                send_protocol_response(
                    &service,
                    &task_id,
                    &control,
                    Some(thread_id),
                    Some(turn_id),
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
                send_protocol_response(
                    &service,
                    &task_id,
                    &control,
                    Some(thread_id),
                    Some(turn_id),
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
            send_protocol_response(
                &service,
                &task_id,
                &control,
                Some(thread_id),
                Some(turn_id),
                json!({"id":id,"result":result}),
            );
        }
    });
}

// Only fieldless, explicitly identified forms can be remembered as consent.
// Unknown schema keywords may impose constraints on {}, so leave them interactive.
pub(crate) fn mcp_tool_consent(params: &Value) -> bool {
    if !matches!(params.get("mode").and_then(Value::as_str), Some("form" | "openai/form" | "openaiForm"))
        || ["serverName", "message"].iter().any(|key| params.get(*key).and_then(Value::as_str).is_none_or(|text| text.trim().is_empty()))
        || params.pointer("/_meta/codex_approval_kind").and_then(Value::as_str) != Some("mcp_tool_call")
    {
        return false;
    }
    let Some(schema) = params.get("requestedSchema").and_then(Value::as_object) else {
        return false;
    };
    schema.get("type").and_then(Value::as_str) == Some("object")
        && schema.get("properties").and_then(Value::as_object).is_some_and(|properties| properties.is_empty())
        && schema.get("required").is_none_or(|required| required.as_array().is_some_and(|required| required.is_empty()))
        && schema.get("$schema").is_none_or(|dialect| dialect.as_str().is_some_and(|dialect| matches!(dialect, "http://json-schema.org/draft-07/schema#" | "https://json-schema.org/draft/2020-12/schema")))
        && schema.keys().all(|key| matches!(key.as_str(), "type" | "properties" | "required" | "$schema"))
}

pub(crate) fn mcp_tool_consent_allows(params: &Value, scope: &str) -> bool {
    match params.pointer("/_meta/persist") {
        Some(Value::String(value)) => value == scope,
        Some(Value::Array(values)) => values.iter().any(|value| value.as_str() == Some(scope)),
        _ => false,
    }
}

fn mcp_tool_consent_wire_response(response: &Value) -> Value {
    // The internal envelope is produced only by the validated MCP approval command.
    // Ordinary form submission ({}) remains a one-shot approval.
    let meta = response.get("monitterMcpApprovalPersist")
        .and_then(Value::as_str)
        .filter(|scope| matches!(*scope, "session" | "always"))
        .map(|scope| json!({"persist":scope}))
        .unwrap_or(Value::Null);
    json!({"action":"accept","content":null,"_meta":meta})
}

fn thread_context_usage_detail(params: &Value, fallback_turn_id: &str) -> String {
    let usage = params.get("tokenUsage");
    json!({
        "providerTurnId": params.get("turnId").and_then(Value::as_str).unwrap_or(fallback_turn_id),
        "usage": {
            "used": usage.and_then(|value| value.pointer("/last/totalTokens")),
            "size": usage.and_then(|value| value.get("modelContextWindow")),
        },
    })
    .to_string()
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
    let trace = disabled_protocol_trace_state();
    handle_notification_with_trace(
        service,
        task_id,
        control,
        value,
        &trace,
        started,
        _terminal,
        message_text,
        turn_items,
        seen,
        lifecycle,
    );
}

#[cfg(feature = "experimental-gpui")]
fn disabled_protocol_trace_state() -> ProtocolTraceState {
    None
}

#[cfg(not(feature = "experimental-gpui"))]
fn disabled_protocol_trace_state() -> ProtocolTraceState {}

fn handle_notification_with_trace(
    service: &Arc<Service>,
    task_id: &str,
    control: &Arc<RunControl>,
    value: &Value,
    trace: &ProtocolTraceState,
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
    service.observe_codex_child_notification(task_id, control, value);
    let trace_method = traceable_notification_method(method);
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
        let scope_matched = control.app_server_turn_is_current(thread_id, turn_id)
            || service
                .adopt_app_server_continuation(task_id, control, thread_id, turn_id)
                .is_ok();
        if let Some(method) = trace_method {
            protocol_trace(
                trace,
                method,
                "notification",
                if scope_matched {
                    "recognized"
                } else {
                    "ignored_scope"
                },
                None,
                Some(scope_matched),
            );
        }
        if scope_matched {
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
    let scope_matched = !thread_id.is_empty()
        && !turn_id.is_empty()
        && control.app_server_turn_is_current(thread_id, turn_id);
    if let Some(method) = trace_method {
        protocol_trace(
            trace,
            method,
            "notification",
            if scope_matched {
                "recognized"
            } else {
                "ignored_scope"
            },
            None,
            Some(scope_matched),
        );
    }
    if matches!(
        method,
        "item/agentMessage/delta"
            | "item/completed"
            | "turn/completed"
            | "thread/tokenUsage/updated"
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
    // Late native PTY exits may clear an existing job without replaying a turn.
    if method == "item/completed" && control.current_app_server_thread().as_deref() == Some(thread_id) {
        if let Some(item) = params.get("item") { control.observe_runtime_command_exit(item); }
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
                control.observe_runtime_command(item);
                let parsed = parse_item_in_turn(item, true, turn_id);
                if parsed
                    .event
                    .as_ref()
                    .is_some_and(|(kind, _, _)| matches!(kind.as_str(), "tool" | "computer"))
                {
                    control.mark_tool_work_observed();
                }
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
                            flushed_len: 0,
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
                        None,
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
                    buffered.flushed_len = buffered.text.len();
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
                                flushed_len: text.len(),
                            },
                        );
                        if let Err(error) = service.app_server_message(
                            task_id,
                            control,
                            turn_id,
                            &stable,
                            text,
                            item.get("phase").and_then(Value::as_str),
                            true,
                            None,
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
                    // Tool result payloads can contain the image's entire
                    // base64 representation. Keep that data out of timeline
                    // details even when preview validation rejects it.
                    let safe_item = if mcp_image(item).is_some() {
                        without_mcp_image_data(item)
                    } else {
                        item.clone()
                    };
                    control.observe_runtime_command(item);
                    let parsed = parse_item_in_turn(&safe_item, false, turn_id);
                    if parsed
                        .event
                        .as_ref()
                        .is_some_and(|(kind, _, _)| matches!(kind.as_str(), "tool" | "computer"))
                    {
                        control.mark_tool_work_observed();
                    }
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
            let status = params
                .pointer("/turn/status")
                .and_then(Value::as_str)
                .unwrap_or("failed");
            // A provider can complete a turn without a final agent-item
            // envelope. Preserve its last buffered delta while this turn is
            // still owned; completion may clear the ownership fence.
            if let Err(error) = flush_buffered_messages(
                service,
                task_id,
                control,
                turn_id,
                message_text,
                turn_items,
                status == "completed",
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
            if let Some(usage) = params
                .get("usage")
                .or_else(|| params.pointer("/turn/usage"))
            {
                let detail = json!({"providerTurnId": turn_id, "usage": usage}).to_string();
                if let Err(error) = service.app_server_event(
                    task_id,
                    control,
                    Some(turn_id),
                    Parsed {
                        native_session_id: None,
                        assistant: None,
                        event: Some(("usage".into(), "Usage updated".into(), detail)),
                        failed: false,
                    },
                ) {
                    service.record(task_id, "error", "Usage capture warning", error);
                }
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

/// The notification's fenced provider turn is authoritative; item IDs may be
/// reused in a later turn. Keep that scope with compaction's durable lifecycle.
fn parse_item_in_turn(item: &Value, started: bool, turn_id: &str) -> Parsed {
    let mut parsed = parse_item(item, started);
    if item.get("type").and_then(Value::as_str)
        .is_some_and(|kind| kind.eq_ignore_ascii_case("ContextCompaction"))
        && !turn_id.trim().is_empty()
    {
        if let Some((_, _, detail)) = parsed.event.as_mut() {
            if let Ok(mut value) = serde_json::from_str::<Value>(detail) {
                value["monitterTurnId"] = json!(turn_id);
                *detail = value.to_string();
            }
        }
    }
    parsed
}

fn parse_item(item: &Value, started: bool) -> Parsed {
    let ty = item.get("type").and_then(Value::as_str).unwrap_or("");
    if let Some(redacted) = crate::mail_triage::redacted_tool_event(item) {
        return Parsed {
            native_session_id: None,
            assistant: None,
            event: Some((
                "tool".into(),
                redacted
                    .get("tool")
                    .and_then(Value::as_str)
                    .unwrap_or("Mail tool")
                    .into(),
                redacted.to_string(),
            )),
            failed: false,
        };
    }
    if !parse_codex_subagent_updates(item, "event").is_empty() {
        return Parsed {
            native_session_id: None,
            assistant: None,
            event: Some((
                "subagent".into(),
                "Sub-agent activity".into(),
                item.to_string(),
            )),
            failed: false,
        };
    }
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
    if ty.eq_ignore_ascii_case("ContextCompaction") {
        let mut detail = item.clone();
        if let Some(object) = detail.as_object_mut() {
            object.insert(
                "monitterPhase".into(),
                Value::String(if started { "started" } else { "completed" }.into()),
            );
        }
        return Parsed {
            native_session_id: None,
            assistant: None,
            // Keep the existing wire kind for mobile/shared clients. Native
            // presentation treats this lifecycle separately from tool work.
            event: Some((
                "tool".into(),
                if started { "Compacting context" } else { "Context compacted" }.into(),
                detail.to_string(),
            )),
            failed: false,
        };
    }
    let title = item
        .get("tool")
        .or_else(|| item.get("command"))
        .and_then(Value::as_str)
        .unwrap_or(ty)
        .to_owned();
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
            item.to_string(),
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

fn without_mcp_image_data(item: &Value) -> Value {
    let mut safe = item.clone();
    if let Some(content) = safe
        .pointer_mut("/result/content")
        .and_then(Value::as_array_mut)
    {
        for entry in content {
            if entry.get("type").and_then(Value::as_str) == Some("image") {
                if let Some(object) = entry.as_object_mut() {
                    object.remove("data");
                    object.insert("imageOmitted".into(), Value::Bool(true));
                }
            }
        }
    }
    safe
}

fn send(control: &RunControl, value: Value) -> Result<(), String> {
    control.send_control(&value.to_string())
}

fn send_protocol_response(
    service: &Arc<Service>,
    task_id: &str,
    control: &Arc<RunControl>,
    thread_id: Option<&str>,
    turn_id: Option<&str>,
    value: Value,
) {
    if let (Some(thread_id), Some(turn_id)) = (thread_id, turn_id) {
        if !control.app_server_turn_is_current(thread_id, turn_id) {
            eprintln!("Codex app-server reply skipped because its turn fence changed.");
            return;
        }
        if let Err(error) = send(control, value) {
            let detail = format!("Codex protocol response was not delivered: {error}");
            let _ = service.fail_app_server_turn_and_retire(task_id, control, turn_id, detail);
        }
    } else if let Err(error) = send(control, value) {
        // Malformed-request errors without a complete turn identity, like
        // initialization failures, remain best-effort protocol replies and
        // cannot mutate or retire a resident owner.
        eprintln!("Codex app-server error reply failed without a turn fence: {error}");
    }
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
    collaboration_endpoint: Option<&str>,
    jev_decisions_endpoint: Option<&str>,
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
    if let Some(endpoint) = collaboration_endpoint {
        config.extend(
            json!({
                "mcp_servers.monitter.url": endpoint,
                "mcp_servers.monitter.bearer_token_env_var":"MONITTER_TOKEN",
                "mcp_servers.monitter.required":true,
                "mcp_servers.monitter.default_tools_approval_mode": if extensions.auto_approve_monitter_mcp { "approve" } else { "prompt" },
                "mcp_servers.monitter.enabled_tools": crate::collaboration_mcp::tool_names(),
                "mcp_servers.monitter.tools.install_shared_skill.approval_mode":"prompt"
            })
            .as_object()
            .cloned()
            .unwrap_or_default(),
        );
    }
    if let Some(endpoint) = jev_decisions_endpoint {
        config.extend(
            json!({
                "mcp_servers.jev_decisions.url": endpoint,
                "mcp_servers.jev_decisions.bearer_token_env_var": "MONITTER_JEV_DECISIONS_TOKEN",
                "mcp_servers.jev_decisions.required": true,
                "mcp_servers.jev_decisions.enabled_tools": ["jev_choose", "jev_assess"]
            })
            .as_object()
            .cloned()
            .unwrap_or_default(),
        );
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
    use std::{io::Cursor, path::PathBuf};


    #[test]
    fn monitter_mcp_auto_approval_is_scoped_and_opt_out_survives_resume() {
        let task: Task = serde_json::from_value(json!({
            "id":"task", "agentId":"agent", "title":"task",
            "nativeSessionId":"thread", "status":"idle", "archived":false,
            "createdAt":0, "updatedAt":0, "parentTaskId":null, "channelId":null,
            "hostId":"host", "cwd":"/tmp", "provider":"codex", "model":"",
            "sandbox":"workspace-write", "projectId":null
        })).unwrap();
        for enabled in [true, false] {
            let mut extensions = crate::extensions_runtime::RuntimeExtensions::default();
            extensions.auto_approve_monitter_mcp = enabled;
            let request = thread_request(&task, Some("http://127.0.0.1:4444/mcp"),
                Some("http://127.0.0.1:4555/mcp"), &extensions);
            assert_eq!(request["method"], "thread/resume");
            assert_eq!(request["params"]["approvalPolicy"], "on-request");
            assert_eq!(request["params"]["sandbox"], "workspace-write");
            let config = request["params"]["config"].as_object().unwrap();
            assert_eq!(config["mcp_servers.monitter.default_tools_approval_mode"],
                if enabled { "approve" } else { "prompt" });
            assert_eq!(config["mcp_servers.monitter.tools.install_shared_skill.approval_mode"], "prompt");
            assert!(config.keys().filter(|key| key.contains("approval_mode"))
                .all(|key| key.starts_with("mcp_servers.monitter.")));
            let no_grant = thread_request(&task, None, None, &extensions);
            assert!(no_grant["params"]["config"].as_object().is_none_or(|config|
                config.keys().all(|key| !key.starts_with("mcp_servers.monitter."))));
        }
    }

    fn mcp_consent_fixture() -> Value {
        json!({"mode":"form","serverName":"chrome-devtools","message":"Allow tool?",
            "requestedSchema":{"type":"object","properties":{}},
            "_meta":{"codex_approval_kind":"mcp_tool_call","persist":["session","always"]}})
    }

    #[test]
    fn mcp_tool_consent_requires_explicit_fieldless_tool_metadata() {
        let params = mcp_consent_fixture();
        assert!(mcp_tool_consent(&params));
        for key in ["serverName", "message", "_meta", "requestedSchema"] {
            let mut changed = params.clone();
            changed.as_object_mut().unwrap().remove(key);
            assert!(!mcp_tool_consent(&changed), "missing {key}");
        }
        for schema in [json!({"type":"object","properties":{"name":{"type":"string"}}}),
            json!({"type":"object","properties":{},"required":["name"]}),
            json!({"type":"object","properties":{},"minProperties":1})] {
            let mut changed = params.clone();
            changed["requestedSchema"] = schema;
            assert!(!mcp_tool_consent(&changed));
        }
        let mut url = params.clone();
        url["mode"] = json!("url");
        assert!(!mcp_tool_consent(&url));
    }

    #[test]
    fn mcp_tool_consent_native_scope_and_wire_contract() {
        let mut params = mcp_consent_fixture();
        assert!(mcp_tool_consent_allows(&params, "session"));
        assert!(mcp_tool_consent_allows(&params, "always"));
        params["_meta"]["persist"] = json!("session");
        assert!(mcp_tool_consent_allows(&params, "session"));
        assert!(!mcp_tool_consent_allows(&params, "always"));
        assert_eq!(mcp_tool_consent_wire_response(&json!({})),
            json!({"action":"accept","content":null,"_meta":null}));
        for scope in ["session", "always"] {
            assert_eq!(mcp_tool_consent_wire_response(&json!({"monitterMcpApprovalPersist":scope})),
                json!({"action":"accept","content":null,"_meta":{"persist":scope}}));
        }
        assert_eq!(mcp_tool_consent_wire_response(&json!({"monitterMcpApprovalPersist":"bad"}))["_meta"], Value::Null);
    }

    #[test]
    fn mcp_readiness_paginates_and_waits_for_every_server() {
        let pages = [
            json!({"data":[{"name":"configured","runtimeStatus":"connected"}],"nextCursor":"page-two"}),
            json!({"data":[{"name":"home-plugin","runtimeStatus":"starting"}],"nextCursor":null}),
        ];
        let mut index = 0;
        let mut cursors = Vec::new();
        assert!(!mcp_inventory_ready(
            |params| {
                cursors.push(params["cursor"].clone());
                let page = pages[index].clone();
                index += 1;
                Ok(page)
            },
            "thread-a",
            &HashMap::new()
        )
        .unwrap());
        assert_eq!(cursors, vec![Value::Null, json!("page-two")]);
        let ready = json!({"data":[{"name":"configured","runtimeStatus":"connected"},
            {"name":"home-plugin","runtimeStatus":"connected"},{"name":"disabled","runtimeStatus":"disabled"}]});
        assert!(mcp_inventory_ready(|_| Ok(ready.clone()), "thread-a", &HashMap::new()).unwrap());
    }

    #[test]
    fn mcp_readiness_fresh_inventory_overrides_stale_notifications() {
        let mut updates = HashMap::new();
        record_mcp_startup_update(
            &json!({"params":{"threadId":"other","name":"home","status":"ready"}}),
            Some("thread-a"),
            &mut updates,
        );
        assert!(updates.is_empty());
        record_mcp_startup_update(
            &json!({"params":{"threadId":"thread-a","name":"home","status":"ready"}}),
            Some("thread-a"),
            &mut updates,
        );
        record_mcp_startup_update(
            &json!({"params":{"threadId":"thread-a","name":"unknown","status":"ready"}}),
            Some("thread-a"),
            &mut updates,
        );
        assert!(!mcp_inventory_ready(
            |_| Ok(json!({"data":[{"name":"home","runtimeStatus":"starting"}]})),
            "thread-a",
            &updates
        )
        .unwrap());
        assert!(mcp_inventory_ready(
            |_| Ok(json!({"data":[{"name":"home"}]})),
            "thread-a",
            &updates
        )
        .unwrap());
        assert!(mcp_inventory_ready(
            |_| Ok(json!({"data":[{"name":"unseen"}]})),
            "thread-a",
            &updates
        )
        .is_err());
    }

    #[test]
    fn mcp_readiness_timeout_and_unsupported_fall_back_without_querying() {
        let control = RunControl::new(false);
        assert!(!wait_for_mcp_startup(
            &control,
            "thread-a",
            &Mutex::new(HashMap::new()),
            Duration::ZERO
        ));
        assert!(mcp_inventory_ready(
            |_| Err("unsupported method".into()),
            "thread-a",
            &HashMap::new()
        )
        .is_err());
        assert!(mcp_inventory_ready(
            |_| Ok(json!({"data":[],"nextCursor":"repeat"})),
            "thread-a",
            &HashMap::new()
        )
        .is_err());
    }

    #[test]
    fn mcp_readiness_completion_starts_original_prompt_once() {
        let (tx, rx) = mpsc::channel();
        let mut pending = Some(rx);
        assert_eq!(take_mcp_startup_result(&mut pending), None);
        tx.send(true).unwrap();
        tx.send(false).unwrap();
        assert_eq!(take_mcp_startup_result(&mut pending), Some(true));
        assert_eq!(take_mcp_startup_result(&mut pending), None);
        let (tx, rx) = mpsc::channel();
        drop(tx);
        let mut pending = Some(rx);
        assert_eq!(take_mcp_startup_result(&mut pending), Some(false));
        assert_eq!(take_mcp_startup_result(&mut pending), None);
    }

    #[test]
    fn protocol_trace_notification_methods_are_allowlisted() {
        assert_eq!(
            traceable_notification_method("item/agentMessage/delta"),
            Some("item/agentMessage/delta")
        );
        assert_eq!(
            traceable_notification_method("turn/completed"),
            Some("turn/completed")
        );
        assert_eq!(
            traceable_notification_method("arbitrary/untrusted-method"),
            None
        );
    }

    fn current_turn_service(name: &str) -> (Arc<Service>, String, Arc<RunControl>, PathBuf) {
        let directory =
            std::env::temp_dir().join(format!("monitter-codex-app-server-{name}-{}", crate::id()));
        let service = Service::open(None, directory.clone()).unwrap();
        let agent_id = service.snapshot().unwrap().agents[0].id.clone();
        let task = service
            .create_task(crate::model::CreateTaskInput {
                agent_id,
                title: "terminal flush fixture".into(),
                native_session_id: None,
                parent_task_id: None,
                channel_id: None,
                project_id: None,
                cwd: None,
                model_settings: None,
                sandbox: None,
            })
            .unwrap();
        service
            .mutate_data(None, |data| {
                data.snapshot
                    .tasks
                    .iter_mut()
                    .find(|item| item.id == task.id)
                    .unwrap()
                    .status = "running".into();
                Ok(())
            })
            .unwrap();
        let control = service.reserve_run(&task.id).unwrap();
        control.set_app_server_thread("thread-terminal".into());
        control.set_app_server_turn("turn-terminal".into());
        (service, task.id, control, directory)
    }

    #[test]
    fn native_continuation_delivers_output_without_another_user_message() {
        let (service, task_id, control, directory) = current_turn_service("native-continuation");
        service
            .mutate_data(None, |data| {
                data.snapshot
                    .tasks
                    .iter_mut()
                    .find(|task| task.id == task_id)
                    .unwrap()
                    .native_session_id = Some("thread-terminal".into());
                Ok(())
            })
            .unwrap();
        assert!(service.complete_app_server_turn(
            &task_id,
            &control,
            Some("turn-terminal"),
            "completed",
            None
        ));
        let mut started = false;
        let mut terminal = false;
        let mut messages = HashMap::new();
        let items = Arc::new(Mutex::new(HashMap::new()));
        let seen = Arc::new(Mutex::new(HashSet::new()));
        let lifecycle = Arc::new(Mutex::new(RequestLifecycle::default()));
        // Unsolicited starts require the schema-mandated thread identity.
        handle_notification(
            &service,
            &task_id,
            &control,
            &json!({"method":"turn/started","params":{"turn":{"id":"automatic-missing-thread"}}}),
            &mut started,
            &mut terminal,
            &mut messages,
            &items,
            &seen,
            &lifecycle,
        );
        assert!(!started);
        assert!(!control.matches_app_server_turn("automatic-missing-thread"));
        handle_notification(
            &service,
            &task_id,
            &control,
            &json!({"method":"turn/started","params":{"threadId":"thread-terminal","turn":{"id":"automatic-1"}}}),
            &mut started,
            &mut terminal,
            &mut messages,
            &items,
            &seen,
            &lifecycle,
        );
        assert!(started);
        assert!(control.app_server_turn_is_current("thread-terminal", "automatic-1"));
        handle_notification(
            &service,
            &task_id,
            &control,
            &json!({"method":"item/completed","params":{"threadId":"thread-terminal","turnId":"automatic-1","item":{"type":"agentMessage","id":"automatic-output","text":"continued work"}}}),
            &mut started,
            &mut terminal,
            &mut messages,
            &items,
            &seen,
            &lifecycle,
        );
        let snapshot = service.snapshot().unwrap();
        assert!(snapshot
            .tasks
            .iter()
            .any(|task| task.id == task_id && task.status == "running"));
        assert!(snapshot
            .messages
            .iter()
            .any(|message| message.task_id == task_id
                && message.role == "assistant"
                && message.text == "continued work"));
        assert!(!snapshot
            .messages
            .iter()
            .any(|message| message.task_id == task_id && message.role == "user"));
        drop(service);
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn rejected_image_preview_warns_and_keeps_the_provider_turn_live() {
        let (service, task_id, control, directory) = current_turn_service("image-preview-warning");
        let mut started = true;
        let mut terminal = false;
        let mut messages = HashMap::new();
        let items = Arc::new(Mutex::new(HashMap::new()));
        let seen = Arc::new(Mutex::new(HashSet::new()));
        let lifecycle = Arc::new(Mutex::new(RequestLifecycle::default()));
        let image_data = "A".repeat(crate::attachments::MAX_GENERATED_IMAGE_DATA_URL_BYTES);

        handle_notification(
            &service,
            &task_id,
            &control,
            &json!({
                "method":"item/completed",
                "params":{
                    "threadId":"thread-terminal",
                    "turnId":"turn-terminal",
                    "item":{
                        "type":"mcpToolCall",
                        "id":"image-tool",
                        "server":"image-tools",
                        "tool":"render",
                        "arguments":{},
                        "result":{"content":[{"type":"image","mimeType":"image/png","data":image_data}]}
                    }
                }
            }),
            &mut started,
            &mut terminal,
            &mut messages,
            &items,
            &seen,
            &lifecycle,
        );

        assert!(!terminal);
        assert!(control.app_server_turn_is_current("thread-terminal", "turn-terminal"));
        let snapshot = service.snapshot().unwrap();
        assert!(snapshot
            .tasks
            .iter()
            .any(|task| task.id == task_id && task.status == "running"));
        let warning = snapshot
            .events
            .iter()
            .find(|event| event.task_id == task_id && event.kind == "warning")
            .expect("rejected preview should be visible as a warning");
        assert_eq!(warning.title, "Codex image preview omitted");
        assert!(warning.detail.contains("provider turn continues"));
        assert!(warning.detail.len() < 256);
        let tool_event = snapshot
            .events
            .iter()
            .find(|event| event.task_id == task_id && event.kind == "tool")
            .expect("tool activity should remain visible");
        assert!(tool_event.detail.contains("imageOmitted"));
        assert!(!tool_event.detail.contains(&image_data));
        assert!(tool_event.detail.len() < 2_000);

        handle_notification(
            &service,
            &task_id,
            &control,
            &json!({
                "method":"item/completed",
                "params":{
                    "threadId":"thread-terminal",
                    "turnId":"turn-terminal",
                    "item":{"type":"agentMessage","id":"after-image","text":"The turn continued."}
                }
            }),
            &mut started,
            &mut terminal,
            &mut messages,
            &items,
            &seen,
            &lifecycle,
        );
        let snapshot = service.snapshot().unwrap();
        assert!(snapshot.messages.iter().any(|message| {
            message.task_id == task_id && message.text == "The turn continued."
        }));
        assert!(!terminal);
        drop(service);
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn image_preview_ownership_failure_still_terminates_the_turn() {
        let (service, task_id, control, directory) =
            current_turn_service("image-preview-state-error");
        let mut started = true;
        let mut terminal = false;
        let mut messages = HashMap::new();
        let items = Arc::new(Mutex::new(HashMap::new()));
        let seen = Arc::new(Mutex::new(HashSet::new()));
        let lifecycle = Arc::new(Mutex::new(RequestLifecycle::default()));
        service.release_app_server_run(&task_id, &control);
        let replacement = service.reserve_run(&task_id).unwrap();
        replacement.set_app_server_thread("thread-terminal".into());
        replacement.set_app_server_turn("turn-terminal".into());

        handle_notification(
            &service,
            &task_id,
            &control,
            &json!({
                "method":"item/completed",
                "params":{
                    "threadId":"thread-terminal",
                    "turnId":"turn-terminal",
                    "item":{
                        "type":"mcpToolCall",
                        "id":"image-tool",
                        "server":"image-tools",
                        "tool":"render",
                        "arguments":{},
                        "result":{"content":[{"type":"image","mimeType":"image/jpeg","data":"/9j/2Q=="}]}
                    }
                }
            }),
            &mut started,
            &mut terminal,
            &mut messages,
            &items,
            &seen,
            &lifecycle,
        );

        assert!(terminal);
        let snapshot = service.snapshot().unwrap();
        assert!(!snapshot
            .events
            .iter()
            .any(|event| event.task_id == task_id && event.kind == "warning"));
        drop(service);
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn native_continuation_rejects_stale_wrong_thread_cancelled_and_retiring_owners() {
        for fence in [
            "stale",
            "wrong-thread",
            "active",
            "cancelled",
            "retiring",
            "error",
            "archived",
        ] {
            let (service, task_id, control, directory) =
                current_turn_service(&format!("continuation-{fence}"));
            service
                .mutate_data(None, |data| {
                    data.snapshot
                        .tasks
                        .iter_mut()
                        .find(|task| task.id == task_id)
                        .unwrap()
                        .native_session_id = Some("thread-terminal".into());
                    Ok(())
                })
                .unwrap();
            if fence != "active" {
                assert!(service.complete_app_server_turn(
                    &task_id,
                    &control,
                    Some("turn-terminal"),
                    if fence == "error" {
                        "error"
                    } else {
                        "completed"
                    },
                    None
                ));
            }
            if fence == "cancelled" {
                control.reserve_cancellation();
            }
            if fence == "retiring" {
                control.set_resume_supported(true);
                assert!(control
                    .try_retire_idle(Instant::now() + Duration::from_secs(1), Duration::ZERO));
            }
            if fence == "archived" {
                service
                    .mutate_data(None, |data| {
                        data.snapshot
                            .tasks
                            .iter_mut()
                            .find(|task| task.id == task_id)
                            .unwrap()
                            .archived = true;
                        Ok(())
                    })
                    .unwrap();
            }
            assert!(
                service
                    .adopt_app_server_continuation(
                        &task_id,
                        &control,
                        if fence == "wrong-thread" {
                            "wrong-thread"
                        } else {
                            "thread-terminal"
                        },
                        if fence == "stale" {
                            "turn-terminal"
                        } else {
                            "automatic-rejected"
                        }
                    )
                    .is_err(),
                "{fence}"
            );
            assert!(!control.matches_app_server_turn("automatic-rejected"));
            drop(service);
            let _ = std::fs::remove_dir_all(directory);
        }
    }

    #[test]
    fn terminal_completion_flushes_dirty_deltas_without_agent_item() {
        let (service, task_id, control, directory) = current_turn_service("terminal-flush");
        let mut started = true;
        let mut terminal = false;
        let mut messages = HashMap::new();
        let items = Arc::new(Mutex::new(HashMap::new()));
        let seen = Arc::new(Mutex::new(HashSet::new()));
        let lifecycle = Arc::new(Mutex::new(RequestLifecycle::default()));
        let delta = |text| {
            json!({
                "method": "item/agentMessage/delta",
                "params": {
                    "threadId": "thread-terminal",
                    "turnId": "turn-terminal",
                    "itemId": "item-terminal",
                    "delta": text,
                }
            })
        };

        handle_notification(
            &service,
            &task_id,
            &control,
            &delta("first"),
            &mut started,
            &mut terminal,
            &mut messages,
            &items,
            &seen,
            &lifecycle,
        );
        handle_notification(
            &service,
            &task_id,
            &control,
            &delta(" second"),
            &mut started,
            &mut terminal,
            &mut messages,
            &items,
            &seen,
            &lifecycle,
        );
        handle_notification(
            &service,
            &task_id,
            &control,
            &json!({
                "method": "turn/completed",
                "params": {
                    "threadId": "thread-terminal",
                    "turnId": "turn-terminal",
                    "turn": {"status": "completed", "error": null},
                }
            }),
            &mut started,
            &mut terminal,
            &mut messages,
            &items,
            &seen,
            &lifecycle,
        );

        let snapshot = service.snapshot().unwrap();
        assert_eq!(
            snapshot
                .messages
                .iter()
                .find(|message| message.task_id == task_id && message.role == "assistant")
                .map(|message| message.text.as_str()),
            Some("first second")
        );
        assert_eq!(
            snapshot
                .messages
                .iter()
                .find(|message| message.task_id == task_id && message.role == "assistant")
                .and_then(|message| message.stream_status.as_deref()),
            Some("complete")
        );
        assert!(snapshot
            .tasks
            .iter()
            .any(|task| task.id == task_id && task.status == "completed"));
        drop(service);
        let _ = std::fs::remove_dir_all(directory);
    }

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
            None,
            &crate::extensions_runtime::RuntimeExtensions::default(),
        );
        assert_eq!(request["method"], "thread/resume");
        assert_eq!(request["params"]["excludeTurns"], Value::Bool(true));
        let with_mcp = thread_request(
            &task,
            Some("http://127.0.0.1:4444/mcp"),
            Some("http://127.0.0.1:4555/mcp"),
            &crate::extensions_runtime::RuntimeExtensions::default(),
        );
        let config = &with_mcp["params"]["config"];
        assert_eq!(
            config["mcp_servers.monitter.url"],
            "http://127.0.0.1:4444/mcp"
        );
        assert_eq!(
            config["mcp_servers.monitter.bearer_token_env_var"],
            "MONITTER_TOKEN"
        );
        assert_eq!(
            config["mcp_servers.monitter.tools.install_shared_skill.approval_mode"],
            "prompt"
        );
        assert!(config["mcp_servers.monitter.enabled_tools"]
            .as_array()
            .unwrap()
            .contains(&json!("install_shared_skill")));
        assert_eq!(
            config["mcp_servers.jev_decisions.url"],
            "http://127.0.0.1:4555/mcp"
        );
        assert_eq!(
            config["mcp_servers.jev_decisions.bearer_token_env_var"],
            "MONITTER_JEV_DECISIONS_TOKEN"
        );
        assert_eq!(
            config["mcp_servers.jev_decisions.enabled_tools"],
            json!(["jev_choose", "jev_assess"])
        );
    }

    #[test]
    fn observes_consecutive_immediate_resident_turn_responses() {
        let control = RunControl::new(false);
        let mut stage_clock = None;

        control.mark_app_server_turn_request(10).unwrap();
        // This models a response waking the reader before its next loop-top
        // observation of the just-sent resident turn.
        observe_subsequent_turn_clock(
            true,
            &control,
            &mut stage_clock,
            &disabled_protocol_trace_state(),
        );
        assert_eq!(
            stage_clock.map(|clock| clock.stage),
            Some(RequestStage::SubsequentTurn)
        );
        assert!(control.take_app_server_turn_request(10));
        stage_clock = None;

        control.mark_app_server_turn_request(11).unwrap();
        observe_subsequent_turn_clock(
            true,
            &control,
            &mut stage_clock,
            &disabled_protocol_trace_state(),
        );
        assert_eq!(
            stage_clock.map(|clock| clock.stage),
            Some(RequestStage::SubsequentTurn)
        );
        assert!(control.take_app_server_turn_request(11));
    }

    #[test]
    fn context_compaction_keeps_its_native_id_and_lifecycle_phase() {
        for item_type in ["ContextCompaction", "contextCompaction"] {
            let item = json!({"type":item_type,"id":"compact-1"});
            for (started, phase) in [(true, "started"), (false, "completed")] {
                let parsed = parse_item_in_turn(&item, started, "turn-1");
                let event = parsed.event.expect("compaction is activity");
                assert_eq!(event.0, "tool");
                assert_eq!(event.1, if started { "Compacting context" } else { "Context compacted" });
                let detail: Value = serde_json::from_str(&event.2).expect("JSON detail");
                assert_eq!(detail["id"], "compact-1");
                assert_eq!(detail["monitterPhase"], phase);
                assert_eq!(detail["type"], item_type);
                assert_eq!(detail["monitterTurnId"], "turn-1");
            }
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
        ))
        .unwrap();
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
