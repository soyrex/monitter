//! Explicit, pane-scoped tool approval presentation. The caller owns resolution.
use crate::design::{self, Palette, ui_scaled_px as px};
use gpui_kit::{
    assets::IconName,
    component::{Icon, Sizable},
};
use gpui_kit::{
    component::{Disableable, button::*},
    prelude::*,
    *,
};
use serde_json::Value;
use std::rc::Rc;
use crate::chat::feedback::{self, Clock, FeedbackClock};

struct TechnicalDetails {
    lines: Vec<String>,
    payload: SharedString,
    muted: u32,
    code: u32,
    clock: Clock,
}
impl Render for TechnicalDetails {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let progress = self.clock.borrow_mut().progress(cx.reduce_motion());
        feedback::painted(
            div().id("approval-expanded-details").flex().flex_col().gap_2()
                .opacity(0.65 + progress * 0.35)
                .children(self.lines.iter().map(|line| div().text_color(rgb(self.muted)).child(line.clone())))
                .child(div().p_2().bg(design::modal_surface_color(self.code)).rounded_md()
                    .font_family(design::FONT_MONO).child(self.payload.clone())),
            self.clock.clone(),
        )
    }
}

fn technical_details(
    id: SharedString, lines: Vec<String>, payload: SharedString, palette: Palette,
    entering: bool, window: &mut Window, cx: &mut App,
) -> Entity<TechnicalDetails> {
    let holder = window.use_keyed_state(id, cx, |_, cx| cx.new(|_| {
        let clock = FeedbackClock::new();
        if entering { clock.borrow_mut().start(std::time::Duration::from_millis(120)); }
        TechnicalDetails { lines: Vec::new(), payload: "".into(), muted: palette.muted, code: palette.code, clock }
    }));
    let details = holder.read(cx).clone();
    details.update(cx, |view, cx| {
        if view.lines != lines || view.payload != payload || view.muted != palette.muted || view.code != palette.code {
            view.lines = lines;
            view.payload = payload;
            view.muted = palette.muted;
            view.code = palette.code;
            cx.notify();
        }
    });
    details
}

