//! Process-local collaboration RPC broker.
//!
//! This deliberately exposes no network listener beyond loopback and keeps grants only in memory.
//! The runner receives a short-lived [`SessionGrant`] for a task; callers cannot supply a task id.

use serde_json::{json, Value};
use std::{
    collections::HashMap,
    io::{Read, Write},
    net::{Shutdown, TcpListener, TcpStream},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use uuid::Uuid;

const MAX_BODY: usize = 128 * 1024;
const MAX_HEADER: usize = 16 * 1024;
const MAX_CONCURRENT: usize = 16;
const IO_TIMEOUT: Duration = Duration::from_secs(5);
const REQUEST_DEADLINE: Duration = Duration::from_secs(20);

pub type Handler = dyn Fn(&str, &str, Value) -> Result<Value, String> + Send + Sync + 'static;

#[derive(Clone, PartialEq, Eq)]
pub struct SessionGrant {
    pub endpoint: String,
    pub token: String,
}

/// A loopback-only broker. Dropping it revokes every outstanding session and joins its listener.
pub struct Broker {
    endpoint: String,
    grants: Arc<Mutex<HashMap<String, String>>>,
    running: Arc<AtomicBool>,
    listener: Option<JoinHandle<()>>,
}

impl Broker {
    pub fn start(handler: Arc<Handler>) -> Result<Self, String> {
        let listener = TcpListener::bind("127.0.0.1:0")
            .map_err(|error| format!("Could not bind collaboration loopback broker: {error}"))?;
        listener
            .set_nonblocking(true)
            .map_err(|error| format!("Could not configure collaboration broker: {error}"))?;
        let endpoint = format!(
            "http://{}/rpc",
            listener.local_addr().map_err(|error| error.to_string())?
        );
        let grants = Arc::new(Mutex::new(HashMap::<String, String>::new()));
        let running = Arc::new(AtomicBool::new(true));
        let active = Arc::new(AtomicUsize::new(0));
        let listener_grants = grants.clone();
        let listener_running = running.clone();
        let listener_active = active.clone();
        let listener = thread::Builder::new()
            .name("monitter-collaboration-broker".into())
            .spawn(move || {
                while listener_running.load(Ordering::Acquire) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            // Accepted sockets can inherit O_NONBLOCK from the listener on macOS.
                            // The per-request deadline below is implemented with socket timeouts, so
                            // restore blocking mode before assigning work to a worker.
                            if stream.set_nonblocking(false).is_err() {
                                let _ = stream.shutdown(Shutdown::Both);
                                continue;
                            }
                            if !try_acquire(&listener_active) {
                                let _ = stream.set_write_timeout(Some(Duration::from_millis(100)));
                                let _ = write_response(
                                    stream,
                                    429,
                                    json!({"error":{"message":"Broker is busy."}}),
                                );
                                continue;
                            }
                            let grants = listener_grants.clone();
                            let handler = handler.clone();
                            let active = listener_active.clone();
                            thread::spawn(move || {
                                serve(stream, grants, handler);
                                active.fetch_sub(1, Ordering::Release);
                            });
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(10));
                        }
                        Err(_) => break,
                    }
                }
            })
            .map_err(|error| format!("Could not start collaboration broker: {error}"))?;
        Ok(Self {
            endpoint,
            grants,
            running,
            listener: Some(listener),
        })
    }

    /// Issues a random bearer grant bound to `task_id`. The task id never travels in the request.
    pub fn session(&self, task_id: &str) -> SessionGrant {
        let token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
        if let Ok(mut grants) = self.grants.lock() {
            grants.insert(token.clone(), task_id.to_owned());
        }
        SessionGrant {
            endpoint: self.endpoint.clone(),
            token,
        }
    }

    pub fn revoke(&self, token: &str) {
        if let Ok(mut grants) = self.grants.lock() {
            grants.remove(token);
        }
    }
}

impl Drop for Broker {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Release);
        if let Ok(mut grants) = self.grants.lock() {
            grants.clear();
        }
        if let Some(listener) = self.listener.take() {
            let _ = listener.join();
        }
    }
}

