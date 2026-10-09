//! Versioned, bounded same-user Unix IPC. No TCP endpoint or LAN grants.
use super::*;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

const VERSION: u32 = 1;
const MAX_FRAME: usize = 32 * 1024 * 1024;
const MAX_CONNECTIONS: usize = 32;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Hello {
    version: u32,
    profile: String,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Welcome {
    version: u32,
    instance_id: String,
    pid: u32,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Request {
    id: String,
    method: String,
    args: Value,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Response {
    id: String,
    result: Value,
    error: Option<String>,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Event {
    instance_id: String,
    changed: bool,
}

fn write_frame(writer: &mut impl std::io::Write, value: &impl Serialize) -> Result<(), String> {
    let bytes = serde_json::to_vec(value).map_err(|_| "Cannot encode private IPC frame.")?;
    if bytes.is_empty() || bytes.len() > MAX_FRAME {
        return Err("Private IPC frame exceeds the 32 MiB limit.".into());
    }
    writer.write_all(&(bytes.len() as u32).to_be_bytes()).map_err(|_| "Private IPC connection lost; delivery may be ambiguous. Refresh state before retrying a mutation.")?;
    writer.write_all(&bytes).map_err(|_| "Private IPC connection lost; delivery may be ambiguous. Refresh state before retrying a mutation.".into())
}
fn read_frame<T: DeserializeOwned>(reader: &mut impl std::io::Read) -> Result<T, String> {
    let mut size = [0u8; 4];
    reader.read_exact(&mut size).map_err(|_| "Private IPC connection lost; delivery may be ambiguous. Refresh state before retrying a mutation.")?;
    let size = u32::from_be_bytes(size) as usize;
    if size == 0 || size > MAX_FRAME {
        return Err("Invalid private IPC frame size.".into());
    }
    let mut bytes = vec![0u8; size];
    reader.read_exact(&mut bytes).map_err(|_| "Private IPC response incomplete; delivery may be ambiguous. Refresh state before retrying a mutation.")?;
    serde_json::from_slice(&bytes).map_err(|_| "Invalid private IPC frame.".into())
}

#[cfg(unix)]
mod unix {
    use super::*;
    use std::{
        fs::{File, OpenOptions},
        io::ErrorKind,
        os::{
            fd::AsRawFd,
            unix::{
                fs::{DirBuilderExt, FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt},
                net::{UnixListener, UnixStream},
            },
        },
        path::Path,
        sync::Mutex,
        thread,
        time::{Duration, Instant},
    };

    fn owner_metadata(path: &Path, directory: bool) -> Result<std::fs::Metadata, String> {
        let metadata =
            std::fs::symlink_metadata(path).map_err(|_| "Private IPC path is unavailable.")?;
        if metadata.file_type().is_symlink()
            || (directory && !metadata.is_dir())
            || metadata.uid() != unsafe { libc::geteuid() }
        {
            return Err(
                "Private IPC path must be owned by the current user and cannot be a link.".into(),
            );
        }
        if metadata.mode() & 0o077 != 0 {
            return Err("Private IPC path permissions must exclude other users.".into());
        }
        Ok(metadata)
    }
    fn secure_profile(profile: &Path, create: bool) -> Result<PathBuf, String> {
        super::super::validate_profile(profile)?;
        if create && !profile.exists() {
            std::fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(profile)
                .map_err(|_| "Cannot create experimental profile.")?;
            std::fs::set_permissions(profile, std::fs::Permissions::from_mode(0o700))
                .map_err(|_| "Cannot protect experimental profile.")?;
        }
        // Existing profile permissions are tightened without changing file contents.
        if create {
            let meta = std::fs::symlink_metadata(profile)
                .map_err(|_| "Experimental profile unavailable.")?;
            if meta.uid() != unsafe { libc::geteuid() } || meta.file_type().is_symlink() {
                return Err("Experimental profile has invalid ownership.".into());
            }
            std::fs::set_permissions(profile, std::fs::Permissions::from_mode(0o700))
                .map_err(|_| "Cannot protect experimental profile.")?;
        }
        owner_metadata(profile, true)?;
        std::fs::canonicalize(profile)
            .map_err(|_| "Cannot resolve experimental profile real path.".into())
    }
    fn socket_directory(profile: &Path) -> Result<PathBuf, String> {
        use sha2::{Digest, Sha256};
        use std::os::unix::ffi::OsStrExt;
        let digest = Sha256::digest(profile.as_os_str().as_bytes());
        let profile_key = digest[..8]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let root = std::fs::canonicalize("/tmp").map_err(|_| "Cannot resolve private IPC root.")?;
        Ok(root.join(format!("monitter-gpui-{}-{profile_key}", unsafe {
            libc::geteuid()
        })))
    }
    fn socket_path(profile: &Path, create: bool) -> Result<PathBuf, String> {
        // macOS sockaddr_un paths are capped at 104 bytes. A short private tmp
        // directory is derived from the canonical profile (not user input).
        let dir = socket_directory(profile)?;
        if create {
            match std::fs::DirBuilder::new().mode(0o700).create(&dir) {
                Ok(()) => std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))
                    .map_err(|_| "Cannot protect private IPC directory.")?,
                Err(e) if e.kind() == ErrorKind::AlreadyExists => {}
                Err(_) => return Err("Cannot create private IPC directory.".into()),
            }
        }
        owner_metadata(&dir, true)?;
        let canonical =
            std::fs::canonicalize(&dir).map_err(|_| "Cannot resolve private IPC directory.")?;
        if canonical != dir {
            return Err("Private IPC directory cannot contain linked path components.".into());
        }
        Ok(dir.join("backend.sock"))
    }
    pub(crate) fn server_availability(profile: &Path) -> Result<BackendAvailability, String> {
        super::super::validate_profile(profile)?;
        match std::fs::symlink_metadata(profile) {
            Err(error) if error.kind() == ErrorKind::NotFound => {
                return Ok(BackendAvailability::Absent);
            }
            Err(_) => return Err("Cannot inspect experimental profile for startup.".into()),
            Ok(_) => {}
        }
        let profile = secure_profile(profile, false)?;
        // Inspect absence separately from security errors: only a missing
        // private directory or endpoint is a reason to start a backend.
        let directory = socket_directory(&profile)?;
        match std::fs::symlink_metadata(&directory) {
            Err(error) if error.kind() == ErrorKind::NotFound => {
                return Ok(BackendAvailability::Absent);
            }
            Err(_) => return Err("Cannot inspect private IPC directory for startup.".into()),
            Ok(_) => {}
        }
        let socket = socket_path(&profile, false)?;
        match std::fs::symlink_metadata(&socket) {
            Err(error) if error.kind() == ErrorKind::NotFound => {
                return Ok(BackendAvailability::Absent);
            }
            Err(_) => return Err("Cannot inspect private IPC endpoint for startup.".into()),
            Ok(_) => {}
        }
        let metadata = owner_metadata(&socket, false)?;
        if !metadata.file_type().is_socket() {
            return Err("Private IPC endpoint is not a socket.".into());
        }
        match UnixStream::connect(&socket) {
            Ok(stream) => { authenticate(&stream)?; Ok(BackendAvailability::Present) },
            Err(error) if matches!(error.kind(), ErrorKind::NotFound | ErrorKind::ConnectionRefused) => Ok(BackendAvailability::Absent),
            Err(_) => Err("Private IPC endpoint connection failed; backend startup was not authorized by absence.".into()),
        }
    }
    fn authenticate(stream: &UnixStream) -> Result<(), String> {
        #[cfg(any(target_os = "macos", target_os = "freebsd", target_os = "openbsd"))]
        {
            let mut uid = 0;
            let mut gid = 0;
            if unsafe { libc::getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) } != 0
                || uid != unsafe { libc::geteuid() }
            {
                return Err("Private IPC peer is not the current user.".into());
            }
        }
        #[cfg(target_os = "linux")]
        {
            let mut credentials: libc::ucred = unsafe { std::mem::zeroed() };
            let mut size = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
            if unsafe {
                libc::getsockopt(
                    stream.as_raw_fd(),
                    libc::SOL_SOCKET,
                    libc::SO_PEERCRED,
                    &mut credentials as *mut _ as *mut libc::c_void,
                    &mut size,
                )
            } != 0
                || credentials.uid != unsafe { libc::geteuid() }
            {
                return Err("Private IPC peer is not the current user.".into());
            }
        }
        #[cfg(not(any(
            target_os = "macos",
            target_os = "freebsd",
            target_os = "openbsd",
            target_os = "linux"
        )))]
        return Err("Same-user private IPC is unsupported on this platform.".into());
        Ok(())
    }
    struct Lease(File);
    impl Lease {
        fn acquire(profile: &Path) -> Result<Self, String> {
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
                .open(profile.join("gpui-backend.lock"))
                .map_err(|_| "Cannot open experimental server lease.")?;
            let meta = file
                .metadata()
                .map_err(|_| "Cannot inspect experimental server lease.")?;
            if !meta.is_file()
                || meta.uid() != unsafe { libc::geteuid() }
                || meta.mode() & 0o077 != 0
                || meta.nlink() != 1
            {
                return Err("Experimental server lease must be a private regular file.".into());
            }
            if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
                return Err(
                    "The experimental backend server is already running or starting.".into(),
                );
            }
            Ok(Self(file))
        }
    }
    impl Drop for Lease {
        fn drop(&mut self) {
            unsafe {
                libc::flock(self.0.as_raw_fd(), libc::LOCK_UN);
            }
        }
    }

    struct ServerState {
        backend: BackendService,
        profile: PathBuf,
        instance: String,
        stopping: AtomicBool,
        connections: Arc<AtomicUsize>,
        // Remains held by connection workers until they have finished. The
        // backend's existing database lease independently protects SQLite.
        _lease: Lease,
    }
    pub struct DesktopServer {
        state: Arc<ServerState>,
        socket: PathBuf,
        accept: Mutex<Option<thread::JoinHandle<()>>>,
    }
    impl DesktopServer {
        pub fn start() -> Result<Self, String> {
            let profile = secure_profile(&super::super::profile_dir()?, true)?;
            let lease = Lease::acquire(&profile)?;
            // Acquire the server lease before any service/database startup.
            let backend = BackendService::open()?;
            Self::bind(profile, lease, backend)
        }
        fn bind(profile: PathBuf, lease: Lease, backend: BackendService) -> Result<Self, String> {
            let socket = socket_path(&profile, true)?;
            if let Ok(metadata) = std::fs::symlink_metadata(&socket) {
                owner_metadata(&socket, false)?;
                if !metadata.file_type().is_socket() {
                    return Err("Private IPC endpoint is not a socket.".into());
                }
                // Only the exclusive lease holder may retire a stale endpoint.
                std::fs::remove_file(&socket)
                    .map_err(|_| "Cannot retire stale private IPC socket.")?;
            }
            let listener = UnixListener::bind(&socket)
                .map_err(|_| "Cannot bind experimental backend IPC socket.")?;
            std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600))
                .map_err(|_| "Cannot protect private IPC socket.")?;
            listener
                .set_nonblocking(true)
                .map_err(|_| "Cannot configure private IPC socket.")?;
            let state = Arc::new(ServerState {
                backend,
                profile,
                instance: uuid::Uuid::new_v4().to_string(),
                stopping: AtomicBool::new(false),
                connections: Arc::new(AtomicUsize::new(0)),
                _lease: lease,
            });
            let worker = Arc::clone(&state);
            let accept = thread::Builder::new()
                .name("gpui-backend-ipc".into())
                .spawn(move || {
                    while !worker.stopping.load(Ordering::Acquire) {
                        match listener.accept() {
                            Ok((stream, _)) => {
                                if worker.connections.fetch_add(1, Ordering::AcqRel)
                                    >= MAX_CONNECTIONS
                                {
                                    worker.connections.fetch_sub(1, Ordering::AcqRel);
                                    continue;
                                }
                                let peer = Arc::clone(&worker);
                                let connections = Arc::clone(&worker.connections);
                                let result = thread::Builder::new()
                                    .name("gpui-backend-peer".into())
                                    // Match existing native backend workers: usage/
                                    // calendar/provider paths need a full Rust stack.
                                    .stack_size(8 * 1024 * 1024)
                                    .spawn(move || {
                                        let result = serve(stream, &peer);
                                        #[cfg(test)]
                                        if let Err(error) = &result {
                                            eprintln!("Private IPC peer ended: {error}");
                                        }
                                        #[cfg(not(test))]
                                        let _ = result;
                                        // Publish retirement only after releasing all
                                        // authoritative service/database ownership.
                                        drop(peer);
                                        connections.fetch_sub(1, Ordering::AcqRel);
                                    });
                                if result.is_err() {
                                    worker.connections.fetch_sub(1, Ordering::AcqRel);
                                }
                            }
                            Err(e) if e.kind() == ErrorKind::WouldBlock => {
                                thread::sleep(Duration::from_millis(25))
                            }
                            Err(_) => break,
                        }
                    }
                })
                .map_err(|_| "Cannot start private IPC worker.")?;
            Ok(Self {
                state,
                socket,
                accept: Mutex::new(Some(accept)),
            })
        }
        pub fn instance_id(&self) -> &str {
            &self.state.instance
        }
        /// Server/menu-bar action only. No IPC method exposes this capability.
        pub fn shutdown(&self) {
            if self.state.stopping.swap(true, Ordering::AcqRel) {
                return;
            }
            self.state.backend.shutdown();
            if let Ok(mut join) = self.accept.lock() {
                if let Some(join) = join.take() {
                    let _ = join.join();
                }
            }
            let _ = std::fs::remove_file(&self.socket);
        }
    }
    impl Drop for DesktopServer {
        fn drop(&mut self) {
            self.shutdown();
        }
    }

    fn serve(mut stream: UnixStream, state: &Arc<ServerState>) -> Result<(), String> {
        // BSD/macOS accepts inherit listener O_NONBLOCK. Restore blocking I/O
        // before framed reads: a normal gap between hello and request must not
        // become WouldBlock and disconnect a healthy client.
        stream
            .set_nonblocking(false)
            .map_err(|_| "Cannot configure blocking private IPC peer.")?;
        authenticate(&stream)?;
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .map_err(|_| "Cannot configure private IPC peer.")?;
        stream
            .set_write_timeout(Some(Duration::from_secs(10)))
            .map_err(|_| "Cannot configure private IPC peer.")?;
        let hello: Hello = read_frame(&mut stream)?;
        if hello.version != VERSION || hello.profile != state.profile.to_string_lossy() {
            return Err("Incompatible private IPC handshake.".into());
        }
        write_frame(
            &mut stream,
            &Welcome {
                version: VERSION,
                instance_id: state.instance.clone(),
                pid: std::process::id(),
            },
        )?;
        let request: Request = read_frame(&mut stream)
            .map_err(|error| format!("Private IPC request decode failed: {error}"))?;
        if uuid::Uuid::parse_str(&request.id).is_err() {
            return Err("Invalid private IPC request identifier.".into());
        }
        if state.stopping.load(Ordering::Acquire) {
            return Err("Experimental server is shutting down.".into());
        }
        if request.method == "subscribe_changes" {
            let changes = state.backend.subscribe_changes()?;
            write_frame(
                &mut stream,
                &Response {
                    id: request.id,
                    result: Value::Null,
                    error: None,
                },
            )?;
            // Mandatory initial wake closes the connect/subscribe race. Consumers
            // resolve any revision gap through the existing full snapshot path.
            write_frame(
                &mut stream,
                &Event {
                    instance_id: state.instance.clone(),
                    changed: true,
                },
            )?;
            let mut heartbeat = Instant::now();
            while !state.stopping.load(Ordering::Acquire) {
                let changed = changes.try_recv().is_ok();
                if changed || heartbeat.elapsed() >= Duration::from_secs(2) {
                    write_frame(
                        &mut stream,
                        &Event {
                            instance_id: state.instance.clone(),
                            changed,
                        },
                    )?;
                    heartbeat = Instant::now();
                }
                thread::sleep(Duration::from_millis(25));
            }
            return Ok(());
        }
        let result = dispatch(&state.backend, &request.method, request.args);
        let response = match result {
            Ok(result) => Response {
                id: request.id,
                result,
                error: None,
            },
            Err(error) => Response {
                id: request.id,
                result: Value::Null,
                error: Some(error),
            },
        };
        write_frame(&mut stream, &response)
            .map_err(|error| format!("Private IPC response write failed: {error}"))
    }

    #[derive(Debug)]
    struct ClientFailure { message: String, transient: bool, dispatched: bool }
    impl ClientFailure {
        fn permanent(message: impl Into<String>) -> Self { Self { message: message.into(), transient: false, dispatched: false } }
        fn io(error: std::io::Error, dispatched: bool) -> Self {
            let transient = matches!(error.kind(), ErrorKind::UnexpectedEof | ErrorKind::BrokenPipe | ErrorKind::ConnectionReset | ErrorKind::ConnectionAborted | ErrorKind::ConnectionRefused | ErrorKind::NotConnected | ErrorKind::TimedOut | ErrorKind::WouldBlock | ErrorKind::Interrupted);
            Self { message: if dispatched { "Private IPC connection lost; delivery may be ambiguous. Refresh state before retrying a mutation." } else { "Private IPC connection unavailable. Reconnect and try again." }.into(), transient, dispatched }
        }
    }
    fn client_write(stream: &mut UnixStream, value: &impl Serialize, dispatched: bool) -> Result<(), ClientFailure> {
        let bytes = serde_json::to_vec(value).map_err(|_| ClientFailure::permanent("Cannot encode private IPC frame."))?;
        if bytes.is_empty() || bytes.len() > MAX_FRAME { return Err(ClientFailure::permanent("Private IPC frame exceeds the 32 MiB limit.")); }
        use std::io::Write as _;
        stream.write_all(&(bytes.len() as u32).to_be_bytes()).map_err(|e| ClientFailure::io(e, dispatched))?;
        stream.write_all(&bytes).map_err(|e| ClientFailure::io(e, dispatched))
    }
    fn client_read<T: DeserializeOwned>(stream: &mut UnixStream, dispatched: bool) -> Result<T, ClientFailure> {
        use std::io::Read as _;
        let mut size = [0; 4];
        stream.read_exact(&mut size).map_err(|e| ClientFailure::io(e, dispatched))?;
        let size = u32::from_be_bytes(size) as usize;
        if size == 0 || size > MAX_FRAME { return Err(ClientFailure::permanent("Invalid private IPC frame size.")); }
        let mut bytes = vec![0; size];
        stream.read_exact(&mut bytes).map_err(|e| ClientFailure::io(e, dispatched))?;
        serde_json::from_slice(&bytes).map_err(|_| ClientFailure::permanent("Invalid private IPC frame."))
    }
    fn retry_client<T>(read_only: bool, mut attempt: impl FnMut() -> Result<T, ClientFailure>, mut delay: impl FnMut(Duration)) -> Result<T, String> {
        for index in 0..3 {
            match attempt() {
                Ok(value) => return Ok(value),
                Err(error) => {
                    if !error.transient || (error.dispatched && !read_only) { return Err(error.message); }
                    if index == 2 { return Err("Private IPC is temporarily unavailable after 3 attempts. Refresh or reconnect, then try again.".into()); }
                    delay(Duration::from_millis(if index == 0 { 150 } else { 450 }));
                }
            }
        }
        unreachable!()
    }
    fn read_only_request(method: &str, args: &Value) -> bool {
        match method {
            "read_ui_changes" | "read_idle_runtime_status" | "read_task_messages" | "read_channel_messages" | "read_attachment_file" | "load_native_preferences" | "read_lan_status" | "get_agent_extension_assignments" | "subscribe_changes" => true,
            "invoke" => matches!(args["command"].as_str(), Some("get_snapshot" | "get_ui_snapshot" | "get_ui_delta" | "get_process_metrics" | "get_task_messages" | "get_channel_messages" | "get_task_events" | "get_task_event_detail" | "get_usage_overview" | "get_task_goal" | "get_task_slash_commands" | "get_subagent_transcript" | "get_task_git_status" | "get_task_git_diff" | "list_terminals" | "read_terminal" | "get_command_capabilities")),
            _ => false,
        }
    }

    struct ClientState {
        profile: PathBuf,
        socket: PathBuf,
        instance: String,
        disconnected: AtomicBool,
    }
    #[derive(Clone)]
    pub(crate) struct Client(Arc<ClientState>);
    impl Client {
        pub(crate) fn connect(profile: &Path) -> Result<Self, String> {
            let profile = secure_profile(profile, false)?;
            let socket = socket_path(&profile, false)?;
            let (_, welcome) = retry_client(true, || connect_socket_typed(&profile, &socket), thread::sleep)?;
            Ok(Self(Arc::new(ClientState {
                profile,
                socket,
                instance: welcome.instance_id,
                disconnected: AtomicBool::new(false),
            })))
        }
        pub(crate) fn instance_id(&self) -> &str {
            &self.0.instance
        }
        pub(crate) fn disconnect(&self) {
            self.0.disconnected.store(true, Ordering::Release);
        }
        fn socket_typed(&self) -> Result<UnixStream, ClientFailure> {
            if self.0.disconnected.load(Ordering::Acquire) {
                return Err(ClientFailure::permanent("The experimental GUI client is disconnected."));
            }
            let (stream, welcome) = connect_socket_typed(&self.0.profile, &self.0.socket)?;
            if welcome.instance_id != self.0.instance {
                return Err(ClientFailure::permanent("Experimental backend was restarted. Reconnect and refresh state before issuing commands."));
            }
            Ok(stream)
        }
        pub(crate) fn call<T: DeserializeOwned>(
            &self,
            method: &str,
            args: Value,
        ) -> Result<T, String> {
            retry_client(read_only_request(method, &args), || {
                let mut socket = self.socket_typed()?;
                let id = uuid::Uuid::new_v4().to_string();
                request_on_socket_typed(&mut socket, &id, method, args.clone())
            }, thread::sleep)
        }
        #[cfg(test)]
        fn socket(&self) -> Result<UnixStream, String> {
            self.socket_typed().map_err(|e| e.message)
        }

        fn subscription_socket(&self) -> Result<UnixStream, String> {
            let socket = retry_client(true, || {
                let mut socket = self.socket_typed()?;
                let id = uuid::Uuid::new_v4().to_string();
                request_on_socket_typed::<()>(&mut socket, &id, "subscribe_changes", Value::Null)?;
                Ok(socket)
            }, thread::sleep)?;
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .map_err(|_| "Cannot configure private IPC subscription.")?;
            Ok(socket)
        }

        fn next_subscription_event(&self, socket: &mut UnixStream) -> Result<Event, String> {
            for attempt in 0..3 {
                match client_read::<Event>(socket, false) {
                    Ok(event) => return Ok(event),
                    Err(error) if error.transient && attempt < 2 => {
                        *socket = self.subscription_socket()?;
                    }
                    Err(error) => return Err(error.message),
                }
            }
            unreachable!()
        }

        pub(crate) fn subscribe(&self) -> Result<async_channel::Receiver<()>, String> {
            let mut socket = self.subscription_socket()?;
            let (sender, receiver) = async_channel::bounded(1);
            let client = Arc::downgrade(&self.0);
            thread::Builder::new()
                .name("gpui-client-changes".into())
                .spawn(move || {
                    let mut reconnect_backoff = 1;
                    loop {
                        let Some(client) = client.upgrade() else {
                            break;
                        };
                        if client.disconnected.load(Ordering::Acquire) || sender.is_closed() {
                            break;
                        }
                        // Reopen the read-only feed quietly. Its initial wake
                        // catches the GUI up through existing revision logic.
                        let event = match Client(client.clone()).next_subscription_event(&mut socket) {
                            Ok(event) => event,
                            Err(error) => {
                                if reconnect_backoff == 1 { let _ = sender.try_send(()); }
                                if !matches!(error.as_str(),
                                    "Private IPC connection unavailable. Reconnect and try again."
                                    | "Private IPC is temporarily unavailable after 3 attempts. Refresh or reconnect, then try again.") { break; }
                                // Keep the one existing feed worker asleep during
                                // a longer outage instead of abandoning updates.
                                thread::sleep(Duration::from_secs(reconnect_backoff));
                                reconnect_backoff = (reconnect_backoff * 2).min(16);
                                continue;
                            }
                        };
                        reconnect_backoff = 1;
                        if event.instance_id != client.instance {
                            break;
                        }
                        if event.changed {
                            let _ = sender.try_send(());
                        }
                    }
                    // Disconnection is visible as a closed feed; clients must obtain
                    // a fresh facade/snapshot, and never replay pending mutations.
                })
                .map_err(|_| "Cannot start private IPC subscription.")?;
            Ok(receiver)
        }
    }

    #[cfg(test)]
    fn request_on_socket<T: DeserializeOwned>(
        socket: &mut UnixStream,
        id: &str,
        method: &str,
        args: Value,
    ) -> Result<T, String> {
        // Install the long owner-action budget before sending: a fast server may
        // reply and close before a post-write socket option can be applied.
        // No automatic replay, including on timeout/partial response.
        socket
            .set_read_timeout(Some(Duration::from_secs(360)))
            .map_err(|_| "Cannot configure private IPC response.")?;
        write_frame(
            socket,
            &Request {
                id: id.to_owned(),
                method: method.to_owned(),
                args,
            },
        )?;
        decode_response::<T>(read_frame(socket)?, id)
    }
    fn request_on_socket_typed<T: DeserializeOwned>(socket: &mut UnixStream, id: &str, method: &str, args: Value) -> Result<T, ClientFailure> {
        if method != "subscribe_changes" {
            socket.set_read_timeout(Some(Duration::from_secs(360))).map_err(|_| ClientFailure::permanent("Cannot configure private IPC response."))?;
        }
        client_write(socket, &Request { id: id.to_owned(), method: method.to_owned(), args }, true)?;
        decode_response(client_read(socket, true)?, id).map_err(ClientFailure::permanent)
    }
    #[cfg(test)]
    fn connect_socket(profile: &Path, socket: &Path) -> Result<(UnixStream, Welcome), String> {
        connect_socket_typed(profile, socket).map_err(|e| e.message)
    }
    fn connect_socket_typed(profile: &Path, socket: &Path) -> Result<(UnixStream, Welcome), ClientFailure> {
        let metadata = owner_metadata(socket, false).map_err(ClientFailure::permanent)?;
        if !metadata.file_type().is_socket() {
            return Err(ClientFailure::permanent("Private IPC endpoint is not a socket."));
        }
        let mut stream = UnixStream::connect(socket).map_err(|e| ClientFailure::io(e, false))?;
        authenticate(&stream).map_err(ClientFailure::permanent)?;
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .map_err(|_| ClientFailure::permanent("Cannot configure private IPC connection."))?;
        stream
            .set_write_timeout(Some(Duration::from_secs(10)))
            .map_err(|_| ClientFailure::permanent("Cannot configure private IPC connection."))?;
        client_write(
            &mut stream,
            &Hello {
                version: VERSION,
                profile: profile.to_string_lossy().into_owned(),
            },
            false,
        )?;
        let welcome: Welcome = client_read(&mut stream, false)?;
        if welcome.version != VERSION
            || uuid::Uuid::parse_str(&welcome.instance_id).is_err()
            || welcome.pid == 0
        {
            return Err(ClientFailure::permanent("Incompatible experimental backend version/identity."));
        }
        Ok((stream, welcome))
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use serde_json::json;
        #[test]
        fn ipc_retry_policy_is_bounded_and_never_replays_ambiguous_mutations() {
            for (read_only, dispatched, semantic, expected) in [(true,false,false,3),(false,false,false,3),(true,true,false,3),(false,true,false,1),(true,false,true,1)] {
                let mut calls = 0;
                let mut delays = Vec::new();
                let result: Result<(), String> = retry_client(read_only, || { calls += 1; Err(ClientFailure { message: "fixture".into(), transient: !semantic, dispatched }) }, |d| delays.push(d));
                assert!(result.is_err());
                assert_eq!(calls, expected);
                if expected == 3 { assert_eq!(delays, vec![Duration::from_millis(150),Duration::from_millis(450)]); }
            }
            let mut calls = 0;
            assert_eq!(retry_client(true, || { calls += 1; if calls == 3 { Ok(42) } else { Err(ClientFailure::io(std::io::Error::from(ErrorKind::ConnectionReset), false)) } }, |_| {}).unwrap(), 42);
            assert_eq!(calls, 3);
        }
        #[test]
        fn ipc_retry_read_response_loss_reopens_peer_but_mutation_does_not() {
            for read_only in [false, true] {
                let mut calls = 0;
                let result: Result<Value,String> = retry_client(read_only, || {
                    calls += 1;
                    let (mut client, mut peer) = UnixStream::pair().unwrap();
                    let id = uuid::Uuid::new_v4().to_string();
                    let reply_id = id.clone();
                    let reply = calls == 3;
                    let worker = thread::spawn(move || {
                        let _: Request = read_frame(&mut peer).unwrap();
                        if reply { write_frame(&mut peer, &Response { id: reply_id, result: json!(42), error: None }).unwrap(); }
                    });
                    let result = request_on_socket_typed(&mut client, &id, "fixture", Value::Null);
                    worker.join().unwrap();
                    result
                }, |_| {});
                assert_eq!(calls, if read_only {3} else {1});
                assert_eq!(result.is_ok(), read_only);
            }
            assert!(!read_only_request("invoke", &json!({"command":"autoname"})));
            assert!(!read_only_request("invoke", &json!({"command":"rename_task"})));
            assert!(read_only_request("invoke", &json!({"command":"get_task_goal"})));
        }
        #[test]
        fn ipc_retry_backend_errors_are_not_transport_failures() {
            let mut calls = 0;
            let result: Result<Value, String> = retry_client(true, || {
                calls += 1;
                let (mut client, mut peer) = UnixStream::pair().unwrap();
                let id = uuid::Uuid::new_v4().to_string();
                let worker = thread::spawn(move || {
                    let request: Request = read_frame(&mut peer).unwrap();
                    write_frame(&mut peer, &Response { id: request.id, result: Value::Null,
                        error: Some("Private IPC connection lost inside a backend validation error".into()) }).unwrap();
                });
                let result = request_on_socket_typed(&mut client, &id, "read_ui_changes", Value::Null);
                worker.join().unwrap();
                result
            }, |_| panic!("Backend errors must never wait for retry"));
            assert!(result.is_err());
            assert_eq!(calls, 1);
            for command in ["send_message", "rename_task", "autoname", "write_terminal", "unknown"] {
                assert!(!read_only_request("invoke", &json!({"command": command})));
            }
            assert!(!read_only_request("write_terminal_input", &Value::Null));
        }
        fn fixture() -> (PathBuf, DesktopServer) {
            let root = std::env::temp_dir().join(format!("gpui-ipc-test-{}", uuid::Uuid::new_v4()));
            let profile = secure_profile(&root.join(APP_ID), true).unwrap();
            let lease = Lease::acquire(&profile).unwrap();
            let backend = BackendService(Arc::new(Session {
                lan_control: Mutex::new(()),
                preferences: NativePreferenceStore::new(profile.clone()),
                service: Service::open(None, profile.clone()).unwrap(),
            }));
            (root, DesktopServer::bind(profile, lease, backend).unwrap())
        }
        fn retire(root: PathBuf, server: DesktopServer) {
            server.shutdown();
            let deadline = Instant::now() + Duration::from_secs(3);
            while server.state.connections.load(Ordering::Acquire) != 0 && Instant::now() < deadline
            {
                std::thread::sleep(Duration::from_millis(10));
            }
            assert_eq!(
                server.state.connections.load(Ordering::Acquire),
                0,
                "fixture peer workers must retire before removing live SQLite files"
            );
            drop(server);
            std::fs::remove_dir_all(root).unwrap();
        }

        #[test]
        fn ipc_retry_change_feed_reopens_after_drop_and_wakes_for_catchup() {
            let (root, server) = fixture();
            let client = Client::connect(&server.state.profile).unwrap();
            let mut socket = client.subscription_socket().unwrap();
            assert!(client.next_subscription_event(&mut socket).unwrap().changed);
            socket.shutdown(std::net::Shutdown::Both).unwrap();
            let recovered = client.next_subscription_event(&mut socket).unwrap();
            assert!(recovered.changed, "resubscription must trigger snapshot catchup");
            assert_eq!(recovered.instance_id, client.instance_id());
            drop(socket);
            client.disconnect();
            drop(client);
            retire(root, server);
        }

        #[test]
        fn idle_runtime_diagnostics_are_read_only_and_private() {
            let (root, server) = fixture();
            let client = Client::connect(&server.state.profile).unwrap();
            let before: Value = client.call("read_ui_changes", json!({"revision":null})).unwrap();
            let status: Value = client.call("read_idle_runtime_status", Value::Null).unwrap();
            assert_eq!(status["idleTimeoutMs"], crate::runtime_gc::IDLE_RUNTIME_TIMEOUT.as_millis() as u64);
            assert_eq!(status["runtimes"], json!([]));
            let after: Value = client.call("read_ui_changes", json!({"revision":null})).unwrap();
            assert_eq!(before, after, "diagnostics must not mutate task/history state");
            let command_result: Result<Value, _> = client.call("invoke", json!({"command":"read_idle_runtime_status", "args":{}}));
            assert!(command_result.is_err(), "private diagnostics must not become a compatibility command");
            drop(client);
            retire(root, server);
        }
        #[test]
        fn client_reads_framed_response_when_peer_closes_immediately() {
            let (mut client, mut peer) = UnixStream::pair().unwrap();
            let responder = thread::spawn(move || {
                let request: Request = read_frame(&mut peer).unwrap();
                write_frame(
                    &mut peer,
                    &Response {
                        id: request.id,
                        result: json!({"accepted":true}),
                        error: None,
                    },
                )
                .unwrap();
                // Dropping the peer immediately after the frame models a fast
                // one-shot server response. The reply must remain readable.
            });

            let result: Value =
                request_on_socket(&mut client, "immediate-response", "fixture", Value::Null)
                    .unwrap();
            assert_eq!(result, json!({"accepted":true}));
            responder.join().unwrap();
        }
        #[test]
        fn startup_probe_distinguishes_absence_from_live_or_unsafe_endpoints() {
            let (root, server) = fixture();
            let profile = root.join(APP_ID);
            assert_eq!(
                server_availability(&profile).unwrap(),
                BackendAvailability::Present
            );
            let directory = server.socket.parent().unwrap();
            std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o755)).unwrap();
            assert!(server_availability(&profile).is_err());
            std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700)).unwrap();
            server.shutdown();
            assert_eq!(
                server_availability(&profile).unwrap(),
                BackendAvailability::Absent
            );
            retire(root, server);
        }
        #[test]
        fn request_can_arrive_after_a_normal_gap_following_handshake() {
            let (root, server) = fixture();
            let (mut socket, _) = connect_socket(&server.state.profile, &server.socket).unwrap();
            std::thread::sleep(Duration::from_millis(100));
            let id = uuid::Uuid::new_v4().to_string();
            write_frame(
                &mut socket,
                &Request {
                    id: id.clone(),
                    method: "read_ui_changes".into(),
                    args: serde_json::json!({"revision":null}),
                },
            )
            .unwrap();
            let state: UiDeltaResponse =
                decode_response(read_frame(&mut socket).unwrap(), &id).unwrap();
            assert!(state.snapshot.is_some());
            drop(socket);
            retire(root, server);
        }
        #[test]
        fn client_disconnect_and_reconnection_preserve_authority_and_preferences() {
            let (root, server) = fixture();
            let profile = root.join(APP_ID);
            let first = Client::connect(&profile).unwrap();
            let instance = first.instance_id().to_owned();
            let before: UiDeltaResponse = first
                .call("read_ui_changes", json!({"revision":null}))
                .unwrap();
            let agent = before.snapshot.unwrap().agents[0].id.clone();
            let created: Value = first.call("invoke", json!({"command":"create_task","args":{"input":{"agentId":agent,"title":"IPC reconnect fixture"}}})).unwrap();
            let prefs: NativePreferences =
                first.call("load_native_preferences", Value::Null).unwrap();
            let mut data = prefs.data;
            data.drafts.insert(
                created["id"].as_str().unwrap().to_owned(),
                "preserved draft".into(),
            );
            let saved: NativePreferences = first
                .call(
                    "save_native_preferences",
                    json!({"expectedRevision":prefs.revision,"data":data}),
                )
                .unwrap();
            let feed = first.subscribe().unwrap();
            first.disconnect();
            drop(first);
            let second = Client::connect(&profile).unwrap();
            assert_eq!(instance, second.instance_id());
            let after: UiDeltaResponse = second
                .call("read_ui_changes", json!({"revision":null}))
                .unwrap();
            assert!(after
                .snapshot
                .unwrap()
                .tasks
                .iter()
                .any(|task| task.id == created["id"].as_str().unwrap()));
            let restored: NativePreferences =
                second.call("load_native_preferences", Value::Null).unwrap();
            assert_eq!(restored.revision, saved.revision);
            assert_eq!(restored.data.drafts, saved.data.drafts);
            assert!(Lease::acquire(&server.state.profile).is_err());
            assert!(second.call::<Value>("shutdown", Value::Null).is_err());
            drop(feed);
            drop(second);
            retire(root, server);
        }
        #[test]
        fn lost_mutation_response_is_not_replayed_and_does_not_stop_server() {
            let (root, server) = fixture();
            let profile = root.join(APP_ID);
            let client = Client::connect(&profile).unwrap();
            let before: UiDeltaResponse = client
                .call("read_ui_changes", serde_json::json!({"revision":null}))
                .unwrap();
            let agent = before.snapshot.unwrap().agents[0].id.clone();
            let mut socket = client.socket().unwrap();
            write_frame(&mut socket, &Request {
                id: uuid::Uuid::new_v4().to_string(), method: "invoke".into(),
                args: serde_json::json!({"command":"create_task","args":{"input":{"agentId":agent,"title":"Ambiguous delivery fixture"}}}),
            }).unwrap();
            // Simulate a GUI exit after request delivery, before it can receive
            // the result. A new client reads committed state rather than replay.
            drop(socket);
            drop(client);
            let recovered = Client::connect(&profile).unwrap();
            let mut count = 0;
            for _ in 0..100 {
                let state: UiDeltaResponse = recovered
                    .call("read_ui_changes", serde_json::json!({"revision":null}))
                    .unwrap();
                count = state
                    .snapshot
                    .unwrap()
                    .tasks
                    .iter()
                    .filter(|task| task.title == "Ambiguous delivery fixture")
                    .count();
                if count != 0 {
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            assert_eq!(count, 1);
            assert_eq!(recovered.instance_id(), server.instance_id());
            drop(recovered);
            retire(root, server);
        }

        #[test]
        fn handshake_rejects_wrong_profile_and_protocol_before_dispatch() {
            let (root, server) = fixture();
            for hello in [
                Hello {
                    version: VERSION + 1,
                    profile: server.state.profile.to_string_lossy().into_owned(),
                },
                Hello {
                    version: VERSION,
                    profile: "com.monitter.desktop".into(),
                },
            ] {
                let mut socket = UnixStream::connect(&server.socket).unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(1)))
                    .unwrap();
                write_frame(&mut socket, &hello).unwrap();
                assert!(read_frame::<Welcome>(&mut socket).is_err());
            }
            retire(root, server);
        }
        #[test]
        fn linked_or_public_ipc_directories_are_rejected() {
            let (root, server) = fixture();
            let directory = server.socket.parent().unwrap();
            std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o755)).unwrap();
            assert!(Client::connect(&root.join(APP_ID)).is_err());
            std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700)).unwrap();
            retire(root, server);
        }
    }
}

