//! Prepared, bounded transcript overview. Scroll callbacks never walk messages.
use super::*;
use gpui_kit::component::{hover_card::HoverCard};

pub(super) const MINIMAP_WIDTH: f32 = 16.;
const PROXIMITY_DISTANCE: f32 = 64.;
const PREVIEW_SCALE: f32 = 0.8;
const PREVIEW_OPACITY: f32 = 0.8;
#[derive(Default)]
pub(super) struct Emphasis {
    value: f32,
    pub(super) target: f32,
    transition: Option<(std::time::Instant, f32)>,
}
impl Emphasis {
    pub(super) fn set_target(&mut self, target: f32) -> bool {
        if (self.target - target).abs() < 0.01 { return false; }
        let now = std::time::Instant::now();
        let from = self.advance(now);
        self.target = target;
        self.transition = Some((now, from));
        true
    }
    pub(super) fn advance(&mut self, now: std::time::Instant) -> f32 {
        if !crate::native_motion::motion_allowed() {
            self.transition = None;
            self.value = self.target;
        } else if let Some((start, from)) = self.transition {
            let t = (now.saturating_duration_since(start).as_secs_f32() / 0.15).min(1.);
            self.value = from + (self.target - from) * (1. - (1. - t).powi(2));
            if t == 1. { self.transition = None; }
        }
        self.value
    }
    pub(super) fn animating(&self) -> bool { self.transition.is_some() }
}
fn proximity(position: Point<Pixels>, bounds: Bounds<Pixels>) -> f32 {
    let dx = f32::from((bounds.left() - position.x).max(position.x - bounds.right()).max(gpui_kit::px(0.)));
    let dy = f32::from((bounds.top() - position.y).max(position.y - bounds.bottom()).max(gpui_kit::px(0.)));
    (1. - dx.hypot(dy) / PROXIMITY_DISTANCE).clamp(0., 1.)
}
fn marker_opacity(bubbled: bool, emphasis: f32) -> f32 {
    let resting = if bubbled { 0.15 } else { 0.05 };
    resting + (0.50 - resting) * emphasis
}
const MAX_MARKERS: usize = 192;
pub(super) struct MarkerJump {
    pub(super) target: usize,
    tween: crate::composer_scroll::BottomJump,
}
impl MarkerJump {
    pub(super) fn new(target: usize) -> Self { Self { target, tween: crate::composer_scroll::BottomJump::new(std::time::Instant::now()) } }
    pub(super) fn advance(&mut self, state: &ListState) -> bool {
        let (ratio, done) = self.tween.step(std::time::Instant::now());
        if done { state.scroll_to(gpui_kit::ListOffset { item_ix: self.target, offset_in_item: gpui_kit::px(0.) }); return true; }
        if let Some(target) = state.bounds_for_item(self.target) {
            let distance = target.top() - state.viewport_bounds().top();
            state.scroll_by(distance * (1. - ratio));
        } else {
            let logical = state.logical_scroll_top();
            let height = state.bounds_for_item(logical.item_ix).map(|bounds| bounds.size.height).unwrap_or(gpui_kit::px(80.)).max(gpui_kit::px(1.));
            let current = logical.item_ix as f32 + f32::from(logical.offset_in_item / height);
            let next = self.target as f32 + (current - self.target as f32) * ratio;
            let row = next.floor().max(0.) as usize;
            let height = state.bounds_for_item(row).map(|bounds| bounds.size.height).unwrap_or(height);
            state.scroll_to(gpui_kit::ListOffset { item_ix: row, offset_in_item: height * next.fract() });
        }
        false
    }
}
#[derive(Clone)]
pub(super) struct Marker {
    row: usize,
    position: f32,
    weight: f32,
    color: u32,
    bubbled: bool,
}
fn bubble_height(chars: usize) -> f32 {
    (3. + (chars as f32 / 160.).sqrt()).min(16.)
}

