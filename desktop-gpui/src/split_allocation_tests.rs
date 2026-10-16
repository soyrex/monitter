//! Native split insertion must borrow from the whole related region while
//! retaining pane entities, tab-group identities, focus, and orthogonal shape.
use super::{DockWorkspace, PaneFactory};
use gpui_kit::component::dock::{DockPlacement, NodeId, PaneRef};
use gpui_kit::{AppContext as _, Entity, TestAppContext, VisualTestContext, px, size};
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
fn split_allocation_equal_three_columns_become_four_equal_and_keep_entities(
    cx: &mut TestAppContext,
) {
    let (dock, cx) = setup(cx, "columns-3");
    cx.update(|window, cx| {
        dock.update(cx, |dock, cx| {
            let identities = dock
                .panels
                .borrow()
                .iter()
                .map(|(id, panel)| (id.clone(), panel.entity_id()))
                .collect::<Vec<_>>();
            let old_groups = areas(dock, cx).keys().copied().collect::<Vec<_>>();
            let group = dock.focused_group_node(window, cx).unwrap();
            let new_id = dock.add_empty_split(group, true, window, cx).unwrap();
            let after = areas(dock, cx);
            assert_eq!(after.len(), 4);
            for weight in after.values() {
                assert!((*weight - 0.25).abs() < 0.0005, "{after:?}");
            }
            for group in old_groups {
                assert!(after.contains_key(&group), "native tab group retained");
            }
            for (id, entity) in identities {
                assert_eq!(dock.panels.borrow()[&id].entity_id(), entity);
            }
            assert_eq!(dock.focused_pane_id(window, cx), Some(new_id));
        })
    });
}

#[gpui_kit::test]
fn split_allocation_custom_columns_preserve_old_ratios_and_borrow_from_all(
    cx: &mut TestAppContext,
) {
    let (dock, cx) = setup(cx, "columns-3");
    cx.update(|window, cx| {
        dock.update(cx, |dock, cx| {
            let mut snapshots = dock.area.read(cx).split_geometry(cx);
            assert_eq!(snapshots.len(), 1);
            snapshots[0].target_shares = vec![0.2, 0.3, 0.5];
            assert!(dock.area.update(cx, |area, cx| {
                area.equalize_split_geometry(&snapshots, 1., true, cx)
            }));
            let before = areas(dock, cx);
            let group = dock.focused_group_node(window, cx).unwrap();
            let new_id = dock.add_empty_split(group, true, window, cx).unwrap();
            let after = areas(dock, cx);
            assert_eq!(after.len(), 4);
            for (node, weight) in before {
                assert!((after[&node] - weight * 0.75).abs() < 0.0005, "{after:?}");
            }
            let new_key = dock.panel_key(&new_id).unwrap();
            let new_node = dock
                .area
                .read(cx)
                .layout(DockPlacement::Center)
                .unwrap()
                .find_panel_node(new_key)
                .unwrap();
            assert!((after[&new_node] - 0.25).abs() < 0.0005);
        })
    });
}

#[gpui_kit::test]
fn split_allocation_nested_grid_balances_leaf_areas_in_every_direction(cx: &mut TestAppContext) {
    for direction in ["left", "right", "up", "down"] {
        let (dock, cx) = setup(cx, "grid");
        cx.update(|window, cx| {
            dock.update(cx, |dock, cx| {
                let before = areas(dock, cx);
                assert_eq!(before.len(), 4);
                let root_axis = match dock
                    .area
                    .read(cx)
                    .layout(DockPlacement::Center)
                    .unwrap()
                    .root()
                    .kind()
                {
                    PaneRef::Split { axis, .. } => axis,
                    _ => panic!("grid root split"),
                };
                assert!(
                    dock.split_focused_group(direction, None, window, cx)
                        .unwrap()
                );
                let after = areas(dock, cx);
                assert_eq!(after.len(), 5);
                for weight in after.values() {
                    assert!((*weight - 0.2).abs() < 0.0005, "{direction}: {after:?}");
                }
                for node in before.keys() {
                    assert!(after.contains_key(node));
                }
                let area = dock.area.read(cx);
                let tree = area.layout(DockPlacement::Center).unwrap();
                assert!(
                    matches!(tree.root().kind(), PaneRef::Split {axis,..} if axis == root_axis)
                );
            })
        });
    }
}

#[gpui_kit::test]
fn split_allocation_tern_new_tab_balances_physical_panes_but_normal_tabs_do_not(
    cx: &mut TestAppContext,
) {
    let (dock, cx) = setup(cx, "columns-3");
    cx.update(|window, cx| {
        dock.update(cx, |dock, cx| {
            let before = areas(dock, cx);
            let group = dock.focused_group_node(window, cx).unwrap();
            dock.add_empty_tab(group, window, cx).unwrap();
            assert_eq!(
                areas(dock, cx),
                before,
                "ordinary tab creation consumes no extra physical area"
            );
            crate::design::set_tile_theme(
                monitter_lib::experimental_desktop::NativeTileTheme::Tern,
            );
            let settings = serde_json::json!({});
            let palette = crate::design::palette(&settings, gpui_kit::WindowAppearance::Dark);
            dock.set_pane_appearance(&settings, palette, cx);
            dock.set_tab_appearance(&settings, 100, cx);
            let group = dock.focused_group_node(window, cx).unwrap();
            let new_id = dock.add_empty_tab(group, window, cx).unwrap();
            let after = areas(dock, cx);
            assert_eq!(
                after.len(),
                4,
                "Tern new-tab creates a fourth physical pane"
            );
            for weight in after.values() {
                assert!((*weight - 0.25).abs() < 0.0005);
            }
            assert_eq!(dock.focused_pane_id(window, cx), Some(new_id));
            // No frame is rendered between insertions: allocation must read the
            // synchronously committed native sizes, not an animated paint frame.
            let group = dock.focused_group_node(window, cx).unwrap();
            let next_id = dock.add_empty_tab(group, window, cx).unwrap();
            let after = areas(dock, cx);
            assert_eq!(after.len(), 5);
            for weight in after.values() {
                assert!((*weight - 0.2).abs() < 0.0005, "{after:?}");
            }
            assert_eq!(dock.focused_pane_id(window, cx), Some(next_id));
            crate::design::set_tile_theme(
                monitter_lib::experimental_desktop::NativeTileTheme::Monitter,
            );
        })
    });
}

#[gpui_kit::test]
fn split_allocation_first_vertical_split_handles_canonical_single_child_root(
    cx: &mut TestAppContext,
) {
    let (dock, cx) = setup(cx, "single");
    cx.update(|window, cx| {
        dock.update(cx, |dock, cx| {
            assert!(dock.split_focused_group("down", None, window, cx).unwrap());
            let after = areas(dock, cx);
            assert_eq!(after.len(), 2);
            for weight in after.values() {
                assert!((*weight - 0.5).abs() < 0.0005);
            }
        })
    });
}
