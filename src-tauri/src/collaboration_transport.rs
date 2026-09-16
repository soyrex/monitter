//! Loopback-only, stateless Streamable HTTP transport for collaboration MCP.
use crate::collaboration_mcp;
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
const MAX_BATCH: usize = 32;
pub type Handler = dyn Fn(&str, &str, Value) -> Result<Value, String> + Send + Sync + 'static;
#[derive(Clone, PartialEq, Eq)]
pub struct SessionGrant {
    pub endpoint: String,
    pub token: String,
}
pub struct Broker {
    endpoint: String,
    grants: Arc<Mutex<HashMap<String, String>>>,
    running: Arc<AtomicBool>,
    listener: Option<JoinHandle<()>>,
}
impl Broker {
    pub fn start(handler: Arc<Handler>) -> Result<Self, String> {
        let socket = TcpListener::bind("127.0.0.1:0")
            .map_err(|e| format!("Could not bind collaboration loopback broker: {e}"))?;
        socket
            .set_nonblocking(true)
            .map_err(|e| format!("Could not configure collaboration broker: {e}"))?;
        let endpoint = format!(
            "http://{}/mcp",
            socket.local_addr().map_err(|e| e.to_string())?
        );
        let grants = Arc::new(Mutex::new(HashMap::new()));
        let running = Arc::new(AtomicBool::new(true));
        let active = Arc::new(AtomicUsize::new(0));
        let g = grants.clone();
        let r = running.clone();
        let a = active.clone();
        let listener = thread::Builder::new()
            .name("monitter-collaboration-mcp".into())
            .spawn(move || {
                while r.load(Ordering::Acquire) {
                    match socket.accept() {
                        Ok((stream, _)) => {
                            if stream.set_nonblocking(false).is_err() {
                                let _ = stream.shutdown(Shutdown::Both);
                                continue;
                            }
                            if !acquire(&a) {
                                let _ = stream.set_write_timeout(Some(IO_TIMEOUT));
                                let _ = write_response(
                                    stream,
                                    429,
                                    Some(json!({"error":"Broker is busy."})),
                                );
                                continue;
                            }
                            let grants = g.clone();
                            let handler = handler.clone();
                            let active = a.clone();
                            thread::spawn(move || {
                                serve(stream, grants, handler);
                                active.fetch_sub(1, Ordering::Release);
                            });
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(10))
                        }
                        Err(_) => break,
                    }
                }
            })
            .map_err(|e| format!("Could not start collaboration broker: {e}"))?;
        Ok(Self {
            endpoint,
            grants,
            running,
            listener: Some(listener),
        })
    }
    pub fn session(&self, task: &str) -> SessionGrant {
        let token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
        if let Ok(mut g) = self.grants.lock() {
            g.insert(token.clone(), task.into());
        }
        SessionGrant {
            endpoint: self.endpoint.clone(),
            token,
        }
    }
    pub fn revoke(&self, token: &str) {
        if let Ok(mut g) = self.grants.lock() {
            g.remove(token);
        }
    }
}
impl Drop for Broker {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Release);
        if let Ok(mut g) = self.grants.lock() {
            g.clear()
        }
        if let Some(h) = self.listener.take() {
            let _ = h.join();
        }
    }
}
fn acquire(a: &AtomicUsize) -> bool {
    let mut n = a.load(Ordering::Acquire);
    loop {
        if n >= MAX_CONCURRENT {
            return false;
        }
        match a.compare_exchange_weak(n, n + 1, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => return true,
            Err(v) => n = v,
        }
    }
}
struct Request {
    method: String,
    token: String,
    body: Vec<u8>,
    protocol_version: Option<String>,
}
fn serve(mut s: TcpStream, grants: Arc<Mutex<HashMap<String, String>>>, handler: Arc<Handler>) {
    let _ = s.set_read_timeout(Some(IO_TIMEOUT));
    let _ = s.set_write_timeout(Some(IO_TIMEOUT));
    let req = match read_request(&mut s) {
        Ok(v) => v,
        Err((st, msg)) => {
            let _ = write_response(s, st, Some(json!({"error":msg})));
            return;
        }
    };
    let Some(task) = grants.lock().ok().and_then(|g| g.get(&req.token).cloned()) else {
        let _ = write_response(
            s,
            401,
            Some(json!({"error":"Invalid or revoked collaboration grant."})),
        );
        return;
    };
    if req.method == "GET" {
        let _ = write_response(
            s,
            405,
            Some(json!({"error":"This server does not offer an SSE stream."})),
        );
        return;
    }
    if req
        .protocol_version
        .as_deref()
        .is_some_and(|version| !collaboration_mcp::supported_protocol_version(version))
    {
        let _ = write_response(
            s,
            400,
            Some(json!({"error":"Unsupported MCP protocol version."})),
        );
        return;
    }
    let parsed: Value = match serde_json::from_slice(&req.body) {
        Ok(v) => v,
        Err(_) => {
            let _ = write_response(
                s,
                400,
                Some(
                    json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"Parse error."}}),
                ),
            );
            return;
        }
    };
    let out = if let Some(batch) = parsed.as_array() {
        if req.protocol_version.as_deref().unwrap_or("2025-03-26") != "2025-03-26"
            || batch.is_empty()
            || batch.len() > MAX_BATCH
        {
            Some(
                json!({"jsonrpc":"2.0","id":null,"error":{"code":-32600,"message":"Invalid Request."}}),
            )
        } else {
            let replies: Vec<Value> = batch
                .iter()
                .filter_map(|v| collaboration_mcp::dispatch(&task, &*handler, v.clone()))
                .collect();
            if replies.is_empty() {
                None
            } else {
                Some(Value::Array(replies))
            }
        }
    } else {
        collaboration_mcp::dispatch(&task, &*handler, parsed)
    };
    let _ = write_response(s, if out.is_some() { 200 } else { 202 }, out);
}
fn read_request(s: &mut TcpStream) -> Result<Request, (u16, String)> {
    let mut input = Vec::new();
    let mut chunk = [0; 4096];
    let at = Instant::now();
    let end = loop {
        if input.len() > MAX_HEADER {
            return Err((413, "Headers are too large.".into()));
        }
        let n = read_deadline(s, &mut chunk, at)?;
        if n == 0 {
            return Err((400, "Request ended before headers.".into()));
        }
        input.extend_from_slice(&chunk[..n]);
        if let Some(i) = input.windows(4).position(|v| v == b"\r\n\r\n") {
            if i + 4 > MAX_HEADER {
                return Err((413, "Headers are too large.".into()));
            }
            break i + 4;
        }
    };
    let header =
        std::str::from_utf8(&input[..end]).map_err(|_| (400, "Headers must be UTF-8.".into()))?;
    let mut lines = header.split("\r\n");
    let method = match lines.next() {
        Some("POST /mcp HTTP/1.1") => "POST",
        Some("GET /mcp HTTP/1.1") => "GET",
        _ => return Err((404, "MCP endpoint is /mcp.".into())),
    }
    .to_owned();
    let (mut length, mut token, mut authorization_seen) = (None, None, false);
    let mut origin_seen = false;
    let (mut host, mut content_type, mut accept, mut protocol_version) = (None, None, None, None);
    for line in lines {
        if line.is_empty() {
            break;
        }
        let (name, value) = line
            .split_once(':')
            .ok_or((400, "Malformed header.".into()))?;
        let (name, value) = (name.trim().to_ascii_lowercase(), value.trim());
        match name.as_str() {
            "content-length" => {
                if length.is_some() {
                    return Err((400, "Ambiguous content length.".into()));
                }
                let n = value
                    .parse::<usize>()
                    .map_err(|_| (400, "Invalid content length.".into()))?;
                if n > MAX_BODY {
                    return Err((413, "Request body is too large.".into()));
                }
                length = Some(n)
            }
            "authorization" => {
                if authorization_seen {
                    return Err((400, "Ambiguous authorization.".into()));
                }
                authorization_seen = true;
                token = value
                    .strip_prefix("Bearer ")
                    .filter(|v| !v.is_empty())
                    .map(str::to_owned)
            }
            "host" => {
                if host.replace(value.to_owned()).is_some() {
                    return Err((400, "Ambiguous host.".into()));
                }
            }
            "content-type" => {
                if content_type.replace(value.to_ascii_lowercase()).is_some() {
                    return Err((400, "Ambiguous content type.".into()));
                }
            }
            "accept" => {
                if accept.replace(value.to_ascii_lowercase()).is_some() {
                    return Err((400, "Ambiguous accept header.".into()));
                }
            }
            "mcp-protocol-version" => {
                if protocol_version.replace(value.to_owned()).is_some() {
                    return Err((400, "Ambiguous MCP protocol version.".into()));
                }
            }
            "transfer-encoding" => return Err((400, "Chunked requests are not supported.".into())),
            "origin" => {
                if origin_seen || !origin_ok(value) {
                    return Err((403, "Browser origin is not allowed.".into()));
                }
                origin_seen = true;
            }
            _ => {}
        }
    }
    if !host.as_deref().is_some_and(loopback_host) {
        return Err((400, "Host must be loopback.".into()));
    }
    if method == "POST" {
        if !content_type.as_deref().is_some_and(|value| {
            value
                .split(';')
                .next()
                .is_some_and(|kind| kind.trim() == "application/json")
        }) {
            return Err((415, "Content-Type must be application/json.".into()));
        }
        let accept = accept.unwrap_or_default();
        if !accepts(&accept, "application/json") || !accepts(&accept, "text/event-stream") {
            return Err((
                406,
                "Accept must include application/json and text/event-stream.".into(),
            ));
        }
    }
    let token = token.ok_or((401, "Bearer authorization is required.".into()))?;
    let expected = if method == "POST" {
        length.ok_or((411, "Content-Length is required.".into()))?
    } else {
        length.unwrap_or(0)
    };
    let mut body = input[end..].to_vec();
    while body.len() < expected {
        let n = read_deadline(s, &mut chunk, at)?;
        if n == 0 {
            return Err((400, "Request ended before body.".into()));
        }
        body.extend_from_slice(&chunk[..n]);
    }
    if body.len() != expected {
        return Err((400, "Ambiguous request body.".into()));
    }
    Ok(Request {
        method,
        token,
        body,
        protocol_version,
    })
}
fn loopback_host(value: &str) -> bool {
    value == "localhost"
        || value == "127.0.0.1"
        || value.strip_prefix("localhost:").is_some_and(valid_port)
        || value.strip_prefix("127.0.0.1:").is_some_and(valid_port)
}
fn valid_port(value: &str) -> bool {
    value.parse::<u16>().is_ok_and(|port| port != 0)
}
fn origin_ok(v: &str) -> bool {
    let Some((scheme, authority)) = v.split_once("://") else {
        return false;
    };
    if !matches!(scheme, "http" | "https") || authority.contains(['/', '?', '#', '@']) {
        return false;
    }
    loopback_host(authority)
}
fn accepts(value: &str, expected: &str) -> bool {
    value.split(',').any(|part| {
        part.split(';')
            .next()
            .is_some_and(|kind| kind.trim() == expected)
    })
}
fn read_deadline(s: &mut TcpStream, b: &mut [u8], at: Instant) -> Result<usize, (u16, String)> {
    let left = REQUEST_DEADLINE
        .checked_sub(at.elapsed())
        .ok_or((408, "Request timed out.".into()))?;
    s.set_read_timeout(Some(left.min(IO_TIMEOUT)))
        .map_err(|_| (400, "Could not configure request timeout.".into()))?;
    s.read(b).map_err(|e| match e.kind() {
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => {
            (408, "Request timed out.".into())
        }
        _ => (400, "Could not read request.".into()),
    })
}
fn write_response(mut s: TcpStream, status: u16, body: Option<Value>) -> std::io::Result<()> {
    let has_body = body.is_some();
    let mut bytes = body
        .map(|v| {
            serde_json::to_vec(&v)
                .unwrap_or_else(|_| b"{\"error\":\"Response encoding failed.\"}".to_vec())
        })
        .unwrap_or_default();
    if bytes.len() > MAX_BODY {
        bytes = b"{\"error\":\"Monitter broker response exceeded 128 KiB.\"}".to_vec();
    }
    let reason = match status {
        200 => "OK",
        202 => "Accepted",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        406 => "Not Acceptable",
        408 => "Request Timeout",
        411 => "Length Required",
        413 => "Payload Too Large",
        415 => "Unsupported Media Type",
        429 => "Too Many Requests",
        _ => "Error",
    };
    let content_type = if has_body {
        "Content-Type: application/json\r\n"
    } else {
        ""
    };
    write!(s,"HTTP/1.1 {status} {reason}\r\n{content_type}Content-Length: {}\r\nConnection: close\r\n\r\n",bytes.len())?;
    s.write_all(&bytes)?;
    let _ = s.shutdown(Shutdown::Both);
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    fn addr(e: &str) -> &str {
        e.trim_start_matches("http://").trim_end_matches("/mcp")
    }
    fn raw(e: &str, r: &str) -> String {
        let mut s = TcpStream::connect(addr(e)).unwrap();
        s.write_all(r.as_bytes()).unwrap();
        let mut x = String::new();
        s.read_to_string(&mut x).unwrap();
        x
    }
    fn call(e: &str, t: &str, v: Value) -> String {
        let b = serde_json::to_vec(&v).unwrap();
        raw(e,&format!("POST /mcp HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {t}\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\nMCP-Protocol-Version: 2025-03-26\r\nContent-Length: {}\r\n\r\n{}",b.len(),String::from_utf8(b).unwrap()))
    }
    #[test]
    fn auth_lifecycle_and_revoke() {
        let n = Arc::new(AtomicUsize::new(0));
        let seen = n.clone();
        let b = Broker::start(Arc::new(move |task, _, _| {
            seen.fetch_add(1, Ordering::SeqCst);
            Ok(json!({"task":task}))
        }))
        .unwrap();
        let g = b.session("caller");
        assert!(call(
            &g.endpoint,
            "no",
            json!({"jsonrpc":"2.0","id":1,"method":"ping"})
        )
        .starts_with("HTTP/1.1 401"));
        assert!(call(
            &g.endpoint,
            &g.token,
            json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1"}}})
        )
        .contains("protocolVersion"));
        assert!(call(&g.endpoint,&g.token,json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"list_agents","arguments":{}}})).contains("caller"));
        b.revoke(&g.token);
        assert!(call(
            &g.endpoint,
            &g.token,
            json!({"jsonrpc":"2.0","id":3,"method":"ping"})
        )
        .starts_with("HTTP/1.1 401"));
    }
    #[test]
    fn origin_get_and_malformed() {
        let b = Broker::start(Arc::new(|_, _, _| Ok(json!({})))).unwrap();
        let g = b.session("x");
        assert!(raw(&g.endpoint,"POST /mcp HTTP/1.1\r\nAuthorization: Bearer x\r\nOrigin: https://evil.example\r\nContent-Length: 2\r\n\r\n{}").starts_with("HTTP/1.1 403"));
        assert!(call(
            &g.endpoint,
            &g.token,
            json!({"jsonrpc":"2.0","method":"notifications/initialized"})
        )
        .starts_with("HTTP/1.1 202"));
        assert!(raw(
            &g.endpoint,
            &format!(
                "GET /mcp HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {}\r\n\r\n",
                g.token
            )
        )
        .starts_with("HTTP/1.1 405"));
    }

    #[test]
    fn validates_mcp_headers_and_preserves_tool_errors() {
        let b = Broker::start(Arc::new(|_, _, _| {
            Err("Choose a permitted recipient.".into())
        }))
        .unwrap();
        let g = b.session("x");
        let body = r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#;
        assert!(raw(&g.endpoint, &format!("POST /mcp HTTP/1.1\r\nHost: example.test\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\nContent-Length: {}\r\n\r\n{body}", g.token, body.len())).starts_with("HTTP/1.1 400"));
        assert!(raw(&g.endpoint, &format!("POST /mcp HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {}\r\nContent-Type: text/plain\r\nAccept: application/json, text/event-stream\r\nContent-Length: {}\r\n\r\n{body}", g.token, body.len())).starts_with("HTTP/1.1 415"));
        assert!(call(&g.endpoint, &g.token, json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"list_messages","arguments":{}}})).contains("Choose a permitted recipient."));
        assert!(raw(&g.endpoint, &format!("POST /mcp HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer one\r\nAuthorization: Bearer two\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\nContent-Length: 2\r\n\r\n{{}}" )).starts_with("HTTP/1.1 400"));
        assert!(!origin_ok("http://localhost:123@evil.example"));
        assert!(!origin_ok("http://127.0.0.1:80/path"));
        assert!(!accepts(
            "xapplication/json, text/event-stream",
            "application/json"
        ));
    }

    #[test]
    fn rejects_ambiguous_or_oversized_http_input_before_dispatch() {
        let b = Broker::start(Arc::new(|_, _, _| Ok(json!({})))).unwrap();
        let g = b.session("x");
        let headers = format!(
            "Host: localhost\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\n",
            g.token
        );
        assert!(raw(
            &g.endpoint,
            &format!("POST /mcp HTTP/1.1\r\n{headers}Content-Length: 131073\r\n\r\n")
        )
        .starts_with("HTTP/1.1 413"));
        assert!(raw(
            &g.endpoint,
            &format!(
                "POST /mcp HTTP/1.1\r\n{headers}Content-Length: 2\r\nContent-Length: 2\r\n\r\n{{}}"
            )
        )
        .starts_with("HTTP/1.1 400"));
        assert!(raw(
            &g.endpoint,
            &format!("POST /mcp HTTP/1.1\r\n{headers}Origin: http://localhost\r\nOrigin: http://localhost\r\nContent-Length: 2\r\n\r\n{{}}")
        )
        .starts_with("HTTP/1.1 403"));
    }

    #[test]
    fn version_and_batch_rules_are_explicit() {
        let b = Broker::start(Arc::new(|_, _, _| Ok(json!({})))).unwrap();
        let g = b.session("x");
        let batch = r#"[{"jsonrpc":"2.0","id":1,"method":"ping"}]"#;
        let request = |version: &str| {
            raw(&g.endpoint, &format!(
                "POST /mcp HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\nMCP-Protocol-Version: {version}\r\nContent-Length: {}\r\n\r\n{batch}",
                g.token, batch.len()
            ))
        };
        assert!(request("2025-03-26").starts_with("HTTP/1.1 200"));
        assert!(request("2025-06-18").starts_with("HTTP/1.1 200"));
        assert!(request("2025-06-18").contains("-32600"));
        assert!(request("2025-11-25").starts_with("HTTP/1.1 400"));
    }

    #[test]
    fn fragmented_json_request_is_read_to_completion() {
        let b = Broker::start(Arc::new(|_, _, _| Ok(json!({"ok":true})))).unwrap();
        let g = b.session("x");
        let body = b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}";
        let mut stream = TcpStream::connect(addr(&g.endpoint)).unwrap();
        write!(stream, "POST /mcp HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\nMCP-Protocol-Version: 2025-06-18\r\nContent-Length: {}\r\n\r\n", g.token, body.len()).unwrap();
        stream.write_all(&body[..11]).unwrap();
        thread::sleep(Duration::from_millis(10));
        stream.write_all(&body[11..]).unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    }

    #[test]
    fn concurrency_is_bounded() {
        let entered = Arc::new(AtomicUsize::new(0));
        let release = Arc::new(AtomicBool::new(false));
        let seen = entered.clone();
        let gate = release.clone();
        let b = Broker::start(Arc::new(move |_, _, _| {
            seen.fetch_add(1, Ordering::SeqCst);
            while !gate.load(Ordering::Acquire) {
                thread::sleep(Duration::from_millis(1));
            }
            Ok(json!({}))
        }))
        .unwrap();
        let g = b.session("x");
        let mut streams = Vec::new();
        for _ in 0..MAX_CONCURRENT {
            let mut stream = TcpStream::connect(addr(&g.endpoint)).unwrap();
            let body = b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{\"name\":\"list_messages\",\"arguments\":{}}}";
            write!(stream, "POST /mcp HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\nContent-Length: {}\r\n\r\n", g.token, body.len()).unwrap();
            stream.write_all(body).unwrap();
            streams.push(stream);
        }
        let deadline = Instant::now() + Duration::from_secs(1);
        while entered.load(Ordering::SeqCst) != MAX_CONCURRENT {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(5));
        }
        assert!(call(
            &g.endpoint,
            &g.token,
            json!({"jsonrpc":"2.0","id":9,"method":"ping"})
        )
        .starts_with("HTTP/1.1 429"));
        release.store(true, Ordering::Release);
        drop(streams);
    }
}
