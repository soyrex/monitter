//! Offline native question forms: real retained inputs, click routing and command acceptance.
use super::{ChatPane, CommandAcceptance};
use gpui_kit::{AppContext as _, Entity, Focusable as _, TestAppContext, VisualTestContext, size};
use serde_json::{Value, json};
use std::rc::Rc;

fn request(id: &str, task: &str, multiple: bool) -> Value {
    json!({"id":id,"taskId":task,"status":"pending","provider":"acp","tool":"ask_user","summary":"Choose the next step","createdAt":1,
        "input":{"kind":"questions","questions":[{"id":"answer","header":"Next step","question":"Which work should proceed?","isSecret":false,
            "selectionMode":if multiple {"multiple"} else {"single"},"options":[{"label":"Research","description":"Inspect evidence"},{"label":"Implement","description":"Make the change"}]}]}})
}
fn snapshot(requests: Vec<Value>) -> Rc<Value> {
    Rc::new(
        json!({"settings":{"theme":"dark","interfaceScale":100,"interfaceFontSize":14},
        "tasks":[{"id":"question-task","agentId":"fixture-agent","title":"Question fixture","provider":"acp","status":"running","queue":[]}],
        "agents":[{"id":"fixture-agent","name":"Fixture agent","provider":"acp"}],
        "messages":[],"events":[],"sharing":{},"hosts":[],"channels":[],"workPlans":[],"queuedMessages":[],"approvalRequests":requests}),
    )
}
fn draw(visual: &mut VisualTestContext) {
    visual.run_until_parked();
    visual.update(|window, cx| window.draw(cx).clear(cx));
    visual.run_until_parked();
}
fn mount<'a>(
    cx: &'a mut TestAppContext,
    requests: Vec<Value>,
    watch: bool,
) -> (Entity<ChatPane>, &'a mut VisualTestContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::surface_blend::initialize_for_ui_fixture();
        crate::native_motion::initialize_for_ui_fixture(cx);
    });
    let snapshot = snapshot(requests);
    let mut retained = None;
    let (_, visual) = cx.add_window_view(|window, cx| {
        let pane = cx.new(|cx| {
            let mut pane =
                ChatPane::performance_fixture("question-task".into(), snapshot, window, cx);
            pane.restore_draft("Unsent composer draft", window, cx);
            pane.watcher.enabled = watch;
            window.focus(&pane.composer.read(cx).focus_handle(cx), cx);
            pane
        });
        retained = Some(pane.clone());
        gpui_kit::component::Root::new(pane, window, cx)
    });
    visual.simulate_resize(size(gpui_kit::px(720.), gpui_kit::px(860.)));
    draw(visual);
    (retained.unwrap(), visual)
}
fn click(visual: &mut VisualTestContext, selector: &'static str) {
    let bounds = visual
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("missing {selector}"));
    visual.simulate_click(bounds.center(), Default::default());
    draw(visual);
}
fn accept(pane: &Entity<ChatPane>, visual: &mut VisualTestContext, result: Result<Value, String>) {
    visual.update(|_, cx| {
        pane.update(cx, |pane, cx| {
            let (command, args) = pane.user_question_commands.last().unwrap().clone();
            let task = cx.background_executor().spawn(async move { result });
            pane.await_command_acceptance(
                task,
                CommandAcceptance {
                    command,
                    resolved_request: args["approvalId"].as_str().map(str::to_owned),
                    sent: None,
                    sent_attachments: None,
                },
                cx,
            );
        })
    });
    draw(visual);
    draw(visual);
}

