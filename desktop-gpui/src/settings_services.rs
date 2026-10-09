//! Owner-local service integration settings, kept out of workspace snapshots.
//!
//! This pane deliberately uses only the private GPUI service facade. Secrets,
//! extension payloads and mail credentials never come from or enter Snapshot.
use crate::design::{Palette, ui_scaled_px as px};
use futures::channel::oneshot;
use gpui_kit::{
    component::{
        Disableable,
        button::*,
        input::{Input, InputState},
    },
    prelude::*,
    *,
};
use monitter_lib::experimental_desktop::DesktopService;
use serde_json::{Value, json};
use std::{collections::HashSet, thread, time::Duration};

#[path = "settings_extensions.rs"]
mod settings_extensions;
#[path = "settings_secrets.rs"]
mod settings_secrets;

const IPC_STACK: usize = 8 * 1024 * 1024;

fn text(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}
fn rows(value: &Value, key: &str) -> Vec<Value> {
    value
        .get(key)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}
fn card(title: &str, palette: Palette) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(16.))
        .p(px(18.))
        .rounded(px(10.))
        .text_size(crate::settings::settings_text_size(0.86))
        .border_1()
        .border_color(rgb(palette.line))
        .bg(crate::design::surface_color(palette.panel))
        .child(
            div()
                .text_size(crate::settings::settings_text_size(1.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(palette.ink))
                .child(title.to_owned()),
        )
}
fn note(value: impl Into<SharedString>, palette: Palette) -> Div {
    div()
        .text_size(crate::settings::settings_text_size(0.8))
        .text_color(rgb(palette.muted))
        .child(value.into())
}

fn field(label: &str, detail: &str, control: impl IntoElement, palette: Palette) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(5.))
        .child(
            div()
                .text_size(crate::settings::settings_text_size(0.86))
                .text_color(rgb(palette.muted))
                .child(label.to_owned()),
        )
        .child(control)
        .child(note(detail.to_owned(), palette))
}

/// Retained owner-only integration settings. Network/auth/config operations
/// begin only from an explicit button action; IPC work runs off the UI thread.
pub struct ServicePreferencesPane {
    service: DesktopService,
    snapshot: Value,
    accounts: Option<Value>,
    mail: Option<Value>,
    mail_attempt: Option<Value>,
    mail_client_id: Entity<InputState>,
    mail_client_secret: Entity<InputState>,
    extensions: Entity<settings_extensions::ExtensionsPane>,
    secrets: Entity<settings_secrets::SecretsPane>,
    capabilities: HashSet<String>,
    capabilities_checked: bool,
    mail_poll_generation: u64,
    busy: HashSet<String>,
    errors: std::collections::HashMap<String, String>,
}