pub(super) fn prepare<'a>(rows: impl IntoIterator<Item = &'a transcript::ConversationItem>, palette: design::Palette) -> Vec<Marker> {
    let mut markers = Vec::new();
    let mut extent = 0.;
    for (row, item) in rows.into_iter().enumerate() {
        if let transcript::ConversationItem::ExchangeGroup{..}=item {
            markers.push(Marker{row,position:extent,weight:3.,color:palette.accent,bubbled:false});extent+=1.;continue;
        }
        let transcript::ConversationItem::Message(message) = item else { extent += 1.; continue; };
        let role = field(message, "role");
        let text = message["text"].as_str().unwrap_or_default();
        let chars = text.chars().count();
        let weight = 1.;
        if role == "user" || role == "assistant" {
            let bubbled = role == "user" || field(message, "phase") == "final_answer";
            markers.push(Marker { row, position: extent, weight: bubble_height(chars),
                color: if role == "user" { palette.accent } else if bubbled { FINAL_ANSWER_GREEN } else { palette.ink },
                bubbled });
        }
        extent += weight;
    }
    for marker in &mut markers { marker.position /= extent.max(1.); }
    // Dense histories retain a representative marker per band, preferring
    // bubbles over intermediate assistant output. Work is bounded during scroll.
    let mut bands: Vec<Option<Marker>> = vec![None; MAX_MARKERS];
    for marker in markers {
        let band = ((marker.position * MAX_MARKERS as f32) as usize).min(MAX_MARKERS - 1);
        if bands[band].as_ref().is_none_or(|old| !old.bubbled || marker.bubbled) {
            bands[band] = Some(marker);
        }
    }
    bands.into_iter().flatten().collect()
}

