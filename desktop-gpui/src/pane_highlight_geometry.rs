//! A one-pixel inset outline of the pane body joined to its selected tab.
use gpui::{Bounds, Pixels, Point, point, px};

pub(crate) fn contour(pane: Bounds<Pixels>, tab: Option<Bounds<Pixels>>) -> Vec<Point<Pixels>> {
    if pane.size.width < px(1.) || pane.size.height < px(1.) { return Vec::new(); }
    let left = pane.origin.x + px(0.5);
    let right = pane.right() - px(0.5);
    let top = pane.origin.y + px(0.5);
    let bottom = pane.bottom() - px(0.5);
    let Some(tab) = tab.filter(|tab| tab.size.height > px(0.)) else {
        return vec![point(left, top), point(right, top), point(right, bottom), point(left, bottom)];
    };
    let body_top = (tab.bottom() - px(0.5)).max(top).min(bottom);
    let tab_left = (tab.origin.x + px(0.5)).max(left).min(right);
    let tab_right = (tab.right() - px(0.5)).max(left).min(right);
    if tab_right <= tab_left {
        return vec![point(left, body_top), point(right, body_top), point(right, bottom), point(left, bottom)];
    }
    let mut points = Vec::new();
    for p in [point(left, bottom), point(left, body_top), point(tab_left, body_top),
        point(tab_left, top), point(tab_right, top), point(tab_right, body_top),
        point(right, body_top), point(right, bottom)] {
        if points.last() != Some(&p) { points.push(p); }
    }
    points
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::size;
    fn pane() -> Bounds<Pixels> { Bounds::new(point(px(20.), px(10.)), size(px(500.), px(400.))) }
    #[test]
    fn pane_highlight_contour_follows_first_tab_cutout() {
        let tab = Bounds::new(point(px(20.), px(10.)), size(px(180.), px(34.)));
        let points = contour(pane(), Some(tab));
        assert!(points.contains(&point(px(199.5), px(10.5))));
        assert!(points.contains(&point(px(199.5), px(43.5))));
        assert!(points.contains(&point(px(519.5), px(43.5))));
        assert!(!points.contains(&point(px(519.5), px(10.5))));
    }
    #[test]
    fn pane_highlight_contour_tracks_middle_and_clipped_tabs() {
        let tab = Bounds::new(point(px(180.), px(10.)), size(px(180.), px(34.)));
        let points = contour(pane(), Some(tab));
        assert!(points.contains(&point(px(180.5), px(43.5))));
        assert!(points.contains(&point(px(180.5), px(10.5))));
        let clipped = Bounds::new(point(px(-40.), px(10.)), size(px(180.), px(34.)));
        assert!(contour(pane(), Some(clipped)).iter().all(|p| p.x >= px(20.5) && p.x <= px(519.5)));
    }
    #[test]
    fn pane_highlight_contour_without_tabs_is_inset_rectangle() {
        assert_eq!(contour(pane(), None), vec![point(px(20.5), px(10.5)),
            point(px(519.5), px(10.5)), point(px(519.5), px(409.5)), point(px(20.5), px(409.5))]);
    }
}
