//! Explicit, pane-scoped tool approval presentation. The caller owns resolution.
use crate::design::{self, Palette, ui_scaled_px as px};
use gpui_kit::{component::{Disableable, button::*}, prelude::*, *};
use gpui_kit::{assets::IconName, component::{Icon, Sizable}};
use serde_json::Value;
use std::rc::Rc;

pub type DecisionCallback = Rc<dyn Fn(&'static str, &mut Window, &mut App)>;
pub type DisclosureCallback = Rc<dyn Fn(&mut Window, &mut App)>;

fn text(value: &Value, key: &str) -> String {
    value.get(key).and_then(Value::as_str).unwrap_or_default().to_owned()
}

/// Truncates on character boundaries; never dumps an unbounded provider payload.
fn bounded(value: &str, limit: usize) -> String {
    let mut chars = value.chars();
    let head: String = chars.by_ref().take(limit).collect();
    if chars.next().is_some() { format!("{head}\n… Preview truncated") } else { head }
}

pub fn allowed_decisions(request: &Value) -> Vec<(&'static str, &'static str)> {
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

fn named_scope(snapshot: &Value, collection: &str, id: &str) -> String {
    if id.is_empty() { return String::new(); }
    snapshot[collection].as_array().and_then(|rows| rows.iter().find(|row| text(row, "id") == id))
        .map(|row| text(row, "name")).filter(|name| !name.is_empty()).unwrap_or_else(|| id.to_owned())
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
    on_decision: DecisionCallback,
    on_toggle_details: DisclosureCallback,
) -> AnyElement {
    let summary = text(request, "summary");
    let tool = text(request, "tool");
    let mut card = div().id(SharedString::from(format!("approval-prompt-{}", text(request,"id"))))
        .w_full().max_w(px(680.)).p_4().rounded_lg().border_1()
        .border_color(rgb(if text(request,"risk") == "high" { 0xb84c44 } else { palette.accent }))
        .bg(design::modal_surface_color(palette.panel)).text_color(rgb(palette.ink))
        .font_family(interface_family).text_size(interface_size).flex().flex_col().gap_3()
        .shadow(vec![BoxShadow::new(px(0.),px(6.),rgba(0x00000033).into()).blur_radius(px(20.))])
        .child(div().flex().items_center().gap_2()
            .child(Icon::new(IconName::TriangleAlert).size(px(18.)).text_color(rgb(if text(request,"risk") == "high" { 0xb84c44 } else { palette.accent })))
            .child(div().font_weight(FontWeight::SEMIBOLD).child("Approval needed")))
        .child(div().child(if summary.trim().is_empty() || summary.trim_start().starts_with(['{','[']) {
            format!("Allow {}?", bounded(&tool,120))
        } else { bounded(&summary,180).replace("\n… Preview truncated", "…") }))
        .child(Button::new("approval-request-details").text_size(interface_size)
            .label(if expanded { "Hide details" } else { "Show details" })
            .on_click(move |_,window,cx| on_toggle_details(window,cx)));
    if expanded {
    let detail = text(request, "detail");
    let parsed = serde_json::from_str::<Value>(&detail).ok().filter(|value| value.is_object() || value.is_array());
    let command = parsed.as_ref().map(|value| match &value["command"] {
        Value::String(command) => command.clone(),
        Value::Array(parts) => parts.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" "),
        _ => String::new(),
    }).unwrap_or_default();
    let reason = parsed.as_ref().map(|value| text(value, "reason")).unwrap_or_default();
        card = card.child(div().text_color(rgb(palette.muted)).child(format!("{} · {} · {} risk", text(request,"provider"), bounded(&tool,120),text(request,"risk"))));
        if summary.chars().count() > 180 || summary.trim_start().starts_with(['{','[']) {
            card = card.child(div().child(bounded(&summary,6000)));
        }
    for (label, value) in [
        ("Host", named_scope(snapshot,"hosts",&text(task,"hostId"))),
        ("Project", named_scope(snapshot,"projects",&text(task,"projectId"))),
        ("Folder", text(task,"cwd")),
    ] {
        if !value.is_empty() { card = card.child(div().text_color(rgb(palette.muted)).child(format!("{label}: {}", bounded(&value,400)))); }
    }
    if !command.is_empty() {
        card = card.child(div().p_3().rounded_md().border_1().border_color(rgb(palette.line))
            .bg(design::modal_surface_color(palette.code))
            .child(bounded(&command,2000)));
    }
    if !reason.is_empty() { card = card.child(div().child(bounded(&reason,1000))); }
    if parsed.is_none() && !detail.is_empty() { card = card.child(div().child(bounded(&detail,1000))); }
    if parsed.as_ref().is_some_and(|value| !value["permissions"].is_null()) {
        card = card.child(div().text_color(rgb(palette.muted)).child("Additional permissions requested; review request details below."));
    }
    if parsed.as_ref().is_some_and(|value| !value["changes"].is_null()) {
        card = card.child(div().text_color(rgb(palette.muted)).child("Proposed file changes are included in request details."));
    }
    if !detail.is_empty() {
        let payload = parsed.as_ref().and_then(|value| serde_json::to_string_pretty(value).ok()).unwrap_or(detail);
        card = card.child(div().id("approval-payload").max_h(px(180.)).overflow_y_scroll().p_2()
            .bg(design::modal_surface_color(palette.code)).rounded_md()
            .text_size(interface_size).child(bounded(&payload,6000)));
    }
    if text(request,"sessionScope") == "file_changes" {
        card = card.child(div().text_color(rgb(palette.muted)).child("Session approval allows all file edits in this chat until its harness stops. Other permissions still ask separately."));
    }
    if request["rememberable"].as_bool() == Some(true) {
        card = card.child(div().text_color(rgb(palette.muted)).child("Exact-action approval saves a rule for this agent, host and folder. Revoke it in Settings → Approvals."));
    }
    }
    let mut actions = div().flex().flex_wrap().gap_2();
    for (decision,label) in allowed_decisions(request) {
        let callback = on_decision.clone();
        actions = actions.child(Button::new(SharedString::from(format!("approval-choice-{decision}"))).label(label).h(px(44.)).text_size(interface_size)
            .when(decision == "approve_once", |button| button.primary())
            .when(decision == "deny", |button| button.danger())
            .disabled(busy).on_click(move |_,window,cx| callback(decision,window,cx)));
    }
    card = card.child(actions);
    if busy { card = card.child(div().text_color(rgb(palette.muted)).child("Resolving…")); }
    card.into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[::core::prelude::v1::test]
    fn choices_follow_exact_backend_capabilities() {
        let base = json!({"status":"pending"});
        assert_eq!(allowed_decisions(&base).iter().map(|v|v.0).collect::<Vec<_>>(), vec!["approve_once","deny"]);
        let scoped = json!({"status":"pending","sessionScope":"file_changes","rememberable":true});
        assert_eq!(allowed_decisions(&scoped).iter().map(|v|v.0).collect::<Vec<_>>(),vec!["approve_once","approve_session","approve_always","deny"]);
        assert_eq!(allowed_decisions(&json!({"status":"pending","sessionScope":"commands","rememberable":"true"})).len(),2);
        assert!(allowed_decisions(&json!({"status":"approved"})).is_empty());
        assert!(allowed_decisions(&json!({"status":"pending","input":{"kind":"text"}})).is_empty());
    }
    #[::core::prelude::v1::test]
    fn payload_limit_preserves_unicode_and_reports_truncation() {
        assert_eq!(bounded("é🐱界",2),"é🐱\n… Preview truncated");
        assert_eq!(bounded("é🐱",2),"é🐱");
        assert!(bounded(&"界".repeat(10000),6000).chars().count() < 6100);
    }
}
