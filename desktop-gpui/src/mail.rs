//! Owner-local native Gmail. Account administration never uses the LAN bridge.
//! Message bodies remain in this entity/process; only existing MailService
//! calls retrieve them. Remote HTML is never executed in the native reader.
use crate::design::ui_scaled_px as px;
use futures::channel::oneshot;
use gpui_kit::{
    assets::IconName,
    component::{
        Disableable, Icon, Sizable,
        button::*,
        input::{Input, InputState},
    },
    prelude::*,
    *,
};
use monitter_lib::experimental_desktop::DesktopService;
use serde_json::{Value, json};
use std::time::Duration;
#[path = "mail_html.rs"]
mod mail_html;

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
fn nonempty_or(value: String, fallback: &str) -> String {
    if value.trim().is_empty() {
        fallback.to_owned()
    } else {
        value
    }
}
fn mail_text_size(value: f32, settings: &Value) -> gpui_kit::Pixels {
    let ratio = settings["interfaceFontSize"]
        .as_f64()
        .unwrap_or(14.0)
        .clamp(8.0, 32.0) as f32
        / 14.0;
    super::design::ui_scaled_px(value * ratio)
}
fn html_completion_is_current(
    current_generation: u64,
    generation: u64,
    current_request: u64,
    request: u64,
    active: bool,
    pending: bool,
) -> bool {
    current_generation == generation && current_request == request && active && pending
}
fn signal_label(value: &str) -> String {
    value.replace('_', " ")
}
fn card_signals(card: &Value, palette: super::design::Palette) -> AnyElement {
    let importance = nonempty_or(signal_label(&text(card, "importance")), "Not reported");
    let score = card["importanceScore"]
        .as_i64()
        .map(|score| format!("{score}/100"))
        .unwrap_or_else(|| "Not reported".into());
    let confidence = card["confidence"]
        .as_i64()
        .map(|confidence| format!("{confidence}% confidence"))
        .unwrap_or_else(|| "Confidence not reported".into());
    let signals = [
        format!("{importance} · {score}"),
        nonempty_or(signal_label(&text(card, "intent")), "Intent not reported"),
        format!(
            "reply {}",
            nonempty_or(signal_label(&text(card, "replyRequired")), "Not reported")
        ),
        format!(
            "owner {}",
            nonempty_or(signal_label(&text(card, "suggestedOwner")), "Not reported")
        ),
        nonempty_or(
            signal_label(&text(card, "suggestedAction")),
            "Action not reported",
        ),
        confidence,
    ];
    div()
        .flex()
        .flex_wrap()
        .gap(px(5.))
        .children(signals.into_iter().map(|signal| {
            div()
                .px(px(5.))
                .py(px(2.))
                .rounded(px(4.))
                .border_1()
                .border_color(rgb(palette.line))
                .bg(crate::design::surface_color(palette.card))
                .text_size(super::design::ui_scaled_px(9.))
                .text_color(rgb(palette.muted))
                .child(signal)
        }))
        .into_any_element()
}
fn assigned(account: &Value, agent: &str) -> bool {
    list(account, "agentIds")
        .iter()
        .any(|id| id.as_str() == Some(agent))
}
fn qualifies(batch: &Value, item: &Value) -> bool {
    if text(&batch["classifier"], "mode") != "jev"
        || item
            .get("state")
            .and_then(Value::as_str)
            .unwrap_or("active")
            != "active"
        || text(item, "suggestedOwner") != "me"
        || !["reply", "review", "schedule", "delegate", "track"]
            .contains(&text(item, "suggestedAction").as_str())
        || ["newsletter", "transactional", "personal"].contains(&text(item, "intent").as_str())
    {
        return false;
    }
    let fit = &item["inboxFit"];
    let metric = |key: &str| {
        fit[key]
            .as_f64()
            .filter(|value| value.is_finite() && (0.0..=100.0).contains(value))
    };
    matches!((metric("workRelated"), metric("requiresMyAction"), metric("excludedBulkOrAutomated")),
        (Some(work), Some(action), Some(bulk)) if work >= 70.0 && action >= 70.0 && bulk <= 30.0)
}
fn actionable_cards(batch: &Value) -> Vec<Value> {
    let mut cards: Vec<_> = list(batch, "items")
        .into_iter()
        .filter(|item| qualifies(batch, item))
        .collect();
    cards.sort_by(|a, b| {
        b["importanceScore"]
            .as_f64()
            .unwrap_or_default()
            .total_cmp(&a["importanceScore"].as_f64().unwrap_or_default())
            .then_with(|| b["receivedAt"].as_i64().cmp(&a["receivedAt"].as_i64()))
    });
    cards
}
fn batch_checked_at(batch: &Value) -> i64 {
    batch["updatedAt"]
        .as_i64()
        .filter(|updated| *updated != 0)
        .or_else(|| batch["createdAt"].as_i64())
        .unwrap_or_default()
}
fn classifier_label(mode: &str) -> String {
    match mode {
        "jev" => "Jev".into(),
        "pending" => "Jev pending".into(),
        "not_needed" => "Checked".into(),
        "" => "Status not reported".into(),
        _ => format!("{mode} classifier"),
    }
}
fn classifier_message(mode: &str, fallback_reason: &str) -> Option<String> {
    match mode {
        "pending" => Some(
            "Waiting for Jev classification. Messages are hidden until classification is complete."
                .into(),
        ),
        "jev" => None,
        "not_needed" => Some(
            "Jev did not classify this batch, so messages are hidden from the actionable inbox."
                .into(),
        ),
        "" => Some(
            "Classification status is not reported; messages are hidden from the actionable inbox."
                .into(),
        ),
        _ => Some(format!(
            "Jev classification is unavailable ({mode}); messages are hidden from the actionable inbox.{}",
            if fallback_reason.is_empty() {
                String::new()
            } else {
                format!(" {fallback_reason}")
            }
        )),
    }
}
fn eligible(agent: &Value, snapshot: &Value) -> bool {
    if agent["internal"].as_bool().unwrap_or(false)
        || !["codex", "claude", "acp"].contains(&text(agent, "provider").as_str())
        || !list(snapshot, "hosts")
            .iter()
            .any(|host| text(host, "id") == text(agent, "hostId") && text(host, "kind") == "local")
    {
        return false;
    }
    let command = text(&agent["acp"], "command");
    let executable = command.rsplit(['/', '\\']).next().unwrap_or_default();
    !matches!(executable, "monitter-agy-acp" | "agy")
        && !list(&agent["acp"], "args").iter().any(|arg| {
            arg.as_str()
                .is_some_and(|arg| arg.ends_with("agy-acp-bridge.mjs"))
        })
}

