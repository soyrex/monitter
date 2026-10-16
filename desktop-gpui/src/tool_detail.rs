//! Bounded, presentation-only projection of structured tool details.
use crate::design::ui_scaled_px as px;
use gpui_kit::{
    component::{ActiveTheme, Sizable, button::*, text::TextView},
    prelude::*,
    *,
};
use serde_json::Value;

const MAX_BLOCKS: usize = 128;
const MAX_TEXT: usize = 256 * 1024;
const MAX_DEPTH: usize = 12;

#[derive(Debug, PartialEq)]
struct Block {
    label: String,
    text: String,
}

fn human_label(key: &str) -> String {
    let mut label = String::new();
    for (index, character) in key.chars().enumerate() {
        if matches!(character, '_' | '-') {
            label.push(' ');
        } else {
            if index > 0 && character.is_uppercase() {
                label.push(' ');
            }
            if index == 0 {
                label.extend(character.to_uppercase());
            } else {
                label.push(character);
            }
        }
    }
    label
}

fn transport_field(key: &str) -> bool {
    matches!(
        key,
        "id" | "callId"
            | "toolCallId"
            | "itemId"
            | "requestId"
            | "sessionId"
            | "threadId"
            | "pluginId"
    )
}

fn blocks(source: &str) -> Option<Vec<Block>> {
    if source.len() > MAX_TEXT {
        return None;
    }
    let value: Value = serde_json::from_str(source).ok()?;
    let mut result = Vec::new();
    let mut remaining = MAX_TEXT;
    project(&value, "", 0, &mut remaining, &mut result);
    (!result.is_empty()).then_some(result)
}

fn project(
    value: &Value,
    label: &str,
    depth: usize,
    remaining: &mut usize,
    result: &mut Vec<Block>,
) {
    if result.len() >= MAX_BLOCKS || *remaining == 0 {
        return;
    }
    if depth > MAX_DEPTH {
        result.push(Block {
            label: label.into(),
            text: "Nested detail omitted.".into(),
        });
        return;
    }
    match value {
        Value::Null => {}
        Value::Object(object) => {
            let transport_envelope =
                object
                    .get("type")
                    .and_then(Value::as_str)
                    .is_some_and(|kind| {
                        matches!(
                            kind,
                            "commandExecution"
                                | "mcpToolCall"
                                | "toolCall"
                                | "text"
                                | "TextContent"
                        )
                    });
            for (key, value) in object {
                if (transport_envelope && transport_field(key)) || value.is_null() {
                    continue;
                }
                if key == "type"
                    && value.as_str().is_some_and(|kind| {
                        matches!(kind, "commandExecution" | "text" | "TextContent")
                    })
                {
                    continue;
                }
                // Transport wrappers carry no useful visual hierarchy. Text
                // strings and nested tool envelopes retain their actual data.
                let wrapper = (value.is_object() || value.is_array())
                    && matches!(
                        key.as_str(),
                        "content" | "results" | "result" | "data" | "arguments" | "output"
                    );
                let next = if wrapper {
                    label.to_owned()
                } else if label.is_empty() {
                    human_label(key)
                } else {
                    format!("{label} · {}", human_label(key))
                };
                project(value, &next, depth + 1, remaining, result);
            }
        }
        Value::Array(array) => {
            for (index, value) in array.iter().enumerate() {
                let next = if array.len() <= 1 || label.is_empty() {
                    label.to_owned()
                } else {
                    format!("{label} · {}", index + 1)
                };
                project(value, &next, depth + 1, remaining, result);
            }
        }
        _ => {
            if let Value::String(text) = value {
                let trimmed = text.trim();
                if (trimmed.starts_with('{') || trimmed.starts_with('['))
                    && text.len() <= *remaining
                {
                    if let Ok(nested) = serde_json::from_str::<Value>(trimmed) {
                        if nested.is_object() || nested.is_array() {
                            project(&nested, label, depth + 1, remaining, result);
                            return;
                        }
                    }
                }
            }
            let text = match value {
                Value::String(text) => text.clone(),
                Value::Bool(value) => {
                    if *value {
                        "Yes".into()
                    } else {
                        "No".into()
                    }
                }
                Value::Number(number) => number.to_string(),
                _ => unreachable!(),
            };
            if text.trim().is_empty() {
                return;
            }
            let text = if text.len() > *remaining {
                let mut end = *remaining;
                while !text.is_char_boundary(end) {
                    end -= 1;
                }
                format!("{}\n…", &text[..end])
            } else {
                text
            };
            *remaining = remaining.saturating_sub(text.len());
            result.push(Block {
                label: label.into(),
                text,
            });
        }
    }
}

