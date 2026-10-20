//! A successful send acknowledgement clears the real native input before any
//! ChatPane render. The root renders only its Textarea; no provider is invoked.
use super::{
    ChatEvent, ChatPane, CommandAcceptance, DesktopService, Textarea, TextareaState,
    command_wire_name,
};
use gpui_kit::{
    AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, Styled as _,
    TestAppContext, VisualTestContext, Window, div,
};
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

struct HiddenAckHost;

impl Render for HiddenAckHost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full()
    }
}

pub(super) fn mount<'a>(
    cx: &'a mut TestAppContext,
    draft: &str,
) -> (Entity<ChatPane>, &'a mut VisualTestContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        #[cfg(feature = "performance-profiling")]
        {
            crate::surface_blend::initialize_for_ui_fixture();
            crate::native_motion::initialize_for_ui_fixture(cx);
        }
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
                host_id: "ack-host".into(),
                cwd: "/synthetic/ack".into(),
                attachments: vec![monitter_lib::experimental_desktop::NativeAttachmentRef {
                    id: "screenshot-attachment".into(),
                    name: "Screenshot.png".into(),
                    // Metadata-only fixture avoids thumbnail IO and workers.
                    mime_type: "application/octet-stream".into(),
                    size: 128,
                }],
            };
            assert!(pane.restore_attachment_draft("ack-task", &saved, cx));
            assert_eq!(pane.attachment_draft(cx).attachments.len(), 1);
            pane.attachments.update(cx, |attachments, cx| {
                attachments.acknowledge(&["screenshot-attachment".into()], cx);
            });
            assert!(pane.clear_acknowledged_composer("ack-task", sent, window, cx));
            assert!(
                pane.composer.read(cx).value().is_empty(),
                "native input clears without ChatPane render"
            );
            assert_eq!(pane.draft(cx), "");
            assert!(pane.draft_was_edited());
            assert!(pane.attachment_draft(cx).attachments.is_empty());
        });
    });
    visual.run_until_parked();
    assert!(
        clears
            .borrow()
            .iter()
            .any(|(task, text)| task == "ack-task" && text.is_empty()),
        "the persisted draft receives an explicit empty value"
    );
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
    visual.update(|window, cx| {
        pane.update(cx, |pane, cx| {
            assert!(!pane.clear_acknowledged_composer(
                "ack-task",
                "Previously sent text",
                window,
                cx
            ));
            assert_eq!(
                pane.composer.read(cx).value().as_ref(),
                "A newer unsent draft"
            );
            assert_eq!(pane.draft(cx), "A newer unsent draft");
        })
    });
}

#[gpui_kit::test]
fn acknowledgement_for_another_task_preserves_matching_text(cx: &mut TestAppContext) {
    let (pane, visual) = mount(cx, "Identical text");
    visual.update(|window, cx| {
        pane.update(cx, |pane, cx| {
            assert!(!pane.clear_acknowledged_composer("other-task", "Identical text", window, cx));
            assert_eq!(pane.composer.read(cx).value().as_ref(), "Identical text");
            assert_eq!(pane.draft(cx), "Identical text");
        })
    });
}

