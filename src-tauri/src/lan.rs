//! Small, deliberately dependency-free HTTP server for the owner LAN view.
//! It is not a general web server: connections are bounded, one request each,
//! and the only mutable endpoint is the authenticated command bridge.
use crate::Service;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    fs,
    io::{Read, Write},
    net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream},
    path::{Component, Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

pub const PORT: u16 = 18_436;
const MAX_HEADER: usize = 16 * 1024;
/// Attachments are accepted by the ordinary bridge and can be up to 20 MiB;
/// base64 plus JSON needs headroom while still imposing a hard request cap.
const MAX_BODY: usize = 32 * 1024 * 1024;
const MAX_CONNECTIONS: usize = 24;
const MAX_FAILED_AUTH_ATTEMPTS: usize = 5;
const AUTH_COOLDOWN: Duration = Duration::from_secs(5 * 60);
/// Trusted-LAN temporary mode. Keep the access-code machinery in place so it
/// can be re-enabled without changing the HTTP contract.
pub const REQUIRE_ACCESS_CODE: bool = false;
const DEV_BRIDGE_HEADER: &str = "x-monitter-dev-bridge";

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Info {
    pub urls: Vec<String>,
    pub token: String,
    pub access_code_required: bool,
    pub error: Option<String>,
}

pub struct Server {
    stop: Arc<AtomicBool>,
    info: Info,
}

impl Server {
    pub fn start(service: Arc<Service>, asset_roots: Vec<PathBuf>) -> Result<Self, String> {
        let listener = TcpListener::bind((Ipv4Addr::UNSPECIFIED, PORT))
            .map_err(|e| format!("LAN server could not bind port {PORT}: {e}"))?;
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        let token = random_token()?;
        let stop = Arc::new(AtomicBool::new(false));
        let urls = lan_addresses()
            .into_iter()
            .map(|ip| format!("http://{ip}:{PORT}/"))
            .collect();
        let info = Info {
            urls,
            token: token.clone(),
            access_code_required: REQUIRE_ACCESS_CODE,
            error: None,
        };
        let thread_stop = Arc::clone(&stop);
        let active = Arc::new(AtomicUsize::new(0));
        let auth_limiter = Arc::new(Mutex::new(AuthRateLimiter::default()));
        thread::Builder::new()
            .name("monitter-lan".into())
            .spawn(move || {
                while !thread_stop.load(Ordering::Acquire) {
                    match listener.accept() {
                        Ok((stream, peer)) => {
                            if active.fetch_add(1, Ordering::AcqRel) >= MAX_CONNECTIONS {
                                active.fetch_sub(1, Ordering::AcqRel);
                                let _ = reply_busy(stream);
                                continue;
                            }
                            let service = Arc::clone(&service);
                            let token = token.clone();
                            let roots = asset_roots.clone();
                            let active = Arc::clone(&active);
                            let auth_limiter = Arc::clone(&auth_limiter);
                            thread::spawn(move || {
                                handle(
                                    stream,
                                    peer,
                                    service,
                                    &token,
                                    &roots,
                                    &auth_limiter,
                                    REQUIRE_ACCESS_CODE,
                                );
                                active.fetch_sub(1, Ordering::AcqRel);
                            });
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(40))
                        }
                        Err(_) => break,
                    }
                }
            })
            .map_err(|e| format!("Could not start LAN server: {e}"))?;
        Ok(Self { stop, info })
    }
    pub fn info(&self) -> Info {
        self.info.clone()
    }
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Release);
    }
}

fn reply_busy(mut stream: TcpStream) -> Result<(), String> {
    reply(
        &mut stream,
        503,
        "text/plain",
        b"Server busy",
        Some("no-store"),
    )
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop();
    }
}

#[derive(Deserialize)]
struct Invoke {
    command: String,
    #[serde(default)]
    args: Value,
}