fn decode_response<T: DeserializeOwned>(response: Response, id: &str) -> Result<T, String> {
    if response.id != id {
        return Err(
            "Private IPC response identifier mismatch; refresh state before retrying a mutation."
                .into(),
        );
    }
    match response.error {
        None => serde_json::from_value(response.result)
            .map_err(|_| "Invalid private IPC result.".into()),
        Some(error) if response.result.is_null() => Err(error),
        Some(_) => Err("Invalid private IPC response.".into()),
    }
}
fn dispatch(backend: &BackendService, method: &str, args: Value) -> Result<Value, String> {
    match method {
        "invoke" => backend.invoke(&arg::<String>(&args, "command")?, arg(&args, "args")?),
        "read_ui_changes" => encode(backend.read_ui_changes(optional::<String>(&args, "revision")?.as_deref())?),
        "read_idle_runtime_status" => backend.read_idle_runtime_status(),
        "read_task_messages" => encode(backend.read_task_messages(&arg::<String>(&args, "taskId")?, optional::<String>(&args, "beforeId")?.as_deref(), optional(&args, "limit")?)?),
        "read_channel_messages" => encode(backend.read_channel_messages(&arg::<String>(&args, "channelId")?, optional::<String>(&args, "beforeId")?.as_deref(), optional(&args, "limit")?)?),
        "read_attachment_file" => encode(backend.read_attachment_file(&arg::<String>(&args, "sourcePath")?)?),
        "store_selected_attachment" => encode(backend.store_selected_attachment(&arg::<String>(&args, "taskId")?, &arg::<String>(&args, "expectedHostId")?, &arg::<String>(&args, "expectedCwd")?, arg(&args, "file")?, optional(&args, "previewDataUrl")?)?),
        "write_terminal_input" => { backend.write_terminal_input(&arg::<String>(&args, "id")?, &arg::<Vec<u8>>(&args, "bytes")?)?; Ok(Value::Null) },
        "load_native_preferences" => encode(backend.load_native_preferences()?),
        "import_project_history" => encode(backend.import_project_history(arg(&args, "input")?)?),
        "validate_project_history" => { backend.validate_project_history(arg(&args, "input")?)?; Ok(Value::Null) },
        "save_native_preferences" => encode(backend.save_native_preferences(&arg::<String>(&args, "expectedRevision")?, arg(&args, "data")?)?),
        "save_schedule" => encode(backend.save_schedule(arg(&args, "input")?)?),
        "schedule_action" => encode(backend.schedule_action(arg(&args, "id")?, arg(&args, "action")?)?),
        "read_lan_status" => encode(backend.read_lan_status()?),
        "start_lan" => encode(backend.start_lan()?),
        "stop_lan" => encode(backend.stop_lan()?),
        "get_agent_extension_assignments" => encode(backend.get_agent_extension_assignments(&arg::<String>(&args, "agentId")?)?),
        "set_agent_extension_assignment" => encode(backend.set_agent_extension_assignment(&arg::<String>(&args, "agentId")?, &arg::<String>(&args, "entryId")?, arg(&args, "kind")?, arg(&args, "assigned")?, &arg::<String>(&args, "expectedRevision")?)?),
        _ => Err("Unknown private native method. Server shutdown is available only in the menu-bar server.".into()),
    }
}

