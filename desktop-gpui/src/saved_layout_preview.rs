//! Compact, inert previews for persisted native dock layouts.
//!
//! The preview deliberately reads the persisted JSON rather than constructing a
//! `DockArea`: saved-layout menus must remain cheap and must never instantiate,
//! focus, or restore panes merely to draw an icon.

use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, BorderStyle, Bounds, Pixels, canvas, fill, outline, point, px, rgb, size,
};
use serde_json::Value;

const MAX_DEPTH: usize = 32;
const MAX_PANES: usize = 64;
const DEFAULT_CENTER_WIDTH: f32 = 720.;
const DEFAULT_CENTER_HEIGHT: f32 = 500.;
const MIN_DOCK_SHARE: f32 = 0.14;
const MAX_DOCK_SHARE: f32 = 0.42;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PreviewRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl PreviewRect {
    fn horizontal_slice(self, start: f32, share: f32) -> Self {
        Self {
            x: self.x + self.width * start,
            y: self.y,
            width: self.width * share,
            height: self.height,
        }
    }

    fn vertical_slice(self, start: f32, share: f32) -> Self {
        Self {
            x: self.x,
            y: self.y + self.height * start,
            width: self.width,
            height: self.height * share,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PreviewPane {
    pub rect: PreviewRect,
    /// A tab group paints as one pane. The count adds a small tab cue without
    /// pretending its inactive children occupy independent geometry.
    pub tab_count: usize,
    pub active_tab: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SavedLayoutPreviewModel {
    panes: Vec<PreviewPane>,
    valid: bool,
}

impl SavedLayoutPreviewModel {
    /// Parse a serialized `DockWorkspace` layout. Invalid or empty input is a
    /// usable empty model, so one corrupt saved item cannot break its menu.
    pub(crate) fn from_saved_layout(layout: &Value) -> Self {
        Self::try_from_saved_layout(layout).unwrap_or_else(|_| Self {
            panes: Vec::new(),
            valid: false,
        })
    }

    pub(crate) fn try_from_saved_layout(layout: &Value) -> Result<Self, String> {
        if layout.get("version").and_then(Value::as_u64) != Some(1) {
            return Err("Unsupported native workspace layout version.".into());
        }
        let center = layout
            .get("center")
            .ok_or_else(|| "Saved layout has no center tree.".to_string())?;

        let left = visible_dock(layout.get("left_dock"));
        let right = visible_dock(layout.get("right_dock"));
        let bottom = visible_dock(layout.get("bottom_dock"));

        let left_share = left
            .map(|dock| dock_share(dock, DEFAULT_CENTER_WIDTH))
            .unwrap_or(0.);
        let right_share = right
            .map(|dock| dock_share(dock, DEFAULT_CENTER_WIDTH))
            .unwrap_or(0.);
        let bottom_share = bottom
            .map(|dock| dock_share(dock, DEFAULT_CENTER_HEIGHT))
            .unwrap_or(0.);

        // Independently clamped docks could otherwise consume the whole icon.
        let horizontal_scale = if left_share + right_share > 0.68 {
            0.68 / (left_share + right_share)
        } else {
            1.
        };
        let left_share = left_share * horizontal_scale;
        let right_share = right_share * horizontal_scale;
        let content_height = 1. - bottom_share;
        let center_rect = PreviewRect {
            x: left_share,
            y: 0.,
            width: (1. - left_share - right_share).max(0.05),
            height: content_height.max(0.05),
        };

        let mut panes = Vec::new();
        collect_tree(center, center_rect, 0, &mut panes)?;
        if let Some(dock) = left {
            collect_tree(
                dock.get("panel")
                    .ok_or_else(|| "Left dock has no panel tree.".to_string())?,
                PreviewRect {
                    x: 0.,
                    y: 0.,
                    width: left_share,
                    height: content_height,
                },
                0,
                &mut panes,
            )?;
        }
        if let Some(dock) = right {
            collect_tree(
                dock.get("panel")
                    .ok_or_else(|| "Right dock has no panel tree.".to_string())?,
                PreviewRect {
                    x: 1. - right_share,
                    y: 0.,
                    width: right_share,
                    height: content_height,
                },
                0,
                &mut panes,
            )?;
        }
        if let Some(dock) = bottom {
            collect_tree(
                dock.get("panel")
                    .ok_or_else(|| "Bottom dock has no panel tree.".to_string())?,
                PreviewRect {
                    x: 0.,
                    y: content_height,
                    width: 1.,
                    height: bottom_share,
                },
                0,
                &mut panes,
            )?;
        }
        if panes.len() > MAX_PANES {
            return Err("Saved layout has too many panes to preview.".into());
        }
        Ok(Self { panes, valid: true })
    }

    pub(crate) fn panes(&self) -> &[PreviewPane] {
        &self.panes
    }

    pub(crate) fn is_valid(&self) -> bool {
        self.valid
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.panes.is_empty()
    }
}

fn visible_dock(value: Option<&Value>) -> Option<&Value> {
    let value = value?;
    value
        .get("open")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        .then_some(value)
}

fn dock_share(dock: &Value, default_center_extent: f32) -> f32 {
    let extent = dock.get("size").and_then(Value::as_f64).unwrap_or(0.) as f32;
    if !extent.is_finite() || extent <= 0. {
        return 0.24;
    }
    (extent / (extent + default_center_extent)).clamp(MIN_DOCK_SHARE, MAX_DOCK_SHARE)
}

fn collect_tree(
    node: &Value,
    rect: PreviewRect,
    depth: usize,
    panes: &mut Vec<PreviewPane>,
) -> Result<(), String> {
    if depth > MAX_DEPTH {
        return Err("Saved layout tree is too deeply nested.".into());
    }
    if panes.len() >= MAX_PANES {
        return Err("Saved layout has too many panes to preview.".into());
    }
    let children = node
        .get("children")
        .and_then(Value::as_array)
        .ok_or_else(|| "Saved layout node has no children array.".to_string())?;
    let info = node
        .get("info")
        .and_then(Value::as_object)
        .ok_or_else(|| "Saved layout node has no panel information.".to_string())?;

    if let Some(stack) = info.get("stack").and_then(Value::as_object) {
        if children.is_empty() {
            return Ok(());
        }
        let axis = stack.get("axis").and_then(Value::as_u64).unwrap_or(0);
        let weights = normalized_weights(stack.get("sizes"), children.len());
        let mut cursor = 0.;
        for (child, share) in children.iter().zip(weights) {
            let child_rect = if axis == 0 {
                rect.horizontal_slice(cursor, share)
            } else {
                rect.vertical_slice(cursor, share)
            };
            collect_tree(child, child_rect, depth + 1, panes)?;
            cursor += share;
        }
        return Ok(());
    }

    if let Some(tabs) = info.get("tabs").and_then(Value::as_object) {
        if children.is_empty() {
            return Ok(());
        }
        panes.push(PreviewPane {
            rect,
            tab_count: children.len(),
            active_tab: tabs
                .get("active_index")
                .and_then(Value::as_u64)
                .map_or(0, |index| index as usize)
                .min(children.len() - 1),
        });
        return Ok(());
    }

    // Compatibility with a bare panel leaf, which DockArea wraps in a tab
    // group when restoring older layouts.
    if info.contains_key("panel") {
        // Old DockArea builds wrote an empty TabPanel as Panel(null). Its
        // reader explicitly treats that representation as an empty group.
        if node.get("panel_name").and_then(Value::as_str) == Some("TabPanel")
            && info.get("panel").is_some_and(Value::is_null)
        {
            return Ok(());
        }
        panes.push(PreviewPane {
            rect,
            tab_count: 1,
            active_tab: 0,
        });
        return Ok(());
    }

    Err("Saved layout contains an unknown panel node.".into())
}

fn normalized_weights(value: Option<&Value>, child_count: usize) -> Vec<f32> {
    let equal = || vec![1. / child_count as f32; child_count];
    let Some(raw) = value
        .and_then(Value::as_array)
        .filter(|raw| raw.len() == child_count)
    else {
        return equal();
    };
    let Some(weights) = raw.iter().map(Value::as_f64).collect::<Option<Vec<_>>>() else {
        return equal();
    };
    if weights
        .iter()
        .any(|value| !value.is_finite() || *value <= 0.)
    {
        return equal();
    }
    let weights = weights
        .into_iter()
        .map(|weight| weight as f32)
        .collect::<Vec<_>>();
    let total = weights.iter().sum::<f32>();
    if !total.is_finite() || total <= 0. {
        return equal();
    }
    weights.into_iter().map(|weight| weight / total).collect()
}

#[derive(Clone, Copy)]
pub(crate) struct SavedLayoutPreviewColors {
    pub background: u32,
    pub pane: u32,
    pub active: u32,
    pub line: u32,
    pub empty: u32,
}

impl From<crate::design::Palette> for SavedLayoutPreviewColors {
    fn from(palette: crate::design::Palette) -> Self {
        Self {
            background: palette.paper,
            pane: palette.panel,
            active: palette.accent,
            line: palette.line,
            empty: palette.muted,
        }
    }
}

/// Render an icon-sized layout diagram. Callers own selection/hover chrome;
/// this element paints only the layout itself.
pub(crate) fn saved_layout_preview(
    layout: &Value,
    colors: SavedLayoutPreviewColors,
    width: Pixels,
    height: Pixels,
) -> AnyElement {
    let model = SavedLayoutPreviewModel::from_saved_layout(layout);
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| paint_preview(&model, colors, bounds, window),
    )
    .w(width)
    .h(height)
    .flex_shrink_0()
    .into_any_element()
}

fn paint_preview(
    model: &SavedLayoutPreviewModel,
    colors: SavedLayoutPreviewColors,
    bounds: Bounds<Pixels>,
    window: &mut gpui_kit::Window,
) {
    let outer = bounds.inset(px(1.));
    window.paint_quad(fill(outer, rgb(colors.background)).corner_radii(px(3.)));
    window.paint_quad(outline(outer, rgb(colors.line), BorderStyle::Solid).corner_radii(px(3.)));

    if model.is_empty() {
        let inset = px(4.);
        let empty = Bounds::new(
            point(outer.left() + inset, outer.top() + inset),
            size(
                (outer.size.width - inset * 2.).max(px(1.)),
                (outer.size.height - inset * 2.).max(px(1.)),
            ),
        );
        let mut color = rgb(colors.empty);
        color.a = if model.is_valid() { 0.32 } else { 0.18 };
        window.paint_quad(outline(empty, color, BorderStyle::Dashed).corner_radii(px(2.)));
        return;
    }

    let content = Bounds::new(
        point(outer.left() + px(2.), outer.top() + px(2.)),
        size(
            (outer.size.width - px(4.)).max(px(1.)),
            (outer.size.height - px(4.)).max(px(1.)),
        ),
    );
    let gap = px(1.);
    for pane in model.panes() {
        let pane_bounds = scaled_bounds(content, pane.rect, gap);
        window.paint_quad(fill(pane_bounds, rgb(colors.pane)).corner_radii(px(1.5)));
        window.paint_quad(
            outline(pane_bounds, rgb(colors.line), BorderStyle::Solid).corner_radii(px(1.5)),
        );
        if pane.tab_count > 1 && pane_bounds.size.height >= px(7.) {
            paint_tab_cue(*pane, pane_bounds, colors, window);
        }
    }
}

fn scaled_bounds(container: Bounds<Pixels>, rect: PreviewRect, gap: Pixels) -> Bounds<Pixels> {
    let x = container.left() + container.size.width * rect.x;
    let y = container.top() + container.size.height * rect.y;
    let width = container.size.width * rect.width;
    let height = container.size.height * rect.height;
    Bounds::new(
        point(x + gap / 2., y + gap / 2.),
        size((width - gap).max(px(1.)), (height - gap).max(px(1.))),
    )
}

fn paint_tab_cue(
    pane: PreviewPane,
    bounds: Bounds<Pixels>,
    colors: SavedLayoutPreviewColors,
    window: &mut gpui_kit::Window,
) {
    let count = pane.tab_count.min(3);
    let available = (bounds.size.width - px(3.)).max(px(1.));
    let marker_width = (available / count as f32).min(px(8.));
    for index in 0..count {
        let marker = Bounds::new(
            point(
                bounds.left() + px(1.5) + marker_width * index as f32,
                bounds.top() + px(1.5),
            ),
            size((marker_width - px(1.)).max(px(1.)), px(1.5)),
        );
        let color = if index == pane.active_tab.min(count - 1) {
            colors.active
        } else {
            colors.line
        };
        window.paint_quad(fill(marker, rgb(color)).corner_radii(px(0.75)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tab(ids: &[&str], active: usize) -> Value {
        json!({
            "panel_name": "TabPanel",
            "children": ids.iter().map(|id| json!({
                "panel_name": "MonitterNativePane",
                "children": [],
                "info": {"panel": {"id": id}}
            })).collect::<Vec<_>>(),
            "info": {"tabs": {"active_index": active}}
        })
    }

    #[test]
    fn split_geometry_preserves_axis_weights_and_tab_groups() {
        let layout = json!({
            "version": 1,
            "center": {
                "panel_name": "StackPanel",
                "children": [tab(&["chat:a", "chat:b"], 1), tab(&["terminal"], 0)],
                "info": {"stack": {"sizes": [300, 700], "axis": 0}}
            }
        });
        let model = SavedLayoutPreviewModel::try_from_saved_layout(&layout).unwrap();
        assert_eq!(model.panes.len(), 2);
        assert!((model.panes[0].rect.width - 0.3).abs() < 0.001);
        assert!((model.panes[1].rect.x - 0.3).abs() < 0.001);
        assert_eq!(
            (model.panes[0].tab_count, model.panes[0].active_tab),
            (2, 1)
        );
    }

    #[test]
    fn open_edge_docks_reserve_outer_geometry_but_closed_docks_do_not() {
        let layout = json!({
            "version": 1,
            "center": tab(&["chat:a"], 0),
            "left_dock": {"panel": tab(&["projects"], 0), "placement": "Left", "size": 240, "open": true},
            "right_dock": {"panel": tab(&["agents"], 0), "placement": "Right", "size": 240, "open": false},
            "bottom_dock": {"panel": tab(&["terminal"], 0), "placement": "Bottom", "size": 200, "open": true}
        });
        let model = SavedLayoutPreviewModel::try_from_saved_layout(&layout).unwrap();
        assert_eq!(model.panes.len(), 3);
        let center = model.panes[0].rect;
        assert!(
            center.x > 0. && center.x + center.width > 0.95,
            "closed right dock reserves no space"
        );
        assert!(center.height < 0.8, "open bottom dock reserves space");
        assert_eq!(model.panes[1].rect.x, 0.);
        assert!(model.panes[2].rect.y >= center.height);
    }

    #[test]
    fn malformed_empty_and_pathological_layouts_degrade_safely() {
        let malformed = SavedLayoutPreviewModel::from_saved_layout(&json!({"version": 1}));
        assert!(!malformed.is_valid());
        assert!(malformed.is_empty());

        let empty = SavedLayoutPreviewModel::try_from_saved_layout(&json!({
            "version": 1,
            "center": {"panel_name": "TabPanel", "children": [], "info": {"tabs": {"active_index": 9}}}
        })).unwrap();
        assert!(empty.is_valid());
        assert!(empty.is_empty());

        let legacy_empty = SavedLayoutPreviewModel::try_from_saved_layout(&json!({
            "version": 1,
            "center": {"panel_name": "TabPanel", "children": [], "info": {"panel": null}}
        }))
        .unwrap();
        assert!(legacy_empty.is_empty());

        let bad_sizes = SavedLayoutPreviewModel::try_from_saved_layout(&json!({
            "version": 1,
            "center": {
                "panel_name": "StackPanel",
                "children": [tab(&["a"], 0), tab(&["b"], 0)],
                "info": {"stack": {"sizes": [0, -4], "axis": 1}}
            }
        }))
        .unwrap();
        assert_eq!(bad_sizes.panes.len(), 2);
        assert!((bad_sizes.panes[0].rect.height - 0.5).abs() < 0.001);
        assert!((bad_sizes.panes[1].rect.y - 0.5).abs() < 0.001);
    }
}
