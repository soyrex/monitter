//! Auto sizing adapts the entire view after pane/tab insertions and moves,
//! while manual sizing and restored proportions remain authoritative.
use super::{DockWorkspace, PaneFactory};
use gpui_kit::{AppContext as _, Entity, TestAppContext, VisualTestContext, px, size, point, MouseButton, Modifiers};
use std::{rc::Rc, time::{Duration, Instant}};

fn setup(cx: &mut TestAppContext) -> (Entity<DockWorkspace>, &mut VisualTestContext) {
    cx.update(gpui_kit::init);
    let mut dock = None;
    let (_, visual) = cx.add_window_view(|window, cx| {
        let factory: PaneFactory = Rc::new(|_, _, _| None);
        let workspace = cx.new(|cx| DockWorkspace::new("auto-size-test", factory, window, cx));
        dock = Some(workspace.clone());
        gpui_kit::component::Root::new(workspace, window, cx)
    });
    let dock = dock.unwrap();
    visual.simulate_resize(size(px(2400.), px(1600.)));
    visual.update(|window, cx| dock.update(cx, |dock, cx| {
        assert!(dock.auto_size_panes());
        dock.apply_layout_preset("columns-3", window, cx).unwrap();
        dock.focus("empty:preset-1", window, cx);
    }));
    settle(&dock, visual);
    (dock, visual)
}
fn settle(dock: &Entity<DockWorkspace>, visual: &mut VisualTestContext) {
    for _ in 0..5 {
        visual.run_until_parked();
        visual.update(|window, cx| {
            dock.update(cx, |dock, _| {
                if let Some((started, _)) = &mut dock.equalize_motion { *started = Instant::now() - Duration::from_secs(1); }
            });
            window.simulate_next_frame(cx);
            window.draw(cx).clear(cx);
        });
    }
}
fn shares(dock: &DockWorkspace, cx: &gpui_kit::App) -> Vec<f32> {
    dock.area.read(cx).split_geometry(cx)[0].shares.clone()
}
fn unequal(dock: &mut DockWorkspace, cx: &mut gpui_kit::Context<DockWorkspace>) {
    let mut geometry = dock.area.read(cx).split_geometry(cx);
    geometry[0].target_shares = vec![0.2, 0.3, 0.5];
    dock.area.update(cx, |area, cx| area.equalize_split_geometry(&geometry, 1., true, cx));
}
#[gpui_kit::test]
fn auto_size_new_pane_equalizes_and_retains_existing_entities(cx: &mut TestAppContext) {
    let (dock, visual) = setup(cx);
    let before = visual.update(|window, cx| dock.update(cx, |dock, cx| {
        unequal(dock, cx);
        let ids = dock.panels.borrow().iter().map(|(id, panel)| (id.clone(), panel.entity_id())).collect::<Vec<_>>();
        let group = dock.focused_group_node(window, cx).unwrap();
        dock.add_empty_split(group, true, window, cx).unwrap();
        ids
    }));
    settle(&dock, visual);
    visual.update(|_, cx| dock.update(cx, |dock, cx| {
        assert!(dock.auto_size_panes());
        let sizes = shares(dock, cx);
        assert_eq!(sizes.len(), 4);
        assert!(sizes.iter().all(|s| (s - 0.25).abs() < 0.005), "{sizes:?}");
        for (id, entity) in before { assert_eq!(dock.panels.borrow()[&id].entity_id(), entity); }
    }));
}
#[gpui_kit::test]
fn auto_size_keyboard_resize_disables_until_explicitly_reenabled(cx: &mut TestAppContext) {
    let (dock, visual) = setup(cx);
    visual.update(|window, cx| dock.update(cx, |dock, cx| {
        assert!(dock.resize_focused_group("horizontal", Some(40), false, window, cx));
        assert!(!dock.auto_size_panes());
        let group = dock.focused_group_node(window, cx).unwrap();
        dock.add_empty_split(group, true, window, cx).unwrap();
    }));
    settle(&dock, visual);
    visual.update(|window, cx| dock.update(cx, |dock, cx| {
        assert!(!dock.auto_size_panes());
        assert!(shares(dock, cx).iter().any(|s| (s - 0.25).abs() > 0.005));
        dock.set_auto_size_panes(true, window, cx);
    }));
    settle(&dock, visual);
    visual.update(|_, cx| assert!(shares(dock.read(cx), cx).iter().all(|s| (s - 0.25).abs() < 0.005)));
}
#[gpui_kit::test]
fn auto_size_pointer_divider_drag_disables_but_window_resize_does_not(cx: &mut TestAppContext) {
    let (dock, visual) = setup(cx);
    visual.simulate_resize(size(px(2100.), px(1400.)));
    settle(&dock, visual);
    let handle = visual.update(|_, cx| {
        let area = dock.read(cx).area.read(cx);
        assert!(dock.read(cx).auto_size_panes());
        let bounds = area.bounds();
        point(bounds.origin.x + bounds.size.width / 3., bounds.center().y)
    });
    let target = point(handle.x + px(120.), handle.y);
    visual.simulate_mouse_down(handle, MouseButton::Left, Modifiers::none());
    visual.simulate_mouse_move(target, MouseButton::Left, Modifiers::none());
    visual.simulate_mouse_up(target, MouseButton::Left, Modifiers::none());
    settle(&dock, visual);
    visual.update(|_, cx| assert!(!dock.read(cx).auto_size_panes(), "real divider drag must disable auto size"));
}
#[gpui_kit::test]
fn auto_size_restore_preserves_saved_proportions(cx: &mut TestAppContext) {
    let (dock, visual) = setup(cx);
    visual.update(|window, cx| dock.update(cx, |dock, cx| {
        unequal(dock, cx);
        let saved = dock.serialize(cx).unwrap();
        dock.restore_retaining(saved, window, cx).unwrap();
    }));
    settle(&dock, visual);
    visual.update(|_, cx| {
        assert!(dock.read(cx).auto_size_panes());
        let sizes = shares(dock.read(cx), cx);
        assert!(sizes.iter().zip([0.2,0.3,0.5]).all(|(s,t)| (s-t).abs()<0.005), "{sizes:?}");
    });
}

