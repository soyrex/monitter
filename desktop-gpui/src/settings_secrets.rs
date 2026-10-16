//! Redacted metadata and explicit writes for the owner-local secret vault.
use crate::design::{Palette, ui_scaled_px as px};
use futures::channel::oneshot;
use gpui_kit::component::{
    Disableable,
    button::*,
    input::{Input, InputState},
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

pub struct SecretsPane {
    service: DesktopService,
    config: Option<Value>,
    name: Entity<InputState>,
    value: Entity<InputState>,
    description: Entity<InputState>,
    busy: bool,
    error: Option<String>,
    notice: Option<String>,
    replacing: Option<String>,
    clear_after_save: bool,
}

impl SecretsPane {
    pub fn new(service: DesktopService, window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            service,
            config: None,
            name: cx.new(|cx| InputState::new(window, cx).placeholder("Environment variable name")),
            value: cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("Secret value")
                    .masked(true)
            }),
            description: cx
                .new(|cx| InputState::new(window, cx).placeholder("Description (optional)")),
            busy: false,
            error: None,
            notice: None,
            replacing: None,
            clear_after_save: false,
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
            .name("gpui-settings-secrets".into())
            .stack_size(STACK)
            .spawn(move || {
                let _ = tx.send(service.invoke(command, args));
            });
        if let Err(error) = spawn {
            self.busy = false;
            self.error = Some(format!("Could not start vault worker: {error}"));
            cx.notify();
            return;
        }
        cx.spawn(async move |pane, cx| {
            let result = rx.await.unwrap_or_else(|_| {
                Err("Environment vault worker stopped before replying.".into())
            });
            let _ = pane.update(cx, |pane, cx| {
                pane.busy = false;
                match result {
                    Ok(config) => {
                        pane.config = Some(config);
                        pane.error = None;
                        if command == "set_environment_secret" {
                            pane.replacing = None;
                            pane.clear_after_save = true;
                            pane.notice = Some(
                                "Secret saved. Its value is not returned by the vault.".into(),
                            );
                        }
                        if command == "delete_environment_secret" {
                            pane.notice = Some("Secret removed.".into());
                        }
                    }
                    Err(error) => pane.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    pub fn load(&mut self, cx: &mut Context<Self>) {
        self.invoke("list_environment_secrets", json!({}), cx);
    }
    fn replace(
        &mut self,
        name: &str,
        description: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.replacing = Some(name.into());
        self.name
            .update(cx, |input, cx| input.set_value(name, window, cx));
        self.description
            .update(cx, |input, cx| input.set_value(description, window, cx));
        self.value
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.notice = Some("Enter a new value. The saved value cannot be read back.".into());
        cx.notify();
    }
    fn save(&mut self, cx: &mut Context<Self>) {
        let Some(config) = self.config.as_ref() else {
            return;
        };
        let name = self.name.read(cx).value().to_string().trim().to_owned();
        let value = self.value.read(cx).value().to_string();
        let description = self
            .description
            .read(cx)
            .value()
            .to_string()
            .trim()
            .to_owned();
        if name.is_empty() {
            self.error = Some("Environment variable name is required.".into());
            cx.notify();
            return;
        }
        if value.is_empty() {
            self.error = Some("Enter a secret value before saving.".into());
            cx.notify();
            return;
        }
        self.invoke("set_environment_secret",json!({"revision":text(config,"revision"),"name":name,"value":value,"description":description}),cx);
    }
    fn delete(&mut self, name: String, cx: &mut Context<Self>) {
        let Some(config) = self.config.as_ref() else {
            return;
        };
        self.invoke(
            "delete_environment_secret",
            json!({"revision":text(config,"revision"),"name":name}),
            cx,
        );
    }

    pub fn render(
        &mut self,
        palette: Palette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if self.clear_after_save {
            self.value
                .update(cx, |input, cx| input.set_value("", window, cx));
            self.clear_after_save = false;
        }
        let mut panel = super::card("Shared agent environment secrets", palette)
            .child(super::note("Values stay in the native Keychain. Only names and descriptions are listed; values are write-only and are injected only into eligible local agents at launch.", palette));
        if let Some(error) = &self.error {
            panel = panel.child(div().text_color(rgb(0xd65b5b)).child(error.clone()));
        }
        if let Some(notice) = &self.notice {
            panel = panel.child(div().text_color(rgb(palette.accent)).child(notice.clone()));
        }
        if self.config.is_none() {
            if self.busy {
                panel = panel.child("Loading secret metadata…");
            } else {
                panel = panel.child(
                    Button::new("settings-secrets-load")
                        .label("Load secret metadata")
                        .on_click(cx.listener(|pane, _, _, cx| pane.load(cx))),
                );
            }
            return panel.into_any_element();
        }
        let config = self.config.clone().unwrap_or(Value::Null);
        panel = panel.child(
            Button::new("settings-secrets-refresh")
                .label("Refresh metadata")
                .disabled(self.busy)
                .on_click(cx.listener(|pane, _, _, cx| pane.load(cx))),
        );
        if let Some(entries) = config.get("entries").and_then(Value::as_array) {
            if entries.is_empty() {
                panel = panel.child("No environment secrets are configured.");
            }
            for entry in entries {
                let name = text(entry, "name");
                let description = text(entry, "description");
                let replace_name = name.clone();
                let replace_description = description.clone();
                let delete_name = name.clone();
                panel = panel.child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
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
                                .child(div().font_weight(FontWeight::SEMIBOLD).child(name.clone()))
                                .child(
                                    div()
                                        .text_size(crate::settings::settings_text_size(0.8))
                                        .text_color(rgb(palette.muted))
                                        .child(description.clone()),
                                ),
                        )
                        .child(
                            Button::new(format!("secret-replace-{name}"))
                                .label("Replace value")
                                .disabled(self.busy)
                                .on_click(cx.listener(move |pane, _, window, cx| {
                                    pane.replace(&replace_name, &replace_description, window, cx)
                                })),
                        )
                        .child(
                            Button::new(format!("secret-delete-{name}"))
                                .label("Delete")
                                .disabled(self.busy)
                                .on_click(cx.listener(move |pane, _, _, cx| {
                                    pane.delete(delete_name.clone(), cx)
                                })),
                        ),
                );
            }
        }
        panel = panel.child(
            div()
                .flex()
                .flex_col()
                .gap(px(crate::design::space::TIGHT))
                .pt(px(crate::design::space::INSET))
                .child(if self.replacing.is_some() {
                    "Replace secret value"
                } else {
                    "Add a secret"
                })
                .child(super::field("Variable name", "Letters, numbers and underscores; reserved runtime variables cannot be replaced.", Input::new(&self.name).text_size(crate::settings::settings_text_size(0.86)).aria_label("Environment variable name").disabled(self.busy || self.replacing.is_some()), palette))
                .child(super::field("Secret value", "Write-only value stored in the native Keychain.", Input::new(&self.value).text_size(crate::settings::settings_text_size(0.86)).aria_label("Secret value").disabled(self.busy), palette))
                .child(super::field("Description", "Optional note explaining what this variable is used for.", Input::new(&self.description).text_size(crate::settings::settings_text_size(0.86)).aria_label("Secret description").disabled(self.busy), palette))
                .child(
                    Button::new("settings-secret-save")
                        .label(if self.busy {
                            "Saving…"
                        } else {
                            "Save secret"
                        })
                        .disabled(self.busy)
                        .on_click(cx.listener(|pane, _, _, cx| pane.save(cx))),
                ),
        );
        panel.into_any_element()
    }
}
