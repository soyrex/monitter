//! Selection-based Markdown edits through the native input's atomic undo path.
use gpui_kit::{component::input::TextareaState, prelude::*, *};
use std::ops::Range;
#[derive(Clone, Copy)]
pub(super) enum Format {
    Heading,
    Bold,
    Italic,
    Link,
    Code,
    Bullets,
    Numbers,
    Quote,
}
struct Edit {
    range: Range<usize>,
    replacement: String,
    selection: Range<usize>,
}
fn edit(text: &str, range: Range<usize>, format: Format) -> Result<Edit, String> {
    if range.start > range.end
        || range.end > text.len()
        || !text.is_char_boundary(range.start)
        || !text.is_char_boundary(range.end)
    {
        return Err("The text selection changed. Select the text again.".into());
    }
    let selected = &text[range.clone()];
    let wrap = |prefix: &str, suffix: &str| {
        if !selected.is_empty()
            && selected.starts_with(prefix)
            && selected.ends_with(suffix)
            && selected.len() >= prefix.len() + suffix.len()
        {
            let replacement = selected[prefix.len()..selected.len() - suffix.len()].to_owned();
            let selection = range.start..range.start + replacement.len();
            Edit {
                range: range.clone(),
                replacement,
                selection,
            }
        } else if range.start >= prefix.len()
            && text[..range.start].ends_with(prefix)
            && text[range.end..].starts_with(suffix)
        {
            Edit {
                range: range.start - prefix.len()..range.end + suffix.len(),
                replacement: selected.into(),
                selection: range.start - prefix.len()..range.end - prefix.len(),
            }
        } else {
            Edit {
                range: range.clone(),
                replacement: format!("{prefix}{selected}{suffix}"),
                selection: range.start + prefix.len()..range.end + prefix.len(),
            }
        }
    };
    match format {
        Format::Bold => Ok(wrap("**", "**")),
        Format::Italic => Ok(wrap("_", "_")),
        Format::Link => {
            let label = selected
                .replace('\\', "\\\\")
                .replace('[', "\\[")
                .replace(']', "\\]");
            let prefix = format!("[{label}](");
            Ok(Edit {
                range: range.clone(),
                replacement: format!("{prefix}https://)"),
                selection: range.start + prefix.len()..range.start + prefix.len() + 8,
            })
        }
        Format::Code => {
            let longest = selected
                .split(|ch| ch != '`')
                .map(str::len)
                .max()
                .unwrap_or_default();
            if selected.contains('\n') {
                let fence = "`".repeat(3.max(longest + 1));
                let before = if range.start > 0 && !text[..range.start].ends_with('\n') {
                    "\n"
                } else {
                    ""
                };
                let after = if range.end < text.len() && !text[range.end..].starts_with('\n') {
                    "\n"
                } else {
                    ""
                };
                let prefix = format!("{before}{fence}\n");
                let suffix = format!("\n{fence}{after}");
                Ok(wrap(&prefix, &suffix))
            } else {
                let fence = "`".repeat(1.max(longest + 1));
                let pad = if selected.starts_with('`') || selected.ends_with('`') {
                    " "
                } else {
                    ""
                };
                Ok(wrap(&format!("{fence}{pad}"), &format!("{pad}{fence}")))
            }
        }
        Format::Heading | Format::Bullets | Format::Numbers | Format::Quote => {
            let start = text[..range.start]
                .rfind('\n')
                .map(|i| i + 1)
                .unwrap_or_default();
            let last = if range.end > range.start && text[..range.end].ends_with('\n') {
                range.end - 1
            } else {
                range.end
            };
            let end = text[last..]
                .find('\n')
                .map(|i| last + i)
                .unwrap_or(text.len());
            let lines = text[start..end].split('\n').collect::<Vec<_>>();
            let prefix = |i: usize| match format {
                Format::Heading => "## ".to_owned(),
                Format::Bullets => "- ".into(),
                Format::Numbers => format!("{}. ", i + 1),
                _ => "> ".into(),
            };
            let remove = lines
                .iter()
                .enumerate()
                .all(|(i, line)| line.starts_with(&prefix(i)));
            let replacement = lines
                .iter()
                .enumerate()
                .map(|(i, line)| {
                    let marker = prefix(i);
                    if remove {
                        line[marker.len()..].to_owned()
                    } else if line.starts_with(&marker) {
                        (*line).to_owned()
                    } else {
                        // Changing heading level replaces the existing marker.
                        let hashes = line.bytes().take_while(|byte| *byte == b'#').count();
                        let content = if matches!(format, Format::Heading)
                            && (1..=6).contains(&hashes)
                            && line[hashes..].starts_with(' ')
                        {
                            &line[hashes + 1..]
                        } else {
                            line
                        };
                        format!("{marker}{content}")
                    }
                })
                .collect::<Vec<_>>()
                .join("\n");
            let selection = start..start + replacement.len();
            Ok(Edit {
                range: start..end,
                replacement,
                selection,
            })
        }
    }
}
pub(super) fn apply(
    input: &Entity<TextareaState>,
    format: Format,
    window: &mut Window,
    cx: &mut App,
) -> Result<(), String> {
    input.update(cx, |input, cx| {
        if EntityInputHandler::marked_text_range(input, window, cx).is_some() {
            return Err("Finish the current text composition before formatting.".into());
        }
        let value = input.value().to_string();
        let edit = edit(&value, input.selected_range(), format)?;
        input.set_selected_range(edit.range, cx);
        input.replace(edit.replacement, window, cx);
        input.set_selected_range(edit.selection, cx);
        let focus = input.focus_handle(cx);
        focus.focus(window, cx);
        Ok(())
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    fn result(text: &str, range: Range<usize>, format: Format) -> (String, Range<usize>) {
        let e = edit(text, range, format).unwrap();
        let mut result = text.to_owned();
        result.replace_range(e.range, &e.replacement);
        (result, e.selection)
    }
    #[::core::prelude::v1::test]
    fn unicode_selection_and_empty_caret_stay_inside_markup() {
        assert_eq!(
            result("a界b", 1..4, Format::Bold),
            ("a**界**b".into(), 3..6)
        );
        assert_eq!(
            result("hello", 2..2, Format::Italic),
            ("he__llo".into(), 3..3)
        );
        assert!(edit("界", 1..2, Format::Bold).is_err());
    }
    #[::core::prelude::v1::test]
    fn toggles_selected_or_surrounding_bold_without_losing_content() {
        assert_eq!(
            result("**hello**", 2..7, Format::Bold),
            ("hello".into(), 0..5)
        );
        assert_eq!(
            result("**hello**", 0..9, Format::Bold),
            ("hello".into(), 0..5)
        );
    }
    #[::core::prelude::v1::test]
    fn multiline_prefix_excludes_next_line_at_boundary() {
        assert_eq!(
            result("one\ntwo\nthree", 1..8, Format::Bullets).0,
            "- one\n- two\nthree"
        );
        assert_eq!(result("- one\n- two", 0..11, Format::Bullets).0, "one\ntwo");
    }
    #[::core::prelude::v1::test]
    fn mixed_list_and_heading_level_do_not_duplicate_markers() {
        assert_eq!(
            result("- one\ntwo", 0..9, Format::Bullets).0,
            "- one\n- two"
        );
        assert_eq!(result("# title", 0..7, Format::Heading).0, "## title");
    }
    #[::core::prelude::v1::test]
    fn code_uses_safe_fence_and_link_selects_editable_url() {
        assert_eq!(result("a`b", 0..3, Format::Code).0, "``a`b``");
        assert_eq!(
            result("one\ntwo", 0..7, Format::Code).0,
            "```\none\ntwo\n```"
        );
        let (value, selection) = result("x[y]", 0..4, Format::Link);
        assert_eq!(value, "[x\\[y\\]](https://)");
        assert_eq!(&value[selection], "https://");
    }
}