enum ResultKind {
    Config,
    Refresh,
    Detail(u64),
    Connect,
    Status,
    Cancel,
    Summary(u64),
}

pub struct MailPane {
    service: DesktopService,
    snapshot: Value,
    task_id: Option<String>,
    config: Option<Value>,
    client_id: Entity<InputState>,
    client_secret: Entity<InputState>,
    detail: Option<Value>,
    html_reader: Option<Entity<mail_html::MailHtmlReader>>,
    html_pending: bool,
    html_request: u64,
    auto_open_generation: Option<u64>,
    plain_override: bool,
    selected: Option<String>,
    summary: Option<String>,
    generation: u64,
    attempt: Option<Value>,
    settings: bool,
    active: bool,
    busy: bool,
    error: Option<String>,
    notice: Option<String>,
    _poll: Task<()>,
    _release: Subscription,
    _quit: Subscription,
}
impl MailPane {
    pub fn new(service: DesktopService, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let client_id = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("New Google OAuth client ID (blank keeps current)")
        });
        let client_secret = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Optional client secret (blank keeps current)")
                .masked(true)
        });
        let poll = cx.spawn(async move |pane, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(900))
                    .await;
                if pane
                    .update(cx, |pane, cx| {
                        if !pane.busy {
                            if let Some(attempt) = pane.attempt.clone() {
                                let expiry = attempt["expiresAt"].as_i64().unwrap_or_default();
                                let now = std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .unwrap_or_default()
                                    .as_millis() as i64;
                                if now >= expiry {
                                    pane.notice = Some(
                                        "Google sign-in timed out. Start again when ready.".into(),
                                    );
                                    pane.cancel_connect(cx);
                                } else {
                                    pane.invoke(
                                        "mail_connect_status",
                                        json!({"attemptId":text(&attempt,"attemptId")}),
                                        ResultKind::Status,
                                        cx,
                                    );
                                }
                            }
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        let release = cx.on_release(|pane, cx| pane.cancel_on_release(cx));
        let quit = cx.on_app_quit(|pane, cx| {
            pane.close_html(cx);
            let service = pane.service.clone();
            let attempt = pane.attempt.take();
            let worker = cx.background_executor().spawn(async move {
                if let Some(attempt) = attempt {
                    let _ = service.invoke(
                        "mail_cancel_connect",
                        json!({"attemptId":text(&attempt,"attemptId")}),
                    );
                }
            });
            async move {
                worker.await;
            }
        });
        let mut pane = Self {
            service,
            snapshot: Value::Null,
            task_id: None,
            config: None,
            client_id,
            client_secret,
            detail: None,
            html_reader: None,
            html_pending: false,
            html_request: 0,
            auto_open_generation: None,
            plain_override: false,
            selected: None,
            summary: None,
            generation: 0,
            attempt: None,
            settings: false,
            active: true,
            busy: false,
            error: None,
            notice: None,
            _poll: poll,
            _release: release,
            _quit: quit,
        };
        pane.invoke("mail_get_config", json!({}), ResultKind::Config, cx);
        pane
    }

    /// Host calls this with the real backend snapshot and selected chat.
    pub fn set_context(
        &mut self,
        snapshot: &Value,
        task_id: Option<String>,
        cx: &mut Context<Self>,
    ) {
        if self.task_id != task_id {
            self.generation += 1;
            self.selected = None;
            self.detail = None;
            self.auto_open_generation = None;
            self.close_html(cx);
            self.summary = None;
        }
        self.snapshot = snapshot.clone();
        self.task_id = task_id;
        if self
            .selected
            .as_ref()
            .is_some_and(|id| !self.cards().iter().any(|card| text(card, "id") == *id))
        {
            self.generation += 1;
            self.selected = None;
            self.detail = None;
            self.auto_open_generation = None;
            self.close_html(cx);
            self.summary = None;
        }
        cx.notify();
    }

    /// Host calls before leaving the Mail route. Clears private text/detail
    /// state and cancels a pending sign-in without changing account grants.
    pub fn deactivate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.generation += 1;
        self.selected = None;
        self.detail = None;
        self.auto_open_generation = None;
        self.plain_override = false;
        self.close_html(cx);
        self.summary = None;
        self.client_secret
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.active = false;
        self.cancel_on_release(cx);
        cx.notify();
    }

    fn close_html(&mut self, cx: &mut App) {
        self.html_request = self.html_request.wrapping_add(1);
        self.html_pending = false;
        if let Some(reader) = self.html_reader.take() {
            reader.update(cx, |reader, cx| reader.close(cx));
        }
    }
    fn open_html(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.active || self.html_pending {
            return;
        }
        let Some(detail) = self.detail.clone() else {
            return;
        };
        self.plain_override = false;
        self.close_html(cx);
        self.html_pending = true;
        self.error = None;
        let generation = self.generation;
        let request = self.html_request;
        let dark = super::design::palette(&self.snapshot["settings"], window.appearance()).dark;
        let task = cx.background_executor().spawn(async move {
            mail_html::document(
                detail["bodyHtml"].as_str(),
                &text(&detail, "bodyText"),
                dark,
            )
        });
        cx.spawn_in(window, async move |pane, cx| {
            let result = task.await;
            let _ = pane.update_in(cx, |pane, window, cx| {
                if !html_completion_is_current(
                    pane.generation,
                    generation,
                    pane.html_request,
                    request,
                    pane.active,
                    pane.html_pending,
                ) {
                    return;
                }
                pane.html_pending = false;
                match result {
                    Ok(document) => match mail_html::MailHtmlReader::open(document, window, cx) {
                        Ok(reader) => pane.html_reader = Some(reader),
                        Err(error) => pane.error = Some(error),
                    },
                    Err(error) => pane.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn agent_id(&self) -> String {
        list(&self.snapshot, "tasks")
            .iter()
            .find(|task| Some(text(task, "id")) == self.task_id)
            .map(|task| text(task, "agentId"))
            .unwrap_or_default()
    }
    fn accounts(&self) -> Vec<Value> {
        let agent = self.agent_id();
        self.config
            .as_ref()
            .map(|config| {
                list(config, "accounts")
                    .into_iter()
                    .filter(|account| {
                        text(account, "provider") == "gmail" && assigned(account, &agent)
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
    fn batches(&self) -> Vec<Value> {
        let accounts = self.accounts();
        let mut batches: Vec<_> = list(&self.snapshot, "mailBatches")
            .into_iter()
            .filter(|batch| {
                Some(text(batch, "taskId")) == self.task_id
                    && accounts
                        .iter()
                        .any(|account| text(account, "id") == text(batch, "nativeAccountId"))
            })
            .collect();
        batches.sort_by(|a, b| batch_checked_at(b).cmp(&batch_checked_at(a)));
        batches
    }
    fn cards(&self) -> Vec<Value> {
        self.batches()
            .into_iter()
            .flat_map(|batch| actionable_cards(&batch))
            .collect()
    }

    fn invoke(
        &mut self,
        command: &'static str,
        args: Value,
        kind: ResultKind,
        cx: &mut Context<Self>,
    ) {
        let service = self.service.clone();
        self.run(move || service.invoke(command, args), kind, cx);
    }
    fn run(
        &mut self,
        work: impl FnOnce() -> Result<Value, String> + Send + 'static,
        kind: ResultKind,
        cx: &mut Context<Self>,
    ) {
        if self.busy {
            return;
        }
        let (sender, receiver) = oneshot::channel();
        let worker = std::thread::Builder::new()
            .name("monitter-mail-rpc".into())
            .stack_size(8 * 1024 * 1024)
            .spawn(move || {
                let _ = sender.send(work());
            });
        if let Err(error) = worker {
            self.error = Some(format!("Could not start mail worker: {error}"));
            cx.notify();
            return;
        }
        self.busy = true;
        self.error = None;
        cx.spawn(async move |pane,cx| {
            let result = receiver.await.unwrap_or_else(|_| Err("Mail worker ended without a result".into()));
            let _ = pane.update(cx, |pane,cx| {
                pane.busy = false;
                match result {
                    Err(error) => {
                        if !matches!(kind, ResultKind::Detail(g)|ResultKind::Summary(g) if g != pane.generation) { pane.error = Some(error); }
                    }
                    Ok(value) => match kind {
                        ResultKind::Config => {pane.config=Some(value);pane.notice=None;}
                        ResultKind::Refresh => {
                            if let Some(snapshot)=value.get("snapshot").filter(|v|!v.is_null()) {pane.snapshot=snapshot.clone();}
                            pane.notice=Some("Gmail inbox refreshed.".into());
                        }
                        ResultKind::Detail(g) if g==pane.generation => {pane.detail=Some(value);pane.auto_open_generation=Some(g);}
                        ResultKind::Summary(g) if g==pane.generation => {pane.summary=Some(text(&value,"summary"));}
                        ResultKind::Connect => {pane.attempt=Some(value);if pane.active {pane.notice=Some("Complete Google sign-in in your system browser.".into());} else {pane.cancel_connect(cx);}}
                        ResultKind::Status => match text(&value,"status").as_str() {
                            "connected" => {pane.attempt=None;pane.invoke("mail_get_config",json!({}),ResultKind::Config,cx);}
                            "error" => {pane.attempt=None;pane.error=Some(text(&value,"error"));}
                            "cancelled" => {pane.attempt=None;}
                            _ => {}
                        },
                        ResultKind::Cancel => {pane.attempt=None;}
                        _ => {}
                    }
                }
                if pane.selected.as_ref().is_some_and(|id| !pane.cards().iter().any(|card| text(card, "id") == *id)) {
                    pane.generation += 1;
                    pane.selected = None;
                    pane.detail = None;
                    pane.auto_open_generation = None;
                        pane.close_html(cx);
                    pane.summary = None;
                }
                cx.notify();
            });
        }).detach();
        cx.notify();
    }
    fn refresh(&mut self, cx: &mut Context<Self>) {
        let Some(task) = self.task_id.clone() else {
            return;
        };
        let accounts = self.accounts();
        if accounts.is_empty() {
            return;
        }
        let service = self.service.clone();
        self.run(
            move || {
                for account in accounts {
                    service.invoke(
                        "refresh_mail_inbox",
                        json!({"taskId":task,"accountId":text(&account,"id"),"query":"in:inbox"}),
                    )?;
                }
                service.invoke("get_ui_snapshot", json!({"revision":null}))
            },
            ResultKind::Refresh,
            cx,
        );
    }
    fn open_detail(&mut self, id: String, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let Some(task) = self.task_id.clone() else {
            return;
        };
        self.generation += 1;
        self.selected = Some(id.clone());
        self.detail = None;
        self.auto_open_generation = None;
        self.plain_override = false;
        self.close_html(cx);
        self.summary = None;
        let service = self.service.clone();
        self.run(
            move || {
                let args = json!({"taskId":task,"mailId":id});
                let ready = service.invoke("request_mail_detail", args.clone())?;
                if text(&ready, "status") != "ready" {
                    return Err("Native Gmail retrieval did not complete. Try again.".into());
                }
                let detail = service.invoke("get_mail_detail", args)?;
                if detail.is_null() {
                    return Err("Gmail returned no message content. Try again.".into());
                }
                Ok(detail)
            },
            ResultKind::Detail(self.generation),
            cx,
        );
    }
    fn save_client(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(config) = self.config.clone() else {
            return;
        };
        let entered = self.client_id.read(cx).value().to_string();
        let secret = self.client_secret.read(cx).value().to_string();
        self.client_secret
            .update(cx, |input, cx| input.set_value("", window, cx));
        let id = if entered.trim().is_empty() {
            text(&config, "clientId")
        } else {
            entered.trim().to_owned()
        };
        self.invoke("mail_configure_gmail",json!({"revision":text(&config,"revision"),"clientId":id,"clientSecret":if secret.is_empty(){None}else{Some(secret)}}),ResultKind::Config,cx);
    }
    fn cancel_connect(&mut self, cx: &mut Context<Self>) {
        if let Some(attempt) = self.attempt.clone() {
            self.invoke(
                "mail_cancel_connect",
                json!({"attemptId":text(&attempt,"attemptId")}),
                ResultKind::Cancel,
                cx,
            );
        }
    }
    fn cancel_on_release(&mut self, cx: &mut App) {
        self.detail = None;
        self.auto_open_generation = None;
        self.plain_override = false;
        self.close_html(cx);
        if let Some(attempt) = self.attempt.take() {
            let service = self.service.clone();
            cx.background_executor()
                .spawn(async move {
                    let _ = service.invoke(
                        "mail_cancel_connect",
                        json!({"attemptId":text(&attempt,"attemptId")}),
                    );
                })
                .detach();
        }
    }
    fn render_settings(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let mut panel=div().flex().flex_col().gap_3()
            .child("Private Gmail account settings")
            .child("Account credentials are stored in this experimental desktop's isolated Keychain vault.")
            .child(Button::new("mail-config-reload").label("Reload account settings").disabled(self.busy)
                .on_click(cx.listener(|pane,_,_,cx|pane.invoke("mail_get_config",json!({}),ResultKind::Config,cx))));
        if let Some(config) = self.config.clone() {
            panel = panel
                .child(format!(
                    "Current Google client ID: {}",
                    text(&config, "clientId")
                ))
                .child(format!(
                    "Client secret configured: {}",
                    config["clientSecretConfigured"].as_bool().unwrap_or(false)
                ))
                .child(Input::new(&self.client_id))
                .child(Input::new(&self.client_secret))
                .child(
                    Button::new("mail-client-save")
                        .label("Save Google app settings")
                        .disabled(self.busy)
                        .on_click(cx.listener(|pane, _, window, cx| pane.save_client(window, cx))),
                )
                .child(
                    Button::new("mail-connect")
                        .label("Connect Gmail in system browser")
                        .disabled(self.busy || self.attempt.is_some())
                        .on_click(cx.listener(|pane, _, _, cx| {
                            pane.invoke("mail_start_connect", json!({}), ResultKind::Connect, cx)
                        })),
                );
            if self.attempt.is_some() {
                panel = panel.child(
                    Button::new("mail-connect-cancel")
                        .label("Cancel Google sign-in")
                        .disabled(self.busy)
                        .on_click(cx.listener(|pane, _, _, cx| pane.cancel_connect(cx))),
                );
            }
            let agents: Vec<_> = list(&self.snapshot, "agents")
                .into_iter()
                .filter(|agent| eligible(agent, &self.snapshot))
                .collect();
            for account in list(&config, "accounts") {
                let account_id = text(&account, "id");
                let disconnect_id = account_id.clone();
                let revision = text(&config, "revision");
                let mut row = div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .p_3()
                    .rounded_md()
                    .bg(crate::design::surface_color(0x192333))
                    .child(format!(
                        "{} · {}",
                        text(&account, "email"),
                        text(&account, "label")
                    ))
                    .child(
                        Button::new(SharedString::from(format!("mail-disconnect-{account_id}")))
                            .label("Disconnect account")
                            .disabled(self.busy)
                            .on_click(cx.listener(move |pane, _, _, cx| {
                                pane.invoke(
                                    "mail_disconnect",
                                    json!({"revision":revision,"accountId":disconnect_id}),
                                    ResultKind::Config,
                                    cx,
                                )
                            })),
                    );
                for agent in &agents {
                    let agent_id = text(agent, "id");
                    let is_assigned = assigned(&account, &agent_id);
                    let mut ids = list(&account, "agentIds");
                    if is_assigned {
                        ids.retain(|id| id.as_str() != Some(agent_id.as_str()));
                    } else {
                        ids.push(Value::String(agent_id.clone()));
                    }
                    let account_id = account_id.clone();
                    let revision = text(&config, "revision");
                    row = row.child(
                        Button::new(SharedString::from(format!(
                            "mail-grant-{account_id}-{agent_id}"
                        )))
                        .label(format!(
                            "{} {}",
                            if is_assigned { "Assigned:" } else { "Assign:" },
                            text(agent, "name")
                        ))
                        .disabled(self.busy)
                        .on_click(cx.listener(move |pane, _, _, cx| {
                            pane.invoke(
                                "mail_set_account_agents",
                                json!({"revision":revision,"accountId":account_id,"agentIds":ids}),
                                ResultKind::Config,
                                cx,
                            )
                        })),
                    );
                }
                panel = panel.child(row);
            }
            panel=panel.child("Grant changes use the current configuration revision. Reload settings after a conflict. Removing access takes effect immediately; a newly assigned agent may need a new session.");
            panel = panel.child("Agent MCP mail access stays read-only. The native inbox can mark messages read after you open them or when Jev confidently identifies low-value bulk mail. Reauthorize existing accounts here to grant that access. Fetched messages become part of the selected agent's context.");
        } else {
            panel =
                panel.child("Mail settings have not loaded. Retry with Reload account settings.");
        }
        panel.into_any_element()
    }
    fn render_inbox(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        if self.active
            && !self.plain_override
            && self.detail.is_some()
            && self.auto_open_generation == Some(self.generation)
        {
            self.auto_open_generation = None;
            let owner = cx.weak_entity();
            let generation = self.generation;
            window.on_next_frame(move |window, cx| {
                let _ = owner.update(cx, |pane, cx| {
                    if pane.active
                        && pane.generation == generation
                        && pane.detail.is_some()
                        && !pane.plain_override
                    {
                        pane.open_html(window, cx);
                    }
                });
            });
        }
        let accounts = self.accounts();
        let batches = self.batches();
        let palette = super::design::palette(&self.snapshot["settings"], window.appearance());
        let mut header = div()
            .flex()
            .items_center()
            .flex_wrap()
            .gap(px(9.))
            .p(px(10.))
            .px(px(12.))
            .border_1()
            .border_color(rgb(palette.line))
            .rounded(px(9.))
            .bg(crate::design::surface_color(palette.card))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(7.))
                    .mr_auto()
                    .text_size(mail_text_size(12., &self.snapshot["settings"]))
                    .text_color(rgb(palette.ink))
                    .child(Icon::new(IconName::Mail).with_size(px(16.)))
                    .child(div().font_weight(FontWeight::SEMIBOLD).child("Gmail inbox")),
            );
        if !accounts.is_empty() {
            header = header.child(
                Button::new("mail-refresh")
                    .compact()
                    .outline()
                    .min_h(px(31.))
                    .px(px(10.))
                    .text_size(mail_text_size(11., &self.snapshot["settings"]))
                    .font_family(super::design::FONT_MONO)
                    .icon(Icon::new(IconName::RefreshCw).with_size(px(14.)))
                    .accessibility_label(if self.busy {
                        "Refreshing Gmail"
                    } else {
                        "Refresh Gmail"
                    })
                    .child(
                        div()
                            .text_size(mail_text_size(11., &self.snapshot["settings"]))
                            .child(if self.busy {
                                "Refreshing…"
                            } else {
                                "Refresh"
                            }),
                    )
                    .disabled(self.busy || self.task_id.is_none())
                    .on_click(cx.listener(|pane, _, _, cx| pane.refresh(cx))),
            );
        }
        let mut panel = div()
            .flex()
            .flex_col()
            .gap_3()
            .when(!self.settings && self.selected.is_some(), |panel| {
                panel.flex_1().min_h(px(0.))
            })
            .child(header);
        if self.task_id.is_none() {
            return panel.into_any_element();
        }
        if accounts.is_empty() {
            return panel
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(5.))
                        .p(px(18.))
                        .border_1()
                        .border_dashed()
                        .border_color(rgb(palette.line))
                        .rounded(px(9.))
                        .bg(crate::design::surface_color(palette.card))
                        .text_size(mail_text_size(12., &self.snapshot["settings"]))
                        .text_color(rgb(palette.muted))
                        .child(
                            div()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(palette.ink))
                                .child("No Gmail account is assigned to this agent."),
                        )
                        .child("Connect a Gmail account in Settings and assign it to this agent to use its native inbox."),
                )
                .into_any_element();
        }
        for account in accounts {
            let email = nonempty_or(text(&account, "email"), "Email address not reported");
            panel = panel.child(
                div()
                    .text_size(mail_text_size(11., &self.snapshot["settings"]))
                    .text_color(rgb(0x778391))
                    .child(format!("Connected account: {email}")),
            );
        }
        if self.selected.is_some() {
            let selected_id = self.selected.clone().unwrap_or_default();
            let selected_batch = batches
                .iter()
                .find(|batch| {
                    actionable_cards(batch)
                        .iter()
                        .any(|card| text(card, "id") == selected_id)
                })
                .cloned();
            let selected_card = selected_batch.as_ref().and_then(|batch| {
                actionable_cards(batch)
                    .into_iter()
                    .find(|card| text(card, "id") == selected_id)
            });
            panel = panel.child(
                Button::new("mail-detail-close")
                    .label("Back to inbox")
                    .on_click(cx.listener(|pane, _, _, cx| {
                        pane.generation += 1;
                        pane.selected = None;
                        pane.detail = None;
                        pane.close_html(cx);
                        pane.summary = None;
                        cx.notify();
                    })),
            );
            if let Some(detail) = self.detail.clone() {
                let subject = text(&detail, "subject");
                let received_at = detail["receivedAt"]
                    .as_i64()
                    .map(crate::timestamps::datetime_label)
                    .unwrap_or_else(|| "Not reported".into());
                let recipients = |key: &str| {
                    list(&detail, key)
                        .iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(", ")
                };
                let mut title_row = div().flex().items_center().gap_2().child(
                    div()
                        .min_w_0()
                        .flex_1()
                        .truncate()
                        .text_size(super::design::ui_scaled_px(14.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(nonempty_or(subject, "(no subject)")),
                );
                if selected_card
                    .as_ref()
                    .is_some_and(|card| card["isNew"].as_bool().unwrap_or(false))
                {
                    title_row = title_row.child(
                        div()
                            .flex_shrink_0()
                            .text_size(super::design::ui_scaled_px(9.))
                            .text_color(rgb(0x7fb2ff))
                            .child("NEW"),
                    );
                }
                title_row = title_row.child(
                    div()
                        .ml_auto()
                        .flex_shrink_0()
                        .text_size(super::design::ui_scaled_px(10.))
                        .text_color(rgb(palette.muted))
                        .child(received_at.clone()),
                );
                let body = if let Some(reader) = &self.html_reader {
                    div()
                        .id("native-mail-rich-body")
                        .w_full()
                        .flex_1()
                        .min_h(px(160.))
                        .overflow_hidden()
                        .child(reader.clone())
                } else {
                    div()
                        .id("native-mail-plain-body")
                        .w_full()
                        .flex_1()
                        .min_h(px(160.))
                        .overflow_y_scroll()
                        .p_3()
                        .bg(crate::design::surface_color(palette.card))
                        .text_color(rgb(palette.ink))
                        .child(text(&detail, "bodyText"))
                };
                let mut message_header = div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .p_3()
                    .bg(crate::design::surface_color(palette.card))
                    .child(title_row)
                    .child(
                        div()
                            .text_size(super::design::ui_scaled_px(11.))
                            .child(format!(
                                "From: {}",
                                nonempty_or(text(&detail, "from"), "Not reported")
                            )),
                    )
                    .when_some(selected_batch.as_ref(), |header, batch| {
                        header.child(div().text_size(super::design::ui_scaled_px(11.)).child(
                            format!(
                                "Account: {}",
                                nonempty_or(
                                    text(batch, "accountLabel"),
                                    "Account label unavailable"
                                )
                            ),
                        ))
                    })
                    .child(
                        div()
                            .text_size(super::design::ui_scaled_px(11.))
                            .child(format!(
                                "To: {}",
                                nonempty_or(recipients("to"), "Not supplied")
                            )),
                    )
                    .child(
                        div()
                            .text_size(super::design::ui_scaled_px(11.))
                            .child(format!("Received: {received_at}")),
                    );
                if !recipients("cc").is_empty() {
                    message_header = message_header.child(
                        div()
                            .text_size(super::design::ui_scaled_px(11.))
                            .child(format!("Cc: {}", recipients("cc"))),
                    );
                }
                if let Some(card) = selected_card.as_ref() {
                    message_header = message_header.child(card_signals(card, palette));
                }
                panel = panel
                    .child(message_header)
                    .child(div().flex().gap_2()
                        .child(Button::new("mail-rich-reader").label(if self.html_pending {"Preparing safe reader…"}else{"Open rich reader"}).disabled(self.html_pending).on_click(cx.listener(|pane,_,window,cx|pane.open_html(window,cx))))
                        .child(Button::new("mail-plain-reader").label("Plain text").on_click(cx.listener(|pane,_,_,cx|{pane.plain_override=true;pane.auto_open_generation=None;pane.close_html(cx);cx.notify();}))))
                    .child(body)
                    .child("Rich reader blocks scripts, navigation, popups and remote images. Inline raster images only; attachments remain pending.")
                    .child(
                        Button::new("mail-summary")
                            .label("Summarize thread with Jev")
                            .disabled(self.busy)
                            .on_click(cx.listener(|pane, _, _, cx| {
                                if let (Some(task), Some(id)) =
                                    (pane.task_id.clone(), pane.selected.clone())
                                {
                                    pane.invoke(
                                        "summarize_mail_thread",
                                        json!({"taskId":task,"mailId":id}),
                                        ResultKind::Summary(pane.generation),
                                        cx,
                                    );
                                }
                            })),
                    );
                if let Some(summary) = &self.summary {
                    panel = panel.child(summary.clone());
                }
            } else {
                panel = panel.child(if self.busy {
                    "Loading Gmail message…"
                } else {
                    "Message content is unavailable. Return to the inbox and retry."
                });
            }
        } else {
            if batches.is_empty() {
                panel = panel.child(div().text_size(super::design::ui_scaled_px(12.)).child(
                    "No inbox messages loaded. Refresh to read your connected Gmail inbox.",
                ));
            }
            for batch in batches {
                let cards = actionable_cards(&batch);
                let new_count = cards
                    .iter()
                    .filter(|card| card["isNew"].as_bool().unwrap_or(false))
                    .count();
                let mode = text(&batch["classifier"], "mode");
                let mode_label = classifier_label(&mode);
                let fallback_reason = text(&batch["classifier"], "fallbackReason");
                let checked = crate::timestamps::datetime_label(batch_checked_at(&batch));
                let account_label =
                    nonempty_or(text(&batch, "accountLabel"), "Account label unavailable");
                let mut batch_panel = div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .rounded_md()
                    .border_1()
                    .border_color(rgb(palette.line))
                    .bg(crate::design::surface_color(palette.card))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_2()
                            .p_3()
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_1()
                                    .child(
                                        div()
                                            .text_size(super::design::ui_scaled_px(13.))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child("Actionable inbox"),
                                    )
                                    .child(
                                        div()
                                            .text_size(super::design::ui_scaled_px(10.))
                                            .text_color(rgb(palette.muted))
                                            .child(format!("{account_label} · checked {checked}")),
                                    ),
                            )
                            .child(
                                div()
                                    .text_size(super::design::ui_scaled_px(10.))
                                    .child(mode_label),
                            ),
                    )
                    .child(
                        div()
                            .px_3()
                            .text_size(super::design::ui_scaled_px(11.))
                            .child(format!("{} actionable · {new_count} new", cards.len())),
                    );
                if let Some(message) = classifier_message(&mode, &fallback_reason) {
                    batch_panel = batch_panel.child(
                        div()
                            .px_3()
                            .text_size(super::design::ui_scaled_px(11.))
                            .text_color(rgb(palette.muted))
                            .child(message),
                    );
                } else if cards.is_empty() {
                    batch_panel = batch_panel.child(
                        div()
                            .px_3()
                            .py_2()
                            .text_size(super::design::ui_scaled_px(11.))
                            .text_color(rgb(palette.muted))
                            .child("No messages meet the actionable inbox criteria. Only work related messages requiring your action are shown."),
                    );
                }
                for card in cards {
                    let id = text(&card, "id");
                    let subject = nonempty_or(text(&card, "subject"), "(no subject)");
                    let sender = nonempty_or(text(&card, "from"), "Sender not reported");
                    let accessible_label = format!("Open {subject} from {sender}");
                    let received = card["receivedAt"]
                        .as_i64()
                        .map(crate::timestamps::datetime_label)
                        .unwrap_or_else(|| "Not reported".into());
                    let is_new = card["isNew"].as_bool().unwrap_or(false);
                    batch_panel = batch_panel.child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .p_3()
                            .border_t_1()
                            .border_color(rgb(palette.line))
                            .child(
                                Button::new(SharedString::from(format!("mail-message-{id}")))
                                    .text()
                                    .compact()
                                    .w_full()
                                    .accessibility_label(accessible_label)
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap_2()
                                            .child(
                                                div()
                                                    .min_w_0()
                                                    .flex_1()
                                                    .truncate()
                                                    .text_size(super::design::ui_scaled_px(12.5))
                                                    .font_weight(FontWeight::SEMIBOLD)
                                                    .child(subject),
                                            )
                                            .when(is_new, |row| {
                                                row.child(
                                                    div()
                                                        .flex_shrink_0()
                                                        .text_size(super::design::ui_scaled_px(9.))
                                                        .text_color(rgb(0x7fb2ff))
                                                        .child("NEW"),
                                                )
                                            })
                                            .child(
                                                div()
                                                    .ml_auto()
                                                    .flex_shrink_0()
                                                    .text_size(super::design::ui_scaled_px(9.))
                                                    .text_color(rgb(palette.muted))
                                                    .child(received),
                                            ),
                                    )
                                    .disabled(self.busy)
                                    .on_click(cx.listener(move |pane, _, _, cx| {
                                        pane.open_detail(id.clone(), cx)
                                    })),
                            )
                            .child(
                                div()
                                    .text_size(super::design::ui_scaled_px(11.))
                                    .text_color(rgb(palette.muted))
                                    .child(sender),
                            )
                            .child(
                                div()
                                    .text_size(super::design::ui_scaled_px(11.5))
                                    .line_height(relative(1.4))
                                    .child(nonempty_or(
                                        text(&card, "snippet"),
                                        "No snippet available.",
                                    )),
                            )
                            .child(card_signals(&card, palette))
                            .child(
                                div()
                                    .text_size(super::design::ui_scaled_px(10.))
                                    .text_color(rgb(palette.muted))
                                    .child(nonempty_or(
                                        text(&card, "rationale"),
                                        "Rationale not reported.",
                                    )),
                            ),
                    );
                }
                let classifier = &batch["classifier"];
                let provider = nonempty_or(text(classifier, "provider"), "TypeSafe");
                let model = nonempty_or(text(classifier, "model"), "model not reported");
                let latency = classifier["latencyMs"]
                    .as_i64()
                    .map(|value| format!("{value}ms"))
                    .unwrap_or_else(|| "latency not reported".into());
                let inputs = classifier["inputTokens"]
                    .as_i64()
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "—".into());
                let outputs = classifier["outputTokens"]
                    .as_i64()
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "—".into());
                let cost = classifier["costMicrousd"]
                    .as_i64()
                    .map(|value| format!("${:.6}", value as f64 / 1_000_000.))
                    .unwrap_or_else(|| "Not reported".into());
                batch_panel = batch_panel.child(
                    div()
                        .px_3()
                        .py_2()
                        .border_t_1()
                        .border_color(rgb(palette.line))
                        .text_size(super::design::ui_scaled_px(9.))
                        .text_color(rgb(palette.muted))
                        .child(format!("{provider} · {model} · {latency} · {inputs} in · {outputs} out · {cost}")),
                );
                let added = batch["lastAdded"]
                    .as_u64()
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "Not reported".into());
                let updated = batch["lastUpdated"]
                    .as_u64()
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "Not reported".into());
                batch_panel = batch_panel.child(
                    div()
                        .px_3()
                        .py_2()
                        .text_size(super::design::ui_scaled_px(9.))
                        .text_color(rgb(palette.muted))
                        .child(format!(
                            "Checked {checked} · {added} added · {updated} refreshed"
                        )),
                );
                panel = panel.child(batch_panel);
            }
        }
        panel.into_any_element()
    }
}
impl Render for MailPane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.active = true;
        let body = if self.settings {
            self.render_settings(cx)
        } else {
            self.render_inbox(window, cx)
        };
        let palette = super::design::palette(&self.snapshot["settings"], window.appearance());
        let inbox_tab = Button::new("mail-inbox-tab")
            .compact()
            .min_h(px(31.))
            .px(px(10.))
            .text_size(mail_text_size(10., &self.snapshot["settings"]))
            .font_family(super::design::FONT_MONO)
            .accessibility_label("Inbox")
            .child(
                div()
                    .text_size(mail_text_size(10., &self.snapshot["settings"]))
                    .child("Inbox"),
            )
            .on_click(cx.listener(|pane, _, _, cx| {
                pane.settings = false;
                cx.notify();
            }));
        let inbox_tab = if self.settings {
            inbox_tab.text()
        } else {
            inbox_tab
                .text()
                .bg(crate::design::surface_color(palette.card))
                .border_1()
                .border_color(rgb(palette.line))
        };
        let settings_tab = Button::new("mail-settings-tab")
            .compact()
            .min_h(px(31.))
            .px(px(10.))
            .text_size(mail_text_size(10., &self.snapshot["settings"]))
            .font_family(super::design::FONT_MONO)
            .accessibility_label("Account settings")
            .child(
                div()
                    .text_size(mail_text_size(10., &self.snapshot["settings"]))
                    .child("Account settings"),
            )
            .on_click(cx.listener(|pane, _, _, cx| {
                pane.close_html(cx);
                pane.auto_open_generation = None;
                pane.settings = true;
                cx.notify();
            }));
        let settings_tab = if self.settings {
            settings_tab
                .text()
                .bg(crate::design::surface_color(palette.card))
                .border_1()
                .border_color(rgb(palette.line))
        } else {
            settings_tab.text()
        };
        let mut panel = div()
            .id("native-mail-pane")
            .text_size(mail_text_size(12., &self.snapshot["settings"]))
            .size_full()
            .relative()
            .when(!self.settings && self.selected.is_some(), |panel| {
                panel.overflow_hidden()
            })
            .when(self.settings || self.selected.is_none(), |panel| {
                panel.overflow_y_scroll()
            })
            .flex()
            .flex_col()
            .gap_3()
            .p_4()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .self_start()
                    .p(px(3.))
                    .rounded(px(8.))
                    .bg(crate::design::surface_color(palette.soft))
                    .child(inbox_tab)
                    .child(settings_tab),
            );
        if let Some(error) = &self.error {
            panel = panel.child(
                div()
                    .text_size(mail_text_size(11., &self.snapshot["settings"]))
                    .text_color(rgb(0xf87171))
                    .child(error.clone()),
            );
        }
        if let Some(notice) = &self.notice {
            panel = panel.child(
                div()
                    .text_size(mail_text_size(10., &self.snapshot["settings"]))
                    .text_color(rgb(palette.muted))
                    .child(notice.clone()),
            );
        }
        panel.child(body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[::core::prelude::v1::test]
    fn rich_reader_completion_requires_current_request_and_active_detail() {
        assert!(html_completion_is_current(4, 4, 9, 9, true, true));
        assert!(!html_completion_is_current(4, 4, 8, 9, true, true));
        assert!(!html_completion_is_current(5, 4, 9, 9, true, true));
        assert!(!html_completion_is_current(4, 4, 9, 9, false, true));
        assert!(!html_completion_is_current(4, 4, 9, 9, true, false));
    }
    #[::core::prelude::v1::test]
    fn actionable_inbox_rejects_fallback_and_bulk() {
        let mut batch = json!({"classifier":{"mode":"jev"}});
        let mut card = json!({"state":"active","suggestedOwner":"me","suggestedAction":"reply","intent":"business","inboxFit":{"workRelated":70,"requiresMyAction":70,"excludedBulkOrAutomated":30}});
        assert!(qualifies(&batch, &card));
        batch["classifier"]["mode"] = json!("fallback");
        assert!(!qualifies(&batch, &card));
        batch["classifier"]["mode"] = json!("jev");
        card["inboxFit"]["excludedBulkOrAutomated"] = json!(31);
        assert!(!qualifies(&batch, &card));
        card["inboxFit"]["excludedBulkOrAutomated"] = json!(0);
        card["intent"] = json!("newsletter");
        assert!(!qualifies(&batch, &card));
        card["intent"] = json!("business");
        card["inboxFit"]["requiresMyAction"] = Value::Null;
        assert!(!qualifies(&batch, &card));
    }

    #[::core::prelude::v1::test]
    fn actionable_batch_projection_keeps_new_state_and_sorts_by_score_then_time() {
        let batch = json!({
            "classifier":{"mode":"jev"},
            "items":[
                {"id":"older","state":"active","suggestedOwner":"me","suggestedAction":"reply","intent":"business","importanceScore":90,"receivedAt":10,"isNew":false,"inboxFit":{"workRelated":80,"requiresMyAction":80,"excludedBulkOrAutomated":0}},
                {"id":"newer","state":"active","suggestedOwner":"me","suggestedAction":"review","intent":"business","importanceScore":90,"receivedAt":20,"isNew":true,"inboxFit":{"workRelated":80,"requiresMyAction":80,"excludedBulkOrAutomated":0}},
                {"id":"lower","state":"active","suggestedOwner":"me","suggestedAction":"track","intent":"business","importanceScore":70,"receivedAt":30,"isNew":false,"inboxFit":{"workRelated":80,"requiresMyAction":80,"excludedBulkOrAutomated":0}},
                {"id":"bulk","state":"active","suggestedOwner":"me","suggestedAction":"reply","intent":"newsletter","importanceScore":100,"receivedAt":40,"isNew":true,"inboxFit":{"workRelated":80,"requiresMyAction":80,"excludedBulkOrAutomated":0}}
            ]
        });
        let cards = actionable_cards(&batch);
        assert_eq!(
            cards
                .iter()
                .map(|card| text(card, "id"))
                .collect::<Vec<_>>(),
            vec!["newer".to_owned(), "older".to_owned(), "lower".to_owned()]
        );
        assert!(cards[0]["isNew"].as_bool().unwrap_or(false));
    }

    #[::core::prelude::v1::test]
    fn batch_time_and_classifier_status_show_missing_data_honestly() {
        assert_eq!(batch_checked_at(&json!({"updatedAt":0,"createdAt":42})), 42);
        assert_eq!(classifier_label(""), "Status not reported");
        assert!(classifier_message("", "").unwrap().contains("not reported"));
        assert!(
            classifier_message("fallback", "Jev unavailable")
                .unwrap()
                .contains("Jev unavailable")
        );
    }
}