fn async_command_ack(
    cx: &mut TestAppContext,
    outcome: Result<serde_json::Value, String>,
    replacement: Option<&str>,
) {
    let sent = "A real composer draft awaiting acceptance";
    let (pane, visual) = mount(cx, sent);
    let (accept, receiver) = futures::channel::oneshot::channel();
    visual.update(|_, cx| {
        let task = cx
            .background_executor()
            .spawn(async move { receiver.await.expect("test releases acknowledgement") });
        pane.update(cx, |pane, cx| {
            pane.busy = true;
            pane.starting_turn = true;
            pane.composer_footer
                .update(cx, |footer, cx| footer.set_busy(true, cx));
            pane.await_command_acceptance(
                task,
                CommandAcceptance {
                    command: "send_message",
                    resolved_request: None,
                    sent_attachments: Some(Vec::new()),
                    sent: Some(("ack-task".into(), sent.into())),
                },
                cx,
            );
        });
    });
    visual.run_until_parked();
    pane.read_with(visual, |pane, cx| {
        assert!(
            pane.busy && pane.starting_turn,
            "send remains pending until its acknowledgement returns"
        );
        assert_eq!(pane.composer.read(cx).value().as_ref(), sent);
    });
    if let Some(replacement) = replacement {
        visual.update(|window, cx| {
            pane.update(cx, |pane, cx| {
                pane.composer.update(cx, |input, cx| {
                    input.set_value(replacement.to_owned(), window, cx)
                });
            })
        });
    }
    let accepted = outcome.is_ok();
    accept
        .send(outcome)
        .expect("synthetic acknowledgement receiver is alive");
    visual.run_until_parked();
    pane.read_with(visual, |pane, cx| {
        assert!(
            !pane.busy && !pane.starting_turn,
            "async current-window callback releases the send spinner"
        );
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
    async_command_ack(
        cx,
        Ok(json!({"accepted":true})),
        Some("New unsent text written while send was pending"),
    );
}

fn pending_transfer_ack(
    pane: &Entity<ChatPane>,
    visual: &mut VisualTestContext,
    sent: &str,
) -> futures::channel::oneshot::Sender<Result<serde_json::Value, String>> {
    let (accept, receiver) = futures::channel::oneshot::channel();
    visual.update(|_, cx| {
        let task = cx
            .background_executor()
            .spawn(async move { receiver.await.expect("fixture releases acknowledgement") });
        pane.update(cx, |pane, cx| {
            pane.busy = true;
            pane.starting_turn = true;
            pane.composer_footer
                .update(cx, |footer, cx| footer.set_busy(true, cx));
            pane.await_command_acceptance(
                task,
                CommandAcceptance {
                    command: "send_message",
                    resolved_request: None,
                    sent_attachments: None,
                    sent: Some(("ack-task".into(), sent.into())),
                },
                cx,
            );
        });
    });
    visual.run_until_parked();
    accept
}

#[gpui_kit::test]
fn async_acceptance_follows_hidden_pane_after_original_window_closes(cx: &mut TestAppContext) {
    let sent = "Accepted after native window transfer";
    let (pane, visual) = mount(cx, sent);
    let accept = pending_transfer_ack(&pane, visual, sent);
    let moved = pane.clone();
    let destination = visual.add_window(move |window, cx| {
        moved
            .update(cx, |pane, cx| pane.rehost_to_window(window, cx))
            .unwrap();
        // The host does not render ChatPane. A hidden pane's acceptance must
        // still clear the retained native editor in its explicitly assigned host.
        HiddenAckHost
    });
    visual.update(|window, _| window.remove_window());
    accept.send(Ok(json!({"accepted":true}))).unwrap();
    visual.run_until_parked();
    pane.read_with(visual, |pane, cx| {
        assert!(!pane.busy && !pane.starting_turn);
        assert!(pane.composer.read(cx).value().is_empty());
        assert_eq!(pane.draft(cx), "");
        assert!(pane.clear_sent.is_none());
        assert!(pane.error.is_none());
    });
    destination
        .update(visual, |_, window, _| window.remove_window())
        .unwrap();
}

#[cfg(feature = "performance-profiling")]
#[gpui_kit::test]
fn async_acceptance_between_hosts_finalizes_state_and_clears_on_next_render(
    cx: &mut TestAppContext,
) {
    let sent = "Accepted while no native host exists";
    let (pane, visual) = mount(cx, sent);
    let accept = pending_transfer_ack(&pane, visual, sent);
    visual.update(|window, _| window.remove_window());
    accept.send(Ok(json!({"accepted":true}))).unwrap();
    visual.run_until_parked();
    pane.read_with(visual, |pane, cx| {
        assert!(!pane.busy && !pane.starting_turn);
        assert_eq!(
            pane.draft(cx),
            "",
            "accepted input is no longer persisted as a draft"
        );
        assert_eq!(
            pane.composer.read(cx).value().as_ref(),
            sent,
            "physical input waits for a valid host"
        );
        assert!(pane.clear_sent.is_some());
    });
    let moved = pane.clone();
    // Drawing enters the real Root itself. Use the erased window handle so
    // update does not first borrow that Root as a typed mutable view.
    let destination: gpui_kit::AnyWindowHandle = visual
        .add_window(move |window, cx| {
            moved
                .update(cx, |pane, cx| pane.rehost_to_window(window, cx))
                .unwrap();
            gpui_kit::component::Root::new(moved, window, cx)
        })
        .into();
    destination
        .update(visual, |_, window, cx| window.draw(cx).clear(cx))
        .unwrap();
    pane.read_with(visual, |pane, cx| {
        assert!(pane.composer.read(cx).value().is_empty());
        assert!(pane.clear_sent.is_none());
    });
    destination
        .update(visual, |_, window, _| window.remove_window())
        .unwrap();
}

#[gpui_kit::test]
fn hidden_chat_rehosting_and_return_keep_retained_inputs_and_drafts(cx: &mut TestAppContext) {
    use gpui_kit::component::input::InputState;
    let sent = "Retain the unsent draft across both transfers";
    let (pane, visual) = mount(cx, sent);
    let composer = pane.read_with(visual, |pane, _| pane.composer.clone());
    let queue_editor = pane.read_with(visual, |pane, _| pane.queue_editor.clone());
    let answer = visual.update(|window, cx| {
        let input = cx.new(|cx| InputState::new(window, cx).default_value("Retained answer"));
        pane.update(cx, |pane, _| {
            pane.answers
                .insert(("request".into(), "answer".into()), input.clone());
        });
        queue_editor.update(cx, |input, cx| {
            input.set_value("Retained queue edit", window, cx)
        });
        input
    });
    let original_id = visual.update(|window, _| window.window_handle().window_id());
    // A transfer first unmounts its source content. Otherwise the fixture's
    // composer-only root would render the same input in two native hosts and
    // correctly repair its input observers back to the old window.
    visual.update(|window, cx| {
        window.replace_root(cx, |window, cx| {
            gpui_kit::component::Root::new(cx.new(|_| HiddenAckHost), window, cx)
        });
    });
    let moved = pane.clone();
    let destination = visual.add_window(move |window, cx| {
        moved
            .update(cx, |pane, cx| pane.rehost_to_window(window, cx))
            .unwrap();
        HiddenAckHost
    });
    visual.update(|window, cx| {
        for id in [
            pane.entity_id(),
            composer.entity_id(),
            queue_editor.entity_id(),
            answer.entity_id(),
        ] {
            assert_eq!(
                cx.with_window(id, |window, _| window.window_handle().window_id()),
                Some(destination.window_id()),
                "retained entity {id:?} follows its destination before rendering"
            );
        }
        pane.update(cx, |pane, cx| pane.rehost_to_window(window, cx))
            .unwrap();
    });
    destination
        .update(visual, |_, _, cx| {
            for id in [
                pane.entity_id(),
                composer.entity_id(),
                queue_editor.entity_id(),
                answer.entity_id(),
            ] {
                assert_eq!(
                    cx.with_window(id, |window, _| window.window_handle().window_id()),
                    Some(original_id),
                    "retained entity {id:?} follows its original host on return"
                );
            }
            pane.read_with(cx, |pane, cx| {
                assert_eq!(pane.composer.entity_id(), composer.entity_id());
                assert_eq!(pane.queue_editor.entity_id(), queue_editor.entity_id());
                assert_eq!(pane.draft(cx), sent);
                assert_eq!(
                    queue_editor.read(cx).value().as_ref(),
                    "Retained queue edit"
                );
                assert_eq!(
                    pane.answers[&("request".into(), "answer".into())].entity_id(),
                    answer.entity_id()
                );
                assert_eq!(answer.read(cx).value().as_str(), "Retained answer");
            });
        })
        .unwrap();
    destination
        .update(visual, |_, window, _| window.remove_window())
        .unwrap();
}

#[gpui_kit::test]
fn async_failure_between_hosts_releases_spinner_and_retains_error_and_draft(
    cx: &mut TestAppContext,
) {
    let sent = "Keep this failed request";
    let (pane, visual) = mount(cx, sent);
    let accept = pending_transfer_ack(&pane, visual, sent);
    visual.update(|window, _| window.remove_window());
    accept
        .send(Err("Fixture backend rejected the request".into()))
        .unwrap();
    visual.run_until_parked();
    pane.read_with(visual, |pane, cx| {
        assert!(!pane.busy && !pane.starting_turn);
        assert_eq!(pane.draft(cx), sent);
        assert_eq!(pane.composer.read(cx).value().as_ref(), sent);
        assert_eq!(
            pane.error.as_deref(),
            Some("Fixture backend rejected the request")
        );
        assert!(pane.clear_sent.is_none());
    });
}
#[::core::prelude::v1::test]
fn sends_use_bounded_acceptance_wire_command_without_changing_logical_recovery_name() {
    assert_eq!(command_wire_name("send_message"), "send_message_fast");
    assert_eq!(
        command_wire_name("edit_queued_message"),
        "edit_queued_message"
    );
    assert_eq!(command_wire_name("resolve_input"), "resolve_input");
}
