//! The content of a native pane should begin at the bottom of its dock tab
//! strip, without the extra eight design pixels from the generic panel inset.
use super::{NativePanel, PaintState, PaneSpec};
use gpui::{
    App, AppContext as _, Context, InteractiveElement as _, IntoElement, Render, ParentElement as _, Styled as _, TestAppContext, Window, canvas, div, px,
    size,
};
use gpui_kit::component::dock::{
    DockArea, DockLayout, DockSkin, DockTileStyle, TabBarStyle, panel_handle,
};
use monitter_lib::experimental_desktop::NativeTileTheme;
use std::{cell::{Cell, RefCell}, rc::Rc};

const TAB_STRIP_HEIGHT: f32 = 34.0;
const CONTENT_TOP_TOLERANCE: f32 = 1.0;

struct MeasuredContent {
    id: String,
    top: Rc<Cell<Option<f32>>>,
    height: Option<Rc<Cell<gpui::Pixels>>>,
}

impl Render for MeasuredContent {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let top = self.top.clone();
        let height = self.height.clone();
        let id = self.id.clone();
        div().id(gpui::SharedString::from(id.clone())).debug_selector(move || id.clone()).size_full().child(
            canvas(
                |_, _, _| (),
                move |bounds, _, _, _| {
                    top.set(Some(bounds.origin.y.as_f32()));
                    if let Some(height) = &height {
                        height.set(bounds.size.height);
                    }
                },
            )
            .size_full(),
        )
    }
}

fn native_panel(
    id: &str,
    top: Rc<Cell<Option<f32>>>,
    height: Option<Rc<Cell<gpui::Pixels>>>,
    cx: &mut App,
) -> gpui::Entity<NativePanel> {
    let content = cx.new(|_| MeasuredContent { id: format!("pane-probe-{id}"), top, height });
    cx.new(|cx| NativePanel {
        spec: PaneSpec {
            id: id.to_owned(),
            title: id.to_owned().into(),
            content: content.into(),
            focus: cx.focus_handle(),
            lifecycle: None,
            closable: true,
        },
        paint: Rc::new(RefCell::new(PaintState {
            shown: true,
            ..Default::default()
        })),
        removed: false,
        retaining_layout: Rc::new(Cell::new(false)),
        appearance: Rc::new(Cell::new(Some(crate::pane_appearance::PaneAppearance {
            tile_theme: NativeTileTheme::Monitter, tile_backdrop_color: 0x101010,
            border_opacity: 100,
            show_active_border: true, active_border_color: 0x40aaff,
            dim_inactive: false, inactive_opacity: 0.6, focus_follows_mouse: false,
        }))),
        auto_hide_tabs: Rc::new(Cell::new(false)),
    })
}

#[gpui_kit::test]
fn native_tab_content_starts_at_the_tab_strip_bottom(cx: &mut TestAppContext) {
    cx.update(|cx| gpui_kit::init(cx));

    let first_top = Rc::new(Cell::new(None));
    let second_top = Rc::new(Cell::new(None));
    let first_capture = first_top.clone();
    let second_capture = second_top.clone();
    let mut skin_handle = None;
    let (area, mut cx) = cx.add_window_view(|window, cx| {
        let skin = DockSkin::new(cx);
        skin_handle = Some(skin.clone());
        DockArea::new("pane-header-regression", None, window, cx).with_renderer(skin)
    });
    let skin = skin_handle.expect("DockSkin constructed with its area");
    cx.update(|window, cx| {
        skin.set_tab_bar_style(TabBarStyle::Modern, px(TAB_STRIP_HEIGHT), px(12.), cx);
        let first = native_panel("chat:header-first", first_capture, None, cx);
        let second = native_panel("chat:header-second", second_capture, None, cx);
        window.focus(&first.read(cx).spec.focus.clone(), cx);
        let layout = DockLayout::tabs()
            .panel_view(panel_handle(first), cx)
            .panel_view(panel_handle(second), cx);
        area.update(cx, |area, cx| area.set_center(layout, window, cx));
    });

    cx.simulate_resize(size(px(1200.), px(760.)));
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));

    let content_top = first_top
        .get()
        .expect("the active native pane content should be laid out");
    assert!(
        second_top.get().is_none(),
        "the inactive native pane should not be painted"
    );
    let gap = content_top - TAB_STRIP_HEIGHT;
    assert!(
        gap.abs() <= CONTENT_TOP_TOLERANCE,
        "active content should start at the {TAB_STRIP_HEIGHT}px tab-strip bottom; got y={content_top:.1}px, gap={gap:.1}px"
    );
}

