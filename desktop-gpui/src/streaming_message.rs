//! Retained live-message renderer. Its timer never touches agent state.
use crate::{
    design,
    streaming_display::{self, Mode, Options},
};
use gpui_kit::{component::text::{TextView, TextViewState}, prelude::*, *};
use monitter_gpui_desktop::streaming_reveal::{RevealMessage, RevealState};
use serde_json::Value;
use std::rc::Rc;

pub struct StreamingMessage {
    #[cfg(test)]
    pub(crate) render_count: usize,
    #[cfg(test)]
    pub(crate) literal_escaped_bytes: usize,
    source: Rc<Value>,
    text_view: Option<Entity<TextViewState>>,
    literal_bytes: Option<usize>,
    reveal: RevealState,
    options: Options,
    visible: bool,
    timer: Option<Task<()>>,
    reveal_credit: f64,
    list_item: Option<(ListState, usize)>,
    palette: Option<design::Palette>,
    link_owner: Option<WeakEntity<crate::chat::ChatPane>>,
    openui: crate::openui_output::MessageOutput,
}
fn message(source: &Value) -> RevealMessage<'_> {
    RevealMessage {
        id: "body",
        text: source["text"].as_str().unwrap_or_default(),
        is_streaming: source["streamStatus"] == "streaming",
        is_interrupted: source["streamStatus"] == "interrupted",
    }
}
impl StreamingMessage {
    pub(crate) fn set_link_owner(&mut self, owner: WeakEntity<crate::chat::ChatPane>) { self.link_owner = Some(owner); }
    pub fn new(source: Rc<Value>, animate: bool, cx: &mut Context<Self>) -> Self {
        let options = streaming_display::current(cx);
        let mut reveal = RevealState::default();
        if animate && options.mode == Mode::Typing {
            reveal.sync([message(&source)]);
        } else if options.mode == Mode::Typing {
            reveal.seed([message(&source)]);
        }
        cx.observe_global::<streaming_display::Selection>(|body, cx| {
            let options = streaming_display::current(cx);
            if options == body.options {
                return;
            }
            body.timer = None;
            body.reveal_credit = 0.;
            if options.mode != body.options.mode {
                body.reveal.seed([message(&body.source)]);
            }
            body.options = options;
            body.release_finished_reveal();
            body.invalidate();
            body.schedule(cx);
            cx.notify();
        })
        .detach();
        let mut body = Self {
            #[cfg(test)]
            render_count: 0,
            #[cfg(test)]
            literal_escaped_bytes: 0,
            source,
            text_view: None,
            literal_bytes: None,
            reveal,
            options,
            visible: true,
            timer: None,
            reveal_credit: 0.,
            list_item: None,
            palette: None,
            link_owner: None,
            openui: Default::default(),
        };
        body.schedule(cx);
        body
    }
    #[cfg(test)]
    pub(crate) fn source(&self) -> &Rc<Value> { &self.source }
    #[cfg(test)]
    pub(crate) fn has_pending_reveal(&self) -> bool { self.reveal.has_pending() }