pub type DecisionCallback = Rc<dyn Fn(&'static str, &mut Window, &mut App)>;
pub type DisclosureCallback = Rc<dyn Fn(&mut Window, &mut App)>;

fn text(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

/// Truncates on character boundaries; never dumps an unbounded provider payload.
fn bounded(value: &str, limit: usize) -> String {
    let mut chars = value.chars();
    let head: String = chars.by_ref().take(limit).collect();
    if chars.next().is_some() {
        format!("{head}\n… Preview truncated")
    } else {
        head
    }
}

/// Only Codex's explicit MCP tool approval extension advertises persistence.
/// Form questions and arbitrary MCP input never inherit these actions.
pub fn mcp_approval_choices(request: &Value) -> Vec<(&'static str, &'static str)> {
    if text(request, "provider") != "codex"
        || text(request, "status") != "pending"
        || text(&request["input"], "kind") != "form"
    {
        return Vec::new();
    }
    let schema = &request["input"]["schema"];
    if schema["type"] != "object"
        || !schema["properties"]
            .as_object()
            .is_some_and(|fields| fields.is_empty())
        || schema
            .get("required")
            .is_some_and(|names| !names.as_array().is_some_and(|names| names.is_empty()))
        || !schema.as_object().is_some_and(|schema| {
            schema
                .keys()
                .all(|key| matches!(key.as_str(), "type" | "properties" | "required" | "$schema"))
        })
        || schema.get("$schema").is_some_and(|dialect| {
            !dialect.as_str().is_some_and(|dialect| {
                matches!(
                    dialect,
                    "http://json-schema.org/draft-07/schema#"
                        | "https://json-schema.org/draft/2020-12/schema"
                )
            })
        })
    {
        return Vec::new();
    }
    let Ok(detail) = serde_json::from_str::<Value>(&text(request, "detail")) else {
        return Vec::new();
    };
    if text(&detail, "serverName").trim().is_empty()
        || text(&detail, "message").trim().is_empty()
        || !matches!(
            text(&detail, "mode").as_str(),
            "form" | "openai/form" | "openaiForm"
        )
        || detail["requestedSchema"] != *schema
        || detail["_meta"]["codex_approval_kind"] != "mcp_tool_call"
    {
        return Vec::new();
    }
    let supports = |scope: &str| {
        detail["_meta"]["persist"].as_str() == Some(scope)
            || detail["_meta"]["persist"]
                .as_array()
                .is_some_and(|values| values.iter().any(|value| value.as_str() == Some(scope)))
    };
    let mut choices = vec![("approve_once", "Allow once")];
    if supports("session") {
        choices.push(("approve_session", "Allow for this session"));
    }
    if supports("always") {
        choices.push(("approve_always", "Always allow this tool"));
    }
    choices.push(("deny", "Deny"));
    choices
}

pub fn allowed_decisions(request: &Value) -> Vec<(&'static str, &'static str)> {
    let mcp = mcp_approval_choices(request);
    if !mcp.is_empty() {
        return mcp;
    }
    if text(request, "status") != "pending" || !request["input"].is_null() {
        return Vec::new();
    }
    let mut choices = vec![("approve_once", "Approve once")];
    if text(request, "sessionScope") == "file_changes" {
        choices.push(("approve_session", "Allow all edits this session"));
    }
    if request["rememberable"].as_bool() == Some(true) {
        choices.push(("approve_always", "Always allow exact action"));
    }
    choices.push(("deny", "Deny"));
    choices
}

fn visible_decisions(request: &Value, native_mcp_supported: bool) -> Vec<(&'static str, &'static str)> {
    let mcp = !mcp_approval_choices(request).is_empty();
    allowed_decisions(request).into_iter().filter(|(decision, _)| {
        native_mcp_supported || !mcp || matches!(*decision, "approve_once" | "deny")
    }).collect()
}

fn named_scope(snapshot: &Value, collection: &str, id: &str) -> String {
    if id.is_empty() {
        return String::new();
    }
    snapshot[collection]
        .as_array()
        .and_then(|rows| rows.iter().find(|row| text(row, "id") == id))
        .map(|row| text(row, "name"))
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| id.to_owned())
}

pub fn render_approval_prompt(
    request: &Value,
    task: &Value,
    snapshot: &Value,
    palette: Palette,
    interface_family: SharedString,
    interface_size: Pixels,
    busy: bool,
    expanded: bool,
    native_mcp_supported: bool,
    on_decision: DecisionCallback,
    on_toggle_details: DisclosureCallback,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let (chevron, details_entering) = crate::chrome_motion::disclosure_chevron(
        SharedString::from(format!("approval-details-motion-{}", text(request, "id"))),
        expanded, window, cx,
    );
    let summary = text(request, "summary");
    let tool = text(request, "tool");
    let risk_color = if text(request, "risk") == "high" {
        0xb84c44
    } else {
        palette.accent
    };
    let mut body = div()
        .id("approval-dock-body")
        .debug_selector(|| "approval-dock-body".into())
        .min_h(px(0.))
        .max_h(px(220.))
        .overflow_y_scroll()
        .p_3()
        .flex()
        .flex_col()
        .gap_2()
        .child(div().font_weight(FontWeight::SEMIBOLD).child(
            if summary.trim().is_empty() || summary.trim_start().starts_with(['{', '[']) {
                format!("Allow {}?", bounded(&tool, 120))
            } else {
                bounded(&summary, 6000)
            },
        ));
    let detail = text(request, "detail");
    let parsed = serde_json::from_str::<Value>(&detail)
        .ok()
        .filter(|value| value.is_object());
    if !mcp_approval_choices(request).is_empty() {
        if let Some(detail) = parsed.as_ref() {
            body = body.child(div().text_color(rgb(palette.muted)).child(format!(
                "MCP server: {}",
                bounded(&text(detail, "serverName"), 120)
            )));
            let description = text(&detail["_meta"], "tool_description");
            if !description.trim().is_empty() {
                body = body.child(
                    div()
                        .text_color(rgb(palette.muted))
                        .child(bounded(&description, 1000)),
                );
            }
            if let Some(params) = detail["_meta"].get("tool_params") {
                if let Ok(preview) = serde_json::to_string_pretty(params) {
                    body = body.child(
                        div()
                            .p_2()
                            .rounded_md()
                            .bg(design::modal_surface_color(palette.code))
                            .font_family(design::FONT_MONO)
                            .child(bounded(&preview, 2000)),
                    );
                }
            }
            body = body.child(div().text_color(rgb(palette.muted)).child(if native_mcp_supported { "Session and always choices let Codex remember permission for this tool, so matching calls can continue without asking again." } else { "Session and always choices will be available after the pending backend update." }));
        }
    }
    let command = parsed
        .as_ref()
        .map(|value| match &value["command"] {
            Value::String(command) => command.clone(),
            Value::Array(parts) => parts
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(" "),
            _ => String::new(),
        })
        .unwrap_or_default();
    let reason = parsed
        .as_ref()
        .map(|value| text(value, "reason"))
        .unwrap_or_default();
    if !command.is_empty() {
        body = body.child(
            div()
                .p_2()
                .rounded_md()
                .border_1()
                .border_color(rgb(palette.line))
                .bg(design::modal_surface_color(palette.code))
                .font_family(design::FONT_MONO)
                .child(bounded(&command, 2000)),
        );
    }
    if !reason.is_empty() {
        body = body.child(
            div()
                .text_color(rgb(palette.muted))
                .child(bounded(&reason, 1000)),
        );
    }
    if parsed.is_none() && !detail.trim().is_empty() {
        body = body.child(
            div()
                .text_color(rgb(palette.muted))
                .child(bounded(&detail, 1000)),
        );
    }
    if text(request, "sessionScope") == "file_changes" {
        body = body.child(div().text_color(rgb(palette.muted)).child("Session approval covers every file edit in this chat until its harness stops. Commands and other permissions still ask separately."));
    }
    if request["rememberable"].as_bool() == Some(true) {
        body = body.child(div().text_color(rgb(palette.muted)).child("The exact-action option is saved for this agent, host and folder. Revoke it in Settings → Approvals."));
    }
    if !detail.is_empty() {
        body = body.child(
            Button::new("approval-request-details")
                .text_size(interface_size)
                .label(if expanded {
                    "Hide technical request details"
                } else {
                    "Technical request details"
                })
                .child(chevron)
                .on_click(move |_, window, cx| on_toggle_details(window, cx)),
        );
    }
    if expanded {
        let mut lines = Vec::new();
        for (label, value) in [
            (
                "Host",
                named_scope(snapshot, "hosts", &text(task, "hostId")),
            ),
            (
                "Project",
                named_scope(snapshot, "projects", &text(task, "projectId")),
            ),
            ("Folder", text(task, "cwd")),
        ] {
            if !value.is_empty() {
                lines.push(format!("{label}: {}", bounded(&value, 400)));
            }
        }
        let payload = parsed
            .as_ref()
            .and_then(|value| serde_json::to_string_pretty(value).ok())
            .unwrap_or(detail);
        body = body.child(technical_details(
            SharedString::from(format!("approval-details-content-{}", text(request, "id"))),
            lines, bounded(&payload, 6000).into(), palette, details_entering, window, cx,
        ));
    }
    let mut actions = div()
        .id("approval-dock-actions")
        .debug_selector(|| "approval-dock-actions".into())
        .flex()
        .flex_wrap()
        .items_center()
        .gap_2()
        .p_3()
        .border_t_1()
        .border_color(rgb(palette.line))
        .flex_shrink_0();
    for (decision, label) in visible_decisions(request, native_mcp_supported) {
        let callback = on_decision.clone();
        actions = actions.child(
            Button::new(SharedString::from(format!("approval-choice-{decision}")))
                .label(label)
                .icon(Icon::new(if decision == "deny" {
                    IconName::X
                } else {
                    IconName::Check
                }))
                .h(px(44.))
                .text_size(interface_size)
                .when(decision == "approve_once", |button| button.primary())
                .when(decision == "deny", |button| button.danger())
                .disabled(busy)
                .on_click(move |_, window, cx| callback(decision, window, cx)),
        );
    }
    if busy {
        actions = actions.child(
            div()
                .text_color(rgb(palette.muted))
                .child("Sending your response…"),
        );
    }
    let card = div()
        .id(SharedString::from(format!(
            "approval-prompt-{}",
            text(request, "id")
        )))
        .role(Role::Group)
        .aria_label("Pending approvals")
        .w_full()
        .flex_shrink_0()
        .rounded_lg()
        .border_1()
        .border_color(rgb(risk_color))
        .bg(design::modal_surface_color(palette.panel))
        .text_color(rgb(palette.ink))
        .font_family(interface_family)
        .text_size(interface_size)
        .flex()
        .flex_col()
        .overflow_hidden()
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .p_3()
                .border_b_1()
                .border_color(rgb(palette.line))
                .flex_shrink_0()
                .child(
                    Icon::new(IconName::TriangleAlert)
                        .size(px(18.))
                        .text_color(rgb(risk_color)),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child("Approval needed"),
                        )
                        .child(
                            div()
                                .text_sm()
                                .text_color(rgb(palette.muted))
                                .child(format!(
                                    "{} · {} · {} risk",
                                    text(request, "provider"),
                                    bounded(&tool, 120),
                                    text(request, "risk")
                                )),
                        ),
                ),
        )
        .child(body)
        .child(actions);
    card.into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn mcp_request(persist: Value) -> Value {
        json!({"status":"pending","provider":"codex","input":{"kind":"form","schema":{"type":"object","properties":{}}},
            "detail":json!({"mode":"form","message":"Allow tool?","requestedSchema":{"type":"object","properties":{}},"serverName":"test-mcp","_meta":{"codex_approval_kind":"mcp_tool_call","persist":persist}}).to_string()})
    }
    #[::core::prelude::v1::test]
    fn older_backend_retains_one_shot_approval_without_scoped_controls() {
        let request = mcp_request(json!(["session", "always"]));
        assert_eq!(visible_decisions(&request, false).iter().map(|choice| choice.0).collect::<Vec<_>>(), vec!["approve_once", "deny"]);
        assert_eq!(visible_decisions(&request, true).len(), 4);
        assert_eq!(visible_decisions(&json!({"status":"pending", "rememberable":true}), false).len(), 3);
    }
    #[::core::prelude::v1::test]
    fn mcp_choices_follow_native_advertised_scope() {
        let ids = |request: &Value| {
            allowed_decisions(request)
                .iter()
                .map(|choice| choice.0)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            ids(&mcp_request(json!(["session", "always"]))),
            vec!["approve_once", "approve_session", "approve_always", "deny"]
        );
        assert_eq!(
            ids(&mcp_request(json!("session"))),
            vec!["approve_once", "approve_session", "deny"]
        );
        assert_eq!(ids(&mcp_request(json!(null))), vec!["approve_once", "deny"]);
        let mut form = mcp_request(json!("always"));
        form["input"]["schema"]["properties"] = json!({"name":{"type":"string"}});
        assert!(ids(&form).is_empty());
        let mut generic = mcp_request(json!("always"));
        generic["detail"] = json!("{}");
        assert!(ids(&generic).is_empty());
        let mut resolved = mcp_request(json!("always"));
        resolved["status"] = json!("approved");
        assert!(ids(&resolved).is_empty());
    }
    #[::core::prelude::v1::test]
    fn choices_follow_exact_backend_capabilities() {
        let base = json!({"status":"pending"});
        assert_eq!(
            allowed_decisions(&base)
                .iter()
                .map(|v| v.0)
                .collect::<Vec<_>>(),
            vec!["approve_once", "deny"]
        );
        let scoped = json!({"status":"pending","sessionScope":"file_changes","rememberable":true});
        assert_eq!(
            allowed_decisions(&scoped)
                .iter()
                .map(|v| v.0)
                .collect::<Vec<_>>(),
            vec!["approve_once", "approve_session", "approve_always", "deny"]
        );
        assert_eq!(
            allowed_decisions(
                &json!({"status":"pending","sessionScope":"commands","rememberable":"true"})
            )
            .len(),
            2
        );
        assert!(allowed_decisions(&json!({"status":"approved"})).is_empty());
        assert!(allowed_decisions(&json!({"status":"pending","input":{"kind":"text"}})).is_empty());
    }
    #[::core::prelude::v1::test]
    fn payload_limit_preserves_unicode_and_reports_truncation() {
        assert_eq!(bounded("é🐱界", 2), "é🐱\n… Preview truncated");
        assert_eq!(bounded("é🐱", 2), "é🐱");
        assert!(bounded(&"界".repeat(10000), 6000).chars().count() < 6100);
    }
}

