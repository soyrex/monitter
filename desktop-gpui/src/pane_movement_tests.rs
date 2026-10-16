//! Directional movement retains panels and proportional pane allocations.
use super::{DockWorkspace, PaneFactory};
use gpui_kit::component::dock::{DockPlacement, NodeId};
use gpui_kit::{AppContext as _, Entity, Focusable as _, TestAppContext, VisualTestContext, px, size};
use std::{collections::HashMap, rc::Rc};

fn setup<'a>(
    cx: &'a mut TestAppContext,
    preset: &str,
) -> (Entity<DockWorkspace>, &'a mut VisualTestContext) {
    cx.update(gpui_kit::init);
    let mut workspace = None;
    let (_, visual) = cx.add_window_view(|window, cx| {
        let factory: PaneFactory = Rc::new(|_, _, _| None);
        let dock = cx.new(|cx| DockWorkspace::new("split-allocation-test", factory, window, cx));
        workspace = Some(dock.clone());
        gpui_kit::component::Root::new(dock, window, cx)
    });
    let dock = workspace.unwrap();
    visual.simulate_resize(size(px(2400.), px(1600.)));
    visual.update(|window, cx| {
        dock.update(cx, |dock, cx| {
            dock.apply_layout_preset(preset, window, cx).unwrap();
            assert!(dock.focus("empty:preset-1", window, cx));
        })
    });
    visual.run_until_parked();
    visual.update(|window, cx| window.draw(cx).clear(cx));
    (dock, visual)
}

fn areas(dock: &DockWorkspace, cx: &gpui_kit::App) -> HashMap<NodeId, f32> {
    let area = dock.area.read(cx);
    super::split_allocation::leaf_weights(
        area.layout(DockPlacement::Center).unwrap().root(),
        &area.split_geometry(cx),
    )
}

#[gpui_kit::test]
fn standalone_movement_reorders_siblings_without_merging_or_changing_weights(cx: &mut TestAppContext) {
    let (dock, cx) = setup(cx, "columns-3");
    cx.update(|window, cx| dock.update(cx, |dock, cx| {
        assert!(!dock.move_focused_tab("left", window, cx));
        let mut snapshots = dock.area.read(cx).split_geometry(cx);
        snapshots[0].target_shares = vec![0.2, 0.3, 0.5];
        dock.area.update(cx, |area, cx| area.equalize_split_geometry(&snapshots, 1., true, cx));
        let identities = dock.panels.borrow().iter().map(|(id, panel)| (id.clone(), panel.entity_id())).collect::<Vec<_>>();
        assert!(dock.move_focused_tab("right", window, cx));
        let area = dock.area.read(cx);
        let tree = area.layout(DockPlacement::Center).unwrap();
        let groups = super::groups_in(tree.root());
        let keys = ["empty:preset-2", "empty:preset-1", "empty:preset-3"].map(|id| dock.panel_key(id).unwrap());
        assert_eq!(groups.iter().map(|group| group.panels.clone()).collect::<Vec<_>>(), keys.map(|key| vec![key]));
        let weights = super::split_allocation::leaf_weights(tree.root(), &area.split_geometry(cx));
        for (group, expected) in groups.iter().zip([0.3, 0.2, 0.5]) {
            assert!((weights[&group.node] - expected).abs() < 0.0005);
        }
        assert_eq!(area.split_geometry(cx).len(), 1, "same-axis insertion remains flat");
        for (id, entity) in identities { assert_eq!(dock.panels.borrow()[&id].entity_id(), entity); }
        assert_eq!(dock.focused_pane_id(window, cx).as_deref(), Some("empty:preset-1"));
    }));
}

#[gpui_kit::test]
fn tab_movement_extracts_beside_source_and_borrows_from_whole_region(cx: &mut TestAppContext) {
    let (dock, cx) = setup(cx, "columns-3");
    cx.update(|window, cx| dock.update(cx, |dock, cx| {
        let group = dock.focused_group_node(window, cx).unwrap();
        let tab = dock.add_empty_at(super::InsertTarget::Tabs { node: group, ix: None, activate: true }, window, cx).unwrap();
        assert_eq!(areas(dock, cx).len(), 3);
        assert!(dock.move_focused_tab("right", window, cx));
        let after = areas(dock, cx);
        assert_eq!(after.len(), 4);
        assert!(after.values().all(|weight| (*weight - 0.25).abs() < 0.0005));
        let area = dock.area.read(cx);
        let groups = super::groups_in(area.layout(DockPlacement::Center).unwrap().root());
        assert_eq!(groups[0].panels, vec![dock.panel_key("empty:preset-1").unwrap()]);
        assert_eq!(groups[1].panels, vec![dock.panel_key(&tab).unwrap()]);
        assert_eq!(area.split_geometry(cx).len(), 1);
        assert_eq!(dock.focused_pane_id(window, cx), Some(tab));
    }));
}