fn try_acquire(active: &AtomicUsize) -> bool {
    let mut current = active.load(Ordering::Acquire);
    loop {
        if current >= MAX_CONCURRENT {
            return false;
        }
        match active.compare_exchange_weak(
            current,
            current + 1,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => return true,
            Err(observed) => current = observed,
        }
    }
}

fn serve(
    mut stream: TcpStream,
    grants: Arc<Mutex<HashMap<String, String>>>,
    handler: Arc<Handler>,
) {
    let _ = stream.set_read_timeout(Some(IO_TIMEOUT));
    let _ = stream.set_write_timeout(Some(IO_TIMEOUT));
    let request = match read_request(&mut stream) {
        Ok(request) => request,
        Err((status, message)) => {
            let _ = write_response(stream, status, json!({"error":{"message":message}}));
            return;
        }
    };
    let task_id = match grants
        .lock()
        .ok()
        .and_then(|grants| grants.get(&request.token).cloned())
    {
        Some(task_id) => task_id,
        None => {
            let _ = write_response(
                stream,
                401,
                json!({"error":{"message":"Invalid or revoked collaboration grant."}}),
            );
            return;
        }
    };
    let response = match (handler)(&task_id, &request.tool, request.arguments) {
        Ok(result) => json!({"result":result}),
        Err(message) => json!({"error":{"message":message}}),
    };
    let _ = write_response(stream, 200, response);
}

struct Request {
    token: String,
    tool: String,
    arguments: Value,
}

fn read_request(stream: &mut TcpStream) -> Result<Request, (u16, String)> {
    let mut input = Vec::new();
    let mut chunk = [0u8; 4096];
    let started = Instant::now();
    let header_end = loop {
        if input.len() > MAX_HEADER {
            return Err((413, "Headers are too large.".into()));
        }
        let count = read_with_deadline(stream, &mut chunk, started)?;
        if count == 0 {
            return Err((400, "Request ended before headers.".into()));
        }
        input.extend_from_slice(&chunk[..count]);
        if let Some(index) = input.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
            let header_end = index + 4;
            if header_end > MAX_HEADER {
                return Err((413, "Headers are too large.".into()));
            }
            break header_end;
        }
    };
    let headers = std::str::from_utf8(&input[..header_end])
        .map_err(|_| (400, "Headers must be UTF-8.".into()))?;
    let mut lines = headers.split("\r\n");
    if lines.next() != Some("POST /rpc HTTP/1.1") {
        return Err((404, "Only POST /rpc is available.".into()));
    }
    let mut content_length = None;
    let mut token = None;
    let mut authorization_seen = false;
    for line in lines {
        if line.is_empty() {
            break;
        }
        let (name, value) = line
            .split_once(':')
            .ok_or((400, "Malformed header.".into()))?;
        let name = name.trim().to_ascii_lowercase();
        let value = value.trim();
        match name.as_str() {
            "content-length" => {
                if content_length.is_some() {
                    return Err((400, "Ambiguous content length.".into()));
                }
                let length = value
                    .parse::<usize>()
                    .map_err(|_| (400, "Invalid content length.".into()))?;
                if length > MAX_BODY {
                    return Err((413, "Request body is too large.".into()));
                }
                content_length = Some(length);
            }
            "authorization" => {
                if authorization_seen {
                    return Err((400, "Ambiguous authorization.".into()));
                }
                authorization_seen = true;
                token = value
                    .strip_prefix("Bearer ")
                    .filter(|token| !token.is_empty())
                    .map(str::to_owned);
            }
            "transfer-encoding" | "origin" => {
                return Err((
                    400,
                    "This broker does not accept browser or chunked requests.".into(),
                ))
            }
            _ => {}
        }
    }
    let length = content_length.ok_or((411, "Content-Length is required.".into()))?;
    let token = token.ok_or((401, "Bearer authorization is required.".into()))?;
    let mut body = input[header_end..].to_vec();
    while body.len() < length {
        let count = read_with_deadline(stream, &mut chunk, started)?;
        if count == 0 {
            return Err((400, "Request ended before body.".into()));
        }
        body.extend_from_slice(&chunk[..count]);
    }
    if body.len() != length {
        return Err((400, "Ambiguous request body.".into()));
    }
    let value: Value =
        serde_json::from_slice(&body).map_err(|_| (400, "Request body must be JSON.".into()))?;
    let tool = value
        .get("tool")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or((400, "Tool is required.".into()))?
        .to_owned();
    let arguments = value.get("arguments").cloned().unwrap_or_else(|| json!({}));
    if !arguments.is_object() {
        return Err((400, "Arguments must be an object.".into()));
    }
    Ok(Request {
        token,
        tool,
        arguments,
    })
}

