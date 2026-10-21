//! Typed native agent/host settings. Save/configure never starts a model turn.
//! Field map: AppSurface Agent/Host forms, AccessPicker, AcpAgentPicker and
//! AgentModelPicker. Native avatar selection uses bounded local reads; redacted managed-extension assignment uses typed owner-service methods.
use crate::design::ui_scaled_px as px;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use gpui_kit::{
    component::{
        button::*,
        input::{Input, InputState, Textarea, TextareaState},
        Disableable,
    },
    prelude::*,
    *,
};
use monitter_lib::{
    experimental_desktop::{DesktopService, ExtensionAssignmentKind, ExtensionAssignments},
    model::{Agent, Host},
};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    io::Read,
    sync::Arc,
};
fn text(v: &Value, k: &str) -> String {
    v[k].as_str().unwrap_or_default().to_owned()
}
fn items(v: &Value, k: &str) -> Vec<Value> {
    v[k].as_array().cloned().unwrap_or_default()
}
fn apply_catalog_reply(snapshot: &mut Value, reply: &Value) {
    for key in ["agents", "hosts"] {
        if let Some(catalog) = reply.get(key) {
            snapshot[key] = catalog.clone();
        }
    }
}
fn lines(v: &Value, k: &str) -> String {
    items(v, k)
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>()
        .join("\n")
}
fn literal_arguments(value: &str) -> Vec<String> {
    if value.is_empty() {
        vec![]
    } else {
        value
            .split('\n')
            .map(|line| line.trim_end_matches('\r').to_owned())
            .collect()
    }
}
fn avatar_format(bytes: &[u8]) -> Result<(&'static str, ImageFormat), String> {
    if bytes.len() > 2 * 1024 * 1024 {
        return Err("Choose a PNG, JPEG or WebP image up to 2 MiB.".into());
    }
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Ok(("image/png", ImageFormat::Png))
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Ok(("image/jpeg", ImageFormat::Jpeg))
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Ok(("image/webp", ImageFormat::Webp))
    } else {
        Err("Choose a PNG, JPEG or WebP image up to 2 MiB.".into())
    }
}
fn avatar_image(agent: &Value) -> Option<Arc<Image>> {
    let data = agent["avatar"].as_str()?;
    if data.len() > 3 * 1024 * 1024 {
        return None;
    }
    let (prefix, encoded) = data.split_once(",")?;
    if ![
        "data:image/png;base64",
        "data:image/jpeg;base64",
        "data:image/webp;base64",
    ]
    .iter()
    .any(|allowed| prefix.eq_ignore_ascii_case(allowed))
    {
        return None;
    }
    let bytes = STANDARD.decode(encoded).ok()?;
    let (_, format) = avatar_format(&bytes).ok()?;
    Some(Arc::new(Image::from_bytes(format, bytes)))
}
const AGENT_FIELDS: &[(&str, &str)] = &[
    ("name", "Name"),
    ("description", "Description"),
    ("cwd", "Workspace folder"),
    ("model", "Model ID (empty uses harness default)"),
    ("color", "Identity color (#RRGGBB)"),
    ("codexHome", "Existing local Codex home (optional)"),
    ("acpCommand", "External launcher executable"),
    ("tierFast", "System One fast model"),
    ("tierBalanced", "System One balanced model"),
    ("tierStrong", "System One strong model"),
    ("tierFrontier", "System One frontier model"),
];
const AGENT_TEXT: &[(&str, &str)] = &[
    ("instructions", "Instructions"),
    ("expertise", "Expertise (one per line)"),
    ("responsibilities", "Responsibilities (one per line)"),
    ("skills", "Skills (one per line)"),
    (
        "acpArgs",
        "Launcher arguments (one literal argument per line)",
    ),
];
const HOST_FIELDS: &[(&str, &str)] = &[
    ("name", "Host name"),
    ("address", "SSH address / config alias"),
    ("user", "SSH user (empty preserves config)"),
    ("port", "SSH port (0 preserves config)"),
    ("identityFile", "Existing identity file path (optional)"),
    ("defaultCwd", "Default folder"),
    ("codexPath", "Codex executable path"),
    ("claudePath", "Claude executable path"),
    ("opencodePath", "OpenCode executable path"),
    ("hermesPath", "Hermes executable path"),
];
struct Form {
    inputs: HashMap<&'static str, Entity<InputState>>,
    areas: HashMap<&'static str, Entity<TextareaState>>,
}
impl Form {
    fn new(
        inputs: &'static [(&'static str, &'static str)],
        areas: &'static [(&'static str, &'static str)],
        window: &mut Window,
        cx: &mut Context<AgentHostPane>,
    ) -> Self {
        Self {
            inputs: inputs
                .iter()
                .map(|(key, _)| (*key, cx.new(|cx| InputState::new(window, cx))))
                .collect(),
            areas: areas
                .iter()
                .map(|(key, _)| (*key, cx.new(|cx| TextareaState::new(window, cx))))
                .collect(),
        }
    }
    fn read(&self, key: &str, cx: &App) -> String {
        if let Some(input) = self.inputs.get(key) {
            input.read(cx).value().to_string()
        } else {
            self.areas
                .get(key)
                .map(|area| area.read(cx).value().to_string())
                .unwrap_or_default()
        }
    }
    fn set(&self, key: &str, value: String, window: &mut Window, cx: &mut App) {
        if let Some(input) = self.inputs.get(key) {
            input.update(cx, |input, cx| input.set_value(value, window, cx));
        } else if let Some(area) = self.areas.get(key) {
            area.update(cx, |area, cx| area.set_value(value, window, cx));
        }
    }
}
#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Agents,
    Hosts,
}
enum Response {
    AgentDeleted(String),
    AgentSaved(HashSet<String>),
    HostSaved(HashSet<String>),
    Probe,
    Accounts,
    Discovery,
    Verification,
    Models(String),
}
pub struct AgentHostPane {
    service: DesktopService,
    snapshot: Value,
    mode: Mode,
    agent: Value,
    host: Value,
    agent_form: Form,
    host_form: Form,
    agent_drafts: HashMap<String, Value>,
    host_drafts: HashMap<String, Value>,
    busy: bool,
    removal: Option<String>,
    avatar_epoch: u64,
    avatar_preview: Option<Arc<Image>>,
    error: Option<String>,
    notice: Option<String>,
    probe: Option<Value>,
    accounts: Vec<Value>,
    candidates: Vec<Value>,
    verification: Option<Value>,
    assignments: Option<(String, ExtensionAssignments)>,
    catalog: Option<Value>,
    catalog_key: Option<String>,
    _changes: Vec<Subscription>,
}
impl AgentHostPane {
    pub fn new(
        service: DesktopService,
        snapshot: Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let agent_form = Form::new(AGENT_FIELDS, AGENT_TEXT, window, cx);
        let host_form = Form::new(HOST_FIELDS, &[], window, cx);
        let mut changes = vec![];
        for input in agent_form.inputs.values().chain(host_form.inputs.values()) {
            changes.push(cx.observe(input, |_, _, cx| cx.notify()));
        }
        for area in agent_form.areas.values() {
            changes.push(cx.observe(area, |_, _, cx| cx.notify()));
        }
        let local = items(&snapshot, "hosts")
            .into_iter()
            .find(|host| text(host, "kind") == "local");
        let agent = json!({"id":"","name":"","description":"","instructions":"","provider":"codex","model":"","hostId":local.as_ref().map(|host|text(host,"id")).unwrap_or_default(),"cwd":local.as_ref().map(|host|text(host,"defaultCwd")).unwrap_or_default(),"color":"#3f9d6a","avatar":null,"expertise":[],"responsibilities":[],"skills":[],"collaborationEnabled":true,"terminalExecutionEnabled":false,"sandbox":"read-only","codexHome":null,"acp":null,"jevRouting":"inherit","jevDecisionsEnabled":false,"decisionBackend":"inherit","jevModelTiers":{"fast":"","balanced":"","strong":"","frontier":""}});
        let host = json!({"id":"","name":"","kind":"ssh","address":"","user":"","port":22,"identityFile":"","defaultCwd":"","codexPath":"","claudePath":"","opencodePath":"","hermesPath":""});
        let mut pane = Self {
            service,
            snapshot,
            mode: Mode::Agents,
            agent,
            host,
            agent_form,
            host_form,
            agent_drafts: HashMap::new(),
            host_drafts: HashMap::new(),
            busy: false,
            removal: None,
            avatar_epoch: 0,
            avatar_preview: None,
            error: None,
            notice: None,
            probe: None,
            accounts: vec![],
            candidates: vec![],
            verification: None,
            assignments: None,
            catalog: None,
            catalog_key: None,
            _changes: changes,
        };
        pane.bind_agent(window, cx);
        pane.bind_host(window, cx);
        pane
    }
    /// External sync preserves unsaved editor fields and per-record drafts.
    pub fn set_snapshot(&mut self, snapshot: &Value, _window: &mut Window, cx: &mut Context<Self>) {
        self.snapshot = snapshot.clone();
        cx.notify();
    }
    fn bind_agent(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.avatar_epoch = self.avatar_epoch.wrapping_add(1);
        self.avatar_preview = avatar_image(&self.agent);
        for (key, _) in AGENT_FIELDS {
            let value = match *key {
                "acpCommand" => text(&self.agent["acp"], "command"),
                "tierFast" => text(&self.agent["jevModelTiers"], "fast"),
                "tierBalanced" => text(&self.agent["jevModelTiers"], "balanced"),
                "tierStrong" => text(&self.agent["jevModelTiers"], "strong"),
                "tierFrontier" => text(&self.agent["jevModelTiers"], "frontier"),
                _ => text(&self.agent, key),
            };
            self.agent_form.set(key, value, window, cx);
        }
        for (key, _) in AGENT_TEXT {
            self.agent_form.set(
                key,
                match *key {
                    "instructions" => text(&self.agent, key),
                    "acpArgs" => lines(&self.agent["acp"], "args"),
                    _ => lines(&self.agent, key),
                },
                window,
                cx,
            );
        }
    }
    fn bind_host(&self, window: &mut Window, cx: &mut Context<Self>) {
        for (key, _) in HOST_FIELDS {
            self.host_form.set(
                key,
                if *key == "port" {
                    self.host["port"].as_u64().unwrap_or(22).to_string()
                } else {
                    text(&self.host, key)
                },
                window,
                cx,
            );
        }
    }
    fn agent_value(&self, cx: &App) -> Value {
        let mut value = self.agent.clone();
        for (key, _) in AGENT_FIELDS {
            if !key.starts_with("tier") && *key != "acpCommand" {
                value[*key] = json!(self.agent_form.read(key, cx));
            }
        }
        value["instructions"] = json!(self.agent_form.read("instructions", cx));
        for key in ["expertise", "responsibilities", "skills"] {
            value[key] = json!(self
                .agent_form
                .read(key, cx)
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .collect::<Vec<_>>());
        }
        let home = self.agent_form.read("codexHome", cx);
        value["codexHome"] = if home.trim().is_empty() {
            Value::Null
        } else {
            json!(home.trim())
        };
        let command = self.agent_form.read("acpCommand", cx);
        let args = self.agent_form.read("acpArgs", cx);
        value["acp"] = if command.trim().is_empty() {
            Value::Null
        } else {
            json!({"command":command.trim(),"args":literal_arguments(&args)})
        };
        value["jevModelTiers"] = json!({"fast":self.agent_form.read("tierFast",cx),"balanced":self.agent_form.read("tierBalanced",cx),"strong":self.agent_form.read("tierStrong",cx),"frontier":self.agent_form.read("tierFrontier",cx)});
        value
    }
    fn host_value(&self, cx: &App) -> Result<Value, String> {
        let mut value = self.host.clone();
        for (key, _) in HOST_FIELDS {
            value[*key] = json!(self.host_form.read(key, cx));
        }
        value["port"] = json!(self
            .host_form
            .read("port", cx)
            .trim()
            .parse::<u16>()
            .map_err(|_| "SSH port must be an integer from 0 to 65535.")?);
        // Reuse the shared Rust model at the command boundary; the retained
        // native widgets own editing state rather than a generic JSON editor.
        serde_json::from_value::<Host>(value.clone())
            .map_err(|error| format!("Invalid host configuration: {error}"))?;
        Ok(value)
    }
    pub(super) fn select_agent(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.mode = Mode::Agents;
        self.agent_drafts
            .insert(text(&self.agent, "id"), self.agent_value(cx));
        if let Some(agent) = self.agent_drafts.get(id).cloned().or_else(|| {
            items(&self.snapshot, "agents")
                .into_iter()
                .find(|agent| text(agent, "id") == id)
        }) {
            self.agent = agent;
            self.catalog = None;
            self.catalog_key = None;
            self.verification = None;
            self.candidates.clear();
            self.bind_agent(window, cx);
        }
        self.removal = None;
        self.error = None;
        cx.notify();
    }
    pub(super) fn show_agents(&mut self, cx: &mut Context<Self>) {
        self.mode = Mode::Agents;
        cx.notify();
    }
    pub(super) fn show_hosts(&mut self, cx: &mut Context<Self>) {
        self.mode = Mode::Hosts;
        cx.notify();
    }
    fn select_host(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Ok(host) = self.host_value(cx) {
            self.host_drafts.insert(text(&self.host, "id"), host);
        }
        if let Some(host) = self.host_drafts.get(id).cloned().or_else(|| {
            items(&self.snapshot, "hosts")
                .into_iter()
                .find(|host| text(host, "id") == id)
        }) {
            self.host = host;
            self.probe = None;
            self.bind_host(window, cx);
        }
        self.error = None;
        cx.notify();
    }
    fn invoke(
        &mut self,
        command: &'static str,
        args: Value,
        response: Response,
        cx: &mut Context<Self>,
    ) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.error = None;
        self.notice = None;
        let service = self.service.clone();
        let task = cx
            .background_executor()
            .spawn(async move { service.invoke(command, args) });
        cx.spawn(async move |view,cx| {let result=task.await;let _=view.update(cx,|pane,cx| {
            pane.busy=false;
            match result {
                Err(error)=>pane.error=Some(error),
                Ok(value)=>match response {
                    Response::AgentDeleted(id)=>{apply_catalog_reply(&mut pane.snapshot,&value);pane.agent_drafts.remove(&id);pane.agent=Value::Null;pane.removal=None;pane.notice=Some("Agent removed using the selected chat handling policy.".into());}
                    Response::AgentSaved(old)=>{
                        let id=text(&pane.agent,"id");
                        let saved=items(&value,"agents").into_iter().find(|agent|if id.is_empty(){!old.contains(&text(agent,"id"))}else{text(agent,"id")==id});
                        apply_catalog_reply(&mut pane.snapshot,&value);
                        if let Some(agent)=saved {pane.agent_drafts.remove(&text(&agent,"id"));pane.agent=agent;}
                        pane.notice=Some("Agent saved. Existing chats keep their captured runtime configuration.".into());
                    }
                    Response::HostSaved(old)=>{
                        let id=text(&pane.host,"id");let saved=items(&value,"hosts").into_iter().find(|host|if id.is_empty(){!old.contains(&text(host,"id"))}else{text(host,"id")==id});
                        apply_catalog_reply(&mut pane.snapshot,&value);if let Some(host)=saved {pane.host_drafts.remove(&text(&host,"id"));pane.host=host;}
                        pane.notice=Some("Host saved. No credentials were read or copied.".into());
                    }
                    Response::Probe=>pane.probe=Some(value),Response::Accounts=>pane.accounts=value.as_array().cloned().unwrap_or_default(),
                    Response::Discovery=>pane.candidates=value.as_array().cloned().unwrap_or_default(),Response::Verification=>pane.verification=Some(value),
                    Response::Models(key)=>{pane.catalog_key=Some(key);pane.catalog=Some(value);}
                }
            };cx.notify();
        });}).detach();
        cx.notify();
    }
    fn extension_assignments(
        &mut self,
        change: Option<(String, ExtensionAssignmentKind, bool, String)>,
        cx: &mut Context<Self>,
    ) {
        if self.busy {
            return;
        }
        let id = text(&self.agent, "id");
        if id.is_empty() {
            return;
        }
        self.busy = true;
        self.error = None;
        let service = self.service.clone();
        let target = id.clone();
        let task = cx.background_executor().spawn(async move {
            match change {
                Some((entry, kind, assigned, revision)) => service
                    .set_agent_extension_assignment(&target, &entry, kind, assigned, &revision),
                None => service.get_agent_extension_assignments(&target),
            }
        });
        cx.spawn(async move |view, cx| {
            let result = task.await;
            let _ = view.update(cx, |pane, cx| {
                pane.busy = false;
                if text(&pane.agent, "id") == id {
                    match result {
                        Ok(assignments) => pane.assignments = Some((id, assignments)),
                        Err(error) => pane.error = Some(error),
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn choose_avatar(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.error = None;
        let epoch = self.avatar_epoch;
        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Choose a PNG, JPEG or WebP avatar (up to 2 MiB)".into()),
        });
        cx.spawn(async move |view, cx| {
            let result = match prompt.await {
                Ok(Ok(Some(paths))) => {
                    if let Some(path) = paths.into_iter().next() {
                        Some(
                            cx.background_executor()
                                .spawn(async move {
                                    let file = std::fs::File::open(path)
                                        .map_err(|error| error.to_string())?;
                                    let mut bytes = Vec::new();
                                    file.take(2 * 1024 * 1024 + 1)
                                        .read_to_end(&mut bytes)
                                        .map_err(|error| error.to_string())?;
                                    let (mime, format) = avatar_format(&bytes)?;
                                    let data =
                                        format!("data:{mime};base64,{}", STANDARD.encode(&bytes));
                                    Ok::<_, String>((
                                        data,
                                        Arc::new(Image::from_bytes(format, bytes)),
                                    ))
                                })
                                .await,
                        )
                    } else {
                        None
                    }
                }
                Ok(Ok(None)) => None,
                Ok(Err(error)) => Some(Err(error.to_string())),
                Err(error) => Some(Err(error.to_string())),
            };
            let _ = view.update(cx, |pane, cx| {
                pane.busy = false;
                if pane.avatar_epoch == epoch {
                    match result {
                        Some(Ok((data, image))) => {
                            pane.agent["avatar"] = json!(data);
                            pane.avatar_preview = Some(image);
                        }
                        Some(Err(error)) => pane.error = Some(error),
                        None => {}
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn launch_key(value: &Value) -> String {
        json!({"provider":value["provider"],"hostId":value["hostId"],"cwd":value["cwd"],"codexHome":value["codexHome"],"acp":value["acp"]}).to_string()
    }
    fn persisted_runtime(&self, cx: &App) -> bool {
        let value = self.agent_value(cx);
        let id = text(&value, "id");
        !id.is_empty()
            && items(&self.snapshot, "agents").iter().any(|saved| {
                text(saved, "id") == id && Self::launch_key(saved) == Self::launch_key(&value)
            })
    }
    fn save_agent(&mut self, cx: &mut Context<Self>) {
        let mut value = self.agent_value(cx);
        let parsed = match serde_json::from_value::<Agent>(value.clone()) {
            Ok(agent) => agent,
            Err(error) => {
                self.error = Some(format!("Invalid agent configuration: {error}"));
                cx.notify();
                return;
            }
        };
        if parsed.name.trim().is_empty() {
            self.error = Some("An agent name is required.".into());
            cx.notify();
            return;
        }
        if value["provider"] == "acp" && value["acp"].is_null() {
            self.error = Some("Provide the external executable and literal argument rows.".into());
            cx.notify();
            return;
        }
        if value["internal"] == true {
            value["collaborationEnabled"] = json!(false);
        }
        self.agent = value.clone();
        let old = items(&self.snapshot, "agents")
            .iter()
            .map(|agent| text(agent, "id"))
            .collect();
        self.invoke(
            "save_agent",
            json!({"agent":value}),
            Response::AgentSaved(old),
            cx,
        );
    }
    fn save_host(&mut self, cx: &mut Context<Self>) {
        match self.host_value(cx) {
            Ok(host) => {
                self.host = host.clone();
                let old = items(&self.snapshot, "hosts")
                    .iter()
                    .map(|host| text(host, "id"))
                    .collect();
                self.invoke(
                    "save_host",
                    json!({"host":host}),
                    Response::HostSaved(old),
                    cx,
                );
            }
            Err(error) => {
                self.error = Some(error);
                cx.notify();
            }
        }
    }
}
impl Render for AgentHostPane {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut nav = div()
            .id("agent-host-index")
            .w(px(210.))
            .flex_shrink_0()
            .h_full()
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .overflow_y_scroll()
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        Button::new("agents-mode")
                            .compact()
                            .label("Agents")
                            .disabled(self.busy)
                            .on_click(cx.listener(|pane, _, _, cx| {
                                pane.mode = Mode::Agents;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("hosts-mode")
                            .compact()
                            .label("Hosts")
                            .disabled(self.busy)
                            .on_click(cx.listener(|pane, _, _, cx| {
                                pane.mode = Mode::Hosts;
                                cx.notify();
                            })),
                    ),
            );
        let records = items(
            &self.snapshot,
            if self.mode == Mode::Agents {
                "agents"
            } else {
                "hosts"
            },
        );
        for record in records {
            let id = text(&record, "id");
            let mode = self.mode;
            nav = nav.child(
                Button::new(SharedString::from(format!("record-{id}")))
                    .compact()
                    .label(format!(
                        "{}{}",
                        text(&record, "name"),
                        if record["internal"] == true {
                            " · internal"
                        } else {
                            ""
                        }
                    ))
                    .disabled(self.busy)
                    .on_click(cx.listener(move |pane, _, window, cx| {
                        if mode == Mode::Agents {
                            pane.select_agent(&id, window, cx);
                        } else {
                            pane.select_host(&id, window, cx);
                        }
                    })),
            );
        }
        nav=nav.child(Button::new("new-record").label(if self.mode==Mode::Agents{"New agent"}else{"New SSH host"}).disabled(self.busy)
            .on_click(cx.listener(|pane,_,window,cx| {
                if pane.mode==Mode::Agents {
                    pane.agent_drafts.insert(text(&pane.agent,"id"),pane.agent_value(cx));
                    let local=items(&pane.snapshot,"hosts").into_iter().find(|host|text(host,"kind")=="local");
                    pane.agent=json!({"id":"","name":"","description":"","instructions":"","avatar":null,"provider":"codex","model":"","hostId":local.as_ref().map(|host|text(host,"id")).unwrap_or_default(),"cwd":"","color":"#3f9d6a","sandbox":"read-only","expertise":[],"responsibilities":[],"skills":[],"collaborationEnabled":true,"terminalExecutionEnabled":false,"codexHome":null,"acp":null,"jevRouting":"inherit","jevDecisionsEnabled":false,"decisionBackend":"inherit","jevModelTiers":{"fast":"","balanced":"","strong":"","frontier":""}});
                    pane.catalog=None;pane.removal=None;pane.bind_agent(window,cx);
                } else {
                    if let Ok(host)=pane.host_value(cx){pane.host_drafts.insert(text(&pane.host,"id"),host);}
                    pane.host=json!({"id":"","name":"","kind":"ssh","address":"","user":"","port":22,"identityFile":"","defaultCwd":"","codexPath":"","claudePath":"","opencodePath":"","hermesPath":""});pane.probe=None;pane.bind_host(window,cx);
                };pane.error=None;cx.notify();
            })));
        let mut form = div()
            .id("agent-host-form")
            .flex_1()
            .h_full()
            .min_w_0()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap_3()
            .p_5();
        if let Some(error) = &self.error {
            form = form.child(format!("Error: {error}"));
        }
        if let Some(notice) = &self.notice {
            form = form.child(notice.clone());
        }
        if self.mode == Mode::Agents {
            if self.agent.is_null() {
                return div()
                    .size_full()
                    .flex()
                    .child(nav)
                    .child(form.child("Choose another agent or create a new agent."));
            }
            let draft = self.agent_value(cx);
            let internal = draft["internal"] == true;
            form=form.child(div().text_xl().child("Agent configuration"))
                .child("These settings apply to newly created chats. Existing CLI authentication and native sessions are preserved.")
                ;
            for (key, provider, label, command, args) in [
                ("codex", "codex", "Codex", None, None),
                ("claude", "claude", "Claude Code", None, None),
                ("opencode", "opencode", "OpenCode", None, None),
                ("hermes", "acp", "Hermes Agent", Some("hermes"), Some("acp")),
                ("omp", "acp", "OMP (Oh My Pi)", Some("omp"), Some("acp")),
                ("mona", "acp", "Mona", Some("mona-acp"), Some("")),
                ("minimax", "acp", "MiniMax", Some("mcode"), Some("acp")),
                (
                    "gemini",
                    "acp",
                    "Gemini",
                    Some("monitter-agy-acp"),
                    Some(""),
                ),
                (
                    "custom",
                    "acp",
                    "Custom external launcher",
                    Some(""),
                    Some(""),
                ),
            ] {
                form = form.child(
                    Button::new(key)
                        .compact()
                        .label(label)
                        .disabled(self.busy)
                        .on_click(cx.listener(move |pane, _, window, cx| {
                            pane.agent["provider"] = json!(provider);
                            pane.catalog = None;
                            pane.verification = None;
                            pane.candidates.clear();
                            pane.agent_form.set(
                                "acpCommand",
                                command.unwrap_or_default().into(),
                                window,
                                cx,
                            );
                            pane.agent_form.set(
                                "acpArgs",
                                args.unwrap_or_default().into(),
                                window,
                                cx,
                            );
                            if provider == "acp" {
                                pane.agent["sandbox"] = json!("harness-configured");
                            }
                            cx.notify();
                        })),
                );
            }
            for (key, label) in AGENT_FIELDS {
                if *key == "acpCommand" && text(&draft, "provider") != "acp" {
                    continue;
                }
                if *key == "codexHome" && text(&draft, "provider") != "codex" {
                    continue;
                }
                if key.starts_with("tier") && text(&draft, "jevRouting") == "off" {
                    continue;
                }
                form = form.child(*label).child(
                    Input::new(self.agent_form.inputs.get(key).unwrap())
                        .disabled(self.busy || (internal && *key == "name")),
                );
            }
            form = form.child("Saved target host");
            for host in items(&self.snapshot, "hosts") {
                let id = text(&host, "id");
                form = form.child(
                    Button::new(SharedString::from(format!("agent-host-{id}")))
                        .compact()
                        .label(format!(
                            "{} · {}{}",
                            text(&host, "name"),
                            text(&host, "kind"),
                            if text(&draft, "hostId") == id {
                                " ✓"
                            } else {
                                ""
                            }
                        ))
                        .disabled(self.busy)
                        .on_click(cx.listener(move |pane, _, _, cx| {
                            pane.agent["hostId"] = json!(id);
                            pane.catalog = None;
                            pane.verification = None;
                            pane.candidates.clear();
                            cx.notify();
                        })),
                );
            }
            form = form.child("Access for new chats");
            let provider = text(&draft, "provider");
            let command = text(&draft["acp"], "command").to_lowercase();
            let hermes = provider == "hermes" || command.rsplit('/').next() == Some("hermes");
            let omp = provider == "acp" && command.rsplit(['/', '\\']).next() == Some("omp");
            let gemini = command.contains("gemini")
                || command.contains("monitter-agy-acp")
                || command.contains("antigravity");
            let current = match text(&draft, "sandbox").as_str() {
                "read-only" => "read-only",
                "workspace-write" => "auto-approve-edits",
                "yolo" => "full-access",
                _ => {
                    if provider == "codex" {
                        "read-only"
                    } else {
                        "auto-approve-edits"
                    }
                }
            };
            for (level, label) in [
                ("read-only", "Read only"),
                ("auto-approve-edits", "Auto-approve edits"),
                ("full-access", "Full access"),
            ] {
                let supported = match level {
                    "read-only" => !hermes && !gemini && !omp,
                    "full-access" => !hermes && !omp,
                    _ => true,
                };
                form = form.child(
                    Button::new(level)
                        .compact()
                        .label(format!(
                            "{}{}{}",
                            label,
                            if current == level { " ✓" } else { "" },
                            if supported { "" } else { " · unavailable" }
                        ))
                        .disabled(self.busy || !supported)
                        .on_click(cx.listener(move |pane, _, _, cx| {
                            let provider = text(&pane.agent, "provider");
                            pane.agent["sandbox"] = json!(match level {
                                "read-only" => "read-only",
                                "full-access" => "yolo",
                                _ =>
                                    if provider == "codex" {
                                        "workspace-write"
                                    } else {
                                        "harness-configured"
                                    },
                            });
                            cx.notify();
                        })),
                );
            }
            if hermes {
                form=form.child("Hermes uses its configured permission policy; read-only and full-access modes are unavailable.");
            }
            if omp {
                form = form.child(
                    "OMP uses its configured tool approval policy. Configure approvals in OMP.",
                );
            }
            if gemini {
                form = form.child("Gemini's lowest supported permission level allows edits.");
            }
            form=form.child("Access is validated by the saved harness and backend. System One routing never grants permissions.");
            for (key, label) in [
                ("collaborationEnabled", "Allow collaboration"),
                ("terminalExecutionEnabled", "Allow native terminal requests"),
                ("jevDecisionsEnabled", "Enable System One decision tools"),
            ] {
                let enabled = draft[key] == true;
                form = form.child(
                    Button::new(key)
                        .compact()
                        .label(format!("{}: {}", label, if enabled { "on" } else { "off" }))
                        .disabled(self.busy || (internal && key == "collaborationEnabled"))
                        .on_click(cx.listener(move |pane, _, _, cx| {
                            pane.agent[key] = json!(!enabled);
                            cx.notify();
                        })),
                );
            }
            form = form.child("System One backend applies to routing and advisory decisions. Inherit uses the global setting. Monitter starts the configured local Laya service on demand when its model is available. If it requires authentication, save LAYA_API_KEY in Environment & Secrets.");
            for (backend, label) in [("inherit", "Use global setting"), ("type_safe", "TypeSafe"), ("laya", "Laya local")] {
                form = form.child(
                    Button::new(format!("decision-backend-{backend}"))
                        .compact()
                        .label(format!(
                            "System One backend: {label}{}",
                            if (text(&draft, "decisionBackend").is_empty()
                                && backend == "inherit")
                                || text(&draft, "decisionBackend") == backend
                            {
                                " ✓"
                            } else {
                                ""
                            }
                        ))
                        .disabled(self.busy)
                        .on_click(cx.listener(move |pane, _, _, cx| {
                            pane.agent["decisionBackend"] = json!(backend);
                            cx.notify();
                        })),
                );
            }
            for (mode, label) in [("inherit", "Use global setting"), ("off", "Off"), ("recommend", "Recommend only"), ("safe_auto", "Safe auto-route")] {
                form = form.child(
                    Button::new(mode)
                        .compact()
                        .label(format!(
                            "System One routing: {}{}",
                            label,
                            if text(&draft, "jevRouting") == mode || (text(&draft, "jevRouting").is_empty() && mode == "inherit") {
                                " ✓"
                            } else {
                                ""
                            }
                        ))
                        .disabled(self.busy)
                        .on_click(cx.listener(move |pane, _, _, cx| {
                            pane.agent["jevRouting"] = json!(mode);
                            cx.notify();
                        })),
                );
            }
            for (key, label) in AGENT_TEXT {
                if *key == "acpArgs" && text(&draft, "provider") != "acp" {
                    continue;
                }
                form = form.child(*label).child(
                    Textarea::new(self.agent_form.areas.get(key).unwrap())
                        .h(px(if *key == "instructions" { 140. } else { 80. }))
                        .disabled(self.busy),
                );
            }
            form = form.child(
                Button::new("choose-avatar")
                    .label("Choose avatar…")
                    .disabled(self.busy)
                    .on_click(cx.listener(|pane, _, _, cx| pane.choose_avatar(cx))),
            );
            if let Some(image) = self.avatar_preview.clone() {
                form = form.child(img(image).w(px(64.)).h(px(64.)));
            }
            if !draft["avatar"].is_null() {
                form = form.child("Avatar is saved with this profile.").child(
                    Button::new("remove-avatar")
                        .compact()
                        .label("Remove avatar")
                        .disabled(self.busy)
                        .on_click(cx.listener(|pane, _, _, cx| {
                            pane.agent["avatar"] = Value::Null;
                            pane.avatar_preview = None;
                            pane.avatar_epoch = pane.avatar_epoch.wrapping_add(1);
                            cx.notify();
                        })),
                );
            }
            if text(&draft, "provider") == "codex" {
                form = form.child(
                    Button::new("codex-accounts")
                        .label("List existing local Codex homes")
                        .disabled(self.busy)
                        .on_click(cx.listener(|pane, _, _, cx| {
                            pane.invoke("list_codex_accounts", json!({}), Response::Accounts, cx)
                        })),
                );
                for account in &self.accounts {
                    let home = text(account, "home");
                    form = form.child(
                        Button::new(SharedString::from(format!("account-{home}")))
                            .compact()
                            .label(format!("{} · {}", text(account, "label"), home))
                            .disabled(self.busy)
                            .on_click(cx.listener(move |pane, _, window, cx| {
                                pane.agent_form.set("codexHome", home.clone(), window, cx);
                                pane.catalog = None;
                                cx.notify();
                            })),
                    );
                }
            }
            if text(&draft, "provider") == "acp" {
                form=form.child("Arguments are literal argv rows, without shell quoting. Use existing sign-in/environment; do not put secrets in arguments.")
                    .child(Button::new("discover-acp").label("Discover installed compatible launchers").disabled(self.busy||text(&draft,"hostId").is_empty())
                        .on_click(cx.listener(|pane,_,_,cx|pane.invoke("discover_acp_agents",json!({"hostId":pane.agent["hostId"]}),Response::Discovery,cx))));
                for (index, candidate) in self.candidates.iter().enumerate() {
                    let launch = candidate["launch"].clone();
                    form = form.child(
                        Button::new(SharedString::from(format!("candidate-{index}")))
                            .label(format!(
                                "{} · {} · detected {}",
                                text(candidate, "name"),
                                text(candidate, "integration"),
                                candidate["detected"]
                            ))
                            .disabled(self.busy)
                            .on_click(cx.listener(move |pane, _, window, cx| {
                                pane.agent_form.set(
                                    "acpCommand",
                                    text(&launch, "command"),
                                    window,
                                    cx,
                                );
                                pane.agent_form
                                    .set("acpArgs", lines(&launch, "args"), window, cx);
                                pane.verification = None;
                                pane.catalog = None;
                                cx.notify();
                            })),
                    );
                }
                form=form.child(Button::new("verify-acp").label("Verify initialize handshake (no model turn)").disabled(self.busy||draft["acp"].is_null())
                    .on_click(cx.listener(|pane,_,_,cx|{let draft=pane.agent_value(cx);pane.invoke("verify_acp_agent",json!({"hostId":draft["hostId"],"launch":draft["acp"]}),Response::Verification,cx);})))
                    .child("Detection/handshake does not prove authentication or a working model turn.");
                if let Some(verified) = &self.verification {
                    form = form.child(format!(
                        "Protocol {} · {} {} · load {} · resume {}",
                        verified["protocolVersion"],
                        text(verified, "agentName"),
                        text(verified, "agentVersion"),
                        verified["loadSession"],
                        verified["resumeSession"]
                    ));
                }
            }
            let persisted = self.persisted_runtime(cx);
            form = form.child(
                Button::new("agent-models")
                    .label("Read saved harness model catalog")
                    .disabled(self.busy || !persisted)
                    .on_click(cx.listener(|pane, _, _, cx| {
                        let draft = pane.agent_value(cx);
                        let key = Self::launch_key(&draft);
                        pane.invoke(
                            "get_model_catalog",
                            json!({"target":{"agentId":draft["id"],"refresh":true}}),
                            Response::Models(key),
                            cx,
                        );
                    })),
            );
            if !persisted {
                form = form.child(
                    "Save changed provider, host, folder, account or launcher settings before reading models.",
                );
            }
            if self.catalog_key.as_ref() == Some(&Self::launch_key(&draft)) {
                if let Some(catalog) = &self.catalog {
                    form = form.child(format!(
                        "Catalog source: {} · {}",
                        text(catalog, "source"),
                        text(catalog, "warning")
                    ));
                    for model in items(catalog, "models") {
                        let id = text(&model, "id");
                        form = form.child(
                            Button::new(SharedString::from(format!("agent-model-{id}")))
                                .compact()
                                .label(format!(
                                    "{} · {}",
                                    text(&model, "name"),
                                    text(&model, "description")
                                ))
                                .disabled(self.busy)
                                .on_click(cx.listener(move |pane, _, window, cx| {
                                    pane.agent_form.set("model", id.clone(), window, cx);
                                    cx.notify();
                                })),
                        );
                    }
                }
            }
            form = form
                .child(
                    Button::new("save-agent")
                        .primary()
                        .label(if self.busy {
                            "Working…"
                        } else {
                            "Save agent"
                        })
                        .disabled(self.busy)
                        .on_click(cx.listener(|pane, _, _, cx| pane.save_agent(cx))),
                )
                .child("Extension configuration and secrets editors remain separate native work.");
            let saved_id = text(&self.agent, "id");
            if !saved_id.is_empty() && !internal {
                form = form.child(
                    Button::new("refresh-assignments")
                        .label("Refresh extension assignments")
                        .disabled(self.busy)
                        .on_click(
                            cx.listener(|pane, _, _, cx| pane.extension_assignments(None, cx)),
                        ),
                );
                if let Some((agent_id, assignments)) = &self.assignments {
                    if agent_id == &saved_id {
                        if assignments.entries.is_empty() {
                            form = form.child(
                                "No managed extensions are configured in this isolated desktop.",
                            );
                        }
                        for entry in &assignments.entries {
                            let kind = entry.kind.clone();
                            let id = entry.id.clone();
                            let assigned = !entry.assigned;
                            let revision = assignments.revision.clone();
                            form =
                                form.child(
                                    Button::new(SharedString::from(format!(
                                        "extension-{:?}-{id}",
                                        kind
                                    )))
                                    .label(format!(
                                        "{} · {} · {}{}",
                                        entry.name,
                                        match kind {
                                            ExtensionAssignmentKind::Server => "Tool server",
                                            ExtensionAssignmentKind::Skill => "Managed skill",
                                        },
                                        if entry.all_agents {
                                            "All agents"
                                        } else if entry.assigned {
                                            "Assigned"
                                        } else {
                                            "Unassigned"
                                        },
                                        if entry.enabled {
                                            ""
                                        } else {
                                            " · globally disabled"
                                        }
                                    ))
                                    .disabled(self.busy || entry.all_agents)
                                    .on_click(cx.listener(move |pane, _, _, cx| {
                                        pane.extension_assignments(
                                            Some((
                                                id.clone(),
                                                kind.clone(),
                                                assigned,
                                                revision.clone(),
                                            )),
                                            cx,
                                        )
                                    })),
                                );
                        }
                        form=form.child("Assignment changes save immediately through the native owner service. Disabled extensions remain disabled; all-agent skills retain their global scope.");
                    }
                }
            }
            let id = text(&self.agent, "id");
            if !id.is_empty() && !internal {
                form = form.child(
                    Button::new("request-agent-removal")
                        .label("Remove agent…")
                        .disabled(self.busy)
                        .on_click(cx.listener(move |pane, _, _, cx| {
                            pane.removal = Some(id.clone());
                            cx.notify();
                        })),
                );
            }
            if let Some(id) = self.removal.clone() {
                let count = items(&self.snapshot, "tasks")
                    .iter()
                    .filter(|task| text(task, "agentId") == id)
                    .count();
                let mut removal = div().p_3().border_1().rounded_md().flex().flex_col().gap_2().child(format!("Remove {}? This agent owns {count} chats.", text(&self.agent, "name")))
                    .child("Archive keeps chats readable under Archived chats. Permanent deletion removes chats, transcripts, queued follow-ups and verified native session files where possible. Permanent deletion cannot be undone.");
                for (handling, label) in [
                    ("archive", "Archive chats and remove agent"),
                    ("delete", "Permanently delete chats and agent"),
                ] {
                    let target = id.clone();
                    removal = removal.child(
                        Button::new(handling)
                            .label(label)
                            .disabled(self.busy)
                            .on_click(cx.listener(move |pane, _, _, cx| {
                                pane.invoke(
                                    "delete_agent",
                                    json!({"id":target,"chatHandling":handling}),
                                    Response::AgentDeleted(target.clone()),
                                    cx,
                                );
                            })),
                    );
                }
                removal = removal.child(
                    Button::new("cancel-agent-removal")
                        .label("Cancel")
                        .disabled(self.busy)
                        .on_click(cx.listener(|pane, _, _, cx| {
                            pane.removal = None;
                            cx.notify();
                        })),
                );
                form = form.child(removal);
            }
        } else {
            form=form.child(div().text_xl().child("Host configuration"))
                .child("SSH uses saved/system authentication and normal host-key checks. Only key paths are edited; no credential contents are loaded.");
            for kind in ["local", "ssh"] {
                form = form.child(
                    Button::new(kind)
                        .compact()
                        .label(format!(
                            "{}{}",
                            kind,
                            if text(&self.host, "kind") == kind {
                                " ✓"
                            } else {
                                ""
                            }
                        ))
                        .disabled(self.busy)
                        .on_click(cx.listener(move |pane, _, _, cx| {
                            pane.host["kind"] = json!(kind);
                            pane.probe = None;
                            cx.notify();
                        })),
                );
            }
            for (key, label) in HOST_FIELDS {
                if text(&self.host, "kind") == "local"
                    && ["address", "user", "port", "identityFile"].contains(key)
                {
                    continue;
                }
                form = form
                    .child(*label)
                    .child(Input::new(self.host_form.inputs.get(key).unwrap()).disabled(self.busy));
            }
            form = form.child(
                Button::new("probe-host")
                    .label("Probe configured host / executable versions")
                    .disabled(self.busy)
                    .on_click(cx.listener(|pane, _, _, cx| match pane.host_value(cx) {
                        Ok(host) => {
                            pane.invoke("probe_host", json!({"host":host}), Response::Probe, cx)
                        }
                        Err(error) => {
                            pane.error = Some(error);
                            cx.notify();
                        }
                    })),
            );
            if let Some(probe) = &self.probe {
                form = form.child(format!(
                    "Probe ok: {} · {}",
                    probe["ok"],
                    text(probe, "message")
                ));
                if let Some(versions) = probe["versions"].as_object() {
                    for (provider, version) in versions {
                        form = form.child(format!(
                            "{}: {}",
                            provider,
                            version.as_str().unwrap_or_default()
                        ));
                    }
                }
            }
            form=form.child(Button::new("save-host").primary().label("Save host").disabled(self.busy).on_click(cx.listener(|pane,_,_,cx|pane.save_host(cx))))
                .child("A probe/version reports what the host actually returns; it does not prove model authentication. Default local/referenced host deletion is not offered here.");
        }
        div().size_full().flex().child(nav).child(form)
    }
}

#[cfg(test)]
mod tests {
    use super::literal_arguments;
    #[::core::prelude::v1::test]
    fn launcher_arguments_are_literal_rows_not_shell_words() {
        assert_eq!(
            literal_arguments("--path\nfolder with spaces\n$(do-not-execute)\n\"quoted\""),
            [
                "--path",
                "folder with spaces",
                "$(do-not-execute)",
                r#""quoted""#
            ]
        );
    }
    #[::core::prelude::v1::test]
    fn empty_launcher_and_explicit_empty_rows_remain_distinct() {
        assert!(literal_arguments("").is_empty());
        assert_eq!(
            literal_arguments("one\n\nlast\r\n"),
            ["one", "", "last", ""]
        );
    }
}
