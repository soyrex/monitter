//! Retained live-message renderer. Its timer never touches agent state.
use crate::{
    chat::message_stream_presentation,
    design,
    streaming_display::{self, Mode, Options},
};
use gpui_kit::{component::text::TextView, prelude::*, *};
use monitter_gpui_desktop::streaming_reveal::{RevealMessage, RevealState};
use serde_json::Value;
use std::rc::Rc;

pub struct StreamingMessage {
    #[cfg(test)]
    pub(crate) render_count: usize,
    source: Rc<Value>,
    reveal: RevealState,
    options: Options,
    visible: bool,
    timer: Option<Task<()>>,
    reveal_credit: f64,
    list_item: Option<(ListState, usize)>,
    palette: Option<design::Palette>,
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
            source,
            reveal,
            options,
            visible: true,
            timer: None,
            reveal_credit: 0.,
            list_item: None,
            palette: None,
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
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        #[cfg(test)]
        { self.render_count += 1; }
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
        let mut presentation = if typing {
            // Preserve literal text until the final revealed character, including
            // the drain after the authoritative response has already completed.
            message_stream_presentation(
                &serde_json::json!({"role":"assistant","streamStatus":"streaming"}),
                visible_text,
            )
        } else {
            message_stream_presentation(&self.source, text)
        };
        if typing {
            presentation.markdown_source.push('▌');
        }
        let status = if typing && self.source["streamStatus"] != "streaming" {
            Some("Typing…")
        } else {
            presentation.status_label
        };
        let mut content = div()
            .w_full()
            .min_w_0()
            .max_w_full()
            .flex()
            .flex_col()
            .gap(design::ui_scaled_px(design::space::TIGHT))
            .child(
                TextView::markdown(
                    SharedString::from(format!(
                        "streaming-body-{}",
                        self.source["id"].as_str().unwrap_or_default()
                    )),
                    presentation.markdown_source,
                )
                .selectable(true)
                .scrollable(false)
                .w_full()
                .min_w_0()
                .max_w_full(),
            );
        if let Some(status) = status {
            let mut label = div().text_size(design::ui_scaled_px(11.)).child(status);
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
        content
    }
}
