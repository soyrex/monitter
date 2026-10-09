//! Regression for real tab drags landing across the modern bar's trailing
//! empty space. The drag and drop pass through GPUI's hit testing and DockArea.
use gpui_kit::component::dock::{
    BasePanel, BasePanelView, DockArea, DockLayout, DockPlacement, DockSkin, Panel, PanelEvent,
    PanelId, PanelInfo, PanelState, TabBarStyle, panel_handle,
};
use gpui_kit::{
    App, AppContext as _, Context, Empty, Entity, EventEmitter, FocusHandle, Focusable,
    IntoElement, Render, TestAppContext, VisualTestContext, Window, div, point, px, size,
};
use gpui_kit::{Modifiers, MouseButton, Styled as _, ParentElement as _};

const PANE_IDS: [&str; 4] = [
    "chat:drag-fixture-a",
    "chat:drag-fixture-b",
    "chat:drag-fixture-c",
    "chat:drag-fixture-d",
];

struct DragTab {
    id: &'static str,
    focus: FocusHandle,
}

impl BasePanel for DragTab {
    fn panel_name(&self) -> &'static str {
        "MonitterNativePane"
    }

    fn dump(&self, _: &App) -> PanelState {
        PanelState {
            panel_name: "MonitterNativePane".into(),
            children: Vec::new(),
            info: PanelInfo::panel(serde_json::json!({ "id": self.id })),
        }
    }
}

impl Panel for DragTab {
    fn tab_name(&self, _: &App) -> Option<gpui_kit::SharedString> {
        Some(self.id.into())
    }

    // Chat moved its toolbar into the content header. With an active panel in
    // that mode the tab bar has no suffix, so its trailing blank strip must be
    // mounted independently for Modern tab dragging to remain available.
    fn toolbar_in_content(&self, _: &App) -> bool {
        true
    }

    fn tab_background(&mut self, _: &mut Window, _: &mut Context<Self>) -> Option<gpui_kit::AnyElement> {
        Some(div().absolute().size_full().bg(gpui_kit::rgba(0x2587d020)).into_any_element())
    }

    fn title(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        format!("{} — long native tab title for drag coverage", self.id)
    }
}

impl EventEmitter<PanelEvent> for DragTab {}

impl Focusable for DragTab {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for DragTab {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(Empty)
    }
}

fn setup(cx: &mut TestAppContext) -> (Entity<DockArea>, Vec<PanelId>, &mut VisualTestContext) {
    cx.update(gpui_kit::init);
    let mut skin_handle = None;
    let (area, cx) = cx.add_window_view(|window, cx| {
        let skin = DockSkin::new(cx);
        skin_handle = Some(skin.clone());
        DockArea::new("tab-drag-regression", None, window, cx).with_renderer(skin)
    });
    let skin = skin_handle.expect("DockSkin constructed with its area");
    cx.simulate_resize(size(px(2600.), px(720.)));

    let ids = cx.update(|window, cx| {
        skin.set_tab_bar_style(TabBarStyle::Modern, px(34.), px(12.), cx);
        skin.set_tab_label_geometry(px(280.), px(14.), cx);

        let handles: Vec<_> = PANE_IDS
            .iter()
            .map(|id| {
                panel_handle(cx.new(|cx| DragTab {
                    id: *id,
                    focus: cx.focus_handle(),
                }))
            })
            .collect();
        let ids = handles.iter().map(|panel| panel.panel_id(cx)).collect();
        let layout = handles
            .into_iter()
            .fold(DockLayout::tabs(), |layout, panel| {
                layout.panel_view(panel, cx)
            });
        area.update(cx, |area, cx| area.set_center(layout, window, cx));
        ids
    });
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    (area, ids, cx)
}

fn drag_first_tab_to_empty_space(cx: &mut VisualTestContext, target_x: f32) {
    let start = point(px(120.), px(17.));
    cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_move(
        point(px(126.), px(17.)),
        Some(MouseButton::Left),
        Modifiers::none(),
    );
    let target = point(px(target_x), px(17.));
    cx.simulate_mouse_move(target, Some(MouseButton::Left), Modifiers::none());
    cx.simulate_mouse_up(target, MouseButton::Left, Modifiers::none());
    cx.run_until_parked();
}

fn persisted_native_ids(state: &PanelState, ids: &mut Vec<String>) {
    match &state.info {
        PanelInfo::Panel(info) => {
            assert_eq!(state.panel_name, "MonitterNativePane");
            assert!(state.children.is_empty());
            let id = info["id"]
                .as_str()
                .expect("native pane dump has a string id");
            assert!(!id.is_empty() && id.len() <= 128);
            assert!(
                id.bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"-_:./".contains(&byte))
            );
            assert!(
                !ids.iter().any(|existing| existing == id),
                "duplicate pane id {id}"
            );
            ids.push(id.to_owned());
        }
        PanelInfo::Tabs { .. } => {
            assert_eq!(state.panel_name, "TabPanel");
            for child in &state.children {
                persisted_native_ids(child, ids);
            }
        }
        PanelInfo::Stack { axis, .. } => {
            assert_eq!(state.panel_name, "StackPanel");
            assert!(*axis <= 1);
            for child in &state.children {
                persisted_native_ids(child, ids);
            }
        }
    }
}

#[gpui_kit::test]
fn modern_drag_reaches_both_empty_space_ends_with_toolbar_in_content(
    cx: &mut TestAppContext,
) {
    let (area, panel_ids, cx) = setup(cx);
    let original = cx.update(|_, cx| {
        area.read(cx)
            .layout(DockPlacement::Center)
            .expect("center layout")
            .panels()
            .collect::<Vec<_>>()
    });
    assert_eq!(original, panel_ids);

    // Four max-width labels end near x=1200 in this 2600px window. With
    // toolbar_in_content enabled, there is no suffix to host the trailing
    // strip. Both positions still have to remain native DragPanel drop targets.
    drag_first_tab_to_empty_space(cx, 1320.);
    let after_near = cx.update(|_, cx| {
        area.read(cx)
            .layout(DockPlacement::Center)
            .expect("center layout")
            .panels()
            .collect::<Vec<_>>()
    });
    assert_eq!(
        after_near,
        vec![panel_ids[1], panel_ids[2], panel_ids[3], panel_ids[0]]
    );

    let near_dump = cx.update(|_, cx| area.read(cx).dump(cx));
    let mut near_persisted_ids = Vec::new();
    persisted_native_ids(&near_dump.center, &mut near_persisted_ids);
    assert_eq!(
        near_persisted_ids,
        vec![PANE_IDS[1], PANE_IDS[2], PANE_IDS[3], PANE_IDS[0]]
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>()
    );

    drag_first_tab_to_empty_space(cx, 2480.);
    let after_far = cx.update(|_, cx| {
        area.read(cx)
            .layout(DockPlacement::Center)
            .expect("center layout")
            .panels()
            .collect::<Vec<_>>()
    });
    assert_eq!(
        after_far,
        vec![panel_ids[2], panel_ids[3], panel_ids[0], panel_ids[1]]
    );

    let far_dump = cx.update(|_, cx| area.read(cx).dump(cx));
    let mut far_persisted_ids = Vec::new();
    persisted_native_ids(&far_dump.center, &mut far_persisted_ids);
    assert_eq!(
        far_persisted_ids,
        vec![PANE_IDS[2], PANE_IDS[3], PANE_IDS[0], PANE_IDS[1]]
    );
}