fn handle(
    mut stream: TcpStream,
    peer: SocketAddr,
    service: Arc<Service>,
    token: &str,
    roots: &[PathBuf],
    auth_limiter: &Mutex<AuthRateLimiter>,
    require_access_code: bool,
) {
    // BSD/macOS can inherit O_NONBLOCK from the listening socket. Each worker
    // uses blocking read/write_all with timeouts, including multi-MiB snapshots.
    // Leaving it nonblocking can truncate responses as soon as the buffer fills.
    if stream.set_nonblocking(false).is_err() {
        return;
    }
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(10)));
    let result = (|| -> Result<(), String> {
        let request = read_request(&mut stream)?;
        if !local_ip(peer.ip()) {
            return reply(&mut stream, 403, "text/plain", b"LAN clients only", None);
        }
        // Host must be a literal local address. This avoids trusting a DNS name
        // that could be rebound after the browser has received the owner token.
        if !valid_host(request.headers.get("host")) {
            return reply(&mut stream, 421, "text/plain", b"Invalid Host", None);
        }
        let developer_bridge = request
            .headers
            .get(DEV_BRIDGE_HEADER)
            .is_some_and(|value| value == "1");
        if request.method == "GET" && request.path == "/api/access" {
            return reply(
                &mut stream,
                200,
                "application/json",
                serde_json::json!({"required": require_access_code || developer_bridge})
                    .to_string()
                    .as_bytes(),
                Some("no-store"),
            );
        }
        if request.method == "POST" && request.path == "/api/invoke" {
            if !same_origin(
                request.headers.get("origin"),
                request.headers.get("host"),
                developer_bridge,
            ) {
                return reply(
                    &mut stream,
                    403,
                    "application/json",
                    br#"{"ok":false,"error":"Invalid Origin"}"#,
                    None,
                );
            }
            // The lock check, code comparison, and failed-attempt increment are
            // one critical section. Otherwise simultaneous requests could all
            // compare before the fifth failure establishes the server-wide lock.
            let auth = if require_access_code || developer_bridge {
                let mut limiter = auth_limiter
                    .lock()
                    .map_err(|_| "LAN access limiter unavailable.".to_string())?;
                let now = Instant::now();
                if limiter.is_locked_at(now) {
                    AuthCheck::Locked
                } else if authorized(request.headers.get("authorization"), token) {
                    AuthCheck::Authorized
                } else {
                    limiter.record_failure_at(now);
                    AuthCheck::Unauthorized
                }
            } else {
                AuthCheck::Authorized
            };
            match auth {
                AuthCheck::Locked => {
                    return reply(
                        &mut stream,
                        429,
                        "application/json",
                        br#"{"ok":false,"error":"Too many LAN access-code attempts. Try again in five minutes."}"#,
                        Some("no-store"),
                    );
                }
                AuthCheck::Unauthorized => {
                    return reply(
                        &mut stream,
                        401,
                        "application/json",
                        br#"{"ok":false,"error":"Unauthorized"}"#,
                        None,
                    );
                }
                AuthCheck::Authorized => {}
            }
            let invoke: Invoke = serde_json::from_slice(&request.body)
                .map_err(|_| "Invalid invoke JSON.".to_string())?;
            let body = match service.lan_invoke(&invoke.command, invoke.args) {
                Ok(result) => serde_json::json!({"ok":true,"result" : result}),
                Err(error) => serde_json::json!({"ok":false,"error":error}),
            };
            return reply(
                &mut stream,
                200,
                "application/json",
                serde_json::to_string(&body).unwrap().as_bytes(),
                Some("no-store"),
            );
        }
        if request.method != "GET" && request.method != "HEAD" {
            return reply(&mut stream, 405, "text/plain", b"Method Not Allowed", None);
        }
        let path = if request.path == "/" {
            "index.html"
        } else {
            request.path.trim_start_matches('/')
        };
        let Some(file) = asset_file(roots, path) else {
            return reply(&mut stream, 404, "text/plain", b"Not Found", None);
        };
        let mut bytes = fs::read(&file).map_err(|_| "Asset unavailable.".to_string())?;
        let content_type = content_type(&file);
        if content_type == "text/html; charset=utf-8" {
            bytes = inject_lan_marker(bytes);
        }
        if request.method == "HEAD" {
            bytes.clear();
        }
        reply(&mut stream, 200, content_type, &bytes, Some("no-store"))
    })();
    if let Err(error) = result {
        let _ = reply(
            &mut stream,
            400,
            "application/json",
            serde_json::json!({"ok":false,"error":error})
                .to_string()
                .as_bytes(),
            Some("no-store"),
        );
    }
}

enum AuthCheck {
    Authorized,
    Unauthorized,
    Locked,
}

#[derive(Default)]
struct AuthRateLimiter {
    failed_attempts: std::collections::VecDeque<Instant>,
    locked_until: Option<Instant>,
}