pub(super) struct PreparedDetail {
    blocks: Vec<Block>,
    literals: Vec<SharedString>,
    inline_diffs: Vec<Option<crate::git::InlineDiff>>,
}
pub(super) fn prepare(source: &str) -> Option<PreparedDetail> {
    if source.len() > MAX_TEXT {
        return None;
    }
    let blocks = blocks(source).or_else(|| {
        crate::git::InlineDiff::prepare(source).map(|_| {
            vec![Block {
                label: String::new(),
                text: source.to_owned(),
            }]
        })
    })?;
    let literals = blocks
        .iter()
        .map(|block| SharedString::from(super::literal_detail(&block.text)))
        .collect();
    let inline_diffs = blocks
        .iter()
        .map(|block| crate::git::InlineDiff::prepare(&block.text))
        .collect();
    Some(PreparedDetail {
        blocks,
        literals,
        inline_diffs,
    })
}
pub(super) fn render_prepared(
    detail: &PreparedDetail,
    event_id: &str,
    task_id: &str,
    git_owner: Option<WeakEntity<crate::chat::ChatPane>>,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let mut body = div().w_full().min_w_0().flex().flex_col().gap(px(6.));
    for (index, block) in detail.blocks.iter().enumerate() {
        let mut section = div().w_full().min_w_0().flex().flex_col().gap(px(3.));
        if !block.label.is_empty() {
            section = section.child(
                div()
                    .text_size(px(10.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(cx.theme().muted_foreground)
                    .child(block.label.clone()),
            );
        }
        if let Some(diff) = &detail.inline_diffs[index] {
            section =
                section.child(diff.render(&format!("tool-diff-{event_id}-{index}"), window, cx));
            if let Some(owner) = git_owner.clone() {
                let task_id = task_id.to_owned();
                section = section.child(
                    Button::new(format!("tool-diff-open-{event_id}-{index}"))
                        .text()
                        .compact()
                        .xsmall()
                        .label("Open in Changes")
                        .on_click(move |_, _, cx| {
                            let _ = owner.update(cx, |_, cx| {
                                cx.emit(crate::chat::ChatEvent::Git(task_id.clone()));
                            });
                        }),
                );
            }
        } else {
            section = section.child(
                TextView::markdown(
                    SharedString::from(format!("tool-detail-{event_id}-{index}")),
                    detail.literals[index].clone(),
                )
                .selectable(true)
                .scrollable(false),
            );
        }
        body = body.child(section);
    }
    body.into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[::core::prelude::v1::test]
    fn actual_patch_payloads_get_inline_previews() {
        let patch = "--- a/example.rs\n+++ b/example.rs\n@@ -1 +1 @@\n-old\n+new\n";
        let raw = prepare(patch).unwrap();
        assert!(raw.inline_diffs[0].is_some());
        let structured = prepare(&serde_json::json!({"patch": patch}).to_string()).unwrap();
        assert!(structured.inline_diffs[0].is_some());
    }

    #[::core::prelude::v1::test]
    fn ordinary_literals_and_path_counts_are_not_invented_patches() {
        assert!(prepare("ordinary output\nwith punctuation").is_none());
        assert!(prepare("{\"patch\":\"unfinished").is_none());
        let prepared = prepare(
            r##"{"path":"example.rs","additions":1,"deletions":1,"text":"# exact literal"}"##,
        )
        .unwrap();
        assert!(prepared.inline_diffs.iter().all(Option::is_none));
        assert_eq!(prepared.blocks.last().unwrap().text, "# exact literal");
    }

    #[::core::prelude::v1::test]
    fn prepared_detail_keeps_exact_blocks_and_literal_text_without_reprojection() {
        let source = r##"{"command":"echo `hello`","output":"# Literal heading\nnext"}"##;
        let prepared = prepare(source).unwrap();
        assert_eq!(prepared.blocks, blocks(source).unwrap());
        for (block, literal) in prepared.blocks.iter().zip(&prepared.literals) {
            assert_eq!(literal.as_str(), super::super::literal_detail(&block.text));
        }
    }

    #[::core::prelude::v1::test]
    fn command_execution_unescapes_command_and_preserves_execution_results() {
        let result = blocks(r#"{"type":"commandExecution","id":"exec-1","pluginId":null,"command":"printf 'hello\\n'\npwd","exitCode":0,"durationMs":42,"output":"hello\n/work"}"#).unwrap();
        assert!(result.contains(&Block {
            label: "Command".into(),
            text: "printf 'hello\\n'\npwd".into()
        }));
        assert!(result.contains(&Block {
            label: "Exit Code".into(),
            text: "0".into()
        }));
        assert!(result.contains(&Block {
            label: "Duration Ms".into(),
            text: "42".into()
        }));
        assert!(result.iter().all(|block| !block.text.contains("exec-1")));
    }
    #[::core::prelude::v1::test]
    fn text_content_arrays_and_unknown_fields_remain_readable() {
        let result = blocks(r#"{"content":[{"type":"text","text":"Fetched page\nHeading"}],"metadata":{"url":"https://example.test","verified":true},"warnings":["One","Two"]}"#).unwrap();
        assert!(
            result
                .iter()
                .any(|block| block.text == "Fetched page\nHeading")
        );
        assert!(result.contains(&Block {
            label: "Metadata · Url".into(),
            text: "https://example.test".into()
        }));
        assert!(result.contains(&Block {
            label: "Metadata · Verified".into(),
            text: "Yes".into()
        }));
        assert!(result.contains(&Block {
            label: "Warnings · 2".into(),
            text: "Two".into()
        }));
    }
    #[::core::prelude::v1::test]
    fn boxed_arguments_are_projected_and_many_fields_are_bounded() {
        let result = blocks(r#"{"arguments":"{\"command\":\"ls\"}"}"#).unwrap();
        assert!(result.contains(&Block {
            label: "Arguments · Command".into(),
            text: "ls".into()
        }));
        let values = Value::Array((0..300).map(Value::from).collect());
        assert_eq!(blocks(&values.to_string()).unwrap().len(), MAX_BLOCKS);
    }
    #[::core::prelude::v1::test]
    fn data_record_identifiers_are_preserved_inside_tool_envelopes() {
        let result = blocks(r#"{"type":"commandExecution","id":"exec-1","output":{"id":"record-42","name":"Example"}}"#).unwrap();
        assert!(result.contains(&Block {
            label: "Id".into(),
            text: "record-42".into()
        }));
        assert!(result.iter().all(|block| block.text != "exec-1"));
    }

    #[::core::prelude::v1::test]
    fn plain_and_partial_json_retain_existing_literal_fallback() {
        assert!(blocks("ordinary output\nwith punctuation").is_none());
        assert!(blocks("{\"command\":\"unfinished").is_none());
        assert!(blocks(&"x".repeat(MAX_TEXT + 1)).is_none());
    }
}
