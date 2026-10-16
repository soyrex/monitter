//! A landing-tab selection reuses the retained chat and targets its own group.
use super::{DockWorkspace, PaneFactory, PaneSpec};
use gpui_kit::{AppContext as _, Context, IntoElement, ParentElement as _, Render,
    Styled as _, TestAppContext, Window, div, component::dock::DockPlacement};
use std::rc::Rc;

struct Content;
impl Render for Content {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child("Existing chat")
    }
}

#[gpui_kit::test]
fn new_tab_session_selection_reuses_chat_in_destination_group(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let mut workspace = None;
    let (_, cx) = cx.add_window_view(|window, cx| {
        let factory: PaneFactory = Rc::new(|_, _, _| None);
        let dock = cx.new(|cx| DockWorkspace::new("session-picker-test", factory, window, cx));
        workspace = Some(dock.clone());
        gpui_kit::component::Root::new(dock, window, cx)
    });
    let workspace = workspace.unwrap();
    cx.update(|window, cx| {
        let content = cx.new(|_| Content);
        let focus = cx.focus_handle();
        workspace.update(cx, |dock, cx| {
            dock.add(
                PaneSpec {
                    id: "chat:one".into(),
                    title: "One".into(),
                    content: content.into(),
                    focus,
                    lifecycle: None,
                    closable: true,
                },
                DockPlacement::Center,
                None,
                window,
                cx,
            )
            .unwrap();
            dock.focus("chat:one", window, cx);
            let original = dock.panel_key("chat:one").unwrap();
            let origin = dock.focused_group_node(window, cx).unwrap();
            let empty = dock.add_empty_split(origin, true, window, cx).unwrap();
            let empty_key = dock.panel_key(&empty).unwrap();
            let group = dock
                .area
                .read(cx)
                .layout(DockPlacement::Center)
                .unwrap()
                .find_panel_node(empty_key)
                .unwrap();
            assert!(dock.replace_empty_with_chat(&empty, "chat:one", window, cx));
            assert_eq!(dock.panel_key("chat:one"), Some(original));
            assert!(dock.panel_key(&empty).is_none());
            let area = dock.area.read(cx);
            let tree = area.layout(DockPlacement::Center).unwrap();
            assert_eq!(tree.find_panel_node(original), Some(group));
            assert_eq!(tree.panels().collect::<Vec<_>>(), vec![original]);
        });
    });
}

#[gpui_kit::test]
fn new_tab_close_button_removes_last_pane_with_hidden_headers(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let mut workspace = None;
    let (_, cx) = cx.add_window_view(|window, cx| {
        let factory: PaneFactory = Rc::new(|_, _, _| None);
        let dock = cx.new(|cx| DockWorkspace::new("empty-close-test", factory, window, cx));
        workspace = Some(dock.clone());
        let target = dock.clone();
        cx.subscribe_in(&dock, window, move |_, _, event, window, cx| {
            if let super::WorkspaceEvent::EmptyClose { pane_id } = event {
                target.update(cx, |dock, cx| { dock.close_empty(pane_id, window, cx); });
            }
        }).detach();
        gpui_kit::component::Root::new(dock, window, cx)
    });
    let workspace = workspace.unwrap();
    cx.update(|window, cx| workspace.update(cx, |dock, cx| {
        dock.set_tab_appearance(&serde_json::json!({"autoHideTabs":true}), 125, cx);
        let spec = dock.empty_spec("empty:close-test", window, cx).unwrap();
        dock.add(spec, DockPlacement::Center, None, window, cx).unwrap();
        dock.focus("empty:close-test", window, cx);
    }));
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let close = cx.debug_bounds("empty-pane-close").expect("last new tab has a visible close button despite hidden header");
    cx.simulate_click(close.center(), gpui_kit::Modifiers::none());
    cx.run_until_parked();
    cx.update(|_, cx| {
        let dock = workspace.read(cx);
        assert!(dock.panel_key("empty:close-test").is_none());
        assert!(!dock.empty_panes.contains_key("empty:close-test"));
        assert!(DockWorkspace::saved_ids(&dock.serialize(cx).unwrap()).unwrap().is_empty());
    });
    cx.update(|window, cx| workspace.update(cx, |dock, cx| {
        let id = dock.add_focused_empty_tab(window, cx).expect("empty workspace accepts another tab");
        assert!(dock.panel_key(&id).is_some());
    }));
}

