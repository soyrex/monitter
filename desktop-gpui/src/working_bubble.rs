//! Animated run controls live beside cached ChatPane, never beneath it.
use crate::{
    activity_sparkles::PaintBounds,
    design::{self, ui_scaled_px as px},
};
use gpui_kit::{
    assets::IconName,
    component::{Disableable, Icon, Sizable, button::*, spinner::Spinner},
    prelude::*,
    *,
};

pub(crate) enum WorkingBubbleEvent {
    Stop,
}
pub(crate) struct WorkingBubble {
    composer: PaintBounds,
    chat_column: PaintBounds,
    palette: design::Palette,
    visible: bool,
    starting: bool,
    busy: bool,
}
impl EventEmitter<WorkingBubbleEvent> for WorkingBubble {}
impl WorkingBubble {
    pub(crate) fn new(composer: PaintBounds, chat_column: PaintBounds, palette: design::Palette) -> Self {
        Self {
            composer,
            chat_column,
            palette,
            visible: false,
            starting: false,
            busy: false,
        }
    }
    pub(crate) fn set_context(
        &mut self,
        visible: bool,
        starting: bool,
        busy: bool,
        palette: design::Palette,
        cx: &mut Context<Self>,
    ) {
        if (self.visible, self.starting, self.busy, self.palette)
            != (visible, starting, busy, palette)
        {
            self.visible = visible;
            self.starting = starting;
            self.busy = busy;
            self.palette = palette;
            cx.notify();
        }
    }
}
impl Render for WorkingBubble {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.visible {
            return div().into_any_element();
        }
        let p = self.palette;
        ComposerAnchor {
            composer: self.composer.clone(),
            chat_column: self.chat_column.clone(),
            clearance: px(8.),
            flight: 0.,
            child: div()
                .id("working-bubble")
                .flex()
                .items_center()
                .gap(px(5.))
                .px(px(7.))
                .py(px(3.))
                .rounded_full()
                .border_1()
                .border_color(rgb(p.line))
                .bg(crate::surface_blend::floating_control_background(p.panel))
                .text_color(rgb(p.ink))
                .text_size(px(11.))
                .child(Spinner::new().xsmall().color(rgb(p.accent).into()))
                .child(if self.starting {
                    "Starting…"
                } else {
                    "Working…"
                })
                .child(
                    Button::new("working-stop")
                        .compact()
                        .xsmall()
                        .ghost()
                        .w(px(18.))
                        .h(px(18.))
                        .opacity(0.5)
                        .icon(Icon::new(IconName::Square))
                        .tooltip("Stop run")
                        .disabled(self.busy || self.starting)
                        .accessibility_label("Stop run")
                        .on_click(cx.listener(|_, _, _, cx| cx.emit(WorkingBubbleEvent::Stop))),
                )
                .into_any_element(),
        }
        .into_any_element()
    }
}
/// Anchor above the measured goal/Git/composer stack after cached geometry
/// replays. Clip paint and hit boxes to the chat column beside the sidebar.
pub(crate) struct ComposerAnchor {
    composer: PaintBounds,
    chat_column: PaintBounds,
    child: AnyElement,
    clearance: Pixels,
    flight: f32,
}
impl ComposerAnchor {
    pub(crate) fn new(composer: PaintBounds, chat_column: PaintBounds, child: AnyElement, clearance: Pixels) -> Self {
        Self { composer, chat_column, child, clearance, flight: 0. }
    }
    pub(crate) fn fly_down(mut self, progress: f32) -> Self {
        self.flight = progress.clamp(0., 1.);
        self
    }
}
impl IntoElement for ComposerAnchor {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}
impl Element for ComposerAnchor {
    type RequestLayoutState = LayoutId;
    type PrepaintState = bool;
    fn id(&self) -> Option<ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }
    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, LayoutId) {
        let child = self.child.request_layout(window, cx);
        let mut style = Style::default();
        style.position = Position::Absolute;
        (window.request_layout(style, [child], cx), child)
    }
    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        child: &mut LayoutId,
        window: &mut Window,
        cx: &mut App,
    ) -> bool {
        let (Some(composer), Some(column)) = (self.composer.get(), self.chat_column.get()) else {
            return false;
        };
        let child_bounds = window.layout_bounds(*child);
        let size = child_bounds.size;
        if size.width > column.size.width || size.height > column.size.height {
            return false;
        }
        let mut origin = point(
            (composer.origin.x + (composer.size.width - size.width) * 0.5)
                .max(column.origin.x)
                .min(column.origin.x + column.size.width - size.width),
            (composer.origin.y - size.height - self.clearance)
                .min(column.origin.y + column.size.height - size.height - px(8.)),
        );
        // Travel beyond the pane's bottom; its mask clips paint and hit targets.
        origin.y += (column.bottom() + size.height + px(8.) - origin.y) * self.flight;
        // The child already has a layout origin inside its absolute wrapper.
        // Move by the delta, as GPUI Anchored does; prepaint_at would add the
        // existing layout origin a second time and push this below the pane.
        window.with_content_mask(Some(ContentMask::new(column)), |window| {
            window.with_element_offset(origin - child_bounds.origin, |window| {
                self.child.prepaint(window, cx);
            });
        });
        true
    }
    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut LayoutId,
        painted: &mut bool,
        window: &mut Window,
        cx: &mut App,
    ) {
        if *painted {
            if let Some(column) = self.chat_column.get() {
                window.with_content_mask(Some(ContentMask::new(column)), |window| {
                    self.child.paint(window, cx);
                });
            }
        }
    }
}