#[gpui_kit::test]
fn user_question_replaces_composer_and_cancel_waits_for_success_preserving_draft(
    cx: &mut TestAppContext,
) {
    let (pane, visual) = mount(cx, vec![request("question", "question-task", false)], false);
    assert!(visual.debug_bounds("user-question-form").is_some());
    assert!(visual.debug_bounds("normal-chat-composer").is_none());
    for width in [320., 960.] {
        visual.simulate_resize(size(gpui_kit::px(width), gpui_kit::px(860.)));
        draw(visual);
        let form = visual.debug_bounds("user-question-form").unwrap();
        assert!(
            form.left() >= gpui_kit::px(0.) && form.right() <= gpui_kit::px(width),
            "question form stays inside narrow and wide chat panes"
        );
    }
    visual.update(|window, cx| {
        let pane = pane.read(cx);
        assert!(
            pane.answers[&("question".into(), "answer".into())]
                .read(cx)
                .focus_handle(cx)
                .is_focused(window),
            "a newly visible question takes focus from the retained composer"
        );
    });
    visual.simulate_input("A typed answer");
    visual.simulate_keystrokes("enter");
    draw(visual);
    visual.read(|cx| {
        let pane = pane.read(cx);
        assert_eq!(
            pane.answers[&("question".into(), "answer".into())]
                .read(cx)
                .value()
                .as_ref(),
            "A typed answer"
        );
        assert_eq!(
            pane.composer.read(cx).value().as_ref(),
            "Unsent composer draft"
        );
        assert!(
            pane.user_question_commands.is_empty(),
            "typing never sends ordinary chat"
        );
    });
    click(visual, "answer-question-answer-0");
    visual.read(|cx| {
        assert_eq!(
            pane.read(cx).answers[&("question".into(), "answer".into())]
                .read(cx)
                .value()
                .as_ref(),
            "Research"
        );
        assert_eq!(
            pane.read(cx).composer.read(cx).value().as_ref(),
            "Unsent composer draft"
        );
        assert!(
            pane.read(cx).pending_tool_approval().is_none(),
            "questions remain distinct from permission approvals"
        );
    });
    click(visual, "cancel-input-question");
    visual.read(|cx| {
        let pane = pane.read(cx);
        assert!(pane.busy);
        assert_eq!(
            pane.user_question_commands,
            vec![(
                "resolve_approval",
                json!({"approvalId":"question","decision":"deny"})
            )]
        );
    });
    assert!(
        visual.debug_bounds("user-question-form").is_some(),
        "an unacknowledged cancellation stays visible"
    );
    accept(
        &pane,
        visual,
        Err("Disconnected; retry this response".into()),
    );
    assert!(visual.debug_bounds("user-question-form").is_some());
    click(visual, "cancel-input-question");
    accept(&pane, visual, Ok(json!({})));
    assert!(visual.debug_bounds("user-question-form").is_none());
    assert!(visual.debug_bounds("normal-chat-composer").is_some());
    visual.update(|window, cx| {
        let pane = pane.read(cx);
        assert_eq!(pane.user_question_commands.len(), 2);
        assert_eq!(
            pane.composer.read(cx).value().as_ref(),
            "Unsent composer draft"
        );
        assert!(pane.composer.read(cx).focus_handle(cx).is_focused(window));
    });
}

#[gpui_kit::test]
fn user_question_focus_waits_for_visible_pane_activation_without_stealing_other_focus(
    cx: &mut TestAppContext,
) {
    let (pane, visual) = mount(cx, vec![], false);
    visual.update(|_, cx| {
        pane.update(cx, |pane, cx| {
            pane.dock_visible = false;
            pane.sync(
                snapshot(vec![request("deferred", "question-task", false)]),
                Some("pending".into()),
                cx,
            );
        });
    });
    draw(visual);
    visual.update(|window, cx| {
        let pane = pane.read(cx);
        assert!(
            pane.user_question_focus_id.is_none(),
            "hidden panes retain the pending focus transition"
        );
        assert!(pane.composer.read(cx).focus_handle(cx).is_focused(window));
    });
    let other_focus = visual.update(|window, cx| {
        let other = cx.focus_handle();
        window.focus(&other, cx);
        pane.update(cx, |pane, cx| {
            pane.dock_visible = true;
            cx.notify();
        });
        other
    });
    draw(visual);
    visual.update(|window, cx| {
        assert!(
            other_focus.is_focused(window),
            "background publication never steals focus"
        );
        assert!(pane.read(cx).user_question_focus_id.is_none());
        window.focus(&pane.read(cx).focus_handle(cx), cx);
        pane.update(cx, |_, cx| cx.notify());
    });
    draw(visual);
    visual.update(|window, cx| {
        let pane = pane.read(cx);
        assert!(
            pane.answers[&("deferred".into(), "answer".into())]
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
        );
        assert_eq!(pane.user_question_focus_id.as_deref(), Some("deferred"));
    });
}

