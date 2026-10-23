//! An inset outline of the pane body joined to its selected tab.
use gpui::{Bounds, Pixels, Point, point, px};

pub(crate) fn contour(
    pane: Bounds<Pixels>,
    tab: Option<Bounds<Pixels>>,
    stroke_width: Pixels,
) -> Vec<Point<Pixels>> {
    if pane.size.width < px(1.) || pane.size.height < px(1.) {
        return Vec::new();
    }
    let inset = (stroke_width / 2.)
        .min(pane.size.width / 2.)
        .min(pane.size.height / 2.);
    let left = pane.origin.x + inset;
    let right = pane.right() - inset;
    let top = pane.origin.y + inset;
    let bottom = pane.bottom() - inset;
    let Some(tab) = tab.filter(|tab| tab.size.height > px(0.)) else {
        return vec![
            point(left, top),
            point(right, top),
            point(right, bottom),
            point(left, bottom),
        ];
    };
    let body_top = (tab.bottom() - inset).max(top).min(bottom);
    let tab_left = (tab.origin.x + inset).max(left).min(right);
    let tab_right = (tab.right() - inset).max(left).min(right);
    if tab_right <= tab_left {
        return vec![
            point(left, body_top),
            point(right, body_top),
            point(right, bottom),
            point(left, bottom),
        ];
    }
    let mut points = Vec::new();
    for p in [
        point(left, bottom),
        point(left, body_top),
        point(tab_left, body_top),
        point(tab_left, top),
        point(tab_right, top),
        point(tab_right, body_top),
        point(right, body_top),
        point(right, bottom),
    ] {
        if points.last() != Some(&p) {
            points.push(p);
        }
    }
    points
}

/// Paint outside the contour, preserving tab cutouts and rounded corners.
/// Static one-pixel bands need no blur texture, timer, or content invalidation.
pub(crate) fn paint_outward_glow(
    contour: &[Point<Pixels>],
    mut color: gpui::Rgba,
    window: &mut gpui::Window,
) {
    if contour.len() < 3 {
        return;
    }
    let points: Vec<(f32, f32)> = contour.iter().map(|p| (p.x.into(), p.y.into())).collect();
    let area: f32 = points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .take(points.len())
        .map(|(a, b)| a.0 * b.1 - b.0 * a.1)
        .sum();
    let direction = if area >= 0. { -1. } else { 1. };
    let strength = color.a;
    for band in 0..20 {
        let distance = band as f32 + 0.5;
        color.a = strength * 0.18 * (1. - distance / 20.).powi(2);
        let mut path = gpui::PathBuilder::stroke(gpui::px(1.));
        for i in 0..points.len() {
            let previous = points[(i + points.len() - 1) % points.len()];
            let current = points[i];
            let next = points[(i + 1) % points.len()];
            let normal = |a: (f32, f32), b: (f32, f32)| {
                let (dx, dy) = (b.0 - a.0, b.1 - a.1);
                let length = dx.hypot(dy).max(0.001);
                (-dy / length * direction, dx / length * direction)
            };
            let a = normal(previous, current);
            let b = normal(current, next);
            let divisor = (1. + a.0 * b.0 + a.1 * b.1).max(0.1);
            let p = gpui::point(
                gpui::px(current.0 + (a.0 + b.0) * distance / divisor),
                gpui::px(current.1 + (a.1 + b.1) * distance / divisor),
            );
            if i == 0 {
                path.move_to(p);
            } else {
                path.line_to(p);
            }
        }
        path.close();
        if let Ok(path) = path.build() {
            window.paint_path(path, color);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::size;
    fn pane() -> Bounds<Pixels> {
        Bounds::new(point(px(20.), px(10.)), size(px(500.), px(400.)))
    }
    #[test]
    fn pane_highlight_contour_follows_first_tab_cutout() {
        let tab = Bounds::new(point(px(20.), px(10.)), size(px(180.), px(34.)));
        let points = contour(pane(), Some(tab), px(1.));
        assert!(points.contains(&point(px(199.5), px(10.5))));
        assert!(points.contains(&point(px(199.5), px(43.5))));
        assert!(points.contains(&point(px(519.5), px(43.5))));
        assert!(!points.contains(&point(px(519.5), px(10.5))));
    }
    #[test]
    fn pane_highlight_contour_tracks_middle_and_clipped_tabs() {
        let tab = Bounds::new(point(px(180.), px(10.)), size(px(180.), px(34.)));
        let points = contour(pane(), Some(tab), px(1.));
        assert!(points.contains(&point(px(180.5), px(43.5))));
        assert!(points.contains(&point(px(180.5), px(10.5))));
        let clipped = Bounds::new(point(px(-40.), px(10.)), size(px(180.), px(34.)));
        assert!(
            contour(pane(), Some(clipped), px(1.))
                .iter()
                .all(|p| p.x >= px(20.5) && p.x <= px(519.5))
        );
    }
    #[test]
    fn pane_highlight_contour_without_tabs_is_inset_rectangle() {
        assert_eq!(
            contour(pane(), None, px(1.)),
            vec![
                point(px(20.5), px(10.5)),
                point(px(519.5), px(10.5)),
                point(px(519.5), px(409.5)),
                point(px(20.5), px(409.5))
            ]
        );
    }

    #[test]
    fn thick_pane_highlight_stays_inside_the_pane() {
        assert_eq!(
            contour(pane(), None, px(8.)),
            vec![
                point(px(24.), px(14.)),
                point(px(516.), px(14.)),
                point(px(516.), px(406.)),
                point(px(24.), px(406.))
            ]
        );
    }
}