    pub fn set_message(&mut self, source: Rc<Value>, cx: &mut Context<Self>) {
        if Rc::ptr_eq(&self.source, &source) {
            return;
        }
        if self.source["text"] == source["text"]
            && self.source["streamStatus"] == source["streamStatus"]
        {
            self.source = source;
            return;
        }
        if !source["text"].as_str().unwrap_or_default().starts_with(
            self.source["text"].as_str().unwrap_or_default(),
        ) {
            self.literal_bytes = None;
        }
        self.source = source;
        if self.visible && self.options.mode == Mode::Typing {
            self.reveal.sync([message(&self.source)]);
        } else {
            self.reveal.flush();
        }
        self.release_finished_reveal();
        self.schedule(cx);
        cx.notify();
    }
    pub fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        self.openui.set_visible(visible, cx);
        if self.visible == visible {
            return;
        }
        self.visible = visible;
        self.timer = None;
        self.reveal_credit = 0.;
        self.reveal.seed([message(&self.source)]);
        self.release_finished_reveal();
        self.invalidate();
        cx.notify();
    }
    pub fn set_list_item(&mut self, state: ListState, index: usize) {
        self.list_item = Some((state, index));
    }
    pub fn set_palette(&mut self, palette: design::Palette) {
        self.palette = Some(palette);
    }
    fn invalidate(&self) {
        if let Some((state, index)) = &self.list_item {
            if *index < state.item_count() {
                state.remeasure_items(*index..*index + 1);
            }
        }
    }
    fn release_finished_reveal(&mut self) {
        if !self.visible
            || self.options.mode != Mode::Typing
            || (self.source["streamStatus"] != "streaming" && !self.reveal.has_pending())
        {
            self.reveal.flush();
            self.timer = None;
        }
    }
    fn schedule(&mut self, cx: &mut Context<Self>) {
        if !self.reveal.has_pending() {
            self.reveal_credit = 0.;
        }
        if self.timer.is_some()
            || !self.visible
            || self.options.mode != Mode::Typing
            || !self.reveal.has_pending()
        {
            return;
        }
        let interval = self.options.interval();
        let started = std::time::Instant::now();
        self.timer = Some(cx.spawn(async move |body, cx| {
            cx.background_executor().timer(interval).await;
            let _ = body.update(cx, |body, cx| {
                body.timer = None;
                body.reveal_credit += started.elapsed().as_secs_f64().min(0.25)
                    * f64::from(body.options.characters_per_second);
                let characters = body.reveal_credit.floor() as usize;
                body.reveal_credit -= characters as f64;
                if body.visible
                    && body.options.mode == Mode::Typing
                    && body.reveal.advance(characters)
                {
                    body.invalidate();
                    cx.notify();
                }
                body.release_finished_reveal();
                body.schedule(cx);
            });
        }));
    }
}
impl Render for StreamingMessage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        #[cfg(test)]
        { self.render_count += 1; }
        if self.visible {
            if let Some(output) = self.openui.render(&self.source, self.link_owner.clone(), self.palette,
                self.list_item.clone(), window, cx) {
                return output;
            }
        }
        let text = self.source["text"].as_str().unwrap_or_default();
        let typing =
            self.visible && self.options.mode == Mode::Typing && self.reveal.cursor_visible("body");
        let visible_text = if typing {
            &text[..self
                .reveal
                .visible_bytes("body")
                .unwrap_or(text.len())
                .min(text.len())]
        } else {
            text
        };
        let literal = typing || (self.source["role"] == "assistant" && self.source["streamStatus"] == "streaming");
        let visible_bytes = visible_text.len();
        // Keep the selectable TextView entity across reveal ticks. Only escape
        // and append newly revealed bytes; the cursor is separate presentation,
        // so moving it never replaces the body or resets its native selection.
        let append_from = self.literal_bytes.filter(|previous| literal && *previous <= visible_bytes);
        #[cfg(test)]
        if literal { self.literal_escaped_bytes += visible_bytes - append_from.unwrap_or(0); }
        let update = if let Some(previous) = append_from {
            escape_literal_suffix(&visible_text[previous..])
        } else if literal {
            escape_literal_suffix(visible_text)
        } else {
            text.to_owned()
        };
        let text_view = self.text_view.get_or_insert_with(|| {
            cx.new(|cx| TextViewState::markdown("", cx))
        }).clone();
        text_view.update(cx, |view, cx| {
            if append_from.is_some() { view.push_str(&update, cx); }
            else { view.set_text(&update, cx); }
        });
        self.literal_bytes = literal.then_some(visible_bytes);
        let status = if typing && self.source["streamStatus"] != "streaming" {
            Some("Typing…")
        } else {
            match self.source["streamStatus"].as_str() {
                Some("streaming") => Some("Receiving…"),
                Some("interrupted") => Some("Partial reply · interrupted"),
                _ => None,
            }
        };
        let mut content = div()
            .w_full()
            .min_w_0()
            .max_w_full()
            .flex()
            .flex_col()
            .gap(design::ui_scaled_px(design::space::TIGHT))
            .child(
                TextView::new(&text_view)
                .on_link_click({ let owner = self.link_owner.clone(); move |url, _, _, cx| crate::chat::open_markdown_link(owner.clone(), url.as_str(), cx) })
                .selectable(true)
                .scrollable(false)
                .w_full()
                .min_w_0()
                .max_w_full(),
            );
        if let Some(status) = status {
            let mut label = div().text_size(design::ui_scaled_px(11.)).child(status)
                .when(typing, |label| label.child(" ▌"));
            if let Some(palette) = self.palette {
                label = label.text_color(rgb(palette.muted));
            }
            content = content.child(label);
        }
        if typing && self.reveal.is_pending("body") && self.source["streamStatus"] != "streaming" {
            content = content.child(
                gpui_kit::component::button::Button::new("reveal-full-reply")
                    .label("Show full reply")
                    .on_click(cx.listener(|body, _, _, cx| {
                        body.timer = None;
                        body.reveal.seed([message(&body.source)]);
                        body.release_finished_reveal();
                        body.invalidate();
                        cx.notify();
                    })),
            );
        }
        content.into_any_element()
    }
}

