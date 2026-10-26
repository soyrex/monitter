//! Native, retained paired-controller host. Relay locators never authorize access.
//! All peer requests pass fresh ECDH proof, explicit controller/owner grants and
//! the backend's portable allow-list. Mutations are never replayed on reconnect.
use crate::pairing_crypto::{CryptoSession, PairingIdentity, random_bytes};
use base64::{Engine as _, engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD}};
use monitter_lib::experimental_desktop::DesktopService;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::{HashMap, VecDeque}, fs, io::{Read, Write}, net::{TcpListener, TcpStream, ToSocketAddrs}, path::{Path, PathBuf}, sync::{Arc, Mutex, OnceLock, atomic::{AtomicBool, AtomicU64, Ordering}, mpsc}, thread, time::{Duration, Instant, SystemTime, UNIX_EPOCH}};
use tungstenite::{Message, WebSocket, stream::MaybeTlsStream};
const DEFAULT_RELAY: &str = "wss://monitter-opaque-relay.soyrex.workers.dev/relay";
const MAX_OWNER: usize = 32 * 1024 * 1024;
const MAX_FRAME: usize = 2 * 1024 * 1024;
const MAX_CONTROLLER: usize = 1024 * 1024;
const CHUNK: usize = 512 * 1024;
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Status {
    pub enabled: bool,
    pub relay_connected: bool,
    pub connection_state: String,
    pub pin: Option<String>,
    pub error: Option<String>,
    pub pending: Vec<Pending>,
    pub devices: Vec<Device>,
    pub invitation: Option<String>,
    pub inactivity_days: Option<u32>,
    pub migration_note: Option<String>,
}
#[derive(Clone, PartialEq, Eq)]
pub struct Pending { pub id: String, pub name: String, pub sas: String, pub scope: String }
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Device { pub public_key: String, pub name: String, pub scope: String, pub last_access_at: u64 }
#[derive(Clone)]
pub enum Command {
    Approve { id: String, remember: bool }, Reject { id: String },
    Revoke { public_key: String, scope: String }, SetEnabled(bool), Retry,
    RotatePin, SetInactivityDays(Option<u32>),
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    version: u32, enabled: bool, relay: String, room: String,
    identity: PairingIdentity, management_token: String, pin: Option<String>,
    devices: Vec<Device>, inactivity_days: Option<u32>,
    #[serde(default = "terminal_default")] terminal_theme: String,
}
fn terminal_default()->String {"monitter".into()}
fn now()->u64 {SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64}
fn random_token()->Result<String,String> {Ok(URL_SAFE_NO_PAD.encode(random_bytes::<32>()?))}
fn valid_b64(value:&str,len:usize)->bool {URL_SAFE_NO_PAD.decode(value).is_ok_and(|bytes|bytes.len()==len && URL_SAFE_NO_PAD.encode(bytes)==value)}
fn relay_url(value:&str)->Result<url::Url,String>{
    let url=url::Url::parse(value).map_err(|_|"Invalid relay URL.")?;
    if url.path()!="/relay"||url.query().is_some()||url.fragment().is_some()||!url.username().is_empty()||url.password().is_some()|| !(url.scheme()=="wss" || url.scheme()=="ws"&&matches!(url.host_str(),Some("localhost"|"127.0.0.1"|"[::1]"|"::1"))) {return Err("Relay requires wss, or ws on loopback.".into());}Ok(url)
}
fn save(path:&Path,record:&Record)->Result<(),String>{
    let bytes=serde_json::to_vec(record).map_err(|e|format!("Pairing record could not be saved: {e}"))?;
    let temp=path.with_extension(format!("{}.tmp",uuid::Uuid::new_v4()));
    let mut options=fs::OpenOptions::new();options.write(true).create_new(true);
    #[cfg(unix)] {use std::os::unix::fs::OpenOptionsExt;options.mode(0o600);}
    let mut file=options.open(&temp).map_err(|e|format!("Pairing record could not be saved: {e}"))?;
    if let Err(error)=(||{file.write_all(&bytes)?;file.sync_all()?;fs::rename(&temp,path)})(){let _=fs::remove_file(&temp);return Err(format!("Pairing record could not be saved: {error}"));}Ok(())
}
fn load(path:&Path)->Result<Record,String>{
    if path.exists(){
        let meta=fs::symlink_metadata(path).map_err(|e|e.to_string())?;
        if !meta.is_file()||meta.file_type().is_symlink()||meta.len()>128*1024{return Err("Invalid native pairing record.".into());}
        #[cfg(unix)] {use std::os::unix::fs::PermissionsExt;if meta.permissions().mode()&0o077!=0{return Err("Native pairing record must have mode 0600.".into());}}
        let mut record:Record=serde_json::from_slice(&fs::read(path).map_err(|e|e.to_string())?).map_err(|_|"Native pairing record is invalid. Preserve it and reset pairing explicitly.")?;
        relay_url(&record.relay)?;
        if record.version!=1||!valid_b64(&record.room,18)||!valid_b64(&record.management_token,32)||record.devices.len()>64||record.inactivity_days.is_some_and(|days|days==0||days>36500)||record.pin.as_ref().is_some_and(|pin|pin.len()!=9||!pin.bytes().all(|b|b.is_ascii_digit()))||record.devices.iter().any(|d|!valid_b64(&d.public_key,65)||!matches!(d.scope.as_str(),"controller"|"owner")||d.name.len()>80){return Err("Native pairing policy is invalid.".into());}
        record.identity.public_key()?;
        // Both hostnames route to the same Cloudflare Worker and registry.
        // Use its reachable hostname without changing identity, room, PIN or trust.
        if record.relay == "wss://api.monitter.com/relay" {
            record.relay=DEFAULT_RELAY.into();save(path,&record)?;
        }
        return Ok(record);
    }
    let record=Record{version:1,enabled:false,relay:DEFAULT_RELAY.into(),room:URL_SAFE_NO_PAD.encode(random_bytes::<18>()?),identity:PairingIdentity::generate()?,management_token:random_token()?,pin:None,devices:vec![],inactivity_days:None,terminal_theme:terminal_default()};
    save(path,&record)?;Ok(record)
}
fn invitation(record:&Record)->Result<Value,String>{Ok(json!({"version":2,"relayUrl":record.relay,"room":record.room,"publicKey":record.identity.public_key()?,"directUrls":if DIRECT_AVAILABLE.load(Ordering::Relaxed){direct_urls()}else{Vec::<String>::new()}}))}
fn registry(record:&Record,delete:bool)->Result<Option<String>,String>{
    let mut endpoint=relay_url(&record.relay)?;let scheme=if endpoint.scheme()=="wss"{"https"}else{"http"};endpoint.set_scheme(scheme).map_err(|_|"Invalid pairing endpoint.")?;endpoint.set_path("/pairing");
    let client=reqwest::blocking::Client::builder().connect_timeout(Duration::from_secs(8)).timeout(Duration::from_secs(15)).redirect(reqwest::redirect::Policy::none()).build().map_err(|e|e.to_string())?;
    if delete {
        let Some(pin)=&record.pin else{return Ok(None);};endpoint.set_path(&format!("/pairing/{pin}"));
        let response=client.delete(endpoint).bearer_auth(&record.management_token).send().map_err(|_|"Could not revoke fixed pairing PIN. Retry before rotating.")?;
        return if response.status().is_success()||response.status().as_u16()==404{Ok(None)}else{Err("Pairing PIN revocation failed.".into())};
    }
    // Persistent POST is idempotent by the token saved before first dispatch.
    // It recovers the exact PIN after a lost response, including at capacity.
    // POST recovers a lost reservation response but intentionally does not
    // refresh an existing invitation. Renew saved PINs with authenticated PUT
    // so relay/direct candidates stay current without rotating the locator.
    let request=if let Some(pin)=&record.pin {
        endpoint.set_path(&format!("/pairing/{pin}"));
        client.put(endpoint).json(&json!({"invitation":invitation(record)?}))
    } else {
        client.post(endpoint).json(&json!({"invitation":invitation(record)?,"persistent":true}))
    };
    let response=request.bearer_auth(&record.management_token).send().map_err(|_|"Could not register fixed pairing PIN. Retry uses the same saved management token.")?;
    if !response.status().is_success(){return Err(format!("Fixed PIN service returned HTTP {}. The relay may require an update.",response.status().as_u16()));}
    let mut bytes=Vec::new();response.take(4097).read_to_end(&mut bytes).map_err(|e|e.to_string())?;
    if bytes.len()>4096{return Err("Pairing registry response exceeds its limit.".into());}
    let body:Value=serde_json::from_slice(&bytes).map_err(|_|"Invalid pairing registry response.")?;
    let pin=body["code"].as_str().filter(|pin|pin.len()==9&&pin.bytes().all(|b|b.is_ascii_digit())).ok_or("Invalid pairing PIN response.")?;
    if body["persistent"]!=true||!body["expiresAt"].is_null()||body["revocationToken"]!=record.management_token{return Err("Relay did not confirm a persistent PIN.".into());}
    if record.pin.as_ref().is_some_and(|saved| saved != pin) { return Err("The relay returned a different PIN. The saved PIN was preserved; reconnect or explicitly reset pairing.".into()); }
    Ok(Some(pin.into()))
}
struct RuntimeCore { commands:mpsc::SyncSender<Command>, status:Arc<Mutex<Status>> }
#[derive(Clone)]
pub struct RuntimeHandle { core:Arc<RuntimeCore> }
static RUNTIMES:OnceLock<Mutex<HashMap<PathBuf,Arc<RuntimeCore>>>>=OnceLock::new();
impl RuntimeHandle {
    pub fn start(service:DesktopService)->Result<Self,String>{
        let path=monitter_lib::experimental_desktop::profile_dir()?.join("native-pairing-v1.json");
        let mut runtimes=RUNTIMES.get_or_init(||Mutex::new(HashMap::new())).lock().map_err(|_|"Pairing runtime registry is unavailable.")?;
        if let Some(core)=runtimes.get(&path).cloned(){return Ok(Self{core});}
        let record=load(&path)?;let status=Arc::new(Mutex::new(Status::default()));let (tx,rx)=mpsc::sync_channel(64);let shared=status.clone();
        // reqwest's rustls-no-provider mode deliberately does not choose a TLS
        // provider, even when ring is compiled elsewhere in the dependency graph.
        // Install the existing ring provider before either HTTPS or WSS starts.
        // Another native service may already have installed the process provider.
        let _ = rustls::crypto::ring::default_provider().install_default();
        let key=path.clone();thread::Builder::new().name("native-phone-pairing".into()).spawn(move||{
            let stopped=shared.clone();
            if std::panic::catch_unwind(std::panic::AssertUnwindSafe(||run(service,path,record,rx,shared))).is_err(){
                if let Ok(mut status)=stopped.lock(){
                    status.relay_connected=false;status.pending.clear();
                    status.error=Some("Phone pairing stopped unexpectedly. Restart Monitter to reconnect.".into());
                }
            }
        }).map_err(|e|e.to_string())?;
        let core=Arc::new(RuntimeCore{commands:tx,status});runtimes.insert(key,core.clone());Ok(Self{core})
    }
    pub fn command(&self,command:Command)->Result<(),String>{self.core.commands.try_send(command).map_err(|_|"Pairing runtime is busy or stopped.".into())}
    pub fn status(&self)->Status {self.core.status.lock().map(|s|s.clone()).unwrap_or_else(|_|Status{error:Some("Pairing runtime status is unavailable.".into()),..Status::default()})}
}
struct Transfer { size:usize, data:Vec<u8>, index:u64, deadline:Instant }
struct Receipt { fingerprint:String, response:Option<Value> }
struct Completed { generation:u64, epoch:u64, id:String, owner:bool, result:Result<Value,String>, appearance:bool }
struct Connection {
    socket:WebSocket<MaybeTlsStream<TcpStream>>, direct:bool, crypto:CryptoSession, epoch:u64,
    joined:bool, heartbeat:bool, ping_at:Instant, pong_deadline:Option<Instant>, pairing_deadline:Instant,
    requested:bool, approved:bool, owner_requested:bool, owner:bool,
    remembered_controller:bool, remembered_owner:bool, large_chunks:bool,
    transfers:HashMap<String,Transfer>, uploads:HashMap<String,ControllerUpload>, receipts:HashMap<String,Receipt>, active:usize,
    outgoing:VecDeque<Value>, sent:VecDeque<Instant>, generation:Arc<AtomicU64>,
}
impl Drop for Connection {fn drop(&mut self){self.generation.fetch_add(1,Ordering::SeqCst);}}
fn revoke_owner(c:&mut Connection){c.generation.fetch_add(1,Ordering::SeqCst);c.owner=false;c.owner_requested=false;c.transfers.clear();c.outgoing.retain(|v|!v["type"].as_str().is_some_and(|t|t.starts_with("owner-response-")||t=="owner-workspace-approved"));}
fn connect(record:&Record,epoch:u64)->Result<Connection,String>{
    let mut url=relay_url(&record.relay)?;url.query_pairs_mut().append_pair("room",&record.room).append_pair("role","desktop");
    let host=url.host_str().ok_or("Relay hostname is missing.")?;let port=url.port_or_known_default().ok_or("Relay port is missing.")?;
    // Bound TCP/TLS/HTTP handshakes. DNS resolution is delegated to the OS.
    let addresses=(host,port).to_socket_addrs().map_err(|_|"Could not resolve pairing relay.")?;
    let deadline=Instant::now()+Duration::from_secs(10);let mut tcp=None;for address in addresses.take(8){let remaining=deadline.saturating_duration_since(Instant::now());if remaining.is_zero(){break;}if let Ok(stream)=TcpStream::connect_timeout(&address,remaining.min(Duration::from_secs(5))){tcp=Some(stream);break;}}
    let tcp=tcp.ok_or("Could not connect to pairing relay.")?;tcp.set_read_timeout(Some(Duration::from_secs(10))).map_err(|e|e.to_string())?;tcp.set_write_timeout(Some(Duration::from_secs(10))).map_err(|e|e.to_string())?;
    let config=tungstenite::protocol::WebSocketConfig::default().max_message_size(Some(MAX_FRAME)).max_frame_size(Some(MAX_FRAME));
    let (mut socket,_)=tungstenite::client_tls_with_config(url.as_str(),tcp,Some(config),None).map_err(|_|"Secure relay handshake failed.")?;
    match socket.get_mut(){MaybeTlsStream::Plain(tcp)=>{tcp.set_read_timeout(Some(Duration::from_millis(100))).map_err(|e|e.to_string())?;},MaybeTlsStream::Rustls(tls)=>{tls.sock.set_read_timeout(Some(Duration::from_millis(100))).map_err(|e|e.to_string())?;},_=>return Err("Unsupported pairing TLS stream.".into())}
    socket.send(Message::Text(json!({"type":"join","room":record.room,"role":"desktop","capabilities":["relay-heartbeat-v1"]}).to_string().into())).map_err(|e|e.to_string())?;
    new_connection(record,epoch,socket,false)
}
fn new_connection(record:&Record,epoch:u64,socket:WebSocket<MaybeTlsStream<TcpStream>>,direct:bool)->Result<Connection,String>{
    Ok(Connection{socket,direct,crypto:CryptoSession::new(&record.identity)?,epoch,joined:false,heartbeat:direct,ping_at:Instant::now(),pong_deadline:None,pairing_deadline:Instant::now()+Duration::from_secs(300),requested:false,approved:false,owner_requested:false,owner:false,remembered_controller:false,remembered_owner:false,large_chunks:false,transfers:HashMap::new(),uploads:HashMap::new(),receipts:HashMap::new(),active:0,outgoing:VecDeque::new(),sent:VecDeque::new(),generation:Arc::new(AtomicU64::new(0))})
}
// Direct transport carries the identical encrypted protocol and grants. The
// listener is deliberately separate from the unauthenticated LAN web bridge.
// Backend LAN HTTP/TLS already use 19436/19437.
const DIRECT_PORT:u16=19438;
static DIRECT_AVAILABLE:AtomicBool=AtomicBool::new(false);
fn direct_urls()->Vec<String>{
    let mut urls=Vec::new();
    #[cfg(unix)]
    unsafe {
        let mut head: *mut libc::ifaddrs=std::ptr::null_mut();
        if libc::getifaddrs(&mut head)==0 {
            let mut current=head;
            while !current.is_null() {
                let entry=&*current;
                if !entry.ifa_addr.is_null() && (*entry.ifa_addr).sa_family as i32==libc::AF_INET {
                    let address=&*(entry.ifa_addr as *const libc::sockaddr_in);
                    let ip=std::net::Ipv4Addr::from(address.sin_addr.s_addr.to_ne_bytes());
                    if ip.is_private() && entry.ifa_flags as i32 & libc::IFF_UP != 0 {
                        let url=format!("ws://{ip}:{DIRECT_PORT}/pairing/direct");
                        if !urls.contains(&url){urls.push(url);}
                    }
                }
                current=entry.ifa_next;
            }
            libc::freeifaddrs(head);
        }
    }
    urls.truncate(3);urls
}
fn accept_direct(listener:&TcpListener,record:&Record,epoch:u64)->Result<Option<Connection>,String>{
    let (tcp,peer)=match listener.accept(){Ok(value)=>value,Err(e) if e.kind()==std::io::ErrorKind::WouldBlock=>return Ok(None),Err(e)=>return Err(e.to_string())};
    if !matches!(peer.ip(),std::net::IpAddr::V4(ip) if ip.is_private()){return Ok(None);}
    tcp.set_read_timeout(Some(Duration::from_millis(250))).map_err(|e|e.to_string())?;
    tcp.set_write_timeout(Some(Duration::from_millis(250))).map_err(|e|e.to_string())?;
    let config=tungstenite::protocol::WebSocketConfig::default().max_message_size(Some(MAX_FRAME)).max_frame_size(Some(MAX_FRAME));
    let callback=|request:&tungstenite::handshake::server::Request,response:tungstenite::handshake::server::Response| {
        if request.uri().path()!="/pairing/direct" {return Err(tungstenite::http::Response::builder().status(404).body(Some("Not found".into())).unwrap());}Ok(response)
    };
    let socket=tungstenite::accept_hdr_with_config(MaybeTlsStream::Plain(tcp),callback,Some(config)).map_err(|_|"Direct WebSocket handshake failed.".to_owned())?;
    let mut connection=new_connection(record,epoch,socket,true)?;
    connection.pairing_deadline=Instant::now()+Duration::from_secs(5);
    Ok(Some(connection))
}
// Return an authenticated pair request only after the same pinned ECDH proof.
fn poll_direct(c:&mut Connection,record:&Record)->Result<Option<Value>,String>{
    if Instant::now()>c.pairing_deadline{return Err("Direct handshake timed out.".into());}
    match c.socket.read(){
        Ok(Message::Text(text))=>{
            let value:Value=serde_json::from_str(text.as_ref()).map_err(|_|"Invalid direct frame.")?;
            match value["type"].as_str(){
                Some("join") if !c.joined && value["room"]==record.room && value["role"]=="mobile"=>{
                    c.joined=true;
                    for frame in [json!({"type":"joined","capabilities":["relay-heartbeat-v1"]}),json!({"type":"peer"}),c.crypto.hello()?]{c.socket.send(Message::Text(frame.to_string().into())).map_err(|e|e.to_string())?;}
                },
                Some("ping") if c.joined=>{c.socket.send(Message::Text(json!({"type":"pong"}).to_string().into())).map_err(|e|e.to_string())?;},
                Some("hello") if c.joined=>{if let Some(reply)=c.crypto.receive_hello(&value)?{c.socket.send(Message::Text(reply.to_string().into())).map_err(|e|e.to_string())?;}},
                Some("secure") if c.joined=>{let value=c.crypto.decrypt(&value)?;if value["type"]!="pair-request"{return Err("Direct pairing request required.".into());}let proof=c.crypto.encrypt(&json!({"type":"pair-request"}))?;c.socket.send(Message::Text(proof.to_string().into())).map_err(|e|e.to_string())?;return Ok(Some(value));},
                _=>return Err("Unsupported direct handshake frame.".into()),
            }
        },
        Err(tungstenite::Error::Io(e)) if matches!(e.kind(),std::io::ErrorKind::WouldBlock|std::io::ErrorKind::TimedOut)=>{},
        _=>return Err("Direct connection closed.".into()),
    }
    Ok(None)
}
fn trusted(record:&Record,key:&str,scope:&str)->bool{record.enabled&&record.devices.iter().any(|d|d.public_key==key&&d.scope==scope&&record.inactivity_days.is_none_or(|days|now().saturating_sub(d.last_access_at)<days as u64*86400000))}
fn access(record:&mut Record,key:&str,scope:&str){if let Some(device)=record.devices.iter_mut().find(|d|d.public_key==key&&d.scope==scope){device.last_access_at=now();}}
fn remember(record:&mut Record,key:&str,scope:&str)->Result<(),String>{
    if let Some(device)=record.devices.iter_mut().find(|d|d.public_key==key&&d.scope==scope){device.last_access_at=now();return Ok(());}
    if record.devices.iter().filter(|d|d.scope==scope).count()>=32{return Err("Revoke an existing device before remembering another (limit 32 per scope).".into());}
    let name=record.devices.iter().find(|d|d.public_key==key).map(|d|d.name.clone()).unwrap_or_else(||format!("Phone {}",record.devices.iter().map(|d|&d.public_key).collect::<std::collections::HashSet<_>>().len()+1));
    record.devices.push(Device{public_key:key.into(),name,scope:scope.into(),last_access_at:now()});Ok(())
}
fn approve(connection:&mut Connection,owner:bool){if owner{connection.owner=true;connection.owner_requested=false;connection.outgoing.push_back(json!({"type":"owner-workspace-approved"}));}else{connection.approved=true;connection.requested=false;connection.outgoing.push_back(json!({"type":"approved","capabilities":["remembered-device-v1","task-message-pages-v1","owner-workspace-v1","owner-chunks-512k-v1"]}));}}
fn publish(shared:&Mutex<Status>,record:&Record,connection:Option<&Connection>,error:&Option<String>){
    let mut pending=vec![];
    if let Some(c)=connection {for (scope,requested)in [("controller",c.requested),("owner",c.owner_requested)]{if requested{pending.push(Pending{id:format!("{}:{scope}",c.epoch),name:"Phone".into(),sas:c.crypto.verification().unwrap_or("").into(),scope:scope.into()});}}}
    let connection_state=match connection {
        Some(c) if c.owner=>"Phone authenticated · full workspace approved",
        Some(c) if c.owner_requested=>"Phone authenticated · awaiting full-workspace approval",
        Some(c) if c.approved=>"Phone authenticated · controller approved",
        Some(c) if c.requested=>"Phone authenticated · awaiting controller approval",
        Some(c) if c.crypto.peer_public_key().is_some()=>"Verifying phone connection…",
        Some(c) if c.direct=>"Direct LAN connection · waiting for phone",
        Some(c) if c.joined=>"Relay connected · waiting for phone",
        _=>"Connecting to relay…",
    }.to_owned();
    if let Ok(mut status)=shared.lock(){*status=Status{enabled:record.enabled,relay_connected:connection.is_some_and(|c|c.joined&&!c.direct),connection_state,pin:record.pin.clone(),error:error.clone(),pending,devices:record.devices.clone(),invitation:invitation(record).ok().map(|v|v.to_string()),inactivity_days:record.inactivity_days,migration_note:Some("Native pairing uses a fresh device identity. Previously paired web-view devices must pair again; their existing records are preserved.".into())};}
}
fn run(service:DesktopService,path:PathBuf,mut record:Record,commands:mpsc::Receiver<Command>,shared:Arc<Mutex<Status>>){
    let (done_tx,done_rx)=mpsc::sync_channel::<Completed>(8);let mut connection:Option<Connection>=None;let mut error=None;let mut next_connect=Instant::now();let mut backoff=1u64;let mut epoch=0u64;let mut need_registry=record.enabled;
    let listener=TcpListener::bind(("0.0.0.0",DIRECT_PORT)).ok().filter(|listener|listener.set_nonblocking(true).is_ok());
    DIRECT_AVAILABLE.store(listener.is_some(),Ordering::Relaxed);
    let mut direct:Option<Connection>=None;
    publish(&shared,&record,None,&error);
    loop {
        // A disconnected UI command sender means the native application closed.
        let command=match commands.try_recv(){Ok(command)=>Some(command),Err(mpsc::TryRecvError::Empty)=>None,Err(mpsc::TryRecvError::Disconnected)=>break};
        if let Some(command)=command {
            let result=(||->Result<(),String>{match command{
                Command::SetEnabled(enabled)=>{record.enabled=enabled;save(&path,&record)?;connection=None;epoch+=1;next_connect=Instant::now();need_registry=enabled;error=None;},
                Command::Retry=>{connection=None;epoch+=1;next_connect=Instant::now();need_registry=record.enabled;error=None;},
                Command::RotatePin=>{registry(&record,true)?;record.management_token=random_token()?;record.pin=None;save(&path,&record)?;need_registry=record.enabled;},
                Command::SetInactivityDays(days)=>{if days.is_some_and(|d|d==0||d>36500){return Err("Inactivity limit must be 1–36500 days, or unlimited.".into());}record.inactivity_days=days;save(&path,&record)?;},
                Command::Revoke{public_key,scope}=>{if !matches!(scope.as_str(),"controller"|"owner"){return Err("Invalid device scope.".into());}record.devices.retain(|d|!(d.public_key==public_key&&(d.scope==scope||scope=="controller")));save(&path,&record)?;if let Some(c)=connection.as_mut(){if c.crypto.peer_public_key()==Some(public_key.as_str()){if scope=="controller"{connection=None;epoch+=1;}else{revoke_owner(c);c.outgoing.push_back(json!({"type":"owner-workspace-revoked"}));}}}},
                Command::Approve{id,remember:remember_device}=>{let c=connection.as_mut().ok_or("No pending pairing request.")?;let owner=id==format!("{}:owner",c.epoch)&&c.owner_requested&&c.approved;let controller=id==format!("{}:controller",c.epoch)&&c.requested;if !owner&&!controller{return Err("Pairing request changed. Review its current verification number.".into());}let key=c.crypto.peer_public_key().ok_or("Phone identity is unavailable.")?.to_owned();if remember_device{record.inactivity_days=None;remember(&mut record,&key,if owner{"owner"}else{"controller"})?;save(&path,&record)?;}if owner{c.remembered_owner=remember_device;}else{c.remembered_controller=remember_device;}approve(c,owner);},
                Command::Reject{id}=>{let c=connection.as_mut().ok_or("No pending pairing request.")?;if id==format!("{}:owner",c.epoch)&&c.owner_requested{c.owner_requested=false;c.outgoing.push_back(json!({"type":"owner-workspace-rejected"}));}else if id==format!("{}:controller",c.epoch)&&c.requested{c.requested=false;c.outgoing.push_back(json!({"type":"rejected"}));c.pairing_deadline=Instant::now()+Duration::from_secs(3);}else{return Err("Pairing request is no longer pending.".into());}},
            }Ok(())})();if let Err(reason)=result{if reason.starts_with("Pairing record could not be saved:"){record.enabled=false;connection=None;}error=Some(reason);}
        }
        if !record.enabled {direct=None;publish(&shared,&record,None,&error);thread::sleep(Duration::from_millis(100));continue;}
        publish(&shared,&record,connection.as_ref(),&error);
        if need_registry {need_registry=false;match registry(&record,false){Ok(pin)=>{record.pin=pin;if let Err(reason)=save(&path,&record){record.enabled=false;error=Some(reason);continue;}},Err(reason)=>error=Some(reason)}}
        publish(&shared,&record,connection.as_ref(),&error);
        // An unauthenticated socket can never replace a phone with pending or
        // approved access. Direct candidates use a separate provisional session.
        if !connection.as_ref().is_some_and(|c|c.approved||c.requested) {
            if direct.is_none(){if let Some(listener)=&listener{epoch+=1;if let Ok(candidate)=accept_direct(listener,&record,epoch){direct=candidate;}}}
            if let Some(candidate)=direct.as_mut(){
                match poll_direct(candidate,&record){
                    Ok(Some(value))=>{
                        let mut authenticated=direct.take().unwrap();
                        authenticated.pairing_deadline=Instant::now()+Duration::from_secs(300);
                        match secure(&mut authenticated,value,&mut record,&path,&service,&done_tx){
                            Ok(())=>{connection=Some(authenticated);error=None;},Err(reason)=>error=Some(reason),
                        }
                    },
                    Ok(None)=>{},Err(_)=>{direct=None;},
                }
            }
        }else{direct=None;}
        if connection.is_none()&&direct.is_none()&&Instant::now()>=next_connect{epoch+=1;match connect(&record,epoch){Ok(c)=>{connection=Some(c);backoff=1;},Err(reason)=>{error=Some(reason);next_connect=Instant::now()+Duration::from_secs(backoff);backoff=(backoff*2).min(30);}}}
        if let Some(c)=connection.as_mut(){
            let result=(||->Result<(),String>{
                if (!c.approved&&Instant::now()>c.pairing_deadline)||c.pong_deadline.is_some_and(|deadline|Instant::now()>deadline){return Err("Pairing or relay heartbeat timed out.".into());}
                let key=c.crypto.peer_public_key().map(str::to_owned);
                if c.approved&&c.remembered_controller&&!key.as_ref().is_some_and(|key|trusted(&record,key,"controller")){return Err("Remembered phone access expired or was revoked.".into());}
                if c.owner&&c.remembered_owner&&!key.as_ref().is_some_and(|key|trusted(&record,key,"owner")){revoke_owner(c);c.outgoing.push_back(json!({"type":"owner-workspace-revoked"}));}
                if c.heartbeat&&c.pong_deadline.is_none()&&Instant::now()>=c.ping_at{c.socket.send(Message::Text("{\"type\":\"ping\"}".into())).map_err(|e|e.to_string())?;c.pong_deadline=Some(Instant::now()+Duration::from_secs(10));}
                while let Ok(done)=done_rx.try_recv(){if done.epoch!=c.epoch{continue;}c.active=c.active.saturating_sub(1);if !c.approved||done.generation!=c.generation.load(Ordering::SeqCst)||(done.owner&&!c.owner){continue;}if done.appearance{if let Ok(value)=&done.result{if let Some(theme)=value["terminalTheme"].as_str(){record.terminal_theme=theme.into();save(&path,&record)?;}}}let response=match done.result{Ok(result)=>json!({"id":done.id,"ok":true,"result":result}),Err(reason)=>json!({"id":done.id,"ok":false,"error":if done.owner{json!(reason.chars().take(512).collect::<String>())}else{json!({"code":"bridge_error","message":reason.chars().take(512).collect::<String>()})}})};if done.owner{queue_owner(c,&done.id,response)?;}else{let mut response=response;response["version"]=json!(1);if let Some(receipt)=c.receipts.get_mut(&done.id){receipt.response=Some(response.clone());}queue_controller(c,response)?;}}
                let expired=c.transfers.iter().filter(|(_,t)|Instant::now()>t.deadline).map(|(id,_)|id.clone()).collect::<Vec<_>>();for id in expired{c.transfers.remove(&id);c.outgoing.push_back(json!({"type":"owner-response-error","id":id,"error":"Owner transfer timed out."}));}
                // Reserve relay rate capacity for handshake and heartbeat control frames.
                while c.sent.front().is_some_and(|at|at.elapsed()>=Duration::from_secs(10)){c.sent.pop_front();}
                if c.sent.len()<90 {
                    if let Some(value)=c.outgoing.pop_front() {
                        let owner_data=value["type"].as_str().is_some_and(|t|t.starts_with("owner-response-")||t=="owner-workspace-approved");
                        if c.owner || !owner_data {
                            let frame=c.crypto.encrypt(&value)?;let encoded=frame.to_string();
                            if encoded.len()>MAX_FRAME{return Err("Encrypted relay frame exceeds its limit.".into());}
                            c.socket.send(Message::Text(encoded.into())).map_err(|e|e.to_string())?;c.sent.push_back(Instant::now());
                        }
                    }
                }
                match c.socket.read(){
                    Ok(Message::Text(text))=>{if text.len()>MAX_FRAME{return Err("Relay message too large.".into());}let value:Value=serde_json::from_str(text.as_ref()).map_err(|_|"Invalid relay JSON.")?;match value["type"].as_str(){
                        Some("joined") if !c.joined=>{if record.pin.is_some(){error=None;}c.joined=true;c.heartbeat=value["capabilities"].as_array().is_some_and(|v|v.iter().any(|s|s=="relay-heartbeat-v1"));},
                        Some("ping") if c.direct=>{c.socket.send(Message::Text(json!({"type":"pong"}).to_string().into())).map_err(|e|e.to_string())?;},
                        Some("pong") if c.heartbeat=>{c.pong_deadline=None;c.ping_at=Instant::now()+Duration::from_secs(15);},
                        Some("peer") if c.joined=>{let hello=c.crypto.hello()?;c.socket.send(Message::Text(hello.to_string().into())).map_err(|e|e.to_string())?;},
                        Some("hello")=>{if let Some(reply)=c.crypto.receive_hello(&value)?{c.socket.send(Message::Text(reply.to_string().into())).map_err(|e|e.to_string())?;}},
                        Some("peer_left")=>return Err("Phone disconnected; reconnecting relay listener.".into()),
                        Some("secure")=>{let value=c.crypto.decrypt(&value)?;secure(c,value,&mut record,&path,&service,&done_tx)?;},
                        _=>return Err("Unsupported relay frame.".into()),
                    }},
                    Ok(Message::Ping(bytes))=>{c.socket.send(Message::Pong(bytes)).map_err(|e|e.to_string())?;},
                    Ok(Message::Pong(_))=>{},Ok(Message::Close(_))=>return Err("Relay connection closed.".into()),Ok(_)=>return Err("Binary relay frames are unavailable.".into()),
                    Err(tungstenite::Error::Io(e)) if matches!(e.kind(),std::io::ErrorKind::WouldBlock|std::io::ErrorKind::TimedOut)=>{},Err(_)=>return Err("Pairing relay connection interrupted.".into()),
                }Ok(())
            })();if let Err(reason)=result{if reason.starts_with("Pairing record could not be saved:"){record.enabled=false;}error=Some(reason);connection=None;next_connect=Instant::now()+Duration::from_secs(1);}
        }else{thread::sleep(Duration::from_millis(100));}
        publish(&shared,&record,connection.as_ref(),&error);
    }
}
fn queue_controller(c:&mut Connection,mut response:Value)->Result<(),String>{
    if response.to_string().len()>MAX_CONTROLLER{response=json!({"version":1,"id":response["id"],"ok":false,"error":{"code":"response_too_large","message":"Controller response exceeds 1 MiB. Use the owner workspace."}});}
    if c.outgoing.len()>1024{return Err("Pairing response queue is full.".into());}
    c.outgoing.push_back(json!({"type":"response","json":response.to_string()}));Ok(())
}
fn queue_owner(c:&mut Connection,id:&str,response:Value)->Result<(),String>{
    let bytes=response.to_string().into_bytes();
    let queued=c.outgoing.iter().map(|value|value.to_string().len()).sum::<usize>();
    if bytes.len()>MAX_OWNER||queued.saturating_add(bytes.len()*4/3+4096)>64*1024*1024{
        c.outgoing.push_back(json!({"type":"owner-response-error","id":id,"error":"Owner response exceeds bounded transfer capacity."}));return Ok(());
    }
    c.outgoing.push_back(json!({"type":"owner-response-start","id":id,"bytes":bytes.len()}));
    let chunk=if c.large_chunks{CHUNK}else{32*1024};
    for(index,data)in bytes.chunks(chunk).enumerate(){c.outgoing.push_back(json!({"type":"owner-response-chunk","id":id,"index":index,"data":STANDARD.encode(data)}));}
    c.outgoing.push_back(json!({"type":"owner-response-end","id":id}));Ok(())
}
fn valid_id(value:&Value)->bool {value.as_str().is_some_and(|s|s.len()==36&&uuid::Uuid::parse_str(s).is_ok())}
fn exact(value:&Value,names:&[&str])->bool {value.as_object().is_some_and(|object|object.len()==names.len()&&names.iter().all(|name|object.contains_key(*name)))}
fn secure(c:&mut Connection,value:Value,record:&mut Record,path:&Path,service:&DesktopService,done:&mpsc::SyncSender<Completed>)->Result<(),String>{
    let key=c.crypto.peer_public_key().ok_or("Authenticated phone identity is missing.")?.to_owned();
    // Expiry may cross while a blocking socket read is pending. Recheck before
    // refreshing access or dispatching, so the request cannot revive an old grant.
    let kind=value["type"].as_str().unwrap_or("");
    if c.approved&&(kind=="rpc"||kind.starts_with("owner-rpc-")) {
        if c.remembered_controller&&!trusted(record,&key,"controller"){return Err("Remembered phone access expired or was revoked.".into());}
        if kind.starts_with("owner-rpc-")&&c.owner&&c.remembered_owner&&!trusted(record,&key,"owner") {revoke_owner(c);c.outgoing.push_back(json!({"type":"owner-workspace-revoked"}));return Ok(());}
    }
    match value["type"].as_str(){
        Some("pair-request") if !c.approved&&!c.requested=>{
            // Shared visitors have a separate protocol and can never gain owner scope.
            if value.get("operator").is_some(){return Err("Visitor identities cannot pair as a phone.".into());}
            c.large_chunks=value["capabilities"].as_array().is_some_and(|values|values.iter().any(|v|v=="owner-chunks-512k-v1"));
            if trusted(record,&key,"controller"){access(record,&key,"controller");save(path,record)?;c.remembered_controller=true;approve(c,false);}else{c.requested=true;}Ok(())
        },
        Some("owner-workspace-request") if c.approved&&!c.owner&&!c.owner_requested=>{
            if trusted(record,&key,"owner"){access(record,&key,"owner");save(path,record)?;c.remembered_owner=true;approve(c,true);}else{c.owner_requested=true;}Ok(())
        },
        Some("rpc") if c.approved=>{
            let text=value["json"].as_str().ok_or("Invalid controller RPC.")?;
            let parsed=if text.len()<=64*1024{serde_json::from_str::<Value>(text).ok()}else{None};
            let Some(request)=parsed else{return queue_controller(c,json!({"version":1,"id":null,"ok":false,"error":{"code":"invalid_request","message":"Invalid or oversized controller request."}}));};
            let id=request["id"].as_str().unwrap_or("").to_owned();
            let mapped=controller_command(&request);
            let (command,args,mutation)=match mapped{Ok(mapped)=>mapped,Err(reason)=>return queue_controller(c,json!({"version":1,"id":if valid_id(&request["id"]){json!(id)}else{Value::Null},"ok":false,"error":{"code":"invalid_request","message":reason}}))};
            let fingerprint=URL_SAFE_NO_PAD.encode(ring::digest::digest(&ring::digest::SHA256,request.to_string().as_bytes()).as_ref());
            if c.active>=8{return queue_controller(c,json!({"version":1,"id":id,"ok":false,"error":{"code":"busy","message":"Too many paired requests."}}));}
            if mutation {
                if let Some(receipt)=c.receipts.get(&id){if receipt.fingerprint!=fingerprint{return queue_controller(c,json!({"version":1,"id":id,"ok":false,"error":{"code":"id_conflict","message":"Request ID already used."}}));}if let Some(response)=receipt.response.clone(){return queue_controller(c,response);}return Ok(());}
                if c.receipts.len()>=4096||c.receipts.values().filter_map(|r|r.response.as_ref()).map(|r|r.to_string().len()).sum::<usize>()>=32*1024*1024{return queue_controller(c,json!({"version":1,"id":id,"ok":false,"error":{"code":"dedup_saturated","message":"Mutation receipt cache is full. Reconnect the authenticated session."}}));}
                c.receipts.insert(id.clone(),Receipt{fingerprint,response:None});
            }
            if c.remembered_controller{access(record,&key,"controller");save(path,record)?;}
            if command.starts_with("controller_upload_") {
                match controller_upload(c,&command,&args) {
                    Ok(UploadDispatch::Store(args))=>spawn_work(c,service,done,id,false,"store_attachment".into(),args,record.terminal_theme.clone()),
                    result=>{
                        let response=match result {Ok(UploadDispatch::Immediate(result))=>json!({"version":1,"id":id,"ok":true,"result":result}),Err(reason)=>json!({"version":1,"id":id,"ok":false,"error":{"code":"bridge_error","message":reason}}),Ok(UploadDispatch::Store(_))=>unreachable!()};
                        if let Some(receipt)=c.receipts.get_mut(&id){receipt.response=Some(response.clone());}
                        queue_controller(c,response)?;
                    }
                }
                return Ok(());
            }
            spawn_work(c,service,done,id,false,command,args,record.terminal_theme.clone());Ok(())
        },
        Some("owner-rpc-start") if c.approved&&c.owner=>{
            if !valid_id(&value["id"]){return Err("Invalid owner transfer ID.".into());}let id=value["id"].as_str().unwrap().to_owned();let size=value["bytes"].as_u64().filter(|size|*size<=MAX_OWNER as u64).ok_or("Invalid owner transfer size.")? as usize;
            if c.transfers.contains_key(&id)||c.transfers.len()>=8||c.transfers.values().map(|t|t.size).sum::<usize>().saturating_add(size)>MAX_OWNER{return Err("Owner transfer capacity exceeded.".into());}
            c.transfers.insert(id,Transfer{size,data:Vec::with_capacity(size.min(1024*1024)),index:0,deadline:Instant::now()+Duration::from_secs(60)});Ok(())
        },
        Some("owner-rpc-chunk") if c.approved&&c.owner=>{
            let id=value["id"].as_str().ok_or("Invalid owner transfer ID.")?;let transfer=c.transfers.get_mut(id).ok_or("Unknown owner transfer.")?;
            let chunk=if c.large_chunks{CHUNK}else{32*1024};let text=value["data"].as_str().filter(|text|text.len()<=chunk.div_ceil(3)*4).ok_or("Owner chunk exceeds its limit.")?;
            let bytes=STANDARD.decode(text).map_err(|_|"Owner chunk encoding is invalid.")?;
            if value["index"].as_u64()!=Some(transfer.index)||bytes.len()>chunk||STANDARD.encode(&bytes).trim_end_matches('=')!=text.trim_end_matches('=')||transfer.index>=MAX_OWNER.div_ceil(chunk) as u64||transfer.data.len().saturating_add(bytes.len())>transfer.size{return Err("Owner chunks are invalid or out of order.".into());}
            transfer.data.extend(bytes);transfer.index+=1;Ok(())
        },
        Some("owner-rpc-end") if c.approved&&c.owner=>{
            let id=value["id"].as_str().ok_or("Invalid owner transfer ID.")?.to_owned();let transfer=c.transfers.remove(&id).ok_or("Unknown owner transfer.")?;
            if transfer.data.len()!=transfer.size{return Err("Owner transfer is incomplete.".into());}
            let request:Value=serde_json::from_slice(&transfer.data).map_err(|_|"Invalid owner command JSON.")?;
            if !exact(&request,&["id","command","args"])||request["id"]!=id{return Err("Invalid owner command envelope.".into());}
            let command=request["command"].as_str().filter(|s|!s.is_empty()&&s.len()<=128&&s.as_bytes()[0].is_ascii_alphabetic()&&s.bytes().all(|b|b.is_ascii_alphanumeric()||matches!(b,b'_'|b'.'|b'-'))).ok_or("Invalid owner command.")?.to_owned();
            if c.active>=8{c.outgoing.push_back(json!({"type":"owner-response-error","id":id,"error":"Too many owner requests are pending."}));return Ok(());}
            if c.remembered_controller{access(record,&key,"controller");}if c.remembered_owner{access(record,&key,"owner");}save(path,record)?;
            spawn_work(c,service,done,id,true,command,request["args"].clone(),record.terminal_theme.clone());Ok(())
        },_=>Err("Unapproved or unsupported paired message.".into()),
    }
}
fn controller_command(request:&Value)->Result<(String,Value,bool),String>{
    if !exact(request,&["version","id","action","params"])||request["version"]!=1||!valid_id(&request["id"])||!request["params"].is_object(){return Err("Invalid controller request fields.".into());}
    let p=&request["params"];let action=request["action"].as_str().ok_or("Missing controller action.")?;
    let (command,mutation)=match action{
        "getSnapshot" if exact(p,&[]) =>("get_snapshot",false),
        "getTaskMessages" if p.as_object().is_some_and(|o|o.keys().all(|k|matches!(k.as_str(),"taskId"|"beforeId"|"limit")))&&valid_id(&p["taskId"])&&p.get("beforeId").is_none_or(valid_id)&&p.get("limit").is_none_or(|v|v.as_u64().is_some_and(|n|(1..=40).contains(&n))) =>("get_task_messages",false),
        "beginAttachmentUpload" if valid_upload_begin(p) =>("controller_upload_begin",true),
        "appendAttachmentUpload" if exact(p,&["uploadId","dataBase64"])&&valid_id(&p["uploadId"])&&p["dataBase64"].as_str().is_some_and(valid_upload_chunk) =>("controller_upload_append",true),
        "finishAttachmentUpload" if exact(p,&["uploadId"])&&valid_id(&p["uploadId"]) =>("controller_upload_finish",true),
        "listTerminals" if exact(p,&[]) =>("list_terminals",false),
        "sendMessage" if (exact(p,&["taskId","text"])||exact(p,&["taskId","text","attachmentIds"]))&&valid_id(&p["taskId"])&&p["text"].as_str().is_some_and(|s|s.encode_utf16().count()<=32768)&&p.get("attachmentIds").is_none_or(|v|v.as_array().is_some_and(|a|a.len()<=8&&a.iter().all(valid_id)))&&(p["text"].as_str().is_some_and(|s|!s.is_empty())||p["attachmentIds"].as_array().is_some_and(|a|!a.is_empty())) =>("send_message",true),
        "cancelTask" if exact(p,&["taskId"])&&valid_id(&p["taskId"]) =>("cancel_task",true),
        "resumeTask" if exact(p,&["taskId"])&&valid_id(&p["taskId"]) =>("resume_task",true),
        "readTerminal" if exact(p,&["id","afterSeq"])&&valid_id(&p["id"])&&p["afterSeq"].as_u64().is_some_and(|n|n<=9_007_199_254_740_991) =>("read_terminal",false),
        _=>return Err("Controller action is unsupported or its parameters are invalid. Owner workspace permissions are separate.".into()),
    };Ok((command.into(),p.clone(),mutation))
}
struct ControllerUpload { task_id:String, filename:String, mime_type:String, preview:Option<String>, size:usize, data:Vec<u8>, created:Instant }
enum UploadDispatch { Immediate(Value), Store(Value) }
fn valid_upload_chunk(value:&str)->bool {
    if value.is_empty()||value.len()>32768usize.div_ceil(3)*4{return false;}
    decode_upload_chunk(value).is_ok_and(|bytes|!bytes.is_empty()&&bytes.len()<=32768&&STANDARD.encode(bytes).trim_end_matches('=')==value.trim_end_matches('='))
}
fn decode_upload_chunk(value:&str)->Result<Vec<u8>,base64::DecodeError>{STANDARD.decode(value).or_else(|_|base64::engine::general_purpose::STANDARD_NO_PAD.decode(value))}
fn valid_upload_begin(p:&Value)->bool {
    (exact(p,&["taskId","filename","mimeType","size"])||exact(p,&["taskId","filename","mimeType","size","previewDataUrl"]))
        &&valid_id(&p["taskId"])
        &&p["filename"].as_str().is_some_and(|v|!v.trim().is_empty()&&v.encode_utf16().count()<=255&&!v.chars().any(|c|matches!(c,'/'|'\\'|'\0')))
        &&p["mimeType"].as_str().is_some_and(|v|v.encode_utf16().count()<=255)
        &&p["size"].as_u64().is_some_and(|v|(1..=8*1024*1024).contains(&v))
        &&p.get("previewDataUrl").is_none_or(|v|v.as_str().is_some_and(|v|{
            let Some((header,data))=v.split_once(',')else{return false;};
            v.len()<=48000&&["data:image/png;base64","data:image/jpeg;base64","data:image/webp;base64"].iter().any(|h|header.eq_ignore_ascii_case(h))&&!data.is_empty()&&data.bytes().all(|b|b.is_ascii_alphanumeric()||matches!(b,b'+'|b'/'|b'='))&&STANDARD.decode(data).is_ok()
        }))
}
fn controller_upload(c:&mut Connection,command:&str,args:&Value)->Result<UploadDispatch,String>{
    c.uploads.retain(|_,upload|upload.created.elapsed()<Duration::from_secs(120));
    if command=="controller_upload_begin" {
        let size=args["size"].as_u64().ok_or("Invalid attachment size.")? as usize;
        if c.uploads.len()>=8||c.uploads.values().map(|u|u.size).sum::<usize>().saturating_add(size)>64*1024*1024{return Err("Too many attachment uploads are in progress.".into());}
        let id=uuid::Uuid::new_v4().to_string();
        c.uploads.insert(id.clone(),ControllerUpload{task_id:args["taskId"].as_str().ok_or("Missing upload task.")?.into(),filename:args["filename"].as_str().ok_or("Missing upload filename.")?.into(),mime_type:args["mimeType"].as_str().ok_or("Missing upload MIME type.")?.into(),preview:args["previewDataUrl"].as_str().map(str::to_owned),size,data:Vec::new(),created:Instant::now()});
        return Ok(UploadDispatch::Immediate(json!({"uploadId":id})));
    }
    let id=args["uploadId"].as_str().ok_or("Invalid upload ID.")?;
    if command=="controller_upload_append" {
        let bytes=decode_upload_chunk(args["dataBase64"].as_str().ok_or("Invalid attachment chunk.")?).map_err(|_|"Invalid attachment chunk.")?;
        let upload=c.uploads.get_mut(id).ok_or("Attachment upload expired or is unavailable.")?;
        if upload.data.len().saturating_add(bytes.len())>upload.size {c.uploads.remove(id);return Err("Attachment exceeds its declared size.".into());}
        upload.data.extend(bytes);
        return Ok(UploadDispatch::Immediate(json!({"uploadId":id})));
    }
    let upload=c.uploads.remove(id).ok_or("Attachment upload expired or is unavailable.")?;
    if upload.data.len()!=upload.size{return Err("Attachment upload is incomplete.".into());}
    let mut args=json!({"target":{"taskId":upload.task_id},"filename":upload.filename,"mimeType":upload.mime_type,"dataBase64":STANDARD.encode(upload.data)});
    if let Some(preview)=upload.preview{args["previewDataUrl"]=json!(preview);}
    Ok(UploadDispatch::Store(args))
}
fn controller_message_page(mut page:Value)->Result<Value,String>{
    if let Some(messages)=page["messages"].as_array_mut(){for message in messages{
        if let Some(text)=message["text"].as_str(){if text.encode_utf16().count()>8000{let mut length=0;let truncated:String=text.chars().take_while(|c|{length+=c.len_utf16();length<=8000}).collect();message["text"]=json!(truncated);}}
        if let Some(attachments)=message["attachments"].as_array_mut(){for attachment in attachments{if let Some(object)=attachment.as_object_mut(){object.remove("previewDataUrl");}}}
    }}
    if page.to_string().len()>MAX_CONTROLLER-1024{return Err("Controller history exceeds its response budget.".into());}Ok(page)
}
static BACKEND_WORKERS:std::sync::atomic::AtomicUsize=std::sync::atomic::AtomicUsize::new(0);
struct WorkerPermit;
impl Drop for WorkerPermit {fn drop(&mut self){BACKEND_WORKERS.fetch_sub(1,Ordering::SeqCst);}}
fn spawn_work(c:&mut Connection,service:&DesktopService,done:&mpsc::SyncSender<Completed>,id:String,owner:bool,command:String,args:Value,terminal_theme:String){
    if BACKEND_WORKERS.fetch_update(Ordering::SeqCst,Ordering::SeqCst,|count|(count<8).then_some(count+1)).is_err(){
        if owner{c.outgoing.push_back(json!({"type":"owner-response-error","id":id,"error":"Too many backend requests are pending."}));}
        else{let response=json!({"version":1,"id":id,"ok":false,"error":{"code":"busy","message":"Too many backend requests are pending."}});if let Some(receipt)=c.receipts.get_mut(&id){receipt.response=Some(response.clone());}let _=queue_controller(c,response);}
        return;
    }
    let permit=WorkerPermit;
    c.active+=1;let epoch=c.epoch;let authority=c.generation.clone();let generation=authority.load(Ordering::SeqCst);let service=service.clone();let done=done.clone();
    thread::spawn(move||{
        let _permit=permit;
        if authority.load(Ordering::SeqCst)!=generation{let _=done.send(Completed{generation,epoch,id,owner,result:Err("Pairing authorization changed.".into()),appearance:false});return;}
        let appearance=owner&&command=="set_owner_appearance";
        let result=if owner&&matches!(command.as_str(),"get_owner_appearance"|"set_owner_appearance"){owner_appearance(&service,&command,args,&terminal_theme)}else{service.pairing_host_invoke(&command,args)}
            .and_then(|value| {
                if !owner && command == "send_message" && value["accepted"] == true && value.get("tasks").is_none() {
                    service.pairing_host_invoke("get_snapshot", json!({}))
                } else { Ok(value) }
            })
            .and_then(|value|if !owner&&matches!(command.as_str(),"get_snapshot"|"send_message"|"cancel_task"|"resume_task"){controller_snapshot(value)}else if !owner&&command=="get_task_messages"{controller_message_page(value)}else if !owner&&value.to_string().len()>MAX_CONTROLLER-1024{Err("Controller response exceeds 1 MiB. Use the owner workspace.".into())}else{Ok(value)});
        let _=done.send(Completed{generation,epoch,id,owner,result,appearance});
    });
}
fn bounded_strings(value:&mut Value){match value{Value::String(text)=>{if text.chars().count()>4096{*text=text.chars().take(4060).collect::<String>()+"\n[Remaining text available on desktop]";}},Value::Array(items)=>for item in items{bounded_strings(item);},Value::Object(object)=>{object.remove("previewDataUrl");for value in object.values_mut(){bounded_strings(value);}},_=>{}}}
fn controller_snapshot(mut snapshot:Value)->Result<Value,String>{
    if let Some(messages)=snapshot["messages"].as_array_mut(){messages.sort_by_key(|v|v["createdAt"].as_u64().unwrap_or(0));let mut counts:HashMap<String,usize>=HashMap::new();let mut kept=Vec::new();for message in messages.iter().rev(){let key=message["taskId"].as_str().unwrap_or("").to_owned();let count=counts.entry(key).or_default();if *count<32&&kept.len()<600{kept.push(message.clone());*count+=1;}}kept.reverse();*messages=kept;}
    if let Some(events)=snapshot["events"].as_array_mut(){events.sort_by_key(|v|v["createdAt"].as_u64().unwrap_or(0));if events.len()>100{events.drain(..events.len()-100);}}
    if let Some(channels)=snapshot["channels"].as_array_mut(){let mut remaining=300;for channel in channels{if let Some(messages)=channel["messages"].as_array_mut(){let keep=messages.len().min(24).min(remaining);remaining-=keep;messages.drain(..messages.len()-keep);}}}
    for field in ["collaborations","queuedMessages","approvalRequests","approvalRules","workPlans","projectBoardMessages","subagentSessions","schedules","scheduleRuns","pendingThrowawayTaskIds"]{snapshot[field]=json!([]);}snapshot["subagentTranscripts"]=json!({});bounded_strings(&mut snapshot);
    while snapshot.to_string().len()>MAX_CONTROLLER-2048{
        if snapshot["messages"].as_array_mut().is_some_and(|messages|{if messages.is_empty(){false}else{messages.remove(0);true}}){continue;}
        if snapshot["events"].as_array_mut().is_some_and(|events|{if events.is_empty(){false}else{events.remove(0);true}}){continue;}
        return Err("Controller metadata exceeds the response budget. Use the owner workspace.".into());
    }Ok(snapshot)
}
fn owner_appearance(service:&DesktopService,command:&str,value:Value,terminal:&str)->Result<Value,String>{
    let mut preferences=service.load_native_preferences()?;
    let mut terminal=terminal.to_owned();
    if command=="set_owner_appearance"{
        const PALETTES:&[&str]=&["monitter","catppuccin","dracula","gruvbox","nord","one","solarized","tokyo-night","wombat","xterm"];
        const TERMINALS:&[&str]=&["monitter","catppuccin-mocha","dracula","gruvbox-dark","molokai","nord","one-dark","solarized-dark","wombat","xterm"];
        let theme=&value["appTheme"];let light=theme["light"].as_str().filter(|s|PALETTES.contains(s)).ok_or("Invalid light palette.")?;let dark=theme["dark"].as_str().filter(|s|PALETTES.contains(s)).ok_or("Invalid dark palette.")?;
        if value["version"]!=1||!value["surfaceTint"].as_u64().is_some_and(|v|v<=50)||!theme["contrast"].as_u64().is_some_and(|v|v<=100){return Err("Invalid owner appearance values.".into());}
        let accent=if theme["accent"].is_null(){None}else{Some(theme["accent"].as_str().filter(|s|s.len()==7&&s.starts_with('#')&&s[1..].bytes().all(|c|c.is_ascii_hexdigit())).ok_or("Invalid appearance accent.")?.to_owned())};
        terminal=value["terminalTheme"].as_str().filter(|s|TERMINALS.contains(s)).ok_or("Invalid terminal theme.")?.to_owned();
        let appearance=&mut preferences.data.appearance;appearance.light_palette=light.into();appearance.dark_palette=dark.into();appearance.custom_accent=accent;appearance.contrast=theme["contrast"].as_u64().unwrap() as u16;appearance.surface_tint=value["surfaceTint"].as_u64().unwrap() as u16;
        preferences=service.save_native_preferences(&preferences.revision,preferences.data)?;
    }
    let appearance=preferences.data.appearance;
    Ok(json!({"version":1,"appTheme":{"light":appearance.light_palette,"dark":appearance.dark_palette,"accent":appearance.custom_accent,"contrast":appearance.contrast},"surfaceTint":appearance.surface_tint,"terminalTheme":terminal}))
}