#[gpui_kit::test]
fn new_chat_replaces_its_own_tab_and_preserves_other_drafts(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let mut workspace = None;
    let (_, cx) = cx.add_window_view(|window, cx| {
        let factory: PaneFactory = Rc::new(|_, _, _| None);
        let dock = cx.new(|cx| DockWorkspace::new("draft-replacement-test", factory, window, cx));
        workspace = Some(dock.clone());
        gpui_kit::component::Root::new(dock, window, cx)
    });
    let workspace = workspace.unwrap();
    cx.update(|window, cx| {
        let content = cx.new(|_| Content);
        workspace.update(cx, |dock, cx| {
            for id in ["draft:first", "draft:second", "chat:sent"] {
                dock.add(PaneSpec { id: id.into(), title: id.into(), content: content.clone().into(), focus: cx.focus_handle(), lifecycle: None, closable: true }, DockPlacement::Center, None, window, cx).unwrap();
            }
            let second = dock.panel_key("draft:second").unwrap();
            let first = dock.panel_key("draft:first").unwrap();
            let group = dock.area.read(cx).layout(DockPlacement::Center).unwrap().find_panel_node(first).unwrap();
            assert!(dock.replace_pane("draft:first", "chat:sent", window, cx));
            assert!(dock.panel_key("draft:first").is_none());
            assert_eq!(dock.panel_key("draft:second"), Some(second));
            let sent = dock.panel_key("chat:sent").unwrap();
            let area = dock.area.read(cx);
            let tree = area.layout(DockPlacement::Center).unwrap();
            assert_eq!(tree.find_panel_node(sent), Some(group));
            assert_eq!(tree.panels().count(), 2);
        });
    });
}

#[gpui_kit::test]
fn unsent_tabs_survive_workspace_switch_and_legacy_file_layout_pruning(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let mut workspace = None;
    let (_, cx) = cx.add_window_view(|window, cx| {
        let factory: PaneFactory = Rc::new(|_, _, _| None);
        let dock = cx.new(|cx| DockWorkspace::new("draft-workspace-retention", factory, window, cx));
        workspace = Some(dock.clone());
        gpui_kit::component::Root::new(dock, window, cx)
    });
    let workspace = workspace.unwrap();
    cx.update(|window, cx| {
        let content = cx.new(|_| Content);
        workspace.update(cx, |dock, cx| {
            for id in ["draft:unsent", "neovim:editor:1", "neovim-files:editor"] {
                dock.add(PaneSpec { id: id.into(), title: id.into(), content: content.clone().into(), focus: cx.focus_handle(), lifecycle: None, closable: true }, DockPlacement::Center, None, window, cx).unwrap();
            }
            let original = dock.panel_key("draft:unsent").unwrap();
            let saved = dock.serialize(cx).unwrap();
            let cleaned = crate::workspace_visibility::prune_native_pane_ids(&saved, &std::collections::HashSet::from(["neovim-files:editor".into()])).unwrap();
            let ids = DockWorkspace::saved_ids(&cleaned).unwrap();
            assert!(ids.contains(&"neovim:editor:1".into()));
            assert!(ids.contains(&"draft:unsent".into()));
            assert!(!ids.contains(&"neovim-files:editor".into()));
            let other_workspace = crate::workspace_tabs::empty_layout();
            let requested_ids = DockWorkspace::saved_ids(&other_workspace).unwrap();
            assert_eq!(requested_ids.len(), 1, "a new workspace includes its empty landing tab");
            assert!(requested_ids[0].starts_with("empty:"));
            dock.restore_retaining(other_workspace, window, cx).unwrap();
            assert_eq!(DockWorkspace::saved_ids(&dock.serialize(cx).unwrap()).unwrap(), requested_ids, "only the other workspace's landing pane belongs to the live layout");
            assert_eq!(dock.panel_key("draft:unsent"), Some(original), "the unsent form must remain retained while its workspace is hidden");
            dock.restore_retaining(cleaned, window, cx).unwrap();
            assert_eq!(dock.panel_key("draft:unsent"), Some(original), "switching workspaces must reuse the unsent form entity");
        });
    });
}
