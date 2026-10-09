//! Composer command dispatch through existing backend authorities.
use super::{ChatEvent, ChatPane};
use crate::slash_palette::{SlashPaletteItem, typed_suffix};
use gpui_kit::{AppContext as _, Context, EntityInputHandler, Focusable, Window};
use gpui_kit::component::WindowExt as _;
use serde_json::{Value, json};

/// // escapes only the leading slash token, preserving all other whitespace.
pub(super) fn literal_prompt(input: &str) -> String {
    let leading = input.len() - input.trim_start().len();
    if input[leading..].starts_with("//") {
        format!("{}{}", &input[..leading], &input[leading + 1..])
    } else {
        input.to_owned()
    }
}

pub(super) fn typed_submit_before_menu(input: &str) -> bool {
    let token = input.trim_start().split_whitespace().next().unwrap_or("");
    token.eq_ignore_ascii_case("/terminal")
        || token.eq_ignore_ascii_case("/new")
        || token.eq_ignore_ascii_case("/rename")
}

fn rename_request(task: &str, title: &str) -> (&'static str, Value) {
    if title.is_empty() {
        ("autoname", json!({"target":{"taskId":task}}))
    } else {
        ("rename_task", json!({"id":task,"title":title}))
    }
}

impl ChatPane {
    pub(super) fn composer_keyboard_available(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        !self.approval_popup_open
            && !crate::preferences_modal::workspace_blocked(cx)
            && self.composer.read(cx).focus_handle(cx).is_focused(window)
            && !self.composer.update(cx, |input, cx| {
                EntityInputHandler::marked_text_range(input, window, cx).is_some()
            })
    }
    pub(super) fn handle_slash(
        &mut self,
        item: SlashPaletteItem,
        input: String,
        submitted: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy {
            return;
        }
        let task_id = self.task_id();
        // Events from a previously edited draft must not execute or clear it.
        if self.composer.read(cx).value().as_ref() != input {
            return;
        }
        let suffix = typed_suffix(&input, &item).unwrap_or("").to_owned();
        self.error = None;
        self.slash_notice = None;
        if item.source != "monitter" {
            let command = if suffix.is_empty() {
                item.label()
            } else {
                format!("{} {suffix}", item.label())
            };
            self.slash_rpc(
                vec![(
                    "execute_task_slash_command",
                    json!({"taskId":task_id,"command":command}),
                )],
                input,
                None,
                window,
                cx,
            );
            return;
        }
        if !suffix.is_empty() && !matches!(item.name.as_str(), "new" | "terminal" | "rename") {
            self.error = Some(format!("{} does not take arguments.", item.label()));
            cx.notify();
            return;
        }
        match item.name.as_str() {
            "terminal" if !submitted => {
                self.composer
                    .update(cx, |input, cx| input.set_value("/terminal ", window, cx));
                self.slash.update(cx, |slash, cx| slash.dismiss(cx));
                self.composer.read(cx).focus_handle(cx).focus(window, cx);
            }
            "terminal" => {
                cx.emit(ChatEvent::Terminal {
                    task_id,
                    command: (!suffix.is_empty()).then_some(suffix),
                });
                self.clear_slash_draft(&input, window, cx);
            }
            "settings" => {
                self.clear_slash_draft(&input, window, cx);
                cx.emit(ChatEvent::Settings(task_id));
            }
            "project" => {
                let palette = self.composed_palette.unwrap_or_else(|| {
                    crate::settings::palette(&self.snapshot["settings"], window.appearance())
                });
                self.project_picker.update(cx, |picker, cx| {
                    picker.set_context(&self.snapshot, task_id, palette, cx);
                    picker.open_in_window(window, cx);
                });
                self.clear_slash_draft(&input, window, cx);
            }
            "new" | "clear" => {
                let mut commands = vec![("clear_task_context", json!({"taskId":task_id}))];
                if item.name == "new" && !suffix.is_empty() {
                    commands.push(("rename_task", json!({"id":self.task_id(),"title":suffix})));
                }
                self.slash_rpc(commands, input, Some("Context cleared.".into()), window, cx);
            }
            "rename" if suffix.is_empty() => {
                self.auto_name_chat(&input, window, cx);
            }
            "rename" if crate::activity_effects::is_naming(&task_id, cx) => {
                window.push_notification(gpui_kit::component::notification::Notification::info(
                    "A title is already being generated."), cx);
            }
            "rename" => self.slash_rpc(
                vec![rename_request(&task_id, &suffix)], input,
                Some("Chat title updated.".into()), window, cx,
            ),
            "stop" => self.slash_rpc(
                vec![("cancel_task", json!({"taskId":task_id}))],
                input,
                Some("Stopping task…".into()),
                window,
                cx,
            ),
            _ => {
                self.error = Some(
                    "This command is unavailable. Use // to send a literal slash message.".into(),
                );
                cx.notify();
            }
        }
    }