#[gpui_kit::test]
fn inset_tiles_resize_real_content_and_restore_retained_layout(cx: &mut TestAppContext) {
    cx.update(|cx| gpui_kit::init(cx));
    for split in [false, true] {
        let height = Rc::new(Cell::new(px(0.)));
        let mut skin = None;
        let (area, mut cx) = cx.add_window_view(|window, cx| {
            let renderer = DockSkin::new(cx);
            skin = Some(renderer.clone());
            DockArea::new("tile-switch", None, window, cx).with_renderer(renderer)
        });
        let skin = skin.expect("DockSkin constructed with its area");
        let measured = height.clone();
        let retained = cx.update(|window, cx| {
            let probe = native_panel("chat:tile-first", Rc::new(Cell::new(None)), Some(measured), cx);
            area.update(cx, |area, cx| {
                let tile = DockLayout::tabs().panel_view(panel_handle(probe.clone()), cx);
                let layout = if split {
                    let second = native_panel("chat:tile-second", Rc::new(Cell::new(None)), None, cx);
                    DockLayout::h_split().child(tile, None).child(
                        DockLayout::tabs().panel_view(panel_handle(second), cx), None,
                    )
                } else { tile };
                area.set_center(layout, window, cx);
            });
            probe
        });
        cx.simulate_resize(size(px(1200.), px(760.)));
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let original = height.get();
        let layout = cx.read(|cx| area.read(cx).dump(cx));
        let identity = retained.entity_id();
        cx.update(|_, cx| skin.set_tile_style(DockTileStyle::Rounded {
            gutter: px(6.), radius: px(8.), backdrop: gpui::rgb(0x101010).into(),
            border_color: gpui::rgb(0xeeeeee).into(),
        }, cx));
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert!(height.get() > px(0.));
        assert_eq!(height.get(), original - px(12.), "insets must shrink actual pane content");
        cx.update(|_, cx| skin.set_tile_style(DockTileStyle::Contiguous, cx));
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert_eq!(height.get(), original, "default geometry must be restored");
        assert_eq!(retained.entity_id(), identity);
        assert_eq!(cx.read(|cx| area.read(cx).dump(cx)), layout);
    }
}

fn draw_frame(cx: &mut gpui::VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
}

#[gpui_kit::test]
fn tern_pane_controls_hover_only_at_the_corner_and_close_the_owning_pane(cx: &mut TestAppContext) {
    use gpui_kit::component::dock::{DockPlacement, PanelStyle};
    cx.update(gpui_kit::init);
    let mut skin = None;
    let (area, cx) = cx.add_window_view(|window, cx| {
        let renderer = DockSkin::new(cx);
        skin = Some(renderer.clone());
        DockArea::new("tern-corner-controls", None, window, cx).with_renderer(renderer)
    });
    let skin = skin.unwrap();
    let (first, second) = cx.update(|window, cx| {
        skin.set_panel_style(PanelStyle::Hidden, cx);
        let first = native_panel("terminal:corner-a", Rc::new(Cell::new(None)), None, cx);
        let second = native_panel("terminal:corner-b", Rc::new(Cell::new(None)), None, cx);
        area.update(cx, |area, cx| area.set_center(DockLayout::h_split()
            .child(DockLayout::tabs().panel_view(panel_handle(first.clone()), cx), None)
            .child(DockLayout::tabs().panel_view(panel_handle(second.clone()), cx), None), window, cx));
        (first, second)
    });
    cx.simulate_resize(size(px(1000.), px(700.)));
    draw_frame(cx);
    let left = cx.debug_bounds("pane-probe-terminal:corner-a").unwrap();
    let right = cx.debug_bounds("pane-probe-terminal:corner-b").unwrap();
    assert_eq!(left.top(), px(0.), "hidden tabs leave no strip space");
    assert_eq!(right.top(), px(0.));
    assert!(cx.debug_bounds("dock-pane-close-control").is_none(), "idle controls are not painted");
    cx.simulate_mouse_move(left.center(), None, Default::default());
    draw_frame(cx);
    assert!(cx.debug_bounds("dock-pane-close-control").is_none(), "pane-body hover doesn't reveal controls");
    let corner = cx.debug_bounds("dock-pane-corner").unwrap();
    cx.simulate_mouse_move(corner.center(), None, Default::default());
    draw_frame(cx);
    let close = cx.debug_bounds("dock-pane-close-control").expect("corner hover reveals close");
    let move_button = cx.debug_bounds("dock-pane-move-control").expect("corner hover reveals move");
    assert!(move_button.left() < close.left());
    assert!(close.right() <= right.right() && close.top() < right.top() + px(20.));
    cx.update(|window, cx| window.focus(&first.read(cx).spec.focus.clone(), cx));
    draw_frame(cx);
    cx.simulate_click(close.center(), Default::default());
    draw_frame(cx);
    cx.read(|cx| {
        assert!(second.read(cx).removed, "close targets its owner even when another pane has focus");
        assert!(!first.read(cx).removed);
        assert_eq!(area.read(cx).layout(DockPlacement::Center).unwrap().panels().count(), 1);
    });
    cx.update(|_, cx| skin.set_panel_style(PanelStyle::TabBar, cx));
    draw_frame(cx);
    assert!(cx.debug_bounds("dock-pane-corner").is_none(), "Monitter restores ordinary tabs");
    assert!(cx.debug_bounds("pane-probe-terminal:corner-a").unwrap().top() > px(0.));
}