/// Literal streaming content uses Markdown's native selectable view, but only
/// the newly exposed suffix passes through punctuation escaping.
fn escape_literal_suffix(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        if character == '\n' || character.is_ascii_punctuation() { escaped.push('\\'); }
        escaped.push(character);
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::escape_literal_suffix;

    #[::core::prelude::v1::test]
    fn incremental_literal_reveal_matches_full_escaping_at_unicode_boundaries() {
        let text = "Hello **κόσμος**\n[link](https://example.com) 👋";
        let expected = escape_literal_suffix(text);
        let mut appended = String::new();
        let mut previous = 0;
        for end in text.char_indices().map(|(start, ch)| start + ch.len_utf8()) {
            appended.push_str(&escape_literal_suffix(&text[previous..end]));
            previous = end;
        }
        assert_eq!(appended, expected);
        assert_eq!(escape_literal_suffix(""), "");
    }
}

#[cfg(all(test, feature = "visual-export"))]
mod ui_tests {
    use super::*;
    use ::core::prelude::v1::test;
    use ::gpui::{TestAppContext, VisualTestContext};
    use serde_json::json;

    fn draw(visual: &mut VisualTestContext) {
        visual.run_until_parked();
        visual.update(|window, cx| window.draw(cx).clear(cx));
    }

    #[::gpui::test]
    fn retained_literal_body_appends_suffix_then_switches_to_final_markdown(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            cx.set_global(streaming_display::Selection {
                options:Options { mode:Mode::AsReceived, characters_per_second:600 }, error:None,
            });
        });
        let initial = "**Hello**";
        let (body, visual) = cx.add_window_view(|_, cx| StreamingMessage::new(
            Rc::new(json!({"id":"literal", "role":"assistant", "text":initial, "streamStatus":"streaming"})), false, cx,
        ));
        draw(visual);
        let view = body.read_with(visual, |body, _| body.text_view.clone().unwrap());
        // Native rendered text includes the paragraph terminator; assert it
        // explicitly so literal punctuation and final formatting stay checked.
        assert_eq!(view.read_with(visual, |view, _| view.rendered_text().as_str().to_owned()), "**Hello**\n");
        let extended = "**Hello** κόσμος";
        body.update(visual, |body, cx| body.set_message(Rc::new(json!({
            "id":"literal", "role":"assistant", "text":extended, "streamStatus":"streaming",
        })), cx));
        draw(visual);
        body.read_with(visual, |body, _| {
            assert_eq!(body.text_view.as_ref().unwrap(), &view);
            assert_eq!(body.literal_escaped_bytes, extended.len(), "each byte is escaped once across append updates");
        });
        assert_eq!(view.read_with(visual, |view, _| view.rendered_text().as_str().to_owned()), "**Hello** κόσμος\n");
        body.update(visual, |body, cx| body.set_message(Rc::new(json!({
            "id":"literal", "role":"assistant", "text":extended, "streamStatus":"complete",
        })), cx));
        draw(visual);
        assert_eq!(view.read_with(visual, |view, _| view.rendered_text().as_str().to_owned()), "Hello κόσμος\n");
        body.update(visual, |body, cx| body.set_message(Rc::new(json!({
            "id":"literal", "role":"assistant", "text":"Replacement **body**", "streamStatus":"streaming",
        })), cx));
        draw(visual);
        assert_eq!(view.read_with(visual, |view, _| view.rendered_text().as_str().to_owned()), "Replacement **body**\n");
    }
}
