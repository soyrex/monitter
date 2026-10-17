//! Pane-scoped task rename and archive actions.
//!
//! Each entity captures one task identity at construction. Rename/archive
//! calls are sent to the canonical service and only emit a change event after
//! the backend confirms success.
use crate::design::{self, ui_scaled_px as px};
use futures::channel::oneshot;
use gpui_kit::{
    component::{Disableable, button::*, input::{Input, InputState, InputEvent}},
    prelude::*,
    *,
};
use monitter_lib::experimental_desktop::DesktopService;
use serde_json::{Value, json};

const WORKER_STACK_SIZE: usize = 8 * 1024 * 1024;

#[derive(Clone, Debug)]
pub enum ChatTaskActionsEvent {
    Changed { task_id: String, archived: bool },
    Failed { task_id: String, error: String },
    Cancelled,
}

impl EventEmitter<ChatTaskActionsEvent> for ChatTaskActionsPane {}

pub struct ChatTaskActionsPane {
    service: DesktopService,
    task_id: String,
    title: Entity<InputState>,
    _title_subscription: Subscription,
    palette: design::Palette,
    busy: bool,
    error: Option<String>,
}

impl ChatTaskActionsPane {
    /// Keep an open rename editor, including its undo/selection state, attached
    /// to the same native host as its owning retained chat pane.
    pub(crate) fn rehost_to_window(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Result<(), String> {
        let id = cx.entity_id();
        cx.rehost_entity_window(id, window).map_err(|error| error.to_string())?;
        self.title.update(cx, |input, cx| input.rehost_to_window(window, cx))
    }

    /// Create an action entity bound to one owning chat pane.
    pub fn new(
        service: DesktopService,
        task_id: String,
        current_title: &str,
        palette: design::Palette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let title = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(current_title)
                .placeholder("Chat title")
        });
        let subscription = cx.subscribe(&title, |pane, _, event, cx| {
            match event {
                InputEvent::PressEnter { .. } => pane.submit_rename(cx),
                InputEvent::Change => cx.notify(),
                _ => {}
            }
        });
        Self {
            _title_subscription: subscription,
            service,
            task_id,
            title,
            palette,
            busy: false,
            error: None,
        }
    }

    pub fn focus_title(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.title.read(cx).focus_handle(cx).focus(window, cx);
    }

    pub fn task_id(&self) -> &str {
        &self.task_id
    }

    pub fn is_busy(&self) -> bool {
        self.busy
    }

    pub fn request_cancel(&self, cx: &mut Context<Self>) {
        if !self.busy {
            cx.emit(ChatTaskActionsEvent::Cancelled);
        }
    }

    pub fn submit_rename(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let title = self.title.read(cx).value().to_string();
        if title.trim().is_empty() {
            self.error = Some("Chat title is required.".into());
            cx.notify();
            return;
        }
        self.invoke(
            "rename_task",
            json!({"id": self.task_id.clone(), "title": title.trim()}),
            false,
            cx,
        );
    }

    /// Archive directly from the pane action menu. The service rejects active
    /// tasks; that error remains visible in this entity for the owner to show.
    pub fn archive(&mut self, cx: &mut Context<Self>) {
        self.invoke(
            "set_task_archived",
            json!({"taskId": self.task_id.clone(), "archived": true}),
            true,
            cx,
        );
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref().and_then(crate::chat::connection_recovery::visible_error)
    }

    fn invoke(
        &mut self,
        command: &'static str,
        args: Value,
        archived: bool,
        cx: &mut Context<Self>,
    ) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.error = None;
        let task_id = self.task_id.clone();
        let service = self.service.clone();
        let (sender, receiver) = oneshot::channel();
        let worker_name = if archived {
            "monitter-archive-chat"
        } else {
            "monitter-rename-chat"
        };
        let started = std::thread::Builder::new()
            .name(worker_name.into())
            .stack_size(WORKER_STACK_SIZE)
            .spawn(move || {
                let result = service.invoke(command, args);
                let _ = sender.send(result);
            });
        if let Err(error) = started {
            self.busy = false;
            self.error = Some(format!("Unable to start task action: {error}"));
            cx.emit(ChatTaskActionsEvent::Failed { task_id: self.task_id.clone(), error: self.error.clone().unwrap() });
            cx.notify();
            return;
        }

        cx.spawn(async move |pane, cx| {
            let result = receiver
                .await
                .unwrap_or_else(|_| Err("Task action worker closed before returning.".into()));
            let _ = pane.update(cx, |pane, cx| {
                if pane.task_id != task_id {
                    return;
                }
                pane.busy = false;
                match result {
                    Ok(_canonical_snapshot) => {
                        cx.emit(ChatTaskActionsEvent::Changed { task_id, archived });
                    }
                    Err(error) => {
                        pane.error = Some(error.clone());
                        cx.emit(ChatTaskActionsEvent::Failed { task_id, error });
                    },
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}

impl Render for ChatTaskActionsPane {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = self.palette;
        let title = Input::new(&self.title).w_full();
        let title_value = self.title.read(cx).value().to_string();
        let can_submit = !self.busy && !title_value.trim().is_empty();
        let mut dialog = div()
            .w(px(420.))
            .max_w_full()
            .flex()
            .flex_col()
            .gap(px(design::space::INSET))
            .p(px(design::space::INSET))
            .rounded_lg()
            .border_1()
            .border_color(rgb(palette.line))

            .text_color(rgb(palette.ink))
            .child(title);
        if let Some(error) = self.error() {
            dialog = dialog.child(
                div()
                    .text_size(px(11.))
                    .text_color(rgb(0xdc2626))
                    .child(error.to_owned()),
            );
        }
        dialog.child(
            div()
                .flex()
                .justify_end()
                .gap(px(design::space::INSET))
                .child(
                    Button::new("chat-task-rename-cancel")
                        .label("Cancel")
                        .disabled(self.busy)
                        .on_click(cx.listener(|pane, _, _, cx| pane.request_cancel(cx))),
                )
                .child(
                    Button::new("chat-task-rename-submit")
                        .primary()
                        .label(if self.busy { "Saving…" } else { "Save title" })
                        .disabled(!can_submit)
                        .on_click(cx.listener(|pane, _, _, cx| pane.submit_rename(cx))),
                ),
        )
    }
}