    /// Naming is independent of the chat turn and never disables composing.
    fn auto_name_chat(&mut self, original: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(service) = self.service.clone() else {
            self.error = Some("The experimental backend is not connected.".into());
            cx.notify();
            return;
        };
        let task_id = self.task_id();
        if !crate::activity_effects::begin_naming(&task_id, cx) {
            self.clear_slash_draft(original, window, cx);
            window.push_notification(gpui_kit::component::notification::Notification::info(
                "A title is already being generated."), cx);
            return;
        }
        let (command, args) = rename_request(&task_id, "");
        let (tx, rx) = futures::channel::oneshot::channel();
        let started = std::thread::Builder::new().name("monitter-admin-rename".into())
            .stack_size(8 * 1024 * 1024).spawn(move || {
                let _ = tx.send(service.invoke(command, args));
            });
        if let Err(error) = started {
            crate::activity_effects::end_naming(&task_id, cx);
            self.error = Some(format!("Unable to start naming worker: {error}"));
            cx.notify();
            return;
        }
        self.clear_slash_draft(original, window, cx);
        let window_handle = window.window_handle();
        cx.spawn(async move |pane, cx| {
            let result = rx.await.unwrap_or_else(|_| Err("Naming worker closed before returning.".into()));
            cx.update(|cx| {
                let generated = result.as_ref().ok().and_then(|snapshot| snapshot["tasks"].as_array())
                    .and_then(|tasks| tasks.iter().find(|task| task["id"].as_str() == Some(task_id.as_str())))
                    .and_then(|task| task["title"].as_str());
                if let Some(title) = generated {
                    crate::activity_effects::complete_naming(&task_id, title, cx);
                } else {
                    crate::activity_effects::end_naming(&task_id, cx);
                }
            });
            let _ = pane.update(cx, |pane, cx| {
                if pane.task_id() != task_id { return; }
                match &result {
                    Ok(snapshot) => {
                        cx.emit(ChatEvent::SnapshotUpdated(snapshot.clone()));
                        pane.refresh(cx);
                    }
                    Err(error) => pane.error = Some(error.clone()),
                }
                cx.notify();
            });
            let _ = cx.update_window(window_handle, |_, window, cx| {
                let notice = match result {
                    Ok(_) => gpui_kit::component::notification::Notification::success("Chat title updated."),
                    Err(error) => gpui_kit::component::notification::Notification::error(error),
                };
                window.push_notification(notice, cx);
            });
        }).detach();
        cx.notify();
    }

    fn clear_slash_draft(&mut self, sent: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.composer.read(cx).value().as_ref() == sent {
            self.composer
                .update(cx, |input, cx| input.set_value("", window, cx));
        }
        self.slash.update(cx, |slash, cx| slash.dismiss(cx));
    }

