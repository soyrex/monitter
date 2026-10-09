//! Compact durable transcript boundaries, matching TaskTranscript.svelte.
use crate::design::{self, ui_scaled_px as px};
use gpui_kit::{prelude::*, *};
use serde_json::Value;

fn label(message: &Value) -> Option<String> {
    if message["role"].as_str() != Some("system")
        || message["senderAgentId"]
            .as_str()
            .is_some_and(|id| !id.is_empty())
        || message["attachments"]
            .as_array()
            .is_some_and(|items| !items.is_empty())
    {
        return None;
    }
    let text = message["text"].as_str()?;
    if text == "Context Cleared" {
        return Some(text.into());
    }
    if let Some(change) = text.strip_prefix("[Monitter settings change] ") {
        return (!change.is_empty()).then(|| change.into());
    }
    text.strip_prefix("[Monitter fork] ")
        .map(|source| format!("Forked from {source}"))
}

pub(super) fn render(
    message: &Value,
    settings: &Value,
    palette: design::Palette,
) -> Option<AnyElement> {
    let label = label(message)?;
    let time = message["createdAt"]
        .as_i64()
        .and_then(crate::timestamps::local_datetime)
        .map(|time| time.format("%-I:%M %P").to_string());
    let visible = time
        .map(|time| format!("{time} · {label}"))
        .unwrap_or(label);
    let ratio = settings["interfaceFontSize"]
        .as_f64()
        .unwrap_or(14.)
        .clamp(8., 32.) as f32
        / 14.;
    Some(
        div()
            .id(format!("system-divider-{}", message["id"].as_str().unwrap_or("notice")))
            .w_full()
            .min_w_0()
            .flex()
            .items_center()
            .gap(px(10.))
            .my(px(5.))
            .role(Role::Label)
            .aria_label(visible.clone())
            .child(div().h(px(1.)).flex_1().bg(rgb(palette.line)))
            .child(
                div()
                    .min_w_0()
                    .max_w(relative(0.85))
                    .text_size(px(9. * ratio))
                    .font_family(design::FONT_MONO)
                    .text_color(rgb(palette.muted))
                    .child(visible.to_uppercase()),
            )
            .child(div().h(px(1.)).flex_1().bg(rgb(palette.line)))
            .into_any_element(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[::core::prelude::v1::test]
    fn durable_settings_and_context_boundaries_strip_internal_markers() {
        assert_eq!(label(&json!({"role":"system","text":"[Monitter settings change] Model → gpt6.1-sol · Thinking level → low"})).as_deref(), Some("Model → gpt6.1-sol · Thinking level → low"));
        assert_eq!(
            label(&json!({"role":"system","text":"Context Cleared"})).as_deref(),
            Some("Context Cleared")
        );
        assert_eq!(
            label(&json!({"role":"system","text":"[Monitter fork] Previous task"})).as_deref(),
            Some("Forked from Previous task")
        );
    }
    #[::core::prelude::v1::test]
    fn rich_and_agent_messages_keep_their_original_presentation() {
        for message in [
            json!({"role":"assistant","text":"Context Cleared"}),
            json!({"role":"system","text":"[Monitter settings change] ",}),
            json!({"role":"system","text":"Ordinary system content"}),
            json!({"role":"system","text":"Context Cleared","senderAgentId":"agent"}),
            json!({"role":"system","text":"Context Cleared","attachments":[{"id":"a"}]}),
        ] {
            assert!(label(&message).is_none());
        }
    }
}
