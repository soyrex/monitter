//! A successful send acknowledgement clears the real native input before any
//! ChatPane render. The root renders only its Textarea; no provider is invoked.
use super::{ChatPane, ChatEvent, DesktopService, Textarea, TextareaState, CommandAcceptance, command_wire_name};
use gpui_kit::{AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, Styled as _, TestAppContext, VisualTestContext, Window, div};
use serde_json::json;
use std::{cell::RefCell, rc::Rc};

struct ComposerAckRoot {
    composer: Entity<TextareaState>,
}

impl Render for ComposerAckRoot {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(Textarea::new(&self.composer))
    }
}

pub(super) fn mount<'a>(cx: &'a mut TestAppContext, draft: &str) -> (Entity<ChatPane>, &'a mut VisualTestContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
    });
    let pane_slot = Rc::new(RefCell::new(None));
    let draft = draft.to_owned();
    let (_, visual) = cx.add_window_view({
        let pane_slot = pane_slot.clone();
        move |window, cx| {
            let snapshot = Rc::new(json!({
                "settings":{"theme":"dark", "accent":"#6594db"},
                "tasks":[{"id":"ack-task", "agentId":"ack-agent", "title":"Composer acknowledgement",
                    "provider":"codex", "status":"idle", "hostId":"ack-host", "cwd":"/synthetic/ack"}],
                "agents":[{"id":"ack-agent", "name":"Synthetic agent", "provider":"codex"}],
                "messages":[], "events":[], "approvalRequests":[], "queuedMessages":[],
                "sharing":{}, "hosts":[], "channels":[],
            }));
            let pane = cx.new(|cx| {
                let mut pane = ChatPane::new("ack-task".into(),
                    DesktopService::disconnected_for_ui_fixture(), snapshot, Some("synthetic-ack".into()), window, cx);
                pane.service = None;
                assert!(pane.restore_draft(&draft, window, cx));
                pane
            });
            let composer = pane.read(cx).composer.clone();
            *pane_slot.borrow_mut() = Some(pane);
            let root = cx.new(|_| ComposerAckRoot { composer });
            gpui_kit::component::Root::new(root, window, cx)
        }
    });
    let pane = pane_slot.borrow().as_ref().expect("real ChatPane").clone();
    (pane, visual)
}

fn assert_screenshot_ack_clears_before_render(cx: &mut TestAppContext, hidden: bool, map: bool) {
    let sent = "Please inspect this screenshot";
    let (pane, visual) = mount(cx, sent);
    let clears = Rc::new(RefCell::new(Vec::new()));
    let observed = clears.clone();
    let _subscription = visual.update(|_, cx| {
        cx.subscribe(&pane, move |_, event: &ChatEvent, _| {
            if let ChatEvent::DraftChanged { task_id, text } = event {
                observed.borrow_mut().push((task_id.clone(), text.clone()));
            }
        })
    });
    visual.update(|window, cx| {
        pane.update(cx, |pane, cx| {
            pane.dock_visible = !hidden;
            pane.agent_graph_visible = map;
            let saved = monitter_lib::experimental_desktop::NativeAttachmentDraft {
                host_id: "ack-host".into(), cwd: "/synthetic/ack".into(),
                attachments: vec![monitter_lib::experimental_desktop::NativeAttachmentRef {
                    id: "screenshot-attachment".into(), name: "Screenshot.png".into(),
                    // Metadata-only fixture avoids thumbnail IO and workers.
                    mime_type: "application/octet-stream".into(), size: 128,
                }],
            };
            assert!(pane.restore_attachment_draft("ack-task", &saved, cx));
            assert_eq!(pane.attachment_draft(cx).attachments.len(), 1);
            pane.attachments.update(cx, |attachments, cx| {
                attachments.acknowledge(&["screenshot-attachment".into()], cx);
            });
            assert!(pane.clear_acknowledged_composer("ack-task", sent, window, cx));
            assert!(pane.composer.read(cx).value().is_empty(), "native input clears without ChatPane render");
            assert_eq!(pane.draft(cx), "");
            assert!(pane.draft_was_edited());
            assert!(pane.attachment_draft(cx).attachments.is_empty());
        });
    });
    visual.run_until_parked();
    assert!(clears.borrow().iter().any(|(task, text)| task == "ack-task" && text.is_empty()),
        "the persisted draft receives an explicit empty value");
}

