//! Release fallback for native pane/tab drags whose mouse-up occurred outside
//! the window. No synthetic input is sent into agent/terminal content.
use gpui_kit::*;
pub(crate) struct PointerReleaseGuard;
impl IntoElement for PointerReleaseGuard {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}
impl Element for PointerReleaseGuard {
    type RequestLayoutState = ();
    type PrepaintState = ();
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
    ) -> (LayoutId, ()) {
        (window.request_layout(Style::default(), None, cx), ())
    }
    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        _: &mut Window,
        _: &mut App,
    ) {
    }
    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        _: &mut App,
    ) {
        window.on_mouse_event(|event: &MouseMoveEvent, phase, window, cx| {
            if phase.capture() && event.pressed_button.is_none() {
                cx.stop_active_drag(window);
            }
        });
    }
}