    /// Named worker keeps native RPC and deep provider payloads off the GPUI stack.
    fn slash_rpc(
        &mut self,
        commands: Vec<(&'static str, Value)>,
        original: String,
        notice: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(service) = self.service.clone() else {
            self.error = Some("The experimental backend is not connected.".into());
            cx.notify();
            return;
        };
        let task_id = self.task_id();
        let local_confirmation = notice.is_some();
        let stopping = commands.iter().any(|(command, _)| *command == "cancel_task");
        let (tx, rx) = futures::channel::oneshot::channel();
        let started = std::thread::Builder::new()
            .name("monitter-composer-command".into())
            .stack_size(8 * 1024 * 1024)
            .spawn(move || {
                let result = (|| {
                    let mut last = Value::Null;
                    for (command, args) in commands {
                        last = service.invoke(command, args)?;
                    }
                    Ok::<_, String>(last)
                })();
                let _ = tx.send(result);
            });
        if let Err(error) = started {
            self.error = Some(format!("Unable to start command worker: {error}"));
            cx.notify();
            return;
        }
        self.busy = true;
        self.composer_footer
            .update(cx, |footer, cx| footer.set_busy(true, cx));
        self.slash.update(cx, |slash, cx| slash.dismiss(cx));
        cx.spawn_in(window, async move |pane, cx| {
            let result = rx
                .await
                .unwrap_or_else(|_| Err("Command worker closed before returning a result.".into()));
            let _ = pane.update_in(cx, |pane, window, cx| {
                pane.busy = false;
                pane.composer_footer
                    .update(cx, |footer, cx| footer.set_busy(false, cx));
                if pane.task_id() != task_id {
                    return;
                }
                match result {
                    Ok(result) => {
                        pane.clear_sent = Some((task_id, original));
                        pane.slash_effect = result["effect"].as_str().map(str::to_owned);
                        let message = result["message"]
                            .as_str()
                            .filter(|s| !s.is_empty())
                            .map(str::to_owned)
                            .or(notice);
                        if let Some(message) = message {
                            let goal_confirmation = pane.slash_effect.as_deref() == Some("refreshGoal")
                                && matches!(message.as_str(), "Goal updated." | "Goal cleared." | "Goal paused." | "Goal resumed.");
                            if local_confirmation || goal_confirmation {
                                let toast = if stopping {
                                    gpui_kit::component::notification::Notification::info(message)
                                } else {
                                    gpui_kit::component::notification::Notification::success(message)
                                };
                                window.push_notification(toast, cx);
                                pane.slash_notice = None;
                            } else {
                                // Provider help/status/query output remains readable until
                                // the next draft edit; it may contain actionable details.
                                pane.slash_notice = Some(message);
                            }
                        }
                        pane.refresh(cx);
                    }
                    Err(error) => {
                        pane.error = Some(error);
                        let current = pane.composer.read(cx).value().to_string();
                        pane.slash
                            .update(cx, |slash, cx| slash.set_input(current, cx));
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::{literal_prompt, typed_submit_before_menu, rename_request};
    #[::core::prelude::v1::test]
    fn rename_bare_command_uses_admin_autoname_and_explicit_title_stays_manual() {
        let (command, args) = rename_request("task-123", "");
        assert_eq!(command, "autoname");
        assert_eq!(args, serde_json::json!({"target":{"taskId":"task-123"}}));
        let (command, args) = rename_request("task-123", "A manual title");
        assert_eq!(command, "rename_task");
        assert_eq!(args, serde_json::json!({"id":"task-123","title":"A manual title"}));
    }
    #[::core::prelude::v1::test]
    fn terminal_and_context_enter_preserve_typed_action_semantics() {
        assert!(typed_submit_before_menu(" /terminal echo hello"));
        assert!(typed_submit_before_menu("/NEW renamed pane"));
        assert!(typed_submit_before_menu("/rename My chat"));
        assert!(!typed_submit_before_menu("/terminally"));
        assert!(!typed_submit_before_menu("//terminal"));
        assert!(!typed_submit_before_menu("/ter"));
    }
    #[::core::prelude::v1::test]
    fn literal_slash_escape_changes_only_one_leading_slash() {
        assert_eq!(literal_prompt("  //model a\n//body"), "  /model a\n//body");
        assert_eq!(literal_prompt("é //model"), "é //model");
        assert_eq!(literal_prompt(" /model"), " /model");
        assert_eq!(literal_prompt(" \n"), " \n");
    }
}