fn read_with_deadline(
    stream: &mut TcpStream,
    buffer: &mut [u8],
    started: Instant,
) -> Result<usize, (u16, String)> {
    let remaining = REQUEST_DEADLINE
        .checked_sub(started.elapsed())
        .ok_or((408, "Request timed out.".into()))?;
    stream
        .set_read_timeout(Some(remaining.min(IO_TIMEOUT)))
        .map_err(|_| (400, "Could not configure request timeout.".into()))?;
    stream.read(buffer).map_err(|error| match error.kind() {
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => {
            (408, "Request timed out.".into())
        }
        _ => (400, "Could not read request.".into()),
    })
}

fn write_response(mut stream: TcpStream, status: u16, body: Value) -> std::io::Result<()> {
    let mut bytes = serde_json::to_vec(&body)
        .unwrap_or_else(|_| b"{\"error\":{\"message\":\"Response encoding failed.\"}}".to_vec());
    if bytes.len() > MAX_BODY {
        bytes =
            b"{\"error\":{\"message\":\"Monitter broker response exceeded 128 KiB.\"}}".to_vec();
    }
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        408 => "Request Timeout",
        411 => "Length Required",
        413 => "Payload Too Large",
        429 => "Too Many Requests",
        _ => "Error",
    };
    write!(stream, "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", bytes.len())?;
    stream.write_all(&bytes)?;
    let _ = stream.shutdown(Shutdown::Both);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn call(endpoint: &str, token: &str, body: Value) -> String {
        let address = endpoint
            .trim_start_matches("http://")
            .trim_end_matches("/rpc");
        let bytes = serde_json::to_vec(&body).unwrap();
        let mut stream = TcpStream::connect(address).unwrap();
        write!(stream, "POST /rpc HTTP/1.1\r\nHost: {address}\r\nAuthorization: Bearer {token}\r\nContent-Length: {}\r\n\r\n", bytes.len()).unwrap();
        stream.write_all(&bytes).unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        response
    }

    fn address(endpoint: &str) -> &str {
        endpoint
            .trim_start_matches("http://")
            .trim_end_matches("/rpc")
    }

    fn raw(endpoint: &str, request: &str) -> String {
        let mut stream = TcpStream::connect(address(endpoint)).unwrap();
        stream.write_all(request.as_bytes()).unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        response
    }

    #[test]
    fn only_valid_grants_reach_the_handler() {
        let calls = Arc::new(AtomicUsize::new(0));
        let seen = calls.clone();
        let broker = Broker::start(Arc::new(move |task, tool, _| {
            seen.fetch_add(1, Ordering::SeqCst);
            Ok(json!({"task":task,"tool":tool}))
        }))
        .unwrap();
        let grant = broker.session("caller-task");
        assert!(call(
            &grant.endpoint,
            "forged",
            json!({"tool":"list_agents","arguments":{}})
        )
        .starts_with("HTTP/1.1 401"));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        let response = call(
            &grant.endpoint,
            &grant.token,
            json!({"tool":"list_agents","arguments":{"taskId":"forged"}}),
        );
        assert!(response.contains("caller-task"));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        broker.revoke(&grant.token);
        assert!(call(
            &grant.endpoint,
            &grant.token,
            json!({"tool":"list_agents","arguments":{}})
        )
        .starts_with("HTTP/1.1 401"));
    }

    #[test]
    fn rejects_unauthorized_and_oversized_requests_before_handler() {
        let calls = Arc::new(AtomicUsize::new(0));
        let seen = calls.clone();
        let broker = Broker::start(Arc::new(move |_, _, _| {
            seen.fetch_add(1, Ordering::SeqCst);
            Ok(json!({}))
        }))
        .unwrap();
        let unauthenticated = raw(
            &broker.endpoint,
            "POST /rpc HTTP/1.1\r\nHost: localhost\r\nContent-Length: 2\r\n\r\n{}",
        );
        assert!(unauthenticated.starts_with("HTTP/1.1 401"));
        let oversized = raw(&broker.endpoint, "POST /rpc HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer no\r\nContent-Length: 131073\r\n\r\n");
        assert!(oversized.starts_with("HTTP/1.1 413"));
        let duplicate_auth = raw(&broker.endpoint, "POST /rpc HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer one\r\nAuthorization: Bearer two\r\nContent-Length: 2\r\n\r\n{}");
        assert!(duplicate_auth.starts_with("HTTP/1.1 400"));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn fragmented_request_waits_for_headers_and_body_arrivals() {
        let calls = Arc::new(AtomicUsize::new(0));
        let seen = calls.clone();
        let broker = Broker::start(Arc::new(move |task, tool, arguments| {
            seen.fetch_add(1, Ordering::SeqCst);
            Ok(json!({"task": task, "tool": tool, "arguments": arguments}))
        }))
        .unwrap();
        let grant = broker.session("fragmented-caller");
        let body = b"{\"tool\":\"list_agents\",\"arguments\":{}}";
        let mut stream = TcpStream::connect(address(&broker.endpoint)).unwrap();

        // Give the accept loop time to hand the socket to a worker before any bytes arrive.
        thread::sleep(Duration::from_millis(20));
        stream
            .write_all(b"POST /rpc HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer ")
            .unwrap();
        thread::sleep(Duration::from_millis(20));
        write!(
            stream,
            "{}\r\nContent-Length: {}\r\n\r\n",
            grant.token,
            body.len()
        )
        .unwrap();
        stream.write_all(&body[..8]).unwrap();
        thread::sleep(Duration::from_millis(20));
        stream.write_all(&body[8..]).unwrap();

        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
        assert!(response.contains("fragmented-caller"));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn slow_readers_cannot_exhaust_unbounded_threads() {
        let entered = Arc::new(AtomicUsize::new(0));
        let release = Arc::new(AtomicBool::new(false));
        let handler_entered = entered.clone();
        let handler_release = release.clone();
        let broker = Broker::start(Arc::new(move |_, _, _| {
            handler_entered.fetch_add(1, Ordering::SeqCst);
            while !handler_release.load(Ordering::Acquire) {
                thread::sleep(Duration::from_millis(2));
            }
            Ok(json!({"ok":true}))
        }))
        .unwrap();
        let grant = broker.session("slow-reader-test");
        let mut slow = Vec::new();
        for _ in 0..MAX_CONCURRENT {
            let mut stream = TcpStream::connect(address(&broker.endpoint)).unwrap();
            let body = b"{\"tool\":\"list_agents\",\"arguments\":{}}";
            write!(stream, "POST /rpc HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {}\r\nContent-Length: {}\r\n\r\n", grant.token, body.len()).unwrap();
            stream.write_all(body).unwrap();
            slow.push(stream);
        }
        let deadline = Instant::now() + Duration::from_secs(1);
        while entered.load(Ordering::SeqCst) != MAX_CONCURRENT {
            assert!(
                Instant::now() < deadline,
                "slow readers did not reach the bounded workers"
            );
            thread::sleep(Duration::from_millis(10));
        }
        let response = call(
            &broker.endpoint,
            &grant.token,
            json!({"tool":"list_agents","arguments":{}}),
        );
        assert!(response.starts_with("HTTP/1.1 429"));
        release.store(true, Ordering::Release);
        drop(slow);
    }
}