#[gpui_kit::test]
fn restore_more_groups_preserves_sizes_after_outgoing_motion_and_deferred_events(
    cx: &mut TestAppContext,
) {
    let (dock, visual) = setup(cx);
    let saved = visual.update(|window, cx| dock.update(cx, |dock, cx| {
        unequal(dock, cx);
        let saved = dock.serialize(cx).unwrap();
        dock.schedule_auto_size(window, cx);
        // Seed the actual outgoing geometry clock independently of the host's
        // motion preference, then leave its real next-frame callback queued.
        dock.equalize_motion = Some((Instant::now(), dock.area.read(cx).split_geometry(cx)));
        dock.schedule_equalize_frame(window, cx);
        assert!(dock.auto_size_pending);
        assert!(dock.equalize_motion.is_some());
        let mut single = saved.clone();
        single["center"]["children"].as_array_mut().unwrap().truncate(1);
        single["center"]["info"]["stack"]["sizes"] = serde_json::json!([2400.]);
        dock.restore_retaining(single, window, cx).unwrap();
        assert!(!dock.auto_size_pending);
        assert!(dock.equalize_motion.is_none());
        saved
    }));
    visual.run_until_parked();
    visual.update(|window, cx| dock.update(cx, |dock, cx| {
        assert!(!dock.restoring_layout);
        assert_eq!(dock.live_group_count, 1);
        dock.restore_retaining(saved, window, cx).unwrap();
        assert_eq!(dock.live_group_count, 3);
        assert!(!dock.auto_size_pending);
        assert!(dock.equalize_motion.is_none());
    }));
    settle(&dock, visual);
    visual.update(|_, cx| {
        let workspace = dock.read(cx);
        assert!(workspace.auto_size_panes());
        assert!(!workspace.restoring_layout);
        assert!(!workspace.auto_size_pending);
        assert!(workspace.equalize_motion.is_none());
        let sizes = shares(workspace, cx);
        assert!(sizes.iter().zip([0.2, 0.3, 0.5]).all(|(s,t)| (s-t).abs()<0.005), "{sizes:?}");
    });
    // The next real insertion remains eligible for normal auto sizing.
    visual.update(|window, cx| dock.update(cx, |dock, cx| {
        let group = dock.focused_group_node(window, cx).unwrap();
        dock.add_empty_split(group, true, window, cx).unwrap();
    }));
    settle(&dock, visual);
    visual.update(|_, cx| {
        let sizes = shares(dock.read(cx), cx);
        assert_eq!(sizes.len(), 4);
        assert!(sizes.iter().all(|s| (s - 0.25).abs() < 0.005), "{sizes:?}");
    });
}