#[gpui_kit::test]
fn tern_move_button_drags_a_real_pane_to_another_split(cx: &mut TestAppContext) {
    use gpui_kit::component::dock::{DockPlacement, PanelStyle};
    use gpui::{MouseButton, Modifiers, point};
    cx.update(gpui_kit::init);
    for center_drop in [false, true] {
    let mut skin = None;
    let (area, cx) = cx.add_window_view(|window, cx| {
        let renderer = DockSkin::new(cx);
        skin = Some(renderer.clone());
        DockArea::new("tern-move", None, window, cx).with_renderer(renderer)
    });
    let keys = cx.update(|window, cx| {
        skin.as_ref().unwrap().set_panel_style(PanelStyle::Hidden, cx);
        let first = native_panel("terminal:move-a", Rc::new(Cell::new(None)), None, cx);
        let second = native_panel("terminal:move-b", Rc::new(Cell::new(None)), None, cx);
        let keys = [gpui_kit::component::dock::PanelId::from(first.entity_id()), gpui_kit::component::dock::PanelId::from(second.entity_id())];
        area.update(cx, |area, cx| area.set_center(DockLayout::h_split()
            .child(DockLayout::tabs().panel_view(panel_handle(first), cx), None)
            .child(DockLayout::tabs().panel_view(panel_handle(second), cx), None), window, cx));
        keys
    });
    cx.simulate_resize(size(px(1000.), px(700.)));
    draw_frame(cx);
    let corner = cx.debug_bounds("dock-pane-corner").unwrap();
    cx.simulate_mouse_move(corner.center(), None, Default::default());
    draw_frame(cx);
    let handle = cx.debug_bounds("dock-pane-move-control").unwrap().center();
    cx.simulate_mouse_down(handle, MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_move(handle + point(px(8.), px(8.)), Some(MouseButton::Left), Modifiers::none());
    let target = point(px(if center_drop { 250. } else { 25. }), px(350.));
    cx.simulate_mouse_move(target, Some(MouseButton::Left), Modifiers::none());
    cx.simulate_mouse_up(target, MouseButton::Left, Modifiers::none());
    draw_frame(cx);
    let expected = if center_drop { keys } else { [keys[1], keys[0]] };
    assert_eq!(cx.read(|cx| area.read(cx).layout(DockPlacement::Center).unwrap().panels().collect::<Vec<_>>()), expected, "move handle uses the dock's actual split drag");
    cx.read(|cx| {
        let area = area.read(cx);
        let groups = super::shape_groups(&super::capture_shape(area.layout(DockPlacement::Center).unwrap().root()));
        assert_eq!(groups.len(), 2, "even center drops keep both tabless panes visible");
        assert!(groups.iter().all(|group| group.panels.len() == 1));
    });
    }
}

#[gpui_kit::test]
fn tern_new_tab_creates_a_vertical_split_and_monitter_keeps_tabs(cx: &mut TestAppContext) {
    use super::{DockWorkspace, PaneFactory};
    use gpui_kit::component::dock::{DockPlacement, PaneRef};
    use gpui::Axis;
    use serde_json::json;
    cx.update(gpui_kit::init);
    for (theme, explicit_split) in [(NativeTileTheme::Monitter, None), (NativeTileTheme::Tern, None),
        (NativeTileTheme::Monitter, Some(true)), (NativeTileTheme::Monitter, Some(false)),
        (NativeTileTheme::Tern, Some(false))] {
        crate::design::set_tile_theme(theme);
        let mut workspace = None;
        let (_, cx) = cx.add_window_view(|window, cx| {
            let factory: PaneFactory = Rc::new(|_, _, _| None);
            let dock = cx.new(|cx| DockWorkspace::new("new-tab-layout", factory, window, cx));
            workspace = Some(dock.clone());
            gpui_kit::component::Root::new(dock, window, cx)
        });
        let workspace = workspace.unwrap();
        cx.update(|window, cx| {
            workspace.update(cx, |dock, cx| {
                let settings = json!({});
                let palette = crate::design::palette(&settings, gpui::WindowAppearance::Dark);
                dock.set_pane_appearance(&settings, palette, cx);
                dock.set_tab_appearance(&settings, 100, cx);
                let first = dock.empty_spec("empty:start", cx).unwrap();
                dock.add(first, DockPlacement::Center, None, window, cx).unwrap();
                dock.focus("empty:start", window, cx);
                let group = dock.focused_group_node(window, cx).unwrap();
                let new = if let Some(vertical) = explicit_split {
                    dock.add_empty_split(group, vertical, window, cx).unwrap()
                } else { dock.add_empty_tab(group, window, cx).unwrap() };
                assert_eq!(dock.focused_pane_id(window, cx).as_deref(), Some(new.as_str()));
                let area = dock.area.read(cx);
                let mut root = area.layout(DockPlacement::Center).unwrap().root();
                while let PaneRef::Split { children, .. } = root.kind() {
                    if children.len() != 1 { break; }
                    root = &children[0];
                }
                match (theme, explicit_split, root.kind()) {
                    (_, Some(_), PaneRef::Split { axis, children, .. }) |
                    (NativeTileTheme::Tern, None, PaneRef::Split { axis, children, .. }) => {
                        let expected = if explicit_split == Some(false) { Axis::Vertical } else { Axis::Horizontal };
                        assert_eq!(axis, expected, "split orientation follows the requested divider");
                        assert_eq!(children.len(), 2);
                        assert!(children.iter().all(|child| matches!(child.kind(), PaneRef::Tabs { panels, .. } if panels.len() == 1)));
                    }
                    (NativeTileTheme::Monitter, None, PaneRef::Tabs { panels, .. }) => assert_eq!(panels.len(), 2),
                    _ => panic!("wrong new-tab layout for {theme:?}, split={explicit_split:?}"),
                }
            });
        });
        draw_frame(cx);
    }
    crate::design::set_tile_theme(NativeTileTheme::Monitter);
}

#[gpui_kit::test]
fn tern_transparency_uses_the_configured_alpha_without_a_fill_under_panes(cx: &mut TestAppContext) {
    use super::{DockWorkspace, PaneFactory};
    use gpui_kit::component::dock::DockAreaRenderer;
    use serde_json::json;
    cx.update(gpui_kit::init);
    crate::design::set_tile_theme(NativeTileTheme::Tern);
    let (_, visual) = cx.add_window_view(|window, cx| {
        let factory: PaneFactory = Rc::new(|_, _, _| None);
        let dock = cx.new(|cx| DockWorkspace::new("tern-transparency", factory, window, cx));
        gpui_kit::component::Root::new(dock, window, cx)
    });
    visual.update(|window, cx| {
        for surface in ["opaque", "glass", "translucent"] {
            let settings = json!({"windowSurface":surface,"windowTransparency":70});
            crate::design::set_surface(&settings);
            let palette = crate::design::palette(&settings, gpui::WindowAppearance::Dark);
            let factory: PaneFactory = Rc::new(|_, _, _| None);
            let dock = cx.new(|cx| DockWorkspace::new("tern-alpha", factory, window, cx));
            dock.update(cx, |dock, cx| {
                dock.set_pane_appearance(&settings, palette, cx);
                let mut frame = dock.skin.frame(window, cx);
                assert!(frame.style().background.is_none(), "Tern must not cover transparent pane bodies with a full-area background");
                let expected = if surface == "opaque" { 1. } else { 77. / 255. };
                assert!((frame.style().border_color.unwrap().a - expected).abs() < 0.002, "gutter/corner backdrop honors the chosen transparency");
            });
        }
    });
    crate::design::set_surface(&json!({"windowSurface":"opaque"}));
    crate::design::set_tile_theme(NativeTileTheme::Monitter);
}