#[cfg(unix)]
pub(crate) use unix::server_availability;
#[cfg(not(unix))]
pub(crate) fn server_availability(_: &std::path::Path) -> Result<BackendAvailability, String> {
    Err("Experimental backend IPC currently supports macOS/Unix only.".into())
}

#[cfg(unix)]
pub(crate) use unix::Client;
#[cfg(unix)]
pub use unix::DesktopServer;

#[cfg(not(unix))]
pub struct DesktopServer;
#[cfg(not(unix))]
impl DesktopServer {
    pub fn start() -> Result<Self, String> {
        Err("Experimental backend IPC currently supports macOS/Unix only.".into())
    }
    pub fn shutdown(&self) {}
    pub fn instance_id(&self) -> &str {
        ""
    }
}
#[cfg(not(unix))]
#[derive(Clone)]
pub(crate) struct Client;
#[cfg(not(unix))]
impl Client {
    pub(crate) fn connect(_: &std::path::Path) -> Result<Self, String> {
        Err("Experimental backend IPC currently supports macOS/Unix only.".into())
    }
    pub(crate) fn instance_id(&self) -> &str {
        ""
    }
    pub(crate) fn disconnect(&self) {}
    pub(crate) fn call<T: DeserializeOwned>(&self, _: &str, _: Value) -> Result<T, String> {
        Err("Experimental backend IPC is unsupported.".into())
    }
    pub(crate) fn subscribe(&self) -> Result<async_channel::Receiver<()>, String> {
        Err("Experimental backend IPC is unsupported.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn successful_void_responses_survive_json_null_roundtrip() {
        let mut wire = Vec::new();
        write_frame(
            &mut wire,
            &Response {
                id: "void".into(),
                result: Value::Null,
                error: None,
            },
        )
        .unwrap();
        let response: Response = read_frame(&mut &wire[..]).unwrap();
        decode_response::<()>(response, "void").unwrap();
    }
    #[test]
    fn rejects_oversized_and_truncated_frames_and_mismatched_responses() {
        let oversized = (MAX_FRAME as u32 + 1).to_be_bytes();
        assert!(read_frame::<Value>(&mut &oversized[..]).is_err());
        assert!(read_frame::<Value>(&mut &[0, 0, 0, 2, b'{'][..]).is_err());
        assert!(decode_response::<Value>(
            Response {
                id: "other".into(),
                result: Value::Null,
                error: None
            },
            "expected"
        )
        .is_err());
        assert!(decode_response::<Value>(
            Response {
                id: "expected".into(),
                result: serde_json::json!({"unexpected":true}),
                error: Some("contradiction".into())
            },
            "expected"
        )
        .is_err());
    }
}