#[gpui_kit::test]
fn auto_size_new_tab_rebalances_all_existing_groups(cx: &mut TestAppContext) {
    let (dock, visual) = setup(cx);
    visual.update(|window, cx| dock.update(cx, |dock, cx| {
        unequal(dock, cx);
        let group = dock.focused_group_node(window, cx).unwrap();
        dock.add_empty_tab(group, window, cx).unwrap();
        assert_eq!(dock.visible_group_count(cx), 3);
    }));
    settle(&dock, visual);
    visual.update(|_, cx| {
        let sizes = shares(dock.read(cx), cx);
        assert!(sizes.iter().all(|s| (s - 1. / 3.).abs() < 0.005), "{sizes:?}");
    });
}

#[gpui_kit::test]
fn auto_size_new_tab_respects_disabled_toggle(cx: &mut TestAppContext) {
    let (dock, visual) = setup(cx);
    visual.update(|window, cx| dock.update(cx, |dock, cx| {
        dock.set_auto_size_panes(false, window, cx);
        unequal(dock, cx);
        let group = dock.focused_group_node(window, cx).unwrap();
        dock.add_empty_tab(group, window, cx).unwrap();
    }));
    settle(&dock, visual);
    visual.update(|_, cx| {
        let sizes = shares(dock.read(cx), cx);
        assert!(sizes.iter().zip([0.2, 0.3, 0.5]).all(|(s,t)| (s-t).abs()<0.005), "{sizes:?}");
    });
}

#[gpui_kit::test]
fn auto_size_nested_split_rebalances_every_split_in_view(cx: &mut TestAppContext) {
    let (dock, visual) = setup(cx);
    visual.update(|window, cx| dock.update(cx, |dock, cx| {
        let group = dock.focused_group_node(window, cx).unwrap();
        dock.add_empty_split(group, false, window, cx).unwrap();
    }));
    settle(&dock, visual);
    visual.update(|window, cx| dock.update(cx, |dock, cx| {
        let mut geometry = dock.area.read(cx).split_geometry(cx);
        assert!(geometry.len() >= 2);
        for split in &mut geometry {
            let total = (1..=split.shares.len()).sum::<usize>() as f32;
            split.target_shares = (1..=split.shares.len()).map(|i| i as f32 / total).collect();
        }
        assert!(dock.area.update(cx, |area, cx| area.equalize_split_geometry(&geometry, 1., true, cx)));
        assert!(dock.split_focused_group("right", None, window, cx).unwrap());
    }));
    settle(&dock, visual);
    visual.update(|_, cx| {
        let geometry = dock.read(cx).area.read(cx).split_geometry(cx);
        assert!(geometry.len() >= 2);
        for split in geometry {
            assert!(split.shares.iter().zip(&split.target_shares).all(|(s,t)| (s-t).abs()<0.005), "{:?} != {:?}", split.shares, split.target_shares);
        }
    });
}

