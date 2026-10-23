//! Native, selectable diff rows. No Markdown interpretation or per-frame parsing.
use super::model::{DiffCell, DiffRow, LineKind};
use crate::design::{self, Palette, ui_scaled_px as px};
use gpui_kit::{
    base::{TextSelectionHandle, TextSelectionRegistration, TextSelectionRun},
    prelude::*,
    *,
};

#[derive(Clone, Copy)]
pub(super) struct DiffColors {
    pub add: u32,
    pub del: u32,
    pub signal: u32,
    pub ink: u32,
    pub muted: u32,
    pub context: u32,
}
impl DiffColors {
    pub fn new(p: Palette) -> Self {
        Self {
            add: p.ok(),
            del: p.err(),
            signal: p.accent,
            ink: p.ink,
            muted: p.text(2),
            context: p.text(1),
        }
    }
}

struct SelectableCode {
    selection: TextSelectionHandle,
    text: SharedString,
    styled: StyledText,
    order: u64,
}
impl IntoElement for SelectableCode {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}
impl Element for SelectableCode {
    type RequestLayoutState = ();
    type PrepaintState = Hitbox;
    fn id(&self) -> Option<ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }
    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        w: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        self.styled.request_layout(id, inspector, w, cx)
    }
    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        w: &mut Window,
        cx: &mut App,
    ) -> Hitbox {
        self.styled.prepaint(id, inspector, bounds, &mut (), w, cx);
        let hit = w.insert_hitbox(bounds, HitboxBehavior::Normal);
        self.selection.register(
            TextSelectionRegistration::new(hit.clone(), bounds)
                .with_document_order(self.order)
                .with_text_bounds(vec![bounds]),
            w,
            cx,
        );
        hit
    }
    fn paint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        _: &mut Hitbox,
        w: &mut Window,
        cx: &mut App,
    ) {
        self.selection.update_runs(
            &[
                TextSelectionRun::new(self.text.clone(), self.styled.layout().clone(), bounds)
                    .with_document_order(self.order),
            ],
            cx,
        );
        self.styled
            .paint(id, inspector, bounds, &mut (), &mut (), w, cx);
    }
}
fn code(
    cell: &DiffCell,
    id: String,
    order: u64,
    color: u32,
    word: u32,
    w: &mut Window,
    cx: &mut App,
) -> AnyElement {
    // Long minified lines stay bounded in native shaping. The original remains
    // available through Copy diff; this is an explicit display limit only.
    let end = cell
        .text
        .char_indices()
        .nth(2000)
        .map(|(i, _)| i)
        .unwrap_or(cell.text.len());
    let mut text = cell.text[..end].to_string();
    if end < cell.text.len() {
        text.push_str(" … [long line; copy diff for full text]");
    }
    if cell.no_newline {
        text.push_str("  ⏎ no newline");
    }
    let text: SharedString = text.into();
    let selection = w.use_keyed_state(SharedString::from(id), cx, |_, cx| {
        TextSelectionHandle::new("", cx)
    });
    let handle = selection.read(cx).clone();
    handle.set_fallback_copy_text(format!("{}\n", cell.text), cx);
    let highlights = cell
        .highlights
        .iter()
        .filter_map(|r| {
            let range = r.start.min(end)..r.end.min(end);
            (range.start < range.end).then_some((
                range,
                HighlightStyle {
                    background_color: Some(rgb(word).opacity(0.30).into()),
                    ..Default::default()
                },
            ))
        })
        .collect::<Vec<_>>();
    div()
        .min_w_0()
        .text_color(rgb(color))
        .flex_1()
        .whitespace_normal()
        .child(SelectableCode {
            selection: handle,
            text: text.clone(),
            styled: StyledText::new(text).with_highlights(highlights),
            order,
        })
        .into_any_element()
}
fn gutter(number: Option<usize>, width: f32, color: u32, tint: Option<u32>) -> Div {
    div()
        .w(px(width))
        .h_full()
        .flex_shrink_0()
        .pr(px(10.))
        .text_right()
        .text_color(rgb(color))
        .when_some(tint, |d, c| d.bg(rgb(c).opacity(0.16)))
        .child(number.map(|n| n.to_string()).unwrap_or_default())
}
fn side(
    cell: Option<&DiffCell>,
    kind: LineKind,
    id: String,
    order: u64,
    c: DiffColors,
    ratio: f32,
    w: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let tint = match kind {
        LineKind::Addition => Some(c.add),
        LineKind::Deletion => Some(c.del),
        _ => None,
    };
    let sign = match kind {
        LineKind::Addition => "+",
        LineKind::Deletion => "−",
        _ => " ",
    };
    let mut row = div()
        .flex_1()
        .min_w_0()
        .min_h(px(20. * ratio))
        .flex()
        .items_start()
        .when_some(tint, |d, c| d.bg(rgb(c).opacity(0.10)))
        .child(gutter(
            cell.and_then(|c| c.number),
            36. * ratio,
            c.muted,
            tint,
        ))
        .child(
            div()
                .w(px(20. * ratio))
                .flex_shrink_0()
                .text_center()
                .text_color(rgb(tint.unwrap_or(c.muted)))
                .child(if cell.is_some() { sign } else { " " }),
        );
    if let Some(cell) = cell {
        row = row.child(code(
            cell,
            id,
            order,
            if kind == LineKind::Context {
                c.context
            } else {
                c.ink
            },
            tint.unwrap_or(c.signal),
            w,
            cx,
        ));
    }
    row.into_any_element()
}
pub(super) fn render_code(
    row: &DiffRow,
    index: usize,
    split: bool,
    id: &str,
    c: DiffColors,
    ratio: f32,
    w: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let DiffRow::Code { old, new, kind } = row else {
        return div().into_any_element();
    };
    if split {
        let old_kind = if *kind == LineKind::Context {
            LineKind::Context
        } else {
            LineKind::Deletion
        };
        let new_kind = if *kind == LineKind::Context {
            LineKind::Context
        } else {
            LineKind::Addition
        };
        div()
            .w_full()
            .flex()
            .font_family(design::FONT_MONO)
            .text_size(px(12. * ratio))
            .line_height(px(20. * ratio))
            .child(side(
                old.as_ref(),
                old_kind,
                format!("{id}-old-{index}"),
                index as u64 * 2,
                c,
                ratio,
                w,
                cx,
            ))
            .child(div().w(px(1.)).h_full().bg(rgb(c.muted).opacity(0.15)))
            .child(side(
                new.as_ref(),
                new_kind,
                format!("{id}-new-{index}"),
                index as u64 * 2 + 1,
                c,
                ratio,
                w,
                cx,
            ))
            .into_any_element()
    } else {
        let tint = match kind {
            LineKind::Addition => Some(c.add),
            LineKind::Deletion => Some(c.del),
            _ => None,
        };
        let cell = if *kind == LineKind::Deletion {
            old.as_ref()
        } else {
            new.as_ref().or(old.as_ref())
        };
        let sign = match kind {
            LineKind::Addition => "+",
            LineKind::Deletion => "−",
            _ => " ",
        };
        let mut line = div()
            .w_full()
            .min_h(px(20. * ratio))
            .flex()
            .items_start()
            .font_family(design::FONT_MONO)
            .text_size(px(12. * ratio))
            .line_height(px(20. * ratio))
            .when_some(tint, |d, c| d.bg(rgb(c).opacity(0.10)))
            .child(gutter(
                old.as_ref().and_then(|c| c.number),
                44. * ratio,
                c.muted,
                tint,
            ))
            .child(gutter(
                new.as_ref().and_then(|c| c.number),
                44. * ratio,
                c.muted,
                tint,
            ))
            .child(
                div()
                    .w(px(20. * ratio))
                    .flex_shrink_0()
                    .text_center()
                    .text_color(rgb(tint.unwrap_or(c.muted)))
                    .child(sign),
            );
        if let Some(cell) = cell {
            line = line.child(code(
                cell,
                format!("{id}-unified-{index}"),
                index as u64,
                if *kind == LineKind::Context {
                    c.context
                } else {
                    c.ink
                },
                tint.unwrap_or(c.signal),
                w,
                cx,
            ));
        }
        #[cfg(test)]
        {
            line = line.debug_selector(move || format!("git-code-row-{index}"));
        }
        line.into_any_element()
    }
}
