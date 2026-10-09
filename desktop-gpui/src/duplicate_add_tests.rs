//! DockWorkspace add() must be idempotent for a pane already in its area,
//! while keeping pre-registered panes available for their first insertion.
use super::{DockWorkspace, PaneFactory, PaneSpec};
use gpui::{AppContext as _, Context, IntoElement, Render, Styled as _, TestAppContext, Window, div};
use gpui_kit::component::dock::DockPlacement;
use std::rc::Rc;

struct PaneContent;

impl Render for PaneContent {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full()
    }
}

fn spec(id: &str, cx: &mut Context<DockWorkspace>) -> PaneSpec {
    PaneSpec {
        id: id.to_owned(),
        title: id.to_owned().into(),
        content: cx.new(|_| PaneContent).into(),
        focus: cx.focus_handle(),
        lifecycle: None,
        closable: true,
    }
}

#[gpui_kit::test]
fn moving_terminal_into_new_split_keeps_one_leaf_and_retained_entity(cx: &mut TestAppContext) {
    cx.update(|cx| gpui_kit::init(cx));
    let (_, visual) = cx.add_window_view(|window, cx| {
        let factory: PaneFactory = Rc::new(|_, _, _| None);
        let workspace = cx.new(|cx| DockWorkspace::new("terminal-move-regression", factory, window, cx));
        workspace.update(cx, |workspace, cx| {
            workspace.add(spec("chat:neighbor", cx), DockPlacement::Center, None, window, cx).unwrap();
            workspace.add(spec("terminal:retained", cx), DockPlacement::Center, None, window, cx).unwrap();
            let terminal = workspace.panels.borrow()["terminal:retained"].entity_id();
            workspace.focus("terminal:retained", window, cx);
            assert!(workspace.move_focused_tab("down", window, cx));
            let state = workspace.serialize(cx).unwrap();
            fn occurrences(value: &serde_json::Value, id: &str) -> usize {
                match value {
                    serde_json::Value::Object(fields) => usize::from(fields.get("id").and_then(|id| id.as_str()) == Some(id))
                        + fields.values().map(|value| occurrences(value, id)).sum::<usize>(),
                    serde_json::Value::Array(values) => values.iter().map(|value| occurrences(value, id)).sum(),
                    _ => 0,
                }
            }
            assert_eq!(occurrences(&state, "terminal:retained"), 1, "move must detach the source leaf");
            assert_eq!(occurrences(&state, "chat:neighbor"), 1);
            assert_eq!(workspace.panels.borrow()["terminal:retained"].entity_id(), terminal);
            assert_eq!(workspace.focused_pane_id(window, cx).as_deref(), Some("terminal:retained"));
        });
        gpui_kit::component::Root::new(workspace, window, cx)
    });
    visual.run_until_parked();
}

#[gpui_kit::test]
fn repeated_add_focuses_the_live_pane_without_adding_a_second_layout_leaf(
    cx: &mut TestAppContext,
) {
    cx.update(|cx| gpui_kit::init(cx));
    let ids_slot = Rc::new(std::cell::RefCell::new(None::<Vec<String>>));
    let ids_capture = ids_slot.clone();
    let (_root, cx) = cx.add_window_view(move |window, cx| {
        let factory: PaneFactory = Rc::new(|_, _, _| None);
        let workspace = cx.new(|cx| {
            DockWorkspace::new("duplicate-add-regression", factory, window, cx)
        });
        workspace.update(cx, |workspace, cx| {
            workspace
                .add(
                    spec("chat:duplicate-add", cx),
                    DockPlacement::Center,
                    None,
                    window,
                    cx,
                )
                .unwrap();
            workspace
                .add(
                    spec("chat:duplicate-add", cx),
                    DockPlacement::Center,
                    None,
                    window,
                    cx,
                )
                .unwrap();
            assert_eq!(
                workspace.focused_pane_id(window, cx).as_deref(),
                Some("chat:duplicate-add")
            );
            *ids_capture.borrow_mut() =
                Some(DockWorkspace::saved_ids(&workspace.serialize(cx).unwrap()).unwrap());
        });
        gpui_kit::component::Root::new(workspace, window, cx)
    });
    cx.run_until_parked();

    assert_eq!(
        ids_slot.borrow().as_deref(),
        Some(["chat:duplicate-add".to_owned()].as_slice())
    );
}

#[gpui_kit::test]
fn registered_pane_can_still_be_added_to_the_area(cx: &mut TestAppContext) {
    cx.update(|cx| gpui_kit::init(cx));
    let ids_slot = Rc::new(std::cell::RefCell::new(None::<Vec<String>>));
    let ids_capture = ids_slot.clone();
    let (_root, cx) = cx.add_window_view(move |window, cx| {
        let factory: PaneFactory = Rc::new(|_, _, _| None);
        let workspace = cx.new(|cx| {
            DockWorkspace::new("registered-add-regression", factory, window, cx)
        });
        workspace.update(cx, |workspace, cx| {
            workspace.register(spec("chat:registered-first", cx), cx).unwrap();
            let before_add = workspace.serialize(cx).unwrap();
            assert!(DockWorkspace::saved_ids(&before_add).unwrap().is_empty());

            workspace
                .add(
                    spec("chat:registered-first", cx),
                    DockPlacement::Center,
                    None,
                    window,
                    cx,
                )
                .unwrap();
            *ids_capture.borrow_mut() =
                Some(DockWorkspace::saved_ids(&workspace.serialize(cx).unwrap()).unwrap());
        });
        gpui_kit::component::Root::new(workspace, window, cx)
    });
    cx.run_until_parked();

    assert_eq!(
        ids_slot.borrow().as_deref(),
        Some(["chat:registered-first".to_owned()].as_slice())
    );
}
