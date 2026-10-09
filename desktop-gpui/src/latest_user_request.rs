//! Retained latest-user-request disclosure used by the chat transcript.
//!
//! The virtual list caches row heights, so toggling the disclosure remeasures
//! the containing row. Text layout itself remains native Markdown/TextView.

use crate::design::{Palette, ui_scaled_px as px};
use gpui_kit::{
    ListState, SharedString, Window,
    assets::IconName,
    component::{Icon, Sizable, button::*, text::TextView},
    prelude::*,
};

pub struct LatestUserRequest {
    message_id: String,
    text: String,
    expanded: bool,
    list_item: Option<(ListState, usize)>,
}

fn is_clampable(text: &str) -> bool {
    text.split_whitespace().count() > 200
}

impl LatestUserRequest {
    pub fn new(message_id: String, text: String, _cx: &mut Context<Self>) -> Self {
        Self {
            message_id,
            text,
            expanded: false,
            list_item: None,
        }
    }

    /// Replace the retained request and collapse only when its identity or
    /// contents actually changed (for example, when a task switches).
    pub fn set_request(&mut self, message_id: &str, text: &str, cx: &mut Context<Self>) {
        if self.message_id == message_id && self.text == text {
            return;
        }
        self.message_id.clear();
        self.message_id.push_str(message_id);
        self.text.clear();
        self.text.push_str(text);
        self.expanded = false;
        self.invalidate_list_item();
        cx.notify();
    }

    /// Attach this component to the current variable-height virtual-list row.
    pub fn set_list_item(&mut self, list: ListState, index: usize) {
        self.list_item = Some((list, index));
    }

    fn invalidate_list_item(&self) {
        if let Some((list, index)) = &self.list_item
            && *index < list.item_count()
        {
            list.remeasure_items(*index..*index + 1);
        }
    }

    pub fn build_widget(
        &self,
        palette: Palette,
        font_family: SharedString,
        font_size: f32,
        font_weight: FontWeight,
        line_height: f32,
        interface_ratio: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let can_disclose = is_clampable(&self.text);
        let viewport_height = window.viewport_size().height;
        let viewport_design_height = viewport_height.as_f32() / px(1.).as_f32();
        let compact_viewport = viewport_design_height < 500.;
        let clamped = can_disclose && !compact_viewport && !self.expanded;
        let expanded = can_disclose && !compact_viewport && self.expanded;
        let id = self.message_id.clone();

        let mut markdown = TextView::markdown(
            SharedString::from(format!("latest-user-request-{id}")),
            SharedString::from(self.text.clone()),
        )
        .selectable(true)
        .scrollable(false)
        .w_full()
        .min_w_0()
        .font_family(font_family.clone())
        .text_size(px(font_size))
        .font_weight(font_weight)
        .line_height(px(line_height));
        if clamped {
            markdown = markdown.max_lines(2);
        }

        let mut content = div()
            .id(SharedString::from(format!("latest-request-content-{id}")))
            .w_full()
            .min_w_0()
            .font_family(font_family)
            .text_size(px(font_size))
            .font_weight(font_weight)
            .line_height(px(line_height))
            .child(markdown);

        if expanded {
            let max_height = px((viewport_design_height * 0.42).min(420.));
            content = content.max_h(max_height).overflow_y_scroll();
        }

        let mut root = div()
            .id(SharedString::from(format!("latest-user-request-{id}")))
            .w_full()
            .min_w_0()
            .flex()
            .flex_col()
            .text_color(rgb(palette.ink))
            .child(content);

        if can_disclose && !compact_viewport {
            let weak = cx.weak_entity();
            let next_expanded = !self.expanded;
            let action = if next_expanded { "Expand" } else { "Collapse" };
            let label = if next_expanded {
                "Expand current request"
            } else {
                "Collapse current request"
            };
            root = root.child(
                Button::new(SharedString::from(format!(
                    "latest-user-request-toggle-{id}"
                )))
                .ghost()
                .compact()
                .mt(px(5.))
                .mr(px(-3.))
                .mb(px(-3.))
                .ml_auto()
                .px(px(5.))
                .py(px(2.))
                .gap(px(3.))
                .font_family(SharedString::from(crate::design::FONT_SANS))
                .text_size(px(10. * interface_ratio))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(palette.accent))
                .label(action)
                .child(Icon::new(IconName::ChevronDown).size(px(14.)))
                .accessibility_label(label)
                .toggled(self.expanded)
                .on_click(move |_, _, cx| {
                    let _ = weak.update(cx, |request, cx| {
                        if request.message_id != id {
                            return;
                        }
                        request.expanded = !request.expanded;
                        request.invalidate_list_item();
                        cx.notify();
                    });
                }),
            );
        }

        root.into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::is_clampable;

    #[::core::prelude::v1::test]
    fn clamp_threshold_is_strictly_more_than_two_hundred_words() {
        let two_hundred = std::iter::repeat_n("word", 200)
            .collect::<Vec<_>>()
            .join(" ");
        let two_hundred_one = format!("{two_hundred} word");
        assert!(!is_clampable(&two_hundred));
        assert!(is_clampable(&two_hundred_one));
        assert!(!is_clampable(" \n\t "));
    }
}
