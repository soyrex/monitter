//! Native editor for the private MCP and managed-skill configuration.
use crate::design::{Palette, ui_scaled_px as px};
use futures::channel::oneshot;
use gpui_kit::component::{
    Disableable,
    button::*,
    checkbox::Checkbox,
    input::{Input, InputState, Textarea, TextareaState},
    switch::Switch,
};
use gpui_kit::prelude::*;
use gpui_kit::*;
use monitter_lib::experimental_desktop::DesktopService;
use serde_json::{Value, json};
use std::thread;

const STACK: usize = 8 * 1024 * 1024;

fn text(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}
fn list(value: &Value, key: &str) -> Vec<Value> {
    value
        .get(key)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}
fn lines(value: &Value, key: &str) -> String {
    list(value, key)
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>()
        .join("\n")
}
fn parse_pairs(input: &str, headers: bool) -> Result<Value, String> {
    let mut values = serde_json::Map::new();
    for line in input.lines().filter(|line| !line.is_empty()) {
        let Some((key, value)) = line.split_once('=') else {
            return Err("Each entry must be KEY=value.".into());
        };
        let key = key.trim();
        let valid = if headers {
            !key.is_empty()
                && key
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&c))
        } else {
            !key.is_empty()
                && key.bytes().enumerate().all(|(i, c)| {
                    c.is_ascii_alphabetic() || c == b'_' || (i > 0 && c.is_ascii_digit())
                })
        };
        if !valid {
            return Err("Environment and header names must be valid keys.".into());
        }
        values.insert(key.into(), json!(value));
    }
    Ok(Value::Object(values))
}

pub struct ExtensionsPane {
    service: DesktopService,
    snapshot: Value,
    config: Option<Value>,
    draft: Option<Value>,
    draft_kind: Option<&'static str>,
    busy: bool,
    error: Option<String>,
    notice: Option<String>,
    tab: &'static str,
    name: Entity<InputState>,
    command: Entity<InputState>,
    url: Entity<InputState>,
    description: Entity<InputState>,
    args: Entity<TextareaState>,
    env: Entity<TextareaState>,
    headers: Entity<TextareaState>,
    skill_content: Entity<TextareaState>,
    import_json: Entity<TextareaState>,
    paste_open: bool,
}