impl ServicePreferencesPane {
    pub fn new(
        service: DesktopService,
        snapshot: &Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let extensions = cx.new(|cx| {
            settings_extensions::ExtensionsPane::new(service.clone(), snapshot, window, cx)
        });
        let secrets = cx.new(|cx| settings_secrets::SecretsPane::new(service.clone(), window, cx));
        let mut pane = Self {
            service,
            snapshot: snapshot.clone(),
            accounts: None,
            mail: None,
            mail_attempt: None,
            mail_client_id: cx
                .new(|cx| InputState::new(window, cx).placeholder("Google OAuth client ID")),
            mail_client_secret: cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("Optional client secret (blank keeps current)")
                    .masked(true)
            }),
            extensions,
            secrets,
            capabilities: HashSet::new(),
            capabilities_checked: false,
            mail_poll_generation: 0,
            busy: HashSet::new(),
            errors: Default::default(),
        };
        // Loading configuration is read-only; all auth, writes, and secret
        // values remain behind explicit user actions.
        pane.invoke("capabilities", "get_command_capabilities", json!({}), cx);
        pane.invoke("mail", "mail_get_config", json!({}), cx);
        pane
    }

    pub fn set_snapshot(&mut self, snapshot: &Value, cx: &mut Context<Self>) {
        if self.snapshot != *snapshot {
            self.snapshot = snapshot.clone();
            self.extensions
                .update(cx, |pane, cx| pane.set_snapshot(snapshot, cx));
            cx.notify();
        }
    }

    fn invoke(
        &mut self,
        section: &'static str,
        command: &'static str,
        args: Value,
        cx: &mut Context<Self>,
    ) {
        if !self.busy.insert(section.to_owned()) {
            return;
        }
        self.errors.remove(section);
        let service = self.service.clone();
        let (tx, rx) = oneshot::channel();
        let spawn = thread::Builder::new()
            .name(format!("gpui-settings-{section}"))
            .stack_size(IPC_STACK)
            .spawn(move || {
                let _ = tx.send(service.invoke(command, args));
            });
        if let Err(error) = spawn {
            self.busy.remove(section);
            self.errors.insert(
                section.to_owned(),
                format!("Could not start native settings worker: {error}"),
            );
            cx.notify();
            return;
        }
        let section = section.to_owned();
        let command = command.to_owned();
        cx.spawn(async move |pane, cx| {
            let result = rx
                .await
                .unwrap_or_else(|_| Err("Native settings worker stopped before replying.".into()));
            let _ = pane.update(cx, |pane, cx| {
                pane.busy.remove(&section);
                match result {
                    Ok(value) => match section.as_str() {
                        "capabilities" => {
                            pane.capabilities = rows(&value, "commands")
                                .iter()
                                .filter_map(Value::as_str)
                                .map(str::to_owned)
                                .collect();
                            pane.capabilities_checked = true;
                            if ["get_extension_config", "save_extension_config"]
                                .iter()
                                .all(|c| pane.capabilities.contains(*c))
                            {
                                pane.extensions
                                    .update(cx, |extensions, cx| extensions.load(cx));
                            }
                            if [
                                "list_environment_secrets",
                                "set_environment_secret",
                                "delete_environment_secret",
                            ]
                            .iter()
                            .all(|c| pane.capabilities.contains(*c))
                            {
                                pane.secrets.update(cx, |secrets, cx| secrets.load(cx));
                            }
                        }
                        "accounts" => pane.accounts = Some(value),
                        "mail" if command == "mail_start_connect" => {
                            pane.mail_attempt = Some(value);
                            pane.start_mail_status_poll(cx);
                        }
                        "mail" if command == "mail_connect_status" => {
                            if value["status"] == "connected" {
                                pane.mail_attempt = None;
                                pane.invoke("mail", "mail_get_config", json!({}), cx);
                            }
                            if value["status"] == "error" || value["status"] == "cancelled" {
                                pane.mail_attempt = None;
                                pane.mail_poll_generation =
                                    pane.mail_poll_generation.wrapping_add(1);
                            }
                            pane.errors
                                .insert("mail-status".into(), text(&value, "status"));
                        }
                        "mail" if command == "mail_cancel_connect" => {
                            pane.mail_attempt = None;
                            pane.mail_poll_generation = pane.mail_poll_generation.wrapping_add(1);
                        }
                        "mail" => {
                            pane.mail = Some(value);
                            pane.mail_attempt = None;
                        }
                        "approvals" => pane.snapshot = value,
                        _ => {}
                    },
                    Err(error) => {
                        if section == "capabilities" {
                            pane.capabilities_checked = true;
                        }
                        if section == "mail" && command == "mail_connect_status" {
                            pane.mail_attempt = None;
                            pane.mail_poll_generation = pane.mail_poll_generation.wrapping_add(1);
                        }
                        pane.errors.insert(section.clone(), error);
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn start_mail_status_poll(&mut self, cx: &mut Context<Self>) {
        self.mail_poll_generation = self.mail_poll_generation.wrapping_add(1);
        let generation = self.mail_poll_generation;
        cx.spawn(async move |pane, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(900))
                    .await;
                let should_continue = pane
                    .read_with(cx, |pane, _| {
                        pane.mail_poll_generation == generation && pane.mail_attempt.is_some()
                    })
                    .unwrap_or(false);
                if !should_continue {
                    break;
                }
                let _ = pane.update(cx, |pane, cx| {
                    if pane.mail_poll_generation != generation || pane.busy.contains("mail") {
                        return;
                    }
                    let Some(attempt) = pane.mail_attempt.as_ref() else {
                        return;
                    };
                    let attempt_id = text(attempt, "attemptId");
                    let expires_at = attempt["expiresAt"].as_i64().unwrap_or_default();
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis() as i64;
                    if now >= expires_at {
                        pane.mail_attempt = None;
                        pane.mail_poll_generation = pane.mail_poll_generation.wrapping_add(1);
                        pane.errors.insert(
                            "mail-status".into(),
                            "Google sign-in timed out. Start again when ready.".into(),
                        );
                        pane.invoke(
                            "mail",
                            "mail_cancel_connect",
                            json!({"attemptId":attempt_id}),
                            cx,
                        );
                    } else {
                        pane.invoke(
                            "mail",
                            "mail_connect_status",
                            json!({"attemptId":attempt_id}),
                            cx,
                        );
                    }
                });
            }
        })
        .detach();
    }

    fn render_accounts(&mut self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let mut panel = card("Local Codex accounts", palette)
            .child(note("Account homes are read from the local harness configuration. Choosing an account for a new agent or chat stays in the agent editor; this page does not change a default account.", palette))
            .child(Button::new("settings-accounts-load").label("Load local accounts").disabled(self.busy.contains("accounts"))
                .on_click(cx.listener(|pane, _, _, cx| pane.invoke("accounts", "list_codex_accounts", json!({}), cx))));
        if let Some(accounts) = &self.accounts {
            let list = accounts.as_array().cloned().unwrap_or_default();
            if list.is_empty() {
                panel = panel.child(note(
                    "No Codex accounts were returned by the local harness.",
                    palette,
                ));
            }
            for account in list {
                panel = panel.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(crate::design::space::TIGHT))
                        .p(px(crate::design::space::INSET))
                        .rounded_md()
                        .bg(crate::design::surface_color(palette.paper))
                        .child(text(&account, "label"))
                        .child(
                            div()
                                .text_size(crate::settings::settings_text_size(0.8))
                                .text_color(rgb(palette.muted))
                                .child(text(&account, "home")),
                        ),
                );
            }
        }
        if let Some(error) = self.errors.get("accounts") {
            panel = panel.child(div().text_color(rgb(0xd65b5b)).child(error.clone()));
        }
        panel.into_any_element()
    }

    fn render_providers(&self, palette: Palette) -> AnyElement {
        let agents = rows(&self.snapshot, "agents");
        let hosts = rows(&self.snapshot, "hosts");
        let mut grouped: std::collections::BTreeMap<String, Vec<Value>> = Default::default();
        for agent in agents.into_iter().filter(|agent| agent["internal"] != true) {
            grouped
                .entry(text(&agent, "provider"))
                .or_default()
                .push(agent);
        }
        let mut panel = card("Provider and harness setup", palette)
            .child(note("These rows reflect saved agents in the current native snapshot. Provider, host, model, account and ACP launcher changes belong to each agent's editor; no account probe or provider request runs while viewing this page.", palette));
        if grouped.is_empty() {
            panel = panel.child(note("No user agents are configured.", palette));
        }
        for (provider, agents) in grouped {
            let mut group = div()
                .flex()
                .flex_col()
                .gap(px(crate::design::space::TIGHT))
                .p(px(crate::design::space::INSET))
                .rounded_md()
                .bg(crate::design::surface_color(palette.paper))
                .child(div().text_color(rgb(palette.ink)).child(format!(
                    "{provider} · {} agent{}",
                    agents.len(),
                    if agents.len() == 1 { "" } else { "s" }
                )));
            for agent in agents {
                let host_id = text(&agent, "hostId");
                let host = hosts.iter().find(|host| text(host, "id") == host_id);
                let host_name = host
                    .map(|host| text(host, "name"))
                    .unwrap_or_else(|| "Unknown host".into());
                let model = text(&agent, "model");
                let suffix = if model.is_empty() {
                    "harness default".into()
                } else {
                    model
                };
                group = group.child(
                    div()
                        .text_size(crate::settings::settings_text_size(0.8))
                        .text_color(rgb(palette.muted))
                        .child(format!(
                            "{} · {} · {} · {}",
                            text(&agent, "name"),
                            host_name,
                            text(&agent, "cwd"),
                            suffix
                        )),
                );
            }
            panel = panel.child(group);
        }
        panel.into_any_element()
    }

    fn render_collaboration(&self, palette: Palette) -> AnyElement {
        let agents = rows(&self.snapshot, "agents")
            .into_iter()
            .filter(|agent| agent["internal"] != true)
            .collect::<Vec<_>>();
        let enabled = agents
            .iter()
            .filter(|agent| agent["collaborationEnabled"] == true)
            .count();
        let collaborations = rows(&self.snapshot, "collaborations");
        let queued = rows(&self.snapshot, "queuedMessages");
        let panel = card("Agent collaboration", palette)
            .child(note(format!("{} of {} saved agents have collaboration enabled.", enabled, agents.len()), palette))
            .child(note(format!("Current snapshot: {} collaboration records · {} queued messages.", collaborations.len(), queued.len()), palette))
            .child(note("Per-agent collaboration permissions and provider handoff settings are edited in the agent editor. This overview reads only the current owner snapshot and does not start or send any delegated work.", palette));
        panel.into_any_element()
    }

    fn render_extensions(
        &mut self,
        palette: Palette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if !self.capabilities_checked {
            return card("MCP servers, skills and plugins", palette)
                .child("Checking local settings capabilities…")
                .into_any_element();
        }
        if !["get_extension_config", "save_extension_config"]
            .iter()
            .all(|command| self.capabilities.contains(*command))
        {
            return card("MCP servers, skills and plugins", palette).child(note("This running desktop server does not expose the owner-only extension settings commands. Restart it with a current GPUI build to manage extension configuration.", palette)).into_any_element();
        }
        self.extensions
            .update(cx, |pane, cx| pane.render(palette, window, cx))
    }

    fn render_secrets(
        &mut self,
        palette: Palette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if !self.capabilities_checked {
            return card("Shared agent environment secrets", palette)
                .child("Checking local settings capabilities…")
                .into_any_element();
        }
        if ![
            "list_environment_secrets",
            "set_environment_secret",
            "delete_environment_secret",
        ]
        .iter()
        .all(|command| self.capabilities.contains(*command))
        {
            return card("Shared agent environment secrets", palette).child(note("This running desktop server does not expose the owner-only environment-vault commands. Restart it with a current GPUI build to manage secret metadata.", palette)).into_any_element();
        }
        self.secrets
            .update(cx, |pane, cx| pane.render(palette, window, cx))
    }

    fn eligible_mail_agents(&self) -> Vec<Value> {
        let hosts = rows(&self.snapshot, "hosts");
        rows(&self.snapshot, "agents")
            .into_iter()
            .filter(|agent| {
                let host = hosts
                    .iter()
                    .find(|host| text(host, "id") == text(agent, "hostId"));
                agent["internal"] != true
                    && host.is_some_and(|host| text(host, "kind") == "local")
                    && ["codex", "claude", "acp"].contains(&text(agent, "provider").as_str())
            })
            .collect()
    }

    fn render_mail(&mut self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let mut panel = card("Private Gmail integration", palette)
            .child(note("Gmail credentials are held by the native owner service. Sign-in starts only after you press Connect; configured account access remains explicit per agent.", palette))
            .child(Button::new("settings-mail-load").label("Load Mail settings").disabled(self.busy.contains("mail"))
                .on_click(cx.listener(|pane, _, _, cx| pane.invoke("mail", "mail_get_config", json!({}), cx))));
        if let Some(error) = self.errors.get("mail") {
            panel = panel.child(div().text_color(rgb(0xd65b5b)).child(error.clone()));
        }
        if let Some(config) = self.mail.clone() {
            panel = panel.child(div().text_size(crate::settings::settings_text_size(0.8)).child(format!("Google client ID: {}", text(&config, "clientId"))))
                .child(note(format!("Client secret configured: {}", config["clientSecretConfigured"].as_bool().unwrap_or(false)), palette))
                .child(field("OAuth client ID", "Google application client ID", Input::new(&self.mail_client_id).text_size(crate::settings::settings_text_size(0.86)).aria_label("Google OAuth client ID"), palette))
                .child(field("OAuth client secret", "Leave blank to keep the saved secret. Stored securely and never shown again.", Input::new(&self.mail_client_secret).text_size(crate::settings::settings_text_size(0.86)).aria_label("Google OAuth client secret"), palette))
                .child(Button::new("settings-mail-save").label("Save Google app settings").disabled(self.busy.contains("mail"))
                    .on_click(cx.listener(|pane, _, _, cx| {
                        let draft_id = pane.mail_client_id.read(cx).value().to_string();
                        let current_id = pane.mail.as_ref().map(|value| text(value, "clientId")).unwrap_or_default();
                        let id = if draft_id.trim().is_empty() { current_id } else { draft_id.trim().to_owned() };
                        let secret = pane.mail_client_secret.read(cx).value().to_string();
                        let secret = if secret.is_empty() { Value::Null } else { json!(secret) };
                        let revision = pane.mail.as_ref().map(|value| text(value, "revision")).unwrap_or_default();
                        pane.invoke("mail", "mail_configure_gmail", json!({"revision":revision,"clientId":id.trim(),"clientSecret":secret}), cx);
                    })));
            panel = panel.child(
                Button::new("settings-mail-connect")
                    .label(if rows(&config, "accounts").is_empty() {
                        "Connect Gmail in system browser"
                    } else {
                        "Connect or reauthorize Gmail"
                    })
                    .disabled(
                        self.busy.contains("mail")
                            || self.mail_attempt.is_some()
                            || text(&config, "clientId").trim().is_empty(),
                    )
                    .on_click(cx.listener(|pane, _, _, cx| {
                        pane.invoke("mail", "mail_start_connect", json!({}), cx)
                    })),
            );
            if let Some(attempt) = &self.mail_attempt {
                panel = panel
                    .child(note(
                        format!(
                            "Sign-in attempt {} · expires {}",
                            text(attempt, "attemptId"),
                            attempt["expiresAt"]
                        ),
                        palette,
                    ))
                    .child(
                        Button::new("settings-mail-status")
                            .label("Check sign-in status")
                            .disabled(self.busy.contains("mail"))
                            .on_click(cx.listener(|pane, _, _, cx| {
                                let id = pane
                                    .mail_attempt
                                    .as_ref()
                                    .map(|value| text(value, "attemptId"))
                                    .unwrap_or_default();
                                pane.invoke(
                                    "mail",
                                    "mail_connect_status",
                                    json!({"attemptId":id}),
                                    cx,
                                );
                            })),
                    )
                    .child(
                        Button::new("settings-mail-cancel")
                            .label("Cancel sign-in")
                            .disabled(self.busy.contains("mail"))
                            .on_click(cx.listener(|pane, _, _, cx| {
                                let id = pane
                                    .mail_attempt
                                    .as_ref()
                                    .map(|value| text(value, "attemptId"))
                                    .unwrap_or_default();
                                pane.invoke(
                                    "mail",
                                    "mail_cancel_connect",
                                    json!({"attemptId":id}),
                                    cx,
                                );
                            })),
                    );
            }
            if let Some(status) = self.errors.get("mail-status") {
                panel = panel.child(note(format!("Sign-in status: {status}"), palette));
            }
            for account in rows(&config, "accounts") {
                let account_id = text(&account, "id");
                let revision = text(&config, "revision");
                let mut row = div()
                    .flex()
                    .flex_col()
                    .gap(px(crate::design::space::TIGHT))
                    .p(px(crate::design::space::INSET))
                    .rounded_md()
                    .bg(crate::design::surface_color(palette.paper))
                    .child(format!(
                        "{} · {}",
                        text(&account, "email"),
                        text(&account, "label")
                    ));
                for agent in self.eligible_mail_agents() {
                    let agent_id = text(&agent, "id");
                    let mut ids = rows(&account, "agentIds");
                    let assigned = ids.iter().any(|id| id.as_str() == Some(&agent_id));
                    if assigned {
                        ids.retain(|id| id.as_str() != Some(&agent_id));
                    } else {
                        ids.push(json!(agent_id));
                    }
                    let ids: Vec<String> = ids
                        .into_iter()
                        .filter_map(|id| id.as_str().map(str::to_owned))
                        .collect();
                    let acct = account_id.clone();
                    let rev = revision.clone();
                    let label = format!(
                        "{} {}",
                        if assigned { "Assigned:" } else { "Assign:" },
                        text(&agent, "name")
                    );
                    let key = format!(
                        "settings-mail-agent-{}-{}",
                        text(&account, "id"),
                        text(&agent, "id")
                    );
                    row = row.child(
                        Button::new(SharedString::from(key))
                            .label(label)
                            .disabled(self.busy.contains("mail"))
                            .on_click(cx.listener(move |pane, _, _, cx| {
                                pane.invoke(
                                    "mail",
                                    "mail_set_account_agents",
                                    json!({"revision":rev,"accountId":acct,"agentIds":ids}),
                                    cx,
                                )
                            })),
                    );
                }
                row = row.child(
                    Button::new(SharedString::from(format!(
                        "settings-mail-disconnect-{account_id}"
                    )))
                    .label("Disconnect account")
                    .disabled(self.busy.contains("mail"))
                    .on_click(cx.listener(move |pane, _, _, cx| {
                        pane.invoke(
                            "mail",
                            "mail_disconnect",
                            json!({"revision":revision,"accountId":account_id}),
                            cx,
                        )
                    })),
                );
                panel = panel.child(row);
            }
            panel = panel.child(note("Mail access is read-only for agents. Removing account access takes effect immediately; a newly assigned agent may need a new session.", palette));
        }
        panel.into_any_element()
    }

    fn render_approvals(&self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let mut panel = card("Saved approvals", palette)
            .child(note("These rules apply only to the exact agent, host, folder, provider and tool that created them. Revocation affects future requests.", palette));
        let rules = rows(&self.snapshot, "approvalRules");
        if rules.is_empty() {
            panel = panel.child(note(
                "No saved approval rules are present in this snapshot.",
                palette,
            ));
        }
        for rule in rules {
            let id = text(&rule, "id");
            let label = format!(
                "{} · {} · {} · {}",
                text(&rule, "agentId"),
                text(&rule, "hostId"),
                text(&rule, "cwd"),
                text(&rule, "tool")
            );
            panel = panel.child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(crate::design::space::TIGHT))
                    .p(px(crate::design::space::INSET))
                    .rounded_md()
                    .bg(crate::design::surface_color(palette.paper))
                    .child(label)
                    .child(
                        Button::new(SharedString::from(format!("settings-revoke-{id}")))
                            .label("Revoke")
                            .on_click(cx.listener(move |pane, _, _, cx| {
                                pane.invoke(
                                    "approvals",
                                    "revoke_approval_rule",
                                    json!({"ruleId":id}),
                                    cx,
                                )
                            })),
                    ),
            );
        }
        if let Some(error) = self.errors.get("approvals") {
            panel = panel.child(div().text_color(rgb(0xd65b5b)).child(error.clone()));
        }
        panel.into_any_element()
    }

    pub fn render_section(
        &mut self,
        tab_key: &str,
        palette: Palette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let _ = window;
        match tab_key {
            "accounts" => self.render_accounts(palette, cx),
            "providers" => self.render_providers(palette),
            "collaboration" => self.render_collaboration(palette),
            "mail" => self.render_mail(palette, cx),
            "approvals" => self.render_approvals(palette, cx),
            "extensions" => self.render_extensions(palette, window, cx),
            "secrets" => self.render_secrets(palette, window, cx),
            "lan" => card("LAN access", palette).child(note("LAN listener controls are provided by the dedicated native LAN settings section.", palette)).into_any_element(),
            _ => card("Integration settings", palette).child(note("This settings section is not available.", palette)).into_any_element(),
        }
    }
}