pub(super) fn render(state: ListState, markers: Rc<Vec<Marker>>, rows: StableRowRenderer, geometry: TranscriptGeometry, palette: design::Palette, owner: WeakEntity<TranscriptViewport>, window: &mut Window, cx: &mut App, exit: Option<f32>, emphasis: f32, bottom_inset: Pixels) -> AnyElement {
    let bounds = state.viewport_bounds();
    let row_count = state.item_count().max(1);
    let viewport_height = f32::from(bounds.size.height).max(1.);
    // Composer hiding expands the transcript allocation, but not its overview.
    let height = (viewport_height - f32::from(bottom_inset)).max(1.);
    let logical = state.logical_scroll_top();
    let item_height = state.bounds_for_item(logical.item_ix).map(|bounds| f32::from(bounds.size.height)).unwrap_or(80.).max(1.);
    let within_item = (f32::from(logical.offset_in_item) / item_height).clamp(0., 1.);
    let thumb_fraction = ((logical.item_ix as f32 + within_item) / row_count as f32).clamp(0., 1.);
    let thumb_height = ((viewport_height / item_height / row_count as f32) * height).max(12.).min(height);
    let click_state = state.clone();
    let move_state = state.clone();
    let release_state = state.clone();
    let down_owner = owner.clone();
    let move_owner = owner.clone();
    let up_owner = owner.clone();
    let hover_owner = owner.clone();
    let drag_bounds = Rc::new(std::cell::Cell::new(bounds));
    let write_bounds = drag_bounds.clone();
    let global_bounds = drag_bounds.clone();
    let input = canvas(move |bounds, _, _| { write_bounds.set(bounds); }, move |_, _, window, _cx| {
    window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
        if phase.bubble() {
            let target = proximity(event.position, global_bounds.get());
            let _ = hover_owner.update(cx, |view, cx| {
                if view.minimap_emphasis.set_target(target) { cx.notify(); }
            });
        }
        if phase.bubble() && move_state.is_scrollbar_dragging() && event.pressed_button == Some(MouseButton::Left) {
            let bounds = global_bounds.get();
            let fraction = f32::from((event.position.y - bounds.origin.y) / bounds.size.height).clamp(0., 1.);
            move_state.scroll_to(gpui_kit::ListOffset { item_ix: (fraction * row_count as f32) as usize, offset_in_item: gpui_kit::px(0.) });
            let _ = move_owner.update(cx, |view, cx| view.minimap_navigated(fraction >= 0.999, cx));
            cx.stop_propagation();
        }
    });
    window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
        if phase.bubble() && event.button == MouseButton::Left && release_state.is_scrollbar_dragging() {
            release_state.scrollbar_drag_ended();
            let _ = up_owner.update(cx, |view, cx| view.repaint_scroll(cx));
            cx.stop_propagation();
        }
    });
    }).absolute().inset_0();
    let mut rail = div().id("chat-minimap").absolute().right(gpui_kit::px(0.)).top_0().bottom(bottom_inset)
        .w(gpui_kit::px(MINIMAP_WIDTH)).cursor_pointer().occlude()
        .debug_selector(|| "chat-minimap".into())
        .child(input)
        .on_mouse_down(MouseButton::Left, move |event, _, cx| {
            click_state.scrollbar_drag_started();
            let bounds = drag_bounds.get();
            let fraction = f32::from((event.position.y - bounds.origin.y) / bounds.size.height).clamp(0., 1.);
            click_state.scroll_to(gpui_kit::ListOffset { item_ix: (fraction * row_count as f32) as usize, offset_in_item: gpui_kit::px(0.) });
            let _ = down_owner.update(cx, |view, cx| view.minimap_navigated(fraction >= 0.999, cx));
            cx.stop_propagation();
        })
        .child(div().absolute().left_0().w_full().top(relative(thumb_fraction)).mt(gpui_kit::px(-thumb_fraction * thumb_height)).h(relative(thumb_height / height)).min_h(gpui_kit::px(12.))
            .rounded_sm().bg(rgb(palette.muted)).opacity(marker_opacity(true, emphasis)));
    for (index, marker) in markers.iter().enumerate() {
        let preview_rows = rows.clone();
        let row = marker.row;
        let jump_owner = owner.clone();
        let marker_height = if marker.bubbled { marker.weight } else { 1. };
        let mark = div().id(SharedString::from(format!("chat-minimap-{index}")))
            .debug_selector(move || format!("chat-minimap-marker-{index}"))
            .absolute().right_0()
            .when(marker.bubbled, |mark| mark.w_full())
            .when(!marker.bubbled, |mark| mark.w(gpui_kit::px(8.)))
            .top(relative(marker.position)).mt(gpui_kit::px(-marker.position * marker_height))
            .h(gpui_kit::px(marker_height)).bg(rgb(marker.color)).opacity(marker_opacity(marker.bubbled, emphasis))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(move |_, _, cx| { let _ = jump_owner.update(cx, |view, cx| view.minimap_marker_jump(row, cx)); cx.stop_propagation(); });
        if marker.bubbled {
            let opening = window.use_keyed_state(SharedString::from(format!("minimap-preview-opening-{index}")), cx, |_, _| 0usize);
            let next_opening = opening.clone();
            rail = rail.child(div().id(SharedString::from(format!("chat-minimap-bubble-surface-{index}")))
                .absolute().right_0().w(gpui_kit::px(MINIMAP_WIDTH)).top(relative(marker.position))
                .mt(gpui_kit::px(-marker.position * marker_height)).h(gpui_kit::px(marker_height))
                .hover(move |style| style.w(gpui_kit::px(MINIMAP_WIDTH + 8.)).h(gpui_kit::px(marker_height + 4.))
                    .mt(gpui_kit::px(-marker.position * marker_height - 2.)))
                .child(HoverCard::new(SharedString::from(format!("chat-minimap-hover-{index}")))
                .anchor(Anchor::RightCenter).appearance(false)
                .open_delay(Duration::from_millis(500)).close_delay(Duration::from_millis(250))
                .on_open_change(move |open, _, cx| {
                    if *open { next_opening.update(cx, |generation, _| *generation += 1); }
                })
                .trigger(mark.relative().top_0().mt(gpui_kit::px(0.)).hover(move |style| style.h(gpui_kit::px(marker_height + 4.))))
                .content(move |_, window, cx| {
                    let scroll = window.use_keyed_state(SharedString::from(format!("minimap-preview-scroll-{index}")), cx, |_, _| ScrollHandle::new()).read(cx).clone();
                    // Render the representative bubble itself, rather than flattening
                    // a dense marker's messages into an unrelated plain-text card.
                    let width = geometry.attachment_width() / geometry.bubble_fraction() * PREVIEW_SCALE;
                    let bubble = preview_rows.borrow_mut()(row, window, cx, PREVIEW_SCALE);
                    let preview = div().id("chat-minimap-preview").debug_selector(|| "chat-minimap-preview".into())
                        .w(px(width)).max_h(px(280.)).overflow_y_scroll().track_scroll(&scroll)
                        .relative().opacity(PREVIEW_OPACITY)
                        .child(PreviewRemScale { child: bubble, rem_size: window.rem_size() * PREVIEW_SCALE });
                    let preview = if crate::native_motion::motion_allowed() {
                        preview.with_animation(
                            SharedString::from(format!("minimap-preview-enter-{index}-{}", opening.read(cx))),
                            Animation::new(Duration::from_millis(180)).with_easing(|t| 1. - (1. - t).powi(3)),
                            |view, progress| view.left(px(12. * (1. - progress))).opacity(PREVIEW_OPACITY * progress),
                        ).into_any_element()
                    } else { preview.into_any_element() };
                    // Reserve the final gap and the slide's travel outside the card.
                    div().p(px(12.)).child(preview)
                })));
            continue;
        }
        rail = rail.child(mark);
    }
    if let Some(progress) = exit { return rail.right(gpui_kit::px(-MINIMAP_WIDTH * progress)).opacity(1. - progress).into_any_element(); }
    if !crate::native_motion::motion_allowed() { return rail.into_any_element(); }
    rail.with_animation("chat-minimap-slide", Animation::new(Duration::from_millis(150)), |rail, progress| rail.right(gpui_kit::px(-MINIMAP_WIDTH * (1. - progress))).opacity(progress)).into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[::core::prelude::v1::test]
    fn proximity_limits_opacity_without_brightening_distant_notches() {
        let bounds = Bounds::new(point(gpui_kit::px(100.), gpui_kit::px(0.)), size(gpui_kit::px(16.), gpui_kit::px(200.)));
        let far = proximity(point(gpui_kit::px(36.), gpui_kit::px(100.)), bounds);
        let near = proximity(point(gpui_kit::px(68.), gpui_kit::px(100.)), bounds);
        let inside = proximity(bounds.center(), bounds);
        assert_eq!((far, near, inside), (0., 0.5, 1.));
        assert_eq!(marker_opacity(true, far), 0.15);
        assert_eq!(marker_opacity(false, far), 0.05);
        assert_eq!(marker_opacity(true, inside), 0.50);
        assert_eq!(marker_opacity(false, inside), 0.50);
        assert!(marker_opacity(false, near) > 0.05 && marker_opacity(false, near) < 0.50);
    }
    fn palette() -> design::Palette {
        design::palette(&json!({}), WindowAppearance::Dark)
    }
    #[::core::prelude::v1::test]
    fn classification_and_navigation_keep_projection_indices() {
        use transcript::ConversationItem::Message;
        let rows = vec![
            Message(Rc::new(json!({"role":"system","text":"skip"}))),
            Message(Rc::new(json!({"role":"user","text":"hello"}))),
            Message(Rc::new(json!({"role":"assistant","text":"working"}))),
            Message(Rc::new(json!({"role":"assistant","phase":"final_answer","text":"done"}))),
        ];
        let markers = prepare(&rows, palette());
        assert_eq!(markers.iter().map(|m| m.row).collect::<Vec<_>>(), vec![1, 2, 3]);
        assert!(markers[0].bubbled && !markers[1].bubbled && markers[2].bubbled);
        assert_eq!(markers[0].color, palette().accent);
        assert_eq!(markers[2].color, FINAL_ANSWER_GREEN);
        assert!(markers.windows(2).all(|m| m[0].position < m[1].position));
    }
    #[::core::prelude::v1::test]
    fn dense_history_is_bounded_and_retains_a_final_bubble() {
        let rows = (0..10_000).map(|i| transcript::ConversationItem::Message(Rc::new(json!({
            "role":"assistant", "text":"message", "phase":if i == 9999 { "final_answer" } else { "commentary" }
        })))).collect::<Vec<_>>();
        let markers = prepare(&rows, palette());
        assert!(markers.len() <= MAX_MARKERS);
        assert!(markers.iter().all(|marker| marker.row < rows.len()));
        assert!(markers.last().unwrap().bubbled);
        assert_eq!(markers.last().unwrap().row, 9999);
    }
    #[::core::prelude::v1::test]
    fn longer_bubbles_grow_with_a_three_pixel_minimum() {
        assert!(bubble_height(0) >= 3.);
        assert!(bubble_height(2000) > bubble_height(20));
        assert_eq!(bubble_height(100_000), 16.);
    }
}

// Markdown and utility spacing use rems; apply the same preview scale throughout
// all three element phases so wrapping and painted metrics agree.
struct PreviewRemScale { child: AnyElement, rem_size: Pixels }
impl IntoElement for PreviewRemScale {
    type Element = Self;
    fn into_element(self) -> Self { self }
}
impl Element for PreviewRemScale {
    type RequestLayoutState = ();
    type PrepaintState = ();
    fn id(&self) -> Option<ElementId> { None }
    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> { None }
    fn request_layout(&mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>, window: &mut Window, cx: &mut App) -> (LayoutId, ()) {
        (window.with_rem_size(Some(self.rem_size), |window| self.child.request_layout(window, cx)), ())
    }
    fn prepaint(&mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>, _: Bounds<Pixels>, _: &mut (), window: &mut Window, cx: &mut App) {
        window.with_rem_size(Some(self.rem_size), |window| self.child.prepaint(window, cx));
    }
    fn paint(&mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>, _: Bounds<Pixels>, _: &mut (), _: &mut (), window: &mut Window, cx: &mut App) {
        window.with_rem_size(Some(self.rem_size), |window| self.child.paint(window, cx));
    }
}
