//! Native, paint-driven voice conversation stage. Controls live in its owner.
use crate::design::{self, Palette};
use gpui_kit::{prelude::*, *};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum VoiceStageState { Listening, Thinking, Speaking, Paused }

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct VoiceTranscriptLine {
    pub text: String,
    pub is_user: bool,
    pub partial: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct VoiceStageProps {
    pub state: VoiceStageState,
    pub status: String,
    pub agent_name: String,
    pub accent: u32,
    pub elapsed_seconds: u64,
    pub input_level: f32,
    pub output_level: f32,
    pub lines: Vec<VoiceTranscriptLine>,
    pub activity: Option<String>,
}

/// Retained independently so the orb's frame callback invalidates only this stage.
pub(crate) struct VoiceStage {
    props: VoiceStageProps,
    palette: Palette,
}

impl VoiceStage {
    pub(crate) fn new(props: VoiceStageProps, palette: Palette) -> Self {
        Self { props, palette }
    }

    pub(crate) fn set(&mut self, props: VoiceStageProps, palette: Palette, cx: &mut Context<Self>) {
        if self.props != props || self.palette != palette {
            self.props = props;
            self.palette = palette;
            cx.notify();
        }
    }
}

impl Render for VoiceStage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        render_voice_stage(self.props.clone(), self.palette, window, cx)
    }
}

