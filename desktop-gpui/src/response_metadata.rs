//! Display-safe assistant response attribution, matching the serialized
//! AssistantResponseMetadata projection rather than inferring from task config.
use crate::design::ui_scaled_px as px;
use gpui_kit::{prelude::*, *};
use gpui_kit::{assets::IconName, component::{Icon, button::{Button, ButtonVariants}}};
use serde_json::Value;

use crate::design;

fn text<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
}

fn token_count(value: &Value, key: &str) -> Option<u64> {
    value.get(key).and_then(Value::as_u64)
}

fn grouped_integer(value: u64) -> String {
    let digits = value.to_string();
    let first_group = digits.len() % 3;
    let mut result = String::with_capacity(digits.len() + digits.len() / 3);
    result.push_str(&digits[..first_group]);
    for chunk in digits[first_group..].as_bytes().chunks(3) {
        if !result.is_empty() {
            result.push(',');
        }
        // `digits` contains only ASCII decimal digits, so these chunks are UTF-8 safe.
        result.push_str(std::str::from_utf8(chunk).expect("ASCII digit chunk"));
    }
    result
}

fn route_summary(metadata: &Value) -> &'static str {
    if metadata["routeApplied"] == true {
        "System One applied this route."
    } else if text(metadata, "applicationError").is_some() {
        "System One proposed a route, but Mona kept the working runtime."
    } else if metadata["routeApplied"] == false {
        "System One kept the current runtime."
    } else {
        "System One routing details"
    }
}

fn detail_line(label: &str, value: impl IntoElement, palette: design::Palette) -> AnyElement {
    div()
        .flex()
        .justify_between()
        .gap_4()
        .child(div().text_color(rgb(palette.muted)).child(label.to_owned()))
        .child(div().text_color(rgb(palette.ink)).text_right().child(value))
        .into_any_element()
}

/// Render metadata attached to one assistant message.
/// `message_id` supplies the stable disclosure identity in the virtual timeline.
pub fn render(
    metadata: &Value,
    message_id: &str,
    settings: &Value,
    appearance: WindowAppearance,
    expanded: bool,
    on_toggle: impl Fn(&mut Window, &mut App) + 'static,
) -> AnyElement {
    if !metadata.is_object() {
        return div().into_any_element();
    }

    let palette = design::palette(settings, appearance);
    let model = text(metadata, "model").unwrap_or("Model unavailable");
    let toggle_id = format!("response-metadata-toggle-{message_id}");
    let info_id = format!("response-metadata-info-{message_id}");
    let chevron_id = format!("response-metadata-chevron-{message_id}");
    let details_id = format!("response-details-{message_id}");
    let mut details = div()
        .id(SharedString::from(format!("response-details-{message_id}")))
        .debug_selector(move || details_id.clone())
        .text_size(px(11.))
        .w(px(340.))
        .max_w_full()
        .mt_1()
        .p_2()
        .border_1()
        .border_color(rgb(palette.line))
        .rounded_md()
        .bg(crate::design::surface_color(palette.card))
        .flex()
        .flex_col()
        .gap_1();

    let input = token_count(metadata, "inputTokens");
    let output = token_count(metadata, "outputTokens");
    if let Some(total) = input
        .zip(output)
        .and_then(|(input, output)| input.checked_add(output))
    {
        details = details.child(detail_line(
            "Tokens",
            format!("{} total", grouped_integer(total)),
            palette,
        ));
    }
    if let Some(input) = input {
        details = details.child(detail_line("Input", grouped_integer(input), palette));
    }
    if let Some(output) = output {
        details = details.child(detail_line("Output", grouped_integer(output), palette));
    }
    if let Some(requested) = text(metadata, "requestedModel") {
        let value = match text(metadata, "requestedEffort") {
            Some(effort) => format!("{requested} · {effort}"),
            None => requested.to_owned(),
        };
        details = details.child(detail_line("System One requested", value, palette));
    }
    if let Some(confidence) = metadata
        .get("confidence")
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite() && (0.0..=1.0).contains(value))
    {
        details = details.child(detail_line(
            "Confidence",
            format!("{}%", (confidence * 100.0).round() as u32),
            palette,
        ));
    }

    details = details.child(
        div()
            .mt_1()
            .text_color(rgb(palette.muted))
            .child(route_summary(metadata)),
    );
    if let Some(rationale) = text(metadata, "jevRationale") {
        details = details.child(
            div()
                .mt_1()
                .text_color(rgb(palette.ink))
                .child(rationale.to_owned()),
        );
    }
    if let Some(error) = text(metadata, "applicationError") {
        details = details.child(
            div()
                .mt_1()
                .text_color(rgb(0xd66b6b))
                .child(error.to_owned()),
        );
    }

    div()
        .id(SharedString::from(format!("response-metadata-footer-{message_id}")))
        .mt_1()
        .flex()
        .flex_col()
        .items_start()
        .max_w_full()
        .child(
            Button::new(format!("response-metadata-toggle-{message_id}"))
                .compact()
                .text()
                .text_color(rgb(palette.muted))
                .accessibility_label(format!("{} response details for {model}", if expanded { "Hide" } else { "Show" }))
                .tooltip(if expanded { "Hide response details" } else { "Show response details" })
                .child(
                    div().debug_selector(move || toggle_id.clone()).flex().items_center().gap_1().text_size(px(10.)).text_color(rgb(palette.muted))
                        .child(model.to_owned())
                        .child(div().debug_selector(move || info_id.clone()).child(Icon::new(IconName::Info).size(px(11.))))
                        .child(div().debug_selector(move || chevron_id.clone()).child(Icon::new(if expanded { IconName::ChevronUp } else { IconName::ChevronDown }).size(px(11.)))),
                )
                .on_click(move |_, window, cx| on_toggle(window, cx)),
        )
        .when(expanded, |footer| footer.child(details))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::{grouped_integer, route_summary, token_count};
    use serde_json::json;

    #[::core::prelude::v1::test]
    fn token_format_preserves_zero_and_groups_large_counts() {
        assert_eq!(grouped_integer(0), "0");
        assert_eq!(grouped_integer(12), "12");
        assert_eq!(grouped_integer(1_234_567), "1,234,567");
        assert_eq!(token_count(&json!({}), "inputTokens"), None);
        assert_eq!(
            token_count(&json!({"inputTokens":0}), "inputTokens"),
            Some(0)
        );
    }

    #[::core::prelude::v1::test]
    fn route_summary_uses_only_explicit_route_outcome() {
        assert_eq!(route_summary(&json!({})), "System One routing details");
        assert_eq!(
            route_summary(&json!({"routeApplied":true})),
            "System One applied this route."
        );
        assert_eq!(
            route_summary(&json!({"routeApplied":false})),
            "System One kept the current runtime."
        );
        assert_eq!(
            route_summary(&json!({"applicationError":"rejected"})),
            "System One proposed a route, but Mona kept the working runtime."
        );
        assert_eq!(
            route_summary(&json!({"applicationError":""})),
            "System One routing details"
        );
    }
}

#[cfg(all(test, feature = "visual-export"))]
#[path = "response_metadata_ui_tests.rs"]
mod ui_tests;
