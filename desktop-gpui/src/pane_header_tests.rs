//! The content of a native pane should begin at the bottom of its dock tab
//! strip, without the extra eight design pixels from the generic panel inset.
use super::{NativePanel, PaintState, PaneSpec};
use gpui::{
    App, AppContext as _, Context, IntoElement, Render, ParentElement as _, Styled as _, TestAppContext, Window, canvas, div, px,
    size,
};
use gpui_kit::component::dock::{
    DockArea, DockLayout, DockSkin, TabBarStyle, panel_handle,
};
use std::{cell::{Cell, RefCell}, rc::Rc};

const TAB_STRIP_HEIGHT: f32 = 34.0;
const CONTENT_TOP_TOLERANCE: f32 = 1.0;

struct MeasuredContent {
    top: Rc<Cell<Option<f32>>>,
}

impl Render for MeasuredContent {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let top = self.top.clone();
        div().size_full().child(
            canvas(
                |_, _, _| (),
                move |bounds, _, _, _| top.set(Some(bounds.origin.y.as_f32())),
            )
            .size_full(),
        )
    }
}

fn native_panel(
    id: &str,
    top: Rc<Cell<Option<f32>>>,
    cx: &mut App,
) -> gpui::Entity<NativePanel> {
    let content = cx.new(|_| MeasuredContent { top });
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
        appearance: Rc::new(Cell::new(Some(crate::pane_appearance::PaneAppearance {
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
        let first = native_panel("chat:header-first", first_capture, cx);
        let second = native_panel("chat:header-second", second_capture, cx);
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