impl AuthRateLimiter {
    fn is_locked_at(&mut self, now: Instant) -> bool {
        if let Some(until) = self.locked_until {
            if now < until {
                return true;
            }
            self.locked_until = None;
        }
        self.discard_expired_at(now);
        false
    }

    fn record_failure_at(&mut self, now: Instant) {
        if self.is_locked_at(now) {
            return;
        }
        self.discard_expired_at(now);
        self.failed_attempts.push_back(now);
        if self.failed_attempts.len() >= MAX_FAILED_AUTH_ATTEMPTS {
            self.locked_until = Some(now + AUTH_COOLDOWN);
        }
    }

    fn discard_expired_at(&mut self, now: Instant) {
        while self
            .failed_attempts
            .front()
            .is_some_and(|attempt| now.duration_since(*attempt) >= AUTH_COOLDOWN)
        {
            self.failed_attempts.pop_front();
        }
    }
}

struct Request {
    method: String,
    path: String,
    headers: std::collections::HashMap<String, String>,
    body: Vec<u8>,
}
fn read_request(stream: &mut TcpStream) -> Result<Request, String> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 2048];
    while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
        let n = stream
            .read(&mut chunk)
            .map_err(|_| "Could not read request.".to_string())?;
        if n == 0 || buf.len() + n > MAX_HEADER {
            return Err("Invalid request headers.".into());
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    let split = buf.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
    let head =
        std::str::from_utf8(&buf[..split]).map_err(|_| "Invalid request encoding.".to_string())?;
    let mut lines = head.split("\r\n");
    let first = lines.next().ok_or("Missing request line.")?;
    let mut parts = first.split_whitespace();
    let method = parts.next().ok_or("Missing method.")?.to_string();
    let path = parts
        .next()
        .ok_or("Missing path.")?
        .split('?')
        .next()
        .unwrap_or("/")
        .to_string();
    if parts.next().is_none() || path.contains("..") {
        return Err("Invalid request path.".into());
    }
    let mut headers = std::collections::HashMap::new();
    for line in lines {
        if let Some((key, value)) = line.split_once(':') {
            headers.insert(key.trim().to_ascii_lowercase(), value.trim().to_string());
        }
    }
    let length = headers
        .get("content-length")
        .map(|v| v.parse::<usize>().map_err(|_| "Invalid content length."))
        .transpose()?
        .unwrap_or(0);
    if length > MAX_BODY {
        return Err("Request body too large.".into());
    }
    let mut body = buf[split..].to_vec();
    while body.len() < length {
        let n = stream
            .read(&mut chunk)
            .map_err(|_| "Could not read request body.".to_string())?;
        if n == 0 {
            return Err("Truncated request body.".into());
        }
        body.extend_from_slice(&chunk[..n]);
    }
    body.truncate(length);
    Ok(Request {
        method,
        path,
        headers,
        body,
    })
}
fn reply(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &[u8],
    cache: Option<&str>,
) -> Result<(), String> {
    let reason = match status {
        200 => "OK",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        421 => "Misdirected Request",
        429 => "Too Many Requests",
        503 => "Service Unavailable",
        _ => "Bad Request",
    };
    write!(stream,"HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\nX-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\n",body.len()).map_err(|e|e.to_string())?;
    if let Some(cache) = cache {
        write!(stream, "Cache-Control: {cache}\r\n").map_err(|e| e.to_string())?;
    }
    stream
        .write_all(b"\r\n")
        .and_then(|_| stream.write_all(body))
        .map_err(|e| e.to_string())
}
fn authorized(value: Option<&String>, token: &str) -> bool {
    value
        .map(|v| {
            v.strip_prefix("Bearer ")
                .map(|t| constant_time_eq(t.as_bytes(), token.as_bytes()))
                .unwrap_or(false)
        })
        .unwrap_or(false)
}
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    };
    a.iter().zip(b).fold(0u8, |d, (x, y)| d | (*x ^ *y)) == 0
}
fn valid_host(host: Option<&String>) -> bool {
    let Some(host) = host else { return false };
    let host = host.strip_suffix(&format!(":{PORT}")).unwrap_or(host);
    host.parse::<IpAddr>().map(local_ip).unwrap_or(false)
}
fn local_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => ip.is_private() || ip.is_loopback(),
        IpAddr::V6(ip) => ip.is_loopback() || ip.is_unique_local(),
    }
}
fn same_origin(origin: Option<&String>, host: Option<&String>, required: bool) -> bool {
    let Some(origin) = origin else {
        return !required;
    };
    let Some(host) = host else { return false };
    origin == &format!("http://{host}")
}
fn asset_file(roots: &[PathBuf], request: &str) -> Option<PathBuf> {
    let path = Path::new(request);
    if path
        .components()
        .any(|c| !matches!(c, Component::Normal(_)))
    {
        return None;
    };
    roots
        .iter()
        .map(|root| root.join(path))
        .find(|p| p.is_file())
        .or_else(|| {
            roots
                .iter()
                .map(|root| root.join("index.html"))
                .find(|p| p.is_file())
        })
}
fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|x| x.to_str()).unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" => "application/javascript",
        "css" => "text/css; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "woff2" => "font/woff2",
        "json" => "application/json",
        _ => "application/octet-stream",
    }
}
fn inject_lan_marker(mut bytes: Vec<u8>) -> Vec<u8> {
    let marker = b"<meta name=\"monitter-lan\" content=\"1\">";
    // The document attribute survives SvelteKit hydration; the meta marker is
    // retained for a simple pre-hydration detection path.
    if let Some(html) = bytes
        .windows(5)
        .position(|w| w.eq_ignore_ascii_case(b"<html"))
    {
        if let Some(end) = bytes[html..].iter().position(|byte| *byte == b'>') {
            let end = html + end;
            if !bytes[html..end]
                .windows(b"data-monitter-lan".len())
                .any(|w| w == b"data-monitter-lan")
            {
                bytes.splice(end..end, b" data-monitter-lan=\"1\"".iter().copied());
            }
        }
    }
    if bytes.windows(marker.len()).any(|w| w == marker) {
        return bytes;
    };
    if let Some(at) = bytes
        .windows(6)
        .position(|w| w.eq_ignore_ascii_case(b"<head>"))
    {
        bytes.splice(at + 6..at + 6, marker.iter().copied());
    }
    bytes
}
fn random_token() -> Result<String, String> {
    let mut token = String::with_capacity(6);
    #[cfg(unix)]
    {
        let mut file = fs::File::open("/dev/urandom")
            .map_err(|e| format!("Could not create LAN access token: {e}"))?;
        // 250 is divisible by 10, so rejection sampling avoids modulo bias.
        // Keep this as a string to preserve the existing Info.token contract and
        // to retain leading zeroes in the owner-facing access code.
        while token.len() < 6 {
            let mut byte = [0u8; 1];
            file.read_exact(&mut byte)
                .map_err(|e| format!("Could not create LAN access token: {e}"))?;
            if byte[0] < 250 {
                token.push(char::from(b'0' + byte[0] % 10));
            }
        }
    }
    Ok(token)
}
fn lan_addresses() -> Vec<IpAddr> {
    let mut result = vec![IpAddr::V4(Ipv4Addr::LOCALHOST)];
    if let Ok(output) = std::process::Command::new("/sbin/ifconfig").output() {
        let text = String::from_utf8_lossy(&output.stdout);
        for line in text.lines() {
            let words: Vec<_> = line.split_whitespace().collect();
            if words.get(0) == Some(&"inet") {
                if let Some(ip) = words.get(1).and_then(|v| v.parse::<Ipv4Addr>().ok()) {
                    if !ip.is_loopback() && !result.contains(&IpAddr::V4(ip)) {
                        result.push(IpAddr::V4(ip));
                    }
                }
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    #[test]
    fn rejects_non_literal_host_and_escape() {
        assert!(!valid_host(Some(&"evil.test:18436".into())));
        assert!(valid_host(Some(&"192.168.1.9:18436".into())));
        assert!(asset_file(&[PathBuf::from("/tmp")], "../x").is_none());
    }
    #[test]
    fn token_compare_requires_full_match() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
    }

    #[test]
    fn access_code_is_six_ascii_digits() {
        for _ in 0..32 {
            let code = random_token().unwrap();
            assert_eq!(code.len(), 6);
            assert!(code.bytes().all(|byte| byte.is_ascii_digit()));
        }
    }

    #[test]
    fn auth_limiter_locks_after_five_failures_and_expires_at_boundary() {
        let mut limiter = AuthRateLimiter::default();
        let start = Instant::now();
        for offset in 0..MAX_FAILED_AUTH_ATTEMPTS {
            limiter.record_failure_at(start + Duration::from_secs(offset as u64));
        }
        assert!(limiter.is_locked_at(start + Duration::from_secs(4)));
        assert!(limiter.is_locked_at(start + AUTH_COOLDOWN + Duration::from_secs(3)));
        assert!(!limiter.is_locked_at(start + AUTH_COOLDOWN + Duration::from_secs(4)));
        assert!(limiter.failed_attempts.is_empty());
    }

    #[test]
    fn auth_limiter_counts_concurrent_failures_server_wide() {
        let limiter = Arc::new(Mutex::new(AuthRateLimiter::default()));
        let start = Instant::now();
        let mut workers = Vec::new();
        for _ in 0..MAX_FAILED_AUTH_ATTEMPTS {
            let limiter = Arc::clone(&limiter);
            workers.push(thread::spawn(move || {
                limiter.lock().unwrap().record_failure_at(start);
            }));
        }
        for worker in workers {
            worker.join().unwrap();
        }
        assert!(limiter.lock().unwrap().is_locked_at(start));
    }

    #[test]
    fn live_assets_take_precedence_and_update_without_server_restart() {
        let root =
            std::env::temp_dir().join(format!("monitter-lan-roots-{}", uuid::Uuid::new_v4()));
        let live = root.join("lan-web");
        let bundled = root.join("bundled");
        fs::create_dir_all(&bundled).unwrap();
        fs::write(bundled.join("index.html"), "bundled index").unwrap();
        let roots = [live.clone(), bundled.clone()];

        // Before a live publish exists, the bundled UI remains available.
        let selected = asset_file(&roots, "index.html").unwrap();
        assert_eq!(fs::read_to_string(selected).unwrap(), "bundled index");

        fs::create_dir_all(&live).unwrap();
        fs::write(live.join("index.html"), "live version one").unwrap();
        let selected = asset_file(&roots, "index.html").unwrap();
        assert_eq!(fs::read_to_string(selected).unwrap(), "live version one");

        // asset_file and the request handler read from disk per request, so a
        // later atomic index replacement is observed without a server restart.
        fs::write(live.join("index.html"), "live version two").unwrap();
        let selected = asset_file(&roots, "index.html").unwrap();
        assert_eq!(fs::read_to_string(selected).unwrap(), "live version two");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn developer_bridge_requires_a_code_without_changing_unrestricted_lan_mode() {
        let root =
            std::env::temp_dir().join(format!("monitter-lan-access-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let service = Service::open(None, root.join("state")).unwrap();
        let limiter = Arc::new(Mutex::new(AuthRateLimiter::default()));
        let body = r#"{"command":"get_snapshot","args":{}}"#;
        let request = format!(
            "POST /api/invoke HTTP/1.1\r\nHost: 127.0.0.1:18436\r\nAuthorization: Bearer wrong\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        );
        let unrestricted = one_request_with_limiter(
            Arc::clone(&service),
            root.clone(),
            "owner-token",
            &request,
            Arc::clone(&limiter),
            false,
        );
        assert!(unrestricted.starts_with("HTTP/1.1 200"));
        assert!(limiter.lock().unwrap().failed_attempts.is_empty());

        let dev_bridge_unauthorized = one_request_with_limiter(
            Arc::clone(&service),
            root.clone(),
            "owner-token",
            &format!(
                "POST /api/invoke HTTP/1.1\r\nHost: 127.0.0.1:18436\r\nOrigin: http://127.0.0.1:18436\r\nX-Monitter-Dev-Bridge: 1\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            ),
            Arc::clone(&limiter),
            false,
        );
        assert!(dev_bridge_unauthorized.starts_with("HTTP/1.1 401"));

        let dev_bridge_missing_origin = one_request_with_limiter(
            Arc::clone(&service),
            root.clone(),
            "owner-token",
            &format!(
                "POST /api/invoke HTTP/1.1\r\nHost: 127.0.0.1:18436\r\nX-Monitter-Dev-Bridge: 1\r\nAuthorization: Bearer owner-token\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            ),
            Arc::clone(&limiter),
            false,
        );
        assert!(dev_bridge_missing_origin.starts_with("HTTP/1.1 403"));

        let dev_bridge_access = one_request_with_limiter(
            Arc::clone(&service),
            root.clone(),
            "owner-token",
            "GET /api/access HTTP/1.1\r\nHost: 127.0.0.1:18436\r\nX-Monitter-Dev-Bridge: 1\r\n\r\n",
            Arc::clone(&limiter),
            false,
        );
        assert!(dev_bridge_access.starts_with("HTTP/1.1 200"));
        assert!(dev_bridge_access.contains("\"required\":true"));

        let protected_limiter = Arc::new(Mutex::new(AuthRateLimiter::default()));
        for _ in 0..MAX_FAILED_AUTH_ATTEMPTS {
            let denied = one_request_with_limiter(
                Arc::clone(&service),
                root.clone(),
                "owner-token",
                &request,
                Arc::clone(&protected_limiter),
                true,
            );
            assert!(denied.starts_with("HTTP/1.1 401"));
        }
        let locked = one_request_with_limiter(
            Arc::clone(&service),
            root.clone(),
            "owner-token",
            &request,
            protected_limiter,
            true,
        );
        assert!(locked.starts_with("HTTP/1.1 429"));

        let invalid_host = one_request(
            service,
            root.clone(),
            "owner-token",
            "GET /api/access HTTP/1.1\r\nHost: not-a-lan-host\r\n\r\n",
        );
        assert!(invalid_host.starts_with("HTTP/1.1 421"));
        let _ = fs::remove_dir_all(root);
    }

    fn one_request(service: Arc<Service>, root: PathBuf, token: &str, request: &str) -> String {
        one_request_with_limiter(
            service,
            root,
            token,
            request,
            Arc::new(Mutex::new(AuthRateLimiter::default())),
            true,
        )
    }

    fn one_request_with_limiter(
        service: Arc<Service>,
        root: PathBuf,
        token: &str,
        request: &str,
        limiter: Arc<Mutex<AuthRateLimiter>>,
        require_access_code: bool,
    ) -> String {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let token = token.to_string();
        let thread = thread::spawn(move || {
            let (stream, peer) = listener.accept().unwrap();
            // Reproduce the accepted socket flags on macOS explicitly on all OSes.
            stream.set_nonblocking(true).unwrap();
            handle(
                stream,
                peer,
                service,
                &token,
                &[root],
                &limiter,
                require_access_code,
            );
        });
        let mut client = TcpStream::connect(address).unwrap();
        client.write_all(request.as_bytes()).unwrap();
        let mut response = String::new();
        client.read_to_string(&mut response).unwrap();
        thread.join().unwrap();
        response
    }

    #[test]
    fn http_requires_token_and_serves_authenticated_snapshot_and_assets() {
        let root = std::env::temp_dir().join(format!("monitter-lan-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("index.html"),
            "<html><head></head><body>Monitter</body></html>",
        )
        .unwrap();
        let service = Service::open(None, root.join("state")).unwrap();
        let body = r#"{"command":"get_snapshot","args":{}}"#;
        let unauthorized = one_request(
            Arc::clone(&service), root.clone(), "owner-token",
            &format!("POST /api/invoke HTTP/1.1\r\nHost: 127.0.0.1:18436\r\nContent-Length: {}\r\n\r\n{body}", body.len()),
        );
        assert!(unauthorized.starts_with("HTTP/1.1 401"));
        let authorized = one_request(
            Arc::clone(&service), root.clone(), "owner-token",
            &format!("POST /api/invoke HTTP/1.1\r\nHost: 127.0.0.1:18436\r\nAuthorization: Bearer owner-token\r\nContent-Length: {}\r\n\r\n{body}", body.len()),
        );
        assert!(authorized.starts_with("HTTP/1.1 200"));
        assert!(authorized.contains("\"ok\":true"));
        let asset = one_request(
            Arc::clone(&service),
            root.clone(),
            "owner-token",
            "GET / HTTP/1.1\r\nHost: 127.0.0.1:18436\r\n\r\n",
        );
        assert!(asset.starts_with("HTTP/1.1 200"));
        assert!(asset.contains("data-monitter-lan=\"1\""));
        assert!(asset.contains("monitter-lan\" content=\"1\""));
        let large_body = "x".repeat(8 * 1024 * 1024);
        fs::write(root.join("large.txt"), &large_body).unwrap();
        let large = one_request(
            service,
            root.clone(),
            "owner-token",
            "GET /large.txt HTTP/1.1\r\nHost: 127.0.0.1:18436\r\n\r\n",
        );
        let (headers, received) = large.split_once("\r\n\r\n").unwrap();
        assert!(headers.starts_with("HTTP/1.1 200"));
        assert_eq!(received, large_body);
        let _ = fs::remove_dir_all(root);
    }
}