#[gpui_kit::test]
fn standalone_grid_movement_preserves_all_panes_and_weights_in_every_direction(cx: &mut TestAppContext) {
    for (direction, source) in [("left", 2), ("right", 1), ("up", 3), ("down", 1)] {
        let (dock, visual) = setup(cx, "grid");
        visual.update(|window, cx| dock.update(cx, |dock, cx| {
            assert!(dock.focus(&format!("empty:preset-{source}"), window, cx));
            let prior_panels = dock.panels.borrow().iter().map(|(id, panel)| (id.clone(), panel.entity_id())).collect::<Vec<_>>();
            let mut before = areas(dock, cx).values().copied().collect::<Vec<_>>();
            before.sort_by(f32::total_cmp);
            assert!(dock.move_focused_tab(direction, window, cx), "{direction}");
            let mut after = areas(dock, cx).values().copied().collect::<Vec<_>>();
            after.sort_by(f32::total_cmp);
            assert_eq!(after.len(), 4);
            assert!(before.iter().zip(&after).all(|(a, b)| (a - b).abs() < 0.0005), "{direction}: {before:?} -> {after:?}");
            let area = dock.area.read(cx);
            let groups = super::groups_in(area.layout(DockPlacement::Center).unwrap().root());
            assert!(groups.iter().all(|group| group.panels.len() == 1));
            for (id, entity) in prior_panels { assert_eq!(dock.panels.borrow()[&id].entity_id(), entity); }
            assert_eq!(dock.focused_pane_id(window, cx), Some(format!("empty:preset-{source}")));
        }));
    }
}

#[gpui_kit::test]
fn tab_extraction_all_directions_retains_source_and_balances_nested_grid(cx: &mut TestAppContext) {
    for direction in ["left", "right", "up", "down"] {
        let (dock, visual) = setup(cx, "grid");
        visual.update(|window, cx| dock.update(cx, |dock, cx| {
            let source = dock.focused_group_node(window, cx).unwrap();
            let source_key = dock.panel_key("empty:preset-1").unwrap();
            let tab = dock.add_empty_at(super::InsertTarget::Tabs { node: source, ix: None, activate: true }, window, cx).unwrap();
            let tab_entity = dock.panels.borrow()[&tab].entity_id();
            assert!(dock.move_focused_tab(direction, window, cx), "{direction}");
            let after = areas(dock, cx);
            assert_eq!(after.len(), 5);
            assert!(after.values().all(|weight| (*weight - 0.2).abs() < 0.0005), "{direction}: {after:?}");
            let area = dock.area.read(cx);
            let tree = area.layout(DockPlacement::Center).unwrap();
            assert_eq!(tree.find_panel_node(source_key), Some(source), "source group remains stable");
            let source_node = tree.find_node(source).unwrap();
            assert!(matches!(source_node.kind(), super::PaneRef::Tabs { panels, .. } if panels == [source_key]));
            assert_eq!(dock.panels.borrow()[&tab].entity_id(), tab_entity);
            assert_eq!(dock.focused_pane_id(window, cx), Some(tab));
        }));
    }
}