impl ExtensionsPane {
    pub fn new(
        service: DesktopService,
        snapshot: &Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            service,
            snapshot: snapshot.clone(),
            config: None,
            draft: None,
            draft_kind: None,
            busy: false,
            error: None,
            notice: None,
            tab: "servers",
            name: cx.new(|cx| InputState::new(window, cx).placeholder("Name")),
            command: cx.new(|cx| {
                InputState::new(window, cx).placeholder("Executable command (literal; no shell)")
            }),
            url: cx.new(|cx| InputState::new(window, cx).placeholder("https://server.example/mcp")),
            description: cx.new(|cx| InputState::new(window, cx).placeholder("Short description")),
            args: cx.new(|cx| {
                TextareaState::new(window, cx).placeholder("One literal argument per line")
            }),
            env: cx.new(|cx| TextareaState::new(window, cx).placeholder("KEY=value, one per line")),
            headers: cx
                .new(|cx| TextareaState::new(window, cx).placeholder("Header=value, one per line")),
            paste_open: false,
            import_json: cx.new(|cx| {
                TextareaState::new(window, cx).placeholder("JSON object containing mcpServers")
            }),
            skill_content: cx.new(|cx| {
                TextareaState::new(window, cx)
                    .placeholder("Markdown instructions (maximum 128 KiB)")
            }),
        }
    }

    pub fn set_snapshot(&mut self, snapshot: &Value, cx: &mut Context<Self>) {
        if self.snapshot != *snapshot {
            self.snapshot = snapshot.clone();
            cx.notify();
        }
    }

    fn invoke(&mut self, command: &'static str, args: Value, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.error = None;
        self.notice = None;
        let service = self.service.clone();
        let (tx, rx) = oneshot::channel();
        let spawn = thread::Builder::new()
            .name("gpui-settings-extensions".into())
            .stack_size(STACK)
            .spawn(move || {
                let _ = tx.send(service.invoke(command, args));
            });
        if let Err(error) = spawn {
            self.busy = false;
            self.error = Some(format!("Could not start settings worker: {error}"));
            cx.notify();
            return;
        }
        cx.spawn(async move |pane,cx| {
            let result=rx.await.unwrap_or_else(|_| Err("Extension settings worker stopped before replying.".into()));
            let _=pane.update(cx,|pane,cx| {
                pane.busy=false;
                match result {
                    Ok(value) => { pane.config=Some(value); pane.error=None;
                        if command=="save_extension_config" { pane.draft=None; pane.draft_kind=None; pane.paste_open=false; pane.notice=Some("Saved. Changes apply to the next harness launch; running agents were not restarted.".into()); }
                    }
                    Err(error)=>pane.error=Some(error),
                }
                cx.notify();
            });
        }).detach();
        cx.notify();
    }

    pub fn load(&mut self, cx: &mut Context<Self>) {
        self.invoke("get_extension_config", json!({}), cx);
    }

    fn set_field(
        state: &Entity<InputState>,
        value: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        state.update(cx, |input, cx| input.set_value(value, window, cx));
    }
    fn set_area(
        state: &Entity<TextareaState>,
        value: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        state.update(cx, |input, cx| input.set_value(value, window, cx));
    }
    fn edit_server(&mut self, server: Value, window: &mut Window, cx: &mut Context<Self>) {
        self.draft_kind = Some("server");
        Self::set_field(&self.name, text(&server, "name"), window, cx);
        Self::set_field(&self.command, text(&server, "command"), window, cx);
        Self::set_field(&self.url, text(&server, "url"), window, cx);
        Self::set_area(&self.args, lines(&server, "args"), window, cx);
        Self::set_area(&self.env, pairs(&server["env"]), window, cx);
        Self::set_area(&self.headers, pairs(&server["headers"]), window, cx);
        self.draft = Some(server);
        self.error = None;
    }
    fn edit_skill(&mut self, skill: Value, window: &mut Window, cx: &mut Context<Self>) {
        self.draft_kind = Some("skill");
        Self::set_field(&self.name, text(&skill, "name"), window, cx);
        Self::set_field(&self.description, text(&skill, "description"), window, cx);
        Self::set_area(&self.skill_content, text(&skill, "content"), window, cx);
        self.draft = Some(skill);
        self.error = None;
    }
    fn new_server(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.edit_server(json!({"id":monitter_lib::model::id(),"name":"","enabled":true,"agentIds":[],"transport":"stdio","command":"","args":[],"env":{},"url":"","headers":{}}),window,cx);
    }
    fn new_skill(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.edit_skill(json!({"id":monitter_lib::model::id(),"name":"","description":"","enabled":true,"agentIds":[],"allAgents":false,"content":""}),window,cx);
    }
    fn save_draft(&mut self, cx: &mut Context<Self>) {
        let (Some(mut config), Some(mut draft), Some(kind)) =
            (self.config.clone(), self.draft.clone(), self.draft_kind)
        else {
            return;
        };
        let name = self.name.read(cx).value().to_string().trim().to_owned();
        if name.is_empty() {
            self.error = Some("Name is required.".into());
            cx.notify();
            return;
        }
        draft["name"] = json!(name);
        if kind == "server" {
            let transport = text(&draft, "transport");
            draft["command"] = json!(self.command.read(cx).value().to_string().trim());
            draft["url"] = json!(self.url.read(cx).value().to_string().trim());
            if transport == "stdio" && draft["command"].as_str().unwrap_or_default().is_empty() {
                self.error = Some("A stdio command is required.".into());
                cx.notify();
                return;
            }
            if transport == "http"
                && !draft["url"]
                    .as_str()
                    .unwrap_or_default()
                    .starts_with("http")
            {
                self.error = Some("An HTTP server needs an http:// or https:// URL.".into());
                cx.notify();
                return;
            }
            draft["args"] = json!(
                self.args
                    .read(cx)
                    .value()
                    .lines()
                    .filter(|line| !line.is_empty())
                    .collect::<Vec<_>>()
            );
            match parse_pairs(&self.env.read(cx).value(), false) {
                Ok(value) => draft["env"] = value,
                Err(error) => {
                    self.error = Some(error);
                    cx.notify();
                    return;
                }
            }
            match parse_pairs(&self.headers.read(cx).value(), true) {
                Ok(value) => draft["headers"] = value,
                Err(error) => {
                    self.error = Some(error);
                    cx.notify();
                    return;
                }
            }
            let mut items = list(&config, "mcpServers");
            if let Some(index) = items.iter().position(|item| item["id"] == draft["id"]) {
                items[index] = draft;
            } else {
                items.push(draft);
            }
            config["mcpServers"] = json!(items);
        } else {
            draft["description"] = json!(self.description.read(cx).value().to_string());
            let content = self.skill_content.read(cx).value().to_string();
            if content.trim().is_empty() {
                self.error = Some("Skill content is required.".into());
                cx.notify();
                return;
            }
            if content.len() > 131072 {
                self.error = Some("Skill content is limited to 128 KiB.".into());
                cx.notify();
                return;
            }
            draft["content"] = json!(content);
            let mut items = list(&config, "skills");
            if let Some(index) = items.iter().position(|item| item["id"] == draft["id"]) {
                items[index] = draft;
            } else {
                items.push(draft);
            }
            config["skills"] = json!(items);
        }
        self.invoke("save_extension_config", json!({"config":config}), cx);
    }

    fn import_json_text(&mut self, content: &str, cx: &mut Context<Self>) {
        match parse_mcp_import(content) {
            Ok(servers) => {
                let Some(mut config) = self.config.clone() else {
                    return;
                };
                let mut existing = list(&config, "mcpServers");
                existing.extend(servers);
                config["mcpServers"] = json!(existing);
                self.invoke("save_extension_config", json!({"config": config}), cx);
            }
            Err(error) => {
                self.error = Some(error);
                cx.notify();
            }
        }
    }

    fn choose_import(&mut self, skill: bool, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.error = None;
        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(
                if skill {
                    "Import Markdown skill (maximum 128 KiB)"
                } else {
                    "Import MCP JSON (maximum 512 KiB)"
                }
                .into(),
            ),
        });
        cx.spawn(async move |pane, cx| {
            let selected = match prompt.await {
                Ok(Ok(paths)) => Ok(paths.and_then(|paths| paths.into_iter().next())),
                Ok(Err(error)) => Err(error.to_string()),
                Err(error) => Err(error.to_string()),
            };
            let result = match selected {
                Ok(Some(path)) => Some(cx.background_executor().spawn(async move {
                    use std::io::Read;
                    if skill && !path.extension().and_then(|s| s.to_str()).is_some_and(|s| s.eq_ignore_ascii_case("md")) {
                        return Err("Choose a Markdown .md file.".to_owned());
                    }
                    let limit = if skill { 131072 } else { 524288 };
                    let file = std::fs::File::open(&path).map_err(|_| "Could not read selected file.".to_owned())?;
                    let mut bytes = Vec::new();
                    file.take(limit + 1).read_to_end(&mut bytes).map_err(|_| "Could not read selected file.".to_owned())?;
                    if bytes.len() > limit as usize { return Err("Import exceeds the allowed file size.".to_owned()); }
                    let content = String::from_utf8(bytes).map_err(|_| "Import must contain UTF-8 text.".to_owned())?;
                    let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("Imported skill").to_owned();
                    Ok((name, content))
                }).await),
                Ok(None) => None,
                Err(error) => Some(Err(error)),
            };
            let _ = pane.update(cx, |pane, cx| {
                pane.busy = false;
                match result {
                    Some(Ok((name, content))) if skill => {
                        // Import creates a reviewable editor draft; Save remains explicit.
                        pane.draft = Some(json!({"id":monitter_lib::model::id(),"name":name,"description":"","enabled":true,"agentIds":[],"allAgents":false,"content":content}));
                        pane.draft_kind = Some("skill-import");
                    }
                    Some(Ok((_, content))) => pane.import_json_text(&content, cx),
                    Some(Err(error)) => pane.error = Some(error),
                    None => {},
                }
                cx.notify();
            });
        }).detach();
        cx.notify();
    }

    pub fn render(
        &mut self,
        palette: Palette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if self.draft_kind == Some("skill-import") {
            if let Some(skill) = self.draft.clone() {
                self.edit_skill(skill, window, cx);
            }
        }
        let mut panel = super::card("MCP servers, skills and plugins", palette)
            .child(super::note("Private local configuration. Editing never starts or restarts an agent; changes apply at the next harness launch.", palette));
        if let Some(error) = &self.error {
            panel = panel.child(div().text_color(rgb(0xd65b5b)).child(error.clone()));
        }
        if let Some(notice) = &self.notice {
            panel = panel.child(div().text_color(rgb(palette.accent)).child(notice.clone()));
        }
        if self.config.is_none() {
            if self.busy {
                panel = panel.child("Loading extension configuration…");
            } else {
                panel = panel.child(
                    Button::new("settings-extensions-load")
                        .label("Load extension settings")
                        .on_click(cx.listener(|pane, _, _, cx| pane.load(cx))),
                );
            }
            return panel.into_any_element();
        }
        let config = self.config.clone().unwrap_or(Value::Null);
        panel = panel.child(
            div()
                .flex()
                .gap(px(crate::design::space::TIGHT))
                .child(
                    Button::new("extensions-refresh")
                        .label("Refresh")
                        .disabled(self.busy)
                        .on_click(cx.listener(|pane, _, _, cx| pane.load(cx))),
                )
                .child(
                    Button::new("extensions-tab-servers")
                        .label("MCP servers")
                        .on_click(cx.listener(|pane, _, _, cx| {
                            pane.tab = "servers";
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("extensions-tab-skills")
                        .label("Skills")
                        .on_click(cx.listener(|pane, _, _, cx| {
                            pane.tab = "skills";
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("extensions-tab-plugins")
                        .label("Plugins")
                        .on_click(cx.listener(|pane, _, _, cx| {
                            pane.tab = "plugins";
                            cx.notify();
                        })),
                ),
        );
        if self.tab == "servers" {
            panel = panel.child(super::note("Enabled servers are available only to assigned agents. Leaving assignment blank means no agent can use it.", palette))
                .child(div().flex().flex_wrap().gap(px(7.))
                    .child(Button::new("extensions-import-json").label("Import JSON").disabled(self.busy).on_click(cx.listener(|pane, _, _, cx| pane.choose_import(false, cx))))
                    .child(Button::new("extensions-paste-json").label("Paste JSON").disabled(self.busy).on_click(cx.listener(|pane, _, _, cx| { pane.paste_open = !pane.paste_open; cx.notify(); }))));
            if self.paste_open {
                panel = panel
                    .child(super::field(
                        "Paste MCP JSON",
                        "Only a mcpServers object is accepted. Maximum 512 KiB.",
                        Textarea::new(&self.import_json).text_size(crate::settings::settings_text_size(0.86))
                            .aria_label("Paste MCP JSON")
                            .h(px(105.)),
                        palette,
                    ))
                    .child(
                        Button::new("extensions-import-pasted-json")
                            .label("Import pasted JSON")
                            .disabled(self.busy)
                            .on_click(cx.listener(|pane, _, _, cx| {
                                let content = pane.import_json.read(cx).value().to_string();
                                pane.import_json_text(&content, cx);
                            })),
                    );
            }
            panel = panel.child(
                Button::new("settings-extension-add-server")
                    .label("Add MCP server")
                    .disabled(self.busy)
                    .on_click(cx.listener(|pane, _, window, cx| pane.new_server(window, cx))),
            );
            for server in list(&config, "mcpServers") {
                let id = text(&server, "id");
                let enabled = server["enabled"].as_bool().unwrap_or(false);
                let mut row = div()
                    .flex()
                    .items_center()
                    .gap(px(crate::design::space::TIGHT))
                    .p(px(crate::design::space::INSET))
                    .rounded(px(crate::design::radius::CONTROL))
                    .bg(crate::design::surface_color(palette.paper))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(3.))
                            .child(
                                div()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(text(&server, "name")),
                            )
                            .child(super::note(
                                format!(
                                    "{} · {}",
                                    text(&server, "transport"),
                                    if text(&server, "transport") == "stdio" {
                                        text(&server, "command")
                                    } else {
                                        safe_http_location(&text(&server, "url"))
                                    }
                                ),
                                palette,
                            )),
                    );
                let toggle_id = id.clone();
                row = row.child(
                    Switch::new(format!("extension-enable-{id}"))
                        .accessibility_label(if enabled { "Enabled" } else { "Disabled" })
                        .checked(enabled)
                        .disabled(self.busy)
                        .on_change(cx.listener(move |pane, value, _, cx| {
                            pane.toggle_entry("mcpServers", &toggle_id, *value, cx)
                        })),
                );
                let edit = server.clone();
                row = row.child(
                    Button::new(format!("extension-edit-{id}"))
                        .label("Edit")
                        .disabled(self.busy)
                        .on_click(cx.listener(move |pane, _, window, cx| {
                            pane.edit_server(edit.clone(), window, cx)
                        })),
                );
                let delete_id = id.clone();
                row = row.child(
                    Button::new(format!("extension-delete-{id}"))
                        .label("Remove")
                        .disabled(self.busy)
                        .on_click(cx.listener(move |pane, _, _, cx| {
                            pane.delete_entry("mcpServers", &delete_id, cx)
                        })),
                );
                panel = panel.child(row);
            }
        } else if self.tab == "skills" {
            panel = panel
                .child(super::note(
                    "Skills are Markdown instructions applied on the next harness launch.",
                    palette,
                ))
                .child(
                    Button::new("extensions-import-markdown")
                        .label("Import .md")
                        .disabled(self.busy)
                        .on_click(cx.listener(|pane, _, _, cx| pane.choose_import(true, cx))),
                );
            panel = panel.child(
                Button::new("settings-extension-add-skill")
                    .label("Add skill")
                    .disabled(self.busy)
                    .on_click(cx.listener(|pane, _, window, cx| pane.new_skill(window, cx))),
            );
            for skill in list(&config, "skills") {
                let id = text(&skill, "id");
                let enabled = skill["enabled"].as_bool().unwrap_or(false);
                let mut row = div()
                    .flex()
                    .items_center()
                    .gap(px(crate::design::space::TIGHT))
                    .p(px(crate::design::space::INSET))
                    .rounded(px(crate::design::radius::CONTROL))
                    .bg(crate::design::surface_color(palette.paper))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(3.))
                            .child(
                                div()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(text(&skill, "name")),
                            )
                            .child(super::note(text(&skill, "description"), palette)),
                    );
                let toggle_id = id.clone();
                row = row.child(
                    Switch::new(format!("skill-enable-{id}"))
                        .accessibility_label(if enabled { "Enabled" } else { "Disabled" })
                        .checked(enabled)
                        .disabled(self.busy)
                        .on_change(cx.listener(move |pane, value, _, cx| {
                            pane.toggle_entry("skills", &toggle_id, *value, cx)
                        })),
                );
                let edit = skill.clone();
                row = row.child(
                    Button::new(format!("skill-edit-{id}"))
                        .label("Edit")
                        .disabled(self.busy)
                        .on_click(cx.listener(move |pane, _, window, cx| {
                            pane.edit_skill(edit.clone(), window, cx)
                        })),
                );
                let delete_id = id.clone();
                row = row.child(
                    Button::new(format!("skill-delete-{id}"))
                        .label("Remove")
                        .disabled(self.busy)
                        .on_click(cx.listener(move |pane, _, _, cx| {
                            pane.delete_entry("skills", &delete_id, cx)
                        })),
                );
                panel = panel.child(row);
            }
        } else {
            panel =
                panel.child(super::note("Native plugins are not supported. Add reviewed MCP servers or Markdown skills instead.", palette));
        }
        if let Some(kind) = self.draft_kind {
            let mut editor = div()
                .flex()
                .flex_col()
                .gap(px(crate::design::space::TIGHT))
                .p(px(18.))
                .border_1()
                .border_color(rgb(palette.line))
                .rounded(px(10.))
                .child(if kind == "server" {
                    "MCP server"
                } else {
                    "Managed skill"
                })
                .child(super::field(
                    "Name",
                    "A readable name for this extension.",
                    Input::new(&self.name).text_size(crate::settings::settings_text_size(0.86)).aria_label(if kind == "server" {
                        "MCP server name"
                    } else {
                        "Skill name"
                    }),
                    palette,
                ));
            if kind == "server" {
                let http = self
                    .draft
                    .as_ref()
                    .is_some_and(|d| text(d, "transport") == "http");
                editor = editor.child(
                    div()
                        .flex()
                        .gap(px(crate::design::space::TIGHT))
                        .child(
                            Button::new("extension-stdio")
                                .label("Local stdio")
                                .on_click(cx.listener(|pane, _, _, cx| {
                                    if let Some(d) = &mut pane.draft {
                                        d["transport"] = json!("stdio");
                                    }
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new("extension-http")
                                .label("HTTP endpoint")
                                .on_click(cx.listener(|pane, _, _, cx| {
                                    if let Some(d) = &mut pane.draft {
                                        d["transport"] = json!("http");
                                    }
                                    cx.notify();
                                })),
                        ),
                );
                if http {
                    editor = editor
                        .child("URL")
                        .child(Input::new(&self.url).text_size(crate::settings::settings_text_size(0.86)).aria_label("MCP server URL"))
                        .child("HTTP headers (stored privately; shown only while editing)")
                        .child(
                            Textarea::new(&self.headers).text_size(crate::settings::settings_text_size(0.86))
                                .aria_label("MCP HTTP headers")
                                .h(px(72.)),
                        );
                } else {
                    editor = editor
                        .child("Command")
                        .child(Input::new(&self.command).text_size(crate::settings::settings_text_size(0.86)).aria_label("MCP executable command"))
                        .child("Literal arguments, one per line")
                        .child(
                            Textarea::new(&self.args).text_size(crate::settings::settings_text_size(0.86))
                                .aria_label("MCP literal arguments")
                                .h(px(72.)),
                        )
                        .child("Environment values (stored privately; shown only while editing)")
                        .child(
                            Textarea::new(&self.env).text_size(crate::settings::settings_text_size(0.86))
                                .aria_label("MCP environment values")
                                .h(px(72.)),
                        );
                }
                editor = editor.child("Available to selected agents");
                let assigned = list(self.draft.as_ref().unwrap_or(&Value::Null), "agentIds");
                for agent in list(&self.snapshot, "agents")
                    .into_iter()
                    .filter(|a| a["internal"] != true)
                {
                    let aid = text(&agent, "id");
                    let checked = assigned.iter().any(|v| v.as_str() == Some(&aid));
                    let label = text(&agent, "name");
                    editor = editor.child(
                        Checkbox::new(format!("extension-agent-{aid}"))
                            .label(label)
                            .checked(checked)
                            .on_change(cx.listener(move |pane, value, _, cx| {
                                pane.toggle_assignment(&aid, *value, cx)
                            })),
                    );
                }
            } else {
                editor = editor
                    .child("Description")
                    .child(Input::new(&self.description).text_size(crate::settings::settings_text_size(0.86)).aria_label("Skill description"))
                    .child("Markdown content")
                    .child(
                        Textarea::new(&self.skill_content).text_size(crate::settings::settings_text_size(0.86))
                            .aria_label("Skill Markdown content")
                            .h(px(160.)),
                    )
                    .child(
                        Checkbox::new("skill-all-agents")
                            .label("Available to all agents, including future agents")
                            .checked(self.draft.as_ref().is_some_and(|d| d["allAgents"] == true))
                            .on_change(cx.listener(|pane, value, _, cx| {
                                if let Some(d) = &mut pane.draft {
                                    d["allAgents"] = json!(*value);
                                }
                                cx.notify();
                            })),
                    );
                if self.draft.as_ref().is_some_and(|d| d["allAgents"] != true) {
                    let assigned = list(self.draft.as_ref().unwrap_or(&Value::Null), "agentIds");
                    for agent in list(&self.snapshot, "agents")
                        .into_iter()
                        .filter(|a| a["internal"] != true)
                    {
                        let aid = text(&agent, "id");
                        let checked = assigned.iter().any(|v| v.as_str() == Some(&aid));
                        let label = text(&agent, "name");
                        editor = editor.child(
                            Checkbox::new(format!("skill-agent-{aid}"))
                                .label(label)
                                .checked(checked)
                                .on_change(cx.listener(move |pane, value, _, cx| {
                                    pane.toggle_assignment(&aid, *value, cx)
                                })),
                        );
                    }
                }
            }
            editor =
                editor.child(
                    div()
                        .flex()
                        .gap(px(crate::design::space::TIGHT))
                        .child(
                            Button::new("extension-save")
                                .label(if self.busy { "Saving…" } else { "Save" })
                                .disabled(self.busy)
                                .on_click(cx.listener(|pane, _, _, cx| pane.save_draft(cx))),
                        )
                        .child(Button::new("extension-cancel").label("Cancel").on_click(
                            cx.listener(|pane, _, _, cx| {
                                pane.draft = None;
                                pane.draft_kind = None;
                                cx.notify();
                            }),
                        )),
                );
            panel = panel.child(editor);
        }
        panel.into_any_element()
    }

    fn toggle_entry(&mut self, key: &str, id: &str, enabled: bool, cx: &mut Context<Self>) {
        let Some(mut config) = self.config.clone() else {
            return;
        };
        let mut entries = list(&config, key);
        if let Some(item) = entries.iter_mut().find(|item| text(item, "id") == id) {
            item["enabled"] = json!(enabled);
        }
        config[key] = json!(entries);
        self.invoke("save_extension_config", json!({"config":config}), cx);
    }
    fn delete_entry(&mut self, key: &str, id: &str, cx: &mut Context<Self>) {
        let Some(mut config) = self.config.clone() else {
            return;
        };
        let entries = list(&config, key)
            .into_iter()
            .filter(|item| text(item, "id") != id)
            .collect::<Vec<_>>();
        config[key] = json!(entries);
        self.invoke("save_extension_config", json!({"config":config}), cx);
    }
    fn toggle_assignment(&mut self, id: &str, assigned: bool, cx: &mut Context<Self>) {
        if let Some(draft) = &mut self.draft {
            let mut ids = list(draft, "agentIds")
                .into_iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .filter(|value| value != id)
                .collect::<Vec<_>>();
            if assigned {
                ids.push(id.to_owned());
            }
            draft["agentIds"] = json!(ids);
            cx.notify();
        }
    }
}

fn pairs(value: &Value) -> String {
    value
        .as_object()
        .map(|pairs| {
            pairs
                .iter()
                .map(|(k, v)| format!("{k}={}", v.as_str().unwrap_or_default()))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

// Match the Tauri import schema; never accept arbitrary configuration fields.
fn parse_mcp_import(content: &str) -> Result<Vec<Value>, String> {
    if content.len() > 524288 {
        return Err("JSON imports are limited to 512 KiB.".into());
    }
    let value: Value =
        serde_json::from_str(content).map_err(|_| "Import must contain valid JSON.".to_owned())?;
    let root = value
        .as_object()
        .ok_or("The import must be a JSON object.")?;
    if root.keys().any(|key| key != "mcpServers") {
        return Err("Only mcpServers is accepted in this import.".into());
    }
    let servers = root
        .get("mcpServers")
        .and_then(Value::as_object)
        .ok_or("Expected mcpServers keyed by server name.")?;
    servers.iter().map(|(name, value)| {
        let raw = value.as_object().ok_or("Each server must be an object.")?;
        if name.trim().is_empty() || raw.keys().any(|key| !["command", "args", "env", "type", "url", "headers"].contains(&key.as_str())) {
            return Err("Server name is empty or contains unsupported fields.".into());
        }
        if raw.get("type").is_some_and(|t| t != "http" && t != "stdio") { return Err("Unsupported server transport.".into()); }
        let http = raw.get("type").is_some_and(|t| t == "http") || raw.contains_key("url");
        let command = text(value, "command");
        let url = text(value, "url");
        if http {
            let parsed = url::Url::parse(&url).map_err(|_| "HTTP server needs a valid URL.")?;
            if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() { return Err("HTTP server needs an http:// or https:// URL.".into()); }
        } else if command.trim().is_empty() { return Err("A stdio command is required.".into()); }
        let args = raw.get("args").cloned().unwrap_or(json!([]));
        if !args.as_array().is_some_and(|a| a.iter().all(Value::is_string)) { return Err("Arguments must be text.".into()); }
        let pairs = |key: &str| -> Result<Value, String> {
            let pairs = raw.get(key).cloned().unwrap_or(json!({}));
            if !pairs.as_object().is_some_and(|p| p.values().all(Value::is_string)) { return Err("Environment and headers must contain text values.".into()); }
            Ok(pairs)
        };
        Ok(json!({"id":monitter_lib::model::id(),"name":name.trim(),"enabled":true,"agentIds":[],"transport":if http {"http"} else {"stdio"},"command":command,"args":args,"env":pairs("env")?,"url":url,"headers":pairs("headers")?}))
    }).collect()
}

#[cfg(test)]
mod import_tests {
    use super::*;
    #[::core::prelude::v1::test]
    fn mcp_import_validates_schema_and_keeps_assignment_explicit() {
        let servers = parse_mcp_import(r#"{"mcpServers":{"local":{"command":"npx","args":["tool"],"env":{"TOKEN":"private"}}}}"#).unwrap();
        assert_eq!(servers[0]["agentIds"], json!([]));
        assert_eq!(servers[0]["transport"], "stdio");
        assert!(parse_mcp_import(r#"{"mcpServers":{"bad":{"url":"file:///tmp/tool"}}}"#).is_err());
        assert!(parse_mcp_import(r#"{"mcpServers":{"bad":{"command":"x","args":[1]}}}"#).is_err());
        assert!(parse_mcp_import(r#"{"mcpServers":{},"plugins":[]}"#).is_err());
    }
}

fn safe_http_location(value: &str) -> String {
    url::Url::parse(value)
        .ok()
        .filter(|url| matches!(url.scheme(), "http" | "https"))
        .map(|url| format!("{}{}", url.origin().ascii_serialization(), url.path()))
        .unwrap_or_else(|| "HTTP endpoint".into())
}
