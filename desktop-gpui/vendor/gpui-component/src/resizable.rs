//! Monitter divider appearance. Base owns the wide grab band and drag state;
//! the visible divider lights along its entire length instead of showing a pill.
use crate::theme::ActiveTheme as _;
use gpui::{
    AnyElement, App, Axis, IntoElement, ParentElement as _, Styled as _, Window, div,
    prelude::FluentBuilder as _, px, rgb,
};
pub use gpui_base::{
    ResizablePanel, ResizablePanelEvent, ResizablePanelGroup, ResizableState, resizable_panel,
};
use gpui_base::{ResizeHandleContext, ResizeHandleRenderer, ResizeHandleState};
use std::rc::Rc;
use std::sync::atomic::{AtomicU32, Ordering};

static SEPARATOR_BLEND_MODE: AtomicU32 = AtomicU32::new(0);
/// Blend appearance of divider lines, independently of panel fills.
pub fn set_separator_blend_mode(mode: u32) { SEPARATOR_BLEND_MODE.store(mode.min(2), Ordering::Relaxed); }


pub fn h_resizable(id: impl Into<gpui::ElementId>) -> ResizablePanelGroup {
    gpui_base::h_resizable(id).with_handle_appearance(resize_handle_appearance())
}
pub fn v_resizable(id: impl Into<gpui::ElementId>) -> ResizablePanelGroup {
    gpui_base::v_resizable(id).with_handle_appearance(resize_handle_appearance())
}
pub fn resize_handle_appearance() -> ResizeHandleRenderer {
    Rc::new(|handle, window, cx| Some(render_resize_handle(handle, window, cx)))
}
pub(crate) fn render_resize_handle(
    handle: &ResizeHandleContext,
    _: &mut Window,
    cx: &mut App,
) -> AnyElement {
    // #00AAF0 is the dominant blue in the bundled Monitter wordmark.
    let engaged = !matches!(handle.state(), ResizeHandleState::Idle);
    div()
        .relative()
        .flex_none()
        .bg({
            let mut line: gpui::Background = cx.theme().border.into();
            line.blend_mode = SEPARATOR_BLEND_MODE.load(Ordering::Relaxed);
            line
        })
        .map(|line| match handle.axis() {
            Axis::Horizontal => line.w(px(1.)).h_full(),
            _ => line.h(px(1.)).w_full(),
        })
        .when(engaged, |line| {
            line.child(
                div()
                    .absolute()
                    .bg(rgb(0x00aaf0))
                    .map(|light| match handle.axis() {
                        Axis::Horizontal => light.top_0().bottom_0().left(px(-1.)).w(px(3.)),
                        _ => light.left_0().right_0().top(px(-1.)).h(px(3.)),
                    }),
            )
        })
        .into_any_element()
}