#[gpui_kit::test]
fn git_panes_always_split_below_origin_without_replacing_or_reusing(cx: &mut TestAppContext) {
    let (dock, cx) = setup(cx, "columns");
    cx.update(|window, cx| dock.update(cx, |dock, cx| {
        let origin = "empty:preset-1";
        let original = dock.panels.borrow()[origin].entity_id();
        let other = dock.panels.borrow()["empty:preset-2"].entity_id();
        for id in ["git:task:1", "git:task:2"] {
            // Focus the unrelated pane to prove placement follows the origin.
            assert!(dock.focus("empty:preset-2", window, cx));
            let pane = cx.new(|cx| super::EmptyPane::new(id, window, cx));
            let spec = super::PaneSpec { id: id.into(), title: "Git Changes".into(), content: pane.clone().into(), focus: pane.read(cx).focus_handle(cx), lifecycle: None, closable: true };
            dock.add_below(spec, origin, window, cx).unwrap();
            assert_eq!(dock.focused_pane_id(window, cx).as_deref(), Some(id));
        }
        assert_eq!(dock.panels.borrow()[origin].entity_id(), original);
        assert_eq!(dock.panels.borrow()["empty:preset-2"].entity_id(), other);
        assert_ne!(dock.panels.borrow()["git:task:1"].entity_id(), dock.panels.borrow()["git:task:2"].entity_id());
        let area = dock.area.read(cx);
        let tree = area.layout(DockPlacement::Center).unwrap();
        let groups = super::groups_in(tree.root());
        assert_eq!(groups.len(), 4);
        assert!(groups.iter().all(|group| group.panels.len() == 1));
        let positions = groups.iter().map(|group| group.panels[0]).collect::<Vec<_>>();
        assert_eq!(positions, [origin, "git:task:2", "git:task:1", "empty:preset-2"].map(|id| dock.panel_key(id).unwrap()));
        let mut frames = Vec::new();
        super::collect_nav_frames(tree.root(), super::NavRect { x: 0., y: 0., width: 1., height: 1. }, &mut frames);
        let rect_for = |id: &str| frames.iter().find(|frame| frame.panels.contains(&dock.panel_key(id).unwrap())).unwrap().rect;
        let chat = rect_for(origin);
        let newest = rect_for("git:task:2");
        let older = rect_for("git:task:1");
        assert!(chat.y < newest.y && newest.y < older.y);
        assert!((chat.x - newest.x).abs() < 0.0001 && (chat.x - older.x).abs() < 0.0001);
        assert!((chat.width - newest.width).abs() < 0.0001 && (chat.width - older.width).abs() < 0.0001);
        let saved = serde_json::to_value(area.dump(cx)).unwrap();
        let ids = DockWorkspace::saved_ids(&saved).unwrap();
        assert!(ids.iter().any(|id| id == "git:task:1"));
        assert!(ids.iter().any(|id| id == "git:task:2"));
        let identities = [origin, "git:task:1", "git:task:2", "empty:preset-2"].map(|id| dock.panels.borrow()[id].entity_id());
        dock.restore_retaining(saved, window, cx).unwrap();
        assert_eq!(identities, [origin, "git:task:1", "git:task:2", "empty:preset-2"].map(|id| dock.panels.borrow()[id].entity_id()));
    }));
}

#[gpui_kit::test]
fn browser_links_create_separate_visible_panes_beside_origin(cx: &mut TestAppContext) {
    let (dock, cx) = setup(cx, "columns");
    cx.update(|window, cx| dock.update(cx, |dock, cx| {
        let origin = "empty:preset-1";
        let original = dock.panels.borrow()[origin].entity_id();
        let other = dock.panels.borrow()["empty:preset-2"].entity_id();
        let source_group = dock.group_for_pane(origin, cx).unwrap();
        for id in ["browser:101", "browser:102"] {
            assert!(dock.focus("empty:preset-2", window, cx));
            let pane = cx.new(|cx| super::EmptyPane::new(id, window, cx));
            let spec = super::PaneSpec { id: id.into(), title: "HTML report".into(), content: pane.clone().into(), focus: pane.read(cx).focus_handle(cx), lifecycle: None, closable: true };
            dock.add_browser_beside(spec, Some(source_group), window, cx).unwrap();
            assert_eq!(dock.focused_pane_id(window, cx).as_deref(), Some(id));
        }
        assert_eq!(dock.panels.borrow()[origin].entity_id(), original);
        assert_eq!(dock.panels.borrow()["empty:preset-2"].entity_id(), other);
        let area = dock.area.read(cx);
        let tree = area.layout(DockPlacement::Center).unwrap();
        let groups = super::groups_in(tree.root());
        assert_eq!(groups.len(), 4);
        assert!(groups.iter().all(|group| group.panels.len() == 1), "links must not become tabs covering the source");
        let mut frames = Vec::new();
        super::collect_nav_frames(tree.root(), super::NavRect { x: 0., y: 0., width: 1., height: 1. }, &mut frames);
        let rect_for = |id: &str| frames.iter().find(|frame| frame.panels.contains(&dock.panel_key(id).unwrap())).unwrap().rect;
        for id in ["browser:101", "browser:102"] { assert!(rect_for(id).x > rect_for(origin).x); }
    }));
}