#[gpui_kit::test]
fn user_question_watch_uses_shared_multiselect_form_without_chat_delivery(cx: &mut TestAppContext) {
    let question = request("multi", "question-task", true);
    let (pane, visual) = mount(cx, vec![question.clone()], true);
    assert!(visual.debug_bounds("chat-watcher").is_some());
    assert!(visual.debug_bounds("user-question-form").is_some());
    assert!(visual.debug_bounds("normal-chat-composer").is_none());
    for width in [320., 960.] {
        visual.simulate_resize(size(gpui_kit::px(width), gpui_kit::px(860.)));
        draw(visual);
        let form = visual.debug_bounds("user-question-form").unwrap();
        let watch = visual.debug_bounds("chat-watcher").unwrap();
        assert!(
            form.left() >= watch.left() && form.right() <= watch.right(),
            "question form stays inside narrow and wide Watch panes"
        );
    }
    click(visual, "answer-multi-answer-0");
    click(visual, "answer-multi-answer-1");
    visual.update(|window, cx| {
        pane.update(cx, |pane, cx| {
            pane.answers[&("multi".into(), "answer".into())].update(cx, |state, cx| {
                state.set_value("Also document the result", window, cx)
            });
        })
    });
    draw(visual);
    click(visual, "submit-multi");
    visual.update(|_,cx| pane.update(cx,|pane,cx| {
        pane.submit_questions(&question,cx); // Busy responses cannot submit twice.
        assert_eq!(pane.user_question_commands,vec![("resolve_input",json!({"approvalId":"multi","response":{"answers":{"answer":{"answers":["Research","Implement","Also document the result"]}}}}))]);
        assert!(pane.watcher.enabled,"answering never exits Watch");
        assert_eq!(pane.composer.read(cx).value().as_ref(),"Unsent composer draft");
    }));
    accept(&pane, visual, Ok(json!({})));
    assert!(visual.debug_bounds("chat-watcher").is_some());
    assert!(visual.debug_bounds("user-question-form").is_none());
}

#[gpui_kit::test]
fn user_question_navigation_is_task_scoped_and_resolved_or_stale_forms_cannot_submit(
    cx: &mut TestAppContext,
) {
    let first = request("first", "question-task", false);
    let second = request("second", "question-task", false);
    let foreign = request("foreign", "other-task", false);
    let (pane, visual) = mount(
        cx,
        vec![first.clone(), second.clone(), foreign.clone()],
        false,
    );
    assert!(visual.debug_bounds("input-body-first").is_some());
    assert!(visual.debug_bounds("input-body-foreign").is_none());
    click(visual, "next-user-question");
    assert!(visual.debug_bounds("input-body-second").is_some());
    visual.update(|_, cx| {
        pane.update(cx, |pane, cx| {
            pane.submit_questions(&foreign, cx);
            assert!(pane.user_question_commands.is_empty());
            let mut latest = (*pane.snapshot).clone();
            latest["approvalRequests"][1]["status"] = json!("expired");
            pane.sync(Rc::new(latest), Some("resolved-2".into()), cx);
            pane.submit_questions(&second, cx);
            assert!(
                pane.user_question_commands.is_empty(),
                "stale resolved identity cannot reach the bridge"
            );
        })
    });
    draw(visual);
    assert!(visual.debug_bounds("input-body-second").is_none());
    assert!(visual.debug_bounds("input-body-first").is_some());
    click(visual, "answer-first-answer-1");
    click(visual, "submit-first");
    visual.read(|cx| assert_eq!(pane.read(cx).user_question_commands[0],("resolve_input",json!({"approvalId":"first","response":{"answers":{"answer":{"answers":["Implement"]}}}}))));
}