#[gpui_kit::test]
fn auto_size_moving_tab_rebalances_without_a_new_group(cx: &mut TestAppContext) {
    let (dock, visual) = setup(cx);
    let id = visual.update(|window, cx| dock.update(cx, |dock, cx| {
        let group = dock.focused_group_node(window, cx).unwrap();
        dock.add_empty_tab(group, window, cx).unwrap()
    }));
    settle(&dock, visual);
    visual.update(|window, cx| dock.update(cx, |dock, cx| {
        unequal(dock, cx);
        let group = dock.group_for_pane("empty:preset-2", cx).unwrap();
        assert!(dock.move_pane(&id, gpui_kit::component::dock::InsertTarget::Tabs { node: group, ix: None, activate: true }, window, cx));
        assert_eq!(dock.visible_group_count(cx), 3);
    }));
    settle(&dock, visual);
    visual.update(|_, cx| {
        let sizes = shares(dock.read(cx), cx);
        assert!(sizes.iter().all(|s| (s - 1. / 3.).abs() < 0.005), "{sizes:?}");
    });
}

#[gpui_kit::test]
fn auto_size_closing_pane_rebalances_remaining_space(cx: &mut TestAppContext) {
    let (dock, visual) = setup(cx);
    visual.update(|window, cx| dock.update(cx, |dock, cx| {
        unequal(dock, cx);
        assert!(dock.remove("empty:preset-1", window, cx));
    }));
    settle(&dock, visual);
    visual.update(|_, cx| {
        let sizes = shares(dock.read(cx), cx);
        assert_eq!(sizes.len(), 2);
        assert!(sizes.iter().all(|s| (s - 0.5).abs() < 0.005), "{sizes:?}");
    });
}

#[gpui_kit::test]
fn auto_size_directional_move_rebalances_without_changing_pane_count(cx: &mut TestAppContext) {
    let (dock, visual) = setup(cx);
    visual.update(|window, cx| dock.update(cx, |dock, cx| {
        unequal(dock, cx);
        assert!(dock.move_focused_tab("right", window, cx));
        assert_eq!(dock.visible_panel_count(cx), 3);
    }));
    settle(&dock, visual);
    visual.update(|_, cx| {
        let sizes = shares(dock.read(cx), cx);
        assert_eq!(sizes.len(), 3);
        assert!(sizes.iter().all(|s| (s - 1. / 3.).abs() < 0.005), "{sizes:?}");
    });
}

#[gpui_kit::test]
fn auto_size_tab_reorder_rebalances_without_changing_pane_count(cx: &mut TestAppContext) {
    let (dock, visual) = setup(cx);
    let tab = visual.update(|window, cx| dock.update(cx, |dock, cx| {
        let group = dock.focused_group_node(window, cx).unwrap();
        dock.add_empty_tab(group, window, cx).unwrap()
    }));
    settle(&dock, visual);
    visual.update(|window, cx| dock.update(cx, |dock, cx| {
        unequal(dock, cx);
        assert!(dock.focus(&tab, window, cx));
        assert!(dock.move_focused_tab_order("first", window, cx));
        assert_eq!(dock.visible_panel_count(cx), 4);
    }));
    settle(&dock, visual);
    visual.update(|_, cx| {
        let sizes = shares(dock.read(cx), cx);
        assert_eq!(sizes.len(), 3);
        assert!(sizes.iter().all(|s| (s - 1. / 3.).abs() < 0.005), "{sizes:?}");
    });
}

#[gpui_kit::test]
fn disabled_auto_size_preserves_proportions_when_panes_move(cx: &mut TestAppContext) {
    let (dock, visual) = setup(cx);
    visual.update(|window, cx| dock.update(cx, |dock, cx| {
        dock.set_auto_size_panes(false, window, cx);
        unequal(dock, cx);
        assert!(dock.move_focused_tab("right", window, cx));
    }));
    settle(&dock, visual);
    visual.update(|_, cx| {
        let sizes = shares(dock.read(cx), cx);
        assert!(sizes.iter().zip([0.3, 0.2, 0.5]).all(|(s,t)| (s-t).abs() < 0.005), "{sizes:?}");
    });
}
