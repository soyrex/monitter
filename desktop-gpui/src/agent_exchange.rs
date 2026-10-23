//! Recorded exchange presentation. Grouped diagrams own one stable virtual item;
//! source members remain available for bodies, anchors and disclosures.
use crate::design::{self, Palette, ui_scaled_px as px};
use gpui_kit::{
    assets::IconName,
    component::{Icon, Sizable, button::*},
    prelude::*,
    *,
};
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Direction {
    Incoming,
    Outgoing,
    Unknown,
    Mixed,
}
#[derive(Clone)]
pub(crate) struct Exchange {
    pub direction: Direction,
    pub peer: String,
    pub status: String,
    pub timestamp: Option<i64>,
    pub steps: Vec<(i64, String)>,
}
fn text(v: &Value, key: &str) -> String {
    v[key].as_str().unwrap_or_default().to_owned()
}
pub(crate) fn project(
    record: Option<&Value>,
    task: &str,
    sender: &str,
    name: Option<&str>,
    timestamp: Option<i64>,
) -> Exchange {
    let direction = match record {
        Some(r) if r["fromTaskId"].as_str() == Some(task) => Direction::Outgoing,
        Some(r) if r["toTaskId"].as_str() == Some(task) => Direction::Incoming,
        _ if !sender.is_empty() => Direction::Incoming,
        _ => Direction::Unknown,
    };
    Exchange {
        direction,
        peer: name
            .filter(|s| !s.trim().is_empty())
            .unwrap_or("Unknown peer")
            .into(),
        status: record
            .map(|r| text(r, "status"))
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "Recorded message".into()),
        timestamp: timestamp.filter(|t| *t > 0),
        steps: Vec::new(),
    }
}
fn in_flight(status: &str) -> bool {
    matches!(
        status,
        "queued"
            | "running"
            | "Collaboration queued"
            | "Collaboration running"
            | "Peer delivery started"
    )
}
/// Unfilled 28px route marker; the whole row is the disclosure hit target.
pub(crate) fn render(
    exchange: Exchange,
    id: String,
    toggle_selector: String,
    status_selector: String,
    expanded: bool,
    palette: Palette,
    details: AnyElement,
    toggle: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Div {
    let hide_status = status_selector.starts_with("exchange-group-status-");
    let blue = rgb(palette.agent());
    let failed = matches!(
        exchange.status.as_str(),
        "error" | "failed" | "interrupted" | "Collaboration error"
    );
    let inflight = in_flight(&exchange.status);
    let completed = matches!(
        exchange.status.as_str(),
        "completed" | "succeeded" | "Collaboration completed"
    );
    let ink = if failed { rgb(palette.err()) } else { blue };
    let first_dot_selector = format!("exchange-leading-dot-{id}");
    let endpoint = |filled: bool| {
        div()
            .size(px(7.))
            .rounded_full()
            .border_1()
            .border_color(ink)
            .when(filled, |dot| dot.bg(ink))
    };
    let glyph = div()
        .flex_shrink_0()
        .flex()
        .items_center()
        .gap(px(1.))
        .child(
            endpoint(exchange.direction == Direction::Outgoing)
                .debug_selector(move || first_dot_selector.clone()),
        )
        .child(
            div()
                .w(px(5.))
                .border_t_1()
                .border_color(ink)
                .when(inflight || failed, |line| line.border_dashed()),
        )
        .child(if exchange.direction == Direction::Mixed {
            div()
                .w(px(10.))
                .text_size(px(10.))
                .text_color(ink)
                .child("↔")
                .into_any_element()
        } else {
            Icon::new(if exchange.direction == Direction::Unknown {
                IconName::Minus
            } else {
                IconName::ArrowRight
            })
            .size(px(10.))
            .text_color(ink)
            .into_any_element()
        })
        .child(endpoint(matches!(
            exchange.direction,
            Direction::Incoming | Direction::Mixed
        )));
    let route = match exchange.direction {
        Direction::Incoming => format!("{} → this pane", exchange.peer),
        Direction::Outgoing => format!("This pane → {}", exchange.peer),
        Direction::Unknown => "Direction unknown".into(),
        Direction::Mixed => format!(
            "Recorded exchanges in both directions with {}",
            exchange.peer
        ),
    };
    let direction = match exchange.direction {
        Direction::Incoming => "incoming",
        Direction::Outgoing => "outgoing",
        Direction::Unknown => "unknown",
        Direction::Mixed => "mixed",
    };
    let direction_selector = format!("exchange-direction-{id}-{direction}");
    let peer_selector = format!("exchange-peer-{id}-{}", exchange.peer);
    let time = exchange
        .timestamp
        .and_then(crate::timestamps::local_datetime)
        .map(|v| v.format("%H:%M").to_string());
    let mut root = div().w_full().min_w_0().flex().flex_col().child(
        div()
            .w_full()
            .min_w_0()
            .debug_selector(move || toggle_selector.clone())
            .child(
                Button::new(SharedString::from(format!("exchange-toggle-{id}")))
                    .ghost()
                    .compact()
                    .w_full()
                    .h(px(28.))
                    .px(px(0.))
                    .accessibility_label(format!(
                        "{} exchange details: {route}",
                        if expanded { "Hide" } else { "Show" }
                    ))
                    .child(
                        div()
                            .w_full()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .text_size(px(12.))
                            .font_weight(FontWeight::NORMAL)
                            .text_color(rgb(palette.text(2)))
                            .child(
                                div()
                                    .debug_selector(move || direction_selector.clone())
                                    .child(glyph),
                            )
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .debug_selector(move || peer_selector.clone())
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(rgb(palette.text(1)))
                                    .child(exchange.peer),
                            )
                            .when(!hide_status, |row| {
                                row.child(
                                    div()
                                        .min_w(px(28.))
                                        .max_w(px(160.))
                                        .truncate()
                                        .debug_selector(move || status_selector.clone())
                                        .text_color(if failed { ink } else { rgb(palette.text(2)) })
                                        .child(exchange.status),
                                )
                            })
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(4.))
                                    .border_t_1()
                                    .border_dashed()
                                    .border_color(blue.opacity(0.28)),
                            )
                            .when_some(time, |row, time| {
                                row.child(
                                    div()
                                        .flex_shrink_0()
                                        .font_family(design::FONT_MONO)
                                        .text_size(px(11.))
                                        .text_color(rgb(palette.text(3)))
                                        .child(time),
                                )
                            })
                            .child(Icon::new(if expanded { IconName::ChevronDown } else { IconName::ChevronRight }).size(px(12.)).text_color(rgb(palette.text(3)))),
                    )
                    .on_click(toggle),
            ),
    );
    if expanded {
        let content_selector = format!("exchange-content-{id}");
        let mut content = div()
            .w_full()
            .min_w_0()
            .px(px(12.))
            .pt(px(10.))
            .pb(px(12.))
            .mt(px(6.))
            .mb(px(10.))
            .rounded(px(8.))
            .border_1()
            .border_color(blue.opacity(0.12))
            .bg(rgb(crate::design::mix_rgb(palette.paper, palette.agent(), 0.05)))
            .debug_selector(move || content_selector.clone())
            .flex()
            .flex_col()
            .gap(px(8.))
            .text_size(px(12.));
        let steps: Vec<_> = exchange
            .steps
            .into_iter()
            .filter_map(|(time, label)| {
                crate::timestamps::local_datetime(time).map(|time| (time, label))
            })
            .collect();
        let count = steps.len();
        for (index, (time, label)) in steps.into_iter().enumerate() {
            let last = index + 1 == count;
            let node = if last && failed {
                ink
            } else if last && completed {
                rgb(palette.ok())
            } else {
                blue
            };
            content = content.child(
                div()
                    .w_full()
                    .min_w_0()
                    .flex()
                    .gap(px(8.))
                    .child(
                        div()
                            .relative()
                            .w(px(12.))
                            .min_h(px(24.))
                            .flex_shrink_0()
                            .when(!last, |rail| {
                                rail.child(
                                    div()
                                        .absolute()
                                        .left(px(5.))
                                        .top(px(12.))
                                        .bottom(px(-13.))
                                        .w(px(1.))
                                        .bg(blue.opacity(0.28)),
                                )
                            })
                            .child(
                                div()
                                    .absolute()
                                    .left(px(2.))
                                    .top(px(5.))
                                    .size(px(9.))
                                    .rounded_full()
                                    .border_1()
                                    .border_color(node)
                                    .when(index > 0, |dot| dot.bg(node)),
                            ),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .whitespace_normal()
                            .text_color(rgb(palette.text(1)))
                            .child(label),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .font_family(design::FONT_MONO)
                            .text_size(px(11.))
                            .text_color(rgb(palette.text(3)))
                            .child(time.format("%H:%M:%S").to_string()),
                    ),
            );
        }
        root = root.child(content.child(details));
    }
    root
}
/// A delivery may omit senderAgentId while its recorded route still names the peer.
pub(crate) fn peer_agent_id(record: Option<&Value>, task: &str, sender: &str) -> String {
    if !sender.is_empty() {
        return sender.into();
    }
    record
        .and_then(|record| {
            if record["fromTaskId"].as_str() == Some(task) {
                record["toAgentId"].as_str()
            } else if record["toTaskId"].as_str() == Some(task) {
                record["fromAgentId"].as_str()
            } else {
                None
            }
        })
        .unwrap_or_default()
        .into()
}
/// Preserve observed failure/in-flight facts in a mixed run's collapsed glyph.
pub(crate) fn aggregate(exchanges: &[Exchange]) -> Option<Exchange> {
    let mut summary = exchanges.last()?.clone();
    if exchanges
        .iter()
        .any(|row| row.direction != summary.direction)
    {
        summary.direction = Direction::Mixed;
    }
    let rank = |status: &str| {
        if matches!(
            status,
            "error" | "failed" | "interrupted" | "Collaboration error"
        ) {
            2
        } else if in_flight(status) {
            1
        } else {
            0
        }
    };
    if let Some(state) = exchanges.iter().max_by_key(|row| rank(&row.status)) {
        summary.status = state.status.clone();
    }
    summary.steps.clear();
    Some(summary)
}
/// The same outer marker used by grouped transcript rows and offline previews.
pub(crate) fn render_group(
    mut summary: Exchange,
    count: usize,
    id: String,
    expanded: bool,
    palette: Palette,
    details: AnyElement,
    toggle: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Div {
    if count > 0 {
        summary.peer = format!(
            "{} · {count} {}",
            summary.peer,
            if count == 1 { "exchange" } else { "exchanges" }
        );
    }
    summary.steps.clear();
    render(
        summary,
        id.clone(),
        format!("exchange-group-toggle-{id}"),
        format!("exchange-group-status-{id}"),
        expanded,
        palette,
        details,
        toggle,
    )
    .debug_selector(move || format!("exchange-group-{id}"))
}
/// The group owns the rail so member gaps and nested tool turns cannot break it.
/// Its 12px marker column also centers the native Turn icon on the same axis.
pub(crate) fn timeline(id: &str) -> Div {
    timeline_ending_at(id, None)
}

/// A terminal Turn ends the rail at its icon, even when its details expand.
pub(crate) fn timeline_ending_at(
    id: &str,
    endpoint: Option<crate::activity_sparkles::PaintBounds>,
) -> Div {
    let timeline_selector = format!("exchange-timeline-{id}");
    let rail_selector = format!("exchange-rail-{id}");
    div()
        .relative()
        .w_full()
        .min_w_0()
        .flex()
        .flex_col()
        .gap(px(12.))
        .debug_selector(move || timeline_selector.clone())
        .when_some(endpoint.clone(), |this, endpoint| {
            this.child(
                canvas(
                    |_, _, _| (),
                    move |bounds, _, window, _| {
                        // Descendant prepaint records this frame's actual icon position.
                        let bottom = endpoint
                            .get()
                            .map(|icon| icon.center().y)
                            .unwrap_or(bounds.top())
                            .clamp(bounds.top(), bounds.bottom());
                        let top = bounds.top() + px(9.);
                        if bottom > top {
                            window.paint_quad(fill(
                                Bounds::new(
                                    point(bounds.left() + px(5.5), top),
                                    size(px(1.), bottom - top),
                                ),
                                rgb(palette.agent()).opacity(0.25),
                            ));
                        }
                    },
                )
                .absolute()
                .inset_0(),
            )
        })
        .when(endpoint.is_none(), |this| {
            this.child(
                div()
                    .absolute()
                    .left(px(5.5))
                    .top(px(9.))
                    .bottom(px(0.))
                    .w(px(1.))
                    .bg(rgb(palette.agent()).opacity(0.25))
                    .debug_selector(move || rail_selector.clone()),
            )
        })
}
/// Each member keeps its own direction, observed label and second-resolution time.
pub(crate) fn render_step(
    exchange: Exchange,
    id: String,
    palette: Palette,
    body: AnyElement,
) -> Div {
    let arrow = match exchange.direction {
        Direction::Incoming => "←",
        Direction::Outgoing => "→",
        Direction::Unknown => "?",
        Direction::Mixed => "↔",
    };
    let time = exchange
        .timestamp
        .and_then(crate::timestamps::local_datetime)
        .map(|v| v.format("%H:%M:%S").to_string());
    let dot_selector = format!("exchange-member-dot-{id}");
    let heading_selector = format!("exchange-member-heading-{id}");
    div()
        .w_full()
        .min_w_0()
        .flex()
        .gap(px(10.))
        .debug_selector(move || format!("exchange-member-{id}"))
        .child(
            div().relative().w(px(12.)).flex_shrink_0().child(
                div()
                    .absolute()
                    .left(px(3.))
                    .top(px(6.))
                    .size(px(6.))
                    .rounded_full()
                    .bg(rgb(palette.agent()))
                    .debug_selector(move || dot_selector.clone()),
            ),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(6.))
                .child(
                    div()
                        .w_full()
                        .min_w_0()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(
                            div()
                                .min_w_0()
                                .flex_1()
                                .whitespace_normal()
                                .line_height(px(18.))
                                .text_color(rgb(palette.muted))
                                .debug_selector(move || heading_selector.clone())
                                .child(format!("{arrow} {} · {}", exchange.peer, exchange.status)),
                        )
                        .when_some(time, |row, time| {
                            row.child(
                                div()
                                    .flex_shrink_0()
                                    .font_family(design::FONT_MONO)
                                    .text_size(px(10.))
                                    .child(time),
                            )
                        }),
                )
                .child(body),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[::core::prelude::v1::test]
    fn exchange_group_senderless_summary_resolves_recorded_peer() {
        let record = serde_json::json!({"fromTaskId":"here","toTaskId":"peer-task","fromAgentId":"main","toAgentId":"codex"});
        assert_eq!(peer_agent_id(Some(&record), "here", ""), "codex");
        assert_eq!(peer_agent_id(Some(&record), "peer-task", ""), "main");
        assert_eq!(peer_agent_id(Some(&record), "unrelated", ""), "");
        assert_eq!(
            peer_agent_id(Some(&record), "here", "reported-sender"),
            "reported-sender"
        );
    }
    #[::core::prelude::v1::test]
    fn exchange_group_summary_preserves_failure_and_mixed_directions() {
        let row = |direction, status: &str| Exchange {
            direction,
            peer: "Codex".into(),
            status: status.into(),
            timestamp: Some(1),
            steps: vec![],
        };
        let summary = aggregate(&[
            row(Direction::Outgoing, "Collaboration completed"),
            row(Direction::Incoming, "Peer delivery started"),
        ])
        .unwrap();
        assert_eq!(summary.direction, Direction::Mixed);
        assert_eq!(summary.status, "Peer delivery started");
        let summary =
            aggregate(&[summary, row(Direction::Outgoing, "Collaboration error")]).unwrap();
        assert_eq!(summary.status, "Collaboration error");
    }
    #[::core::prelude::v1::test]
    fn recorded_peer_delivery_started_is_in_flight() {
        assert!(in_flight("Peer delivery started"));
        assert!(!in_flight("Collaboration completed"));
        assert!(!in_flight("Collaboration error"));
    }
    #[::core::prelude::v1::test]
    fn direction_requires_recorded_identity() {
        assert_eq!(
            project(None, "here", "", None, None).direction,
            Direction::Unknown
        );
        assert_eq!(
            project(None, "here", "sender", None, None).direction,
            Direction::Incoming
        );
        assert_eq!(
            project(
                Some(&json!({"fromTaskId":"here","toTaskId":"there"})),
                "here",
                "",
                None,
                None
            )
            .direction,
            Direction::Outgoing
        );
        assert_eq!(project(None, "here", "", None, None).peer, "Unknown peer");
    }
}