pub(crate) fn render_voice_stage(props: VoiceStageProps, palette: Palette, _window: &mut Window, cx: &mut App) -> AnyElement {
    let motion = !cx.reduce_motion() && crate::native_motion::motion_allowed() && props.state != VoiceStageState::Paused;
    let state = props.state;
    let color = match state { VoiceStageState::Listening => 0x26bdf2, VoiceStageState::Thinking => 0x527b9b, VoiceStageState::Speaking => props.accent, VoiceStageState::Paused => palette.muted };
    let label_color = palette.ink;
    let level = match state { VoiceStageState::Listening => props.input_level, VoiceStageState::Speaking => props.output_level, _ => 0. }.clamp(0., 1.);
    let orb = canvas(|bounds, _, _| bounds, move |_, bounds, window, cx| {
        if !bounds.intersects(&window.content_mask().bounds) { return; }
        let width = f32::from(bounds.size.width);
        let height = f32::from(bounds.size.height);
        let radius = (width * 0.32).min(height * 0.34).min(92.).max(0.);
        let x = f32::from(bounds.origin.x) + width / 2.;
        let y = f32::from(bounds.origin.y) + height / 2.;
        let phase = if motion { { let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default(); (now.as_secs() % 1000) as f32 + now.subsec_nanos() as f32 / 1_000_000_000. } } else { 0. };
        let pulse = if motion { (phase * 1.8).sin() * 0.018 + level * 0.045 } else { 0. };
        let r = radius * (1. + pulse);
        let highlight_angle = if motion { phase * 0.30 } else { 0. };
        let highlight_x = -0.24 + highlight_angle.sin() * 0.035;
        let highlight_y = -0.25 + highlight_angle.cos() * 0.025;
        for (scale, alpha) in [(1.48, 8), (1.33, 14), (1.18, 23)] {
            circle(window, x, y, r * scale * (1. + if motion { level * 0.045 } else { 0. }), color, alpha, false);
        }
        // Nested translucent discs approximate a radial gradient without an image asset.
        circle(window, x, y, r, mix(color, 0x071526, 0.56), 255, true);
        for i in (0..25).rev() {
            let t = i as f32 / 25.;
            circle(window, x + r * highlight_x * (1. - t), y + r * highlight_y * (1. - t), r * (0.12 + 0.86 * t), mix(color, 0xffffff, 0.12 + 0.80 * (1. - t)), 22, true);
        }
        if state == VoiceStageState::Thinking {
            let angle = phase * 1.4;
            circle(window, x + angle.cos() * r * 1.16, y + angle.sin() * r * 1.16, 2.4, palette.ink, 190, true);
        }
        if motion {
            let owner = window.current_view();
            window.on_next_frame(move |_, cx| { if !cx.reduce_motion() && crate::native_motion::motion_allowed() { cx.notify(owner); } });
            window.request_animation_frame();
        }
    }).w_full().flex_1().min_h_0().max_h(design::ui_scaled_px(250.));
    let mut transcript = div().w_full().max_w(design::ui_scaled_px(520.)).mx_auto().flex().flex_col().gap(design::ui_scaled_px(12.));
    let count = props.lines.len();
    for (i, line) in props.lines.into_iter().enumerate().skip(count.saturating_sub(4)) {
        let older = i + 2 < count;
        let label = if line.is_user { "YOU".to_owned() } else { props.agent_name.to_uppercase() };
        let ink = if older { palette.muted } else if line.is_user { palette.ink } else { design::mix_rgb(palette.ink, palette.muted, 0.25) };
        let text: SharedString = if line.partial { format!("{}▏", line.text).into() } else { line.text.into() };
        let partial_start = if line.is_user && line.partial {
            text.match_indices(char::is_whitespace).rev().nth(1).map(|(i, ch)| i + ch.len()).unwrap_or(0)
        } else { text.len() };
        let styled = StyledText::new(text.clone()).with_highlights(if partial_start < text.len() {
            vec![(partial_start..text.len(), HighlightStyle { color: Some(rgb(palette.muted).into()), ..Default::default() })]
        } else { Vec::new() });
        transcript = transcript.child(div().flex().flex_col().gap(design::ui_scaled_px(3.))
            .child(div().text_size(design::ui_scaled_px(10.)).text_color(rgb(if line.is_user { palette.muted } else { palette.ink })).child(label))
            .child(div().text_size(design::ui_scaled_px(if line.is_user && !older { 17. } else { 15. })).line_height(design::ui_scaled_px(if line.is_user && !older { 23. } else { 21. })).text_color(rgb(ink)).child(styled)));
    }
    if let Some(activity) = props.activity.filter(|s| !s.trim().is_empty()) {
        transcript = transcript.child(div().text_size(design::ui_scaled_px(12.)).text_color(rgb(palette.muted)).child(activity));
    }
    let elapsed = format!("{:02}:{:02}", props.elapsed_seconds / 60, props.elapsed_seconds % 60);
    div().id("voice-mode-stage").w_full().flex_1().min_h_0().overflow_hidden().flex().flex_col()
        .child(div().w_full().flex().items_center().justify_between().px(design::ui_scaled_px(18.)).pt(design::ui_scaled_px(12.))
            .child(div().rounded_full().px(design::ui_scaled_px(9.)).py(design::ui_scaled_px(3.)).bg(rgba((color << 8) | 30)).text_size(design::ui_scaled_px(11.)).text_color(rgb(label_color)).child("Voice"))
            .child(div().text_size(design::ui_scaled_px(11.)).text_color(rgb(palette.muted)).child(elapsed)))
        .child(orb)
        .child(div().id("voice-mode-status").role(gpui_kit::accesskit::Role::Status).w_full().text_center().text_size(design::ui_scaled_px(11.)).text_color(rgb(label_color)).child(props.status))
        .child(div().w_full().flex_1().min_h_0().overflow_hidden().px(design::ui_scaled_px(22.)).pt(design::ui_scaled_px(20.)).child(transcript))
        .into_any_element()
}

fn mix(a: u32, b: u32, t: f32) -> u32 { design::mix_rgb(a, b, t) }
fn circle(window: &mut Window, x: f32, y: f32, r: f32, color: u32, alpha: u8, filled: bool) {
    let p = |x: f32, y: f32| point(px(x), px(y));
    let mut path = if filled { PathBuilder::fill() } else { PathBuilder::stroke(px(1.)) };
    let k = r * 0.552_284_8;
    path.move_to(p(x + r, y));
    path.cubic_bezier_to(p(x, y + r), p(x + r, y + k), p(x + k, y + r));
    path.cubic_bezier_to(p(x - r, y), p(x - k, y + r), p(x - r, y + k));
    path.cubic_bezier_to(p(x, y - r), p(x - r, y - k), p(x - k, y - r));
    path.cubic_bezier_to(p(x + r, y), p(x + k, y - r), p(x + r, y - k));
    path.close();
    if let Ok(path) = path.build() { window.paint_path(path, rgba((color << 8) | u32::from(alpha))); }
}