#[cfg(all(test, feature = "visual-export"))]
mod layout_tests {
    use super::render_approval_prompt;
    use gpui::{
        AppContext as _, Context, IntoElement, ParentElement as _, Render, Styled as _,
        TestAppContext, Window, div, px, size,
    };
    use std::rc::Rc;
    struct Fixture;
    impl Render for Fixture {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let request = serde_json::json!({"status":"pending","summary":"Long tool explanation. ".repeat(1000), "provider":"codex","tool":"Command execution","risk":"unknown","rememberable":true});
            let palette = crate::design::palette(
                &serde_json::json!({"theme":"dark"}),
                gpui::WindowAppearance::Dark,
            );
            div()
                .w_full()
                .flex()
                .flex_col()
                .child(render_approval_prompt(
                    &request,
                    &serde_json::Value::Null,
                    &serde_json::Value::Null,
                    palette,
                    "Helvetica".into(),
                    px(14.),
                    false,
                    false,
                    true,
                    Rc::new(|_, _, _| {}),
                    Rc::new(|_, _| {}),
                    window,
                    cx,
                ))
        }
    }
    #[gpui_kit::test]
    fn approval_dock_keeps_actions_below_scrollable_body(cx: &mut TestAppContext) {
        cx.update(|cx| gpui_kit::init(cx));
        let (_, visual) = cx.add_window_view(|window, cx| {
            let fixture = cx.new(|_| Fixture);
            gpui_kit::component::Root::new(fixture, window, cx)
        });
        visual.simulate_resize(size(crate::design::ui_scaled_px(400.), crate::design::ui_scaled_px(500.)));
        visual.run_until_parked();
        visual.update(|window, cx| window.draw(cx).clear(cx));
        let body = visual
            .debug_bounds("approval-dock-body")
            .expect("body rendered");
        let actions = visual
            .debug_bounds("approval-dock-actions")
            .expect("actions rendered");
        assert!(body.size.height <= crate::design::ui_scaled_px(221.), "body height {:?}, expected <= {:?}", body.size.height, crate::design::ui_scaled_px(221.));
        assert!(
            actions.top() >= body.bottom(),
            "decisions are outside the scroller"
        );
        assert!(
            actions.bottom() <= crate::design::ui_scaled_px(500.),
            "decisions remain visible in pane"
        );
    }
}