#[gpui_kit::test]
fn hidden_screenshot_and_text_ack_clears_native_composer_before_render(cx: &mut TestAppContext) {
    assert_screenshot_ack_clears_before_render(cx, true, false);
}

#[gpui_kit::test]
fn map_screenshot_and_text_ack_clears_native_composer_before_render(cx: &mut TestAppContext) {
    assert_screenshot_ack_clears_before_render(cx, false, true);
}

#[gpui_kit::test]
fn acknowledgement_preserves_text_typed_after_send(cx: &mut TestAppContext) {
    let (pane, visual) = mount(cx, "A newer unsent draft");
    visual.update(|window, cx| pane.update(cx, |pane, cx| {
        assert!(!pane.clear_acknowledged_composer("ack-task", "Previously sent text", window, cx));
        assert_eq!(pane.composer.read(cx).value().as_ref(), "A newer unsent draft");
        assert_eq!(pane.draft(cx), "A newer unsent draft");
    }));
}

#[gpui_kit::test]
fn acknowledgement_for_another_task_preserves_matching_text(cx: &mut TestAppContext) {
    let (pane, visual) = mount(cx, "Identical text");
    visual.update(|window, cx| pane.update(cx, |pane, cx| {
        assert!(!pane.clear_acknowledged_composer("other-task", "Identical text", window, cx));
        assert_eq!(pane.composer.read(cx).value().as_ref(), "Identical text");
        assert_eq!(pane.draft(cx), "Identical text");
    }));
}


fn async_command_ack(cx: &mut TestAppContext, outcome: Result<serde_json::Value, String>, replacement: Option<&str>) {
    let sent = "A real composer draft awaiting acceptance";
    let (pane, visual) = mount(cx, sent);
    let (accept, receiver) = futures::channel::oneshot::channel();
    visual.update(|_, cx| {
        let task = cx.background_executor().spawn(async move { receiver.await.expect("test releases acknowledgement") });
        pane.update(cx, |pane, cx| {
            pane.busy = true;
            pane.starting_turn = true;
            pane.composer_footer.update(cx, |footer, cx| footer.set_busy(true, cx));
            pane.await_command_acceptance(task, CommandAcceptance {
                command: "send_message", resolved_request: None,
                sent_attachments: Some(Vec::new()), sent: Some(("ack-task".into(), sent.into())),
            }, cx);
        });
    });
    visual.run_until_parked();
    pane.read_with(visual, |pane, cx| {
        assert!(pane.busy && pane.starting_turn, "send remains pending until its acknowledgement returns");
        assert_eq!(pane.composer.read(cx).value().as_ref(), sent);
    });
    if let Some(replacement) = replacement {
        visual.update(|window, cx| pane.update(cx, |pane, cx| {
            pane.composer.update(cx, |input, cx| input.set_value(replacement.to_owned(), window, cx));
        }));
    }
    let accepted = outcome.is_ok();
    accept.send(outcome).expect("synthetic acknowledgement receiver is alive");
    visual.run_until_parked();
    pane.read_with(visual, |pane, cx| {
        assert!(!pane.busy && !pane.starting_turn, "async stored-window callback releases the send spinner");
        let expected = replacement.unwrap_or(if accepted { "" } else { sent });
        assert_eq!(pane.composer.read(cx).value().as_ref(), expected);
        assert_eq!(pane.draft(cx), expected);
        assert_eq!(pane.error.is_some(), !accepted);
    });
}

#[gpui_kit::test]
fn async_acceptance_clears_native_composer_after_pending_future(cx: &mut TestAppContext) {
    async_command_ack(cx, Ok(json!({"accepted":true})), None);
}
#[gpui_kit::test]
fn async_failure_retains_draft_and_releases_spinner(cx: &mut TestAppContext) {
    async_command_ack(cx, Err("Synthetic backend validation error".into()), None);
}
#[gpui_kit::test]
fn async_acceptance_preserves_new_draft_typed_while_pending(cx: &mut TestAppContext) {
    async_command_ack(cx, Ok(json!({"accepted":true})), Some("New unsent text written while send was pending"));
}
#[::core::prelude::v1::test]
fn sends_use_bounded_acceptance_wire_command_without_changing_logical_recovery_name() {
    assert_eq!(command_wire_name("send_message"), "send_message_fast");
    assert_eq!(command_wire_name("edit_queued_message"), "edit_queued_message");
    assert_eq!(command_wire_name("resolve_input"), "resolve_input");
}
