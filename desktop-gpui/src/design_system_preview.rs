//! Explicit development-only synthetic surfaces. Never attach a live service.
use crate::design;
use gpui_kit::{component::{ActiveTheme as _, Theme, ThemeMode, button::*, input::{Input, InputState}, select::{Select, SelectState}}, prelude::*, *};
use serde_json::json;

struct Preview { picker: Option<AnyView>, mode: String, name: Entity<InputState>, choice: Entity<SelectState<Vec<SharedString>>> }
impl Render for Preview {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = design::palette(&json!({"theme":if self.mode.ends_with("light") { "light" } else { "dark" }}), window.appearance());
        let mut root = div().size_full().relative().bg(rgb(palette.paper)).text_color(rgb(palette.ink))
            .font_family(cx.theme().font_family.clone()).text_size(cx.theme().font_size)
            .flex().flex_col().gap(design::ui_scaled_px(16.))
            .child(div().p(design::ui_scaled_px(16.)).font_weight(FontWeight::SEMIBOLD)
                .child("Synthetic preview · no live backend"));
        if let Some(picker) = self.picker.as_ref() {
            return root.child(div().flex_1().flex().items_center().justify_center().child(picker.clone()));
        }
        if self.mode.starts_with("plans") {
            let mut cards = div().p(px(24.)).flex().items_start().gap(px(24.));
            for (index, statuses) in [
                ["completed", "completed", "completed", "in_progress", "pending"],
                ["in_progress", "pending", "pending", "pending", "pending"],
                ["completed"; 5],
            ].into_iter().enumerate() {
                let task_id = format!("preview-plan-{index}");
                let items: Vec<_> = ["Review the reference design", "Implement the popup layout", "Connect live plan updates", "Verify the native interactions", "Final review"]
                    .into_iter().zip(statuses).enumerate().map(|(i, (title, status))|
                        json!({"id":format!("step-{i}"),"title":title,"status":status})).collect();
                let snapshot = json!({"workPlans":[
                    {"id":format!("current-{index}"),"taskId":task_id,"title":"Workplan popup refinement","status":if index==2 {"closed"}else{"active"},"updatedAt":20,"items":items},
                    {"id":format!("earlier-{index}"),"taskId":task_id,"title":"Earlier work","status":"closed","updatedAt":10,"items":[]}]});
                cards = cards.child(div().w(px(360.)).child(crate::plans::render_plan_popup(&snapshot, &task_id, palette, 1.)));
            }
            return root.child(cards);
        }
        {
            let now = chrono::Utc::now().timestamp_millis();
            let plan = json!({"title":"Verify the native design system","status":"active","items":[
                {"title":"Review reference","status":"completed"},
                {"title":"Implement a wrapping step inside a compact pane","status":"in_progress"},
                {"title":"Verify result","status":"pending"},
                {"title":"Unavailable prerequisite","status":"blocked"},
                {"title":"Optional follow-up","status":"skipped"}]});
            let queue = json!({"agents":[{"id":"preview-agent","name":"Preview agent"}],
                "tasks":[{"id":"preview-task","agentId":"preview-agent","title":"Review desktop surfaces"}],
                "buildRequests":[{"id":"request-one","projectId":"preview","status":"succeeded","title":"Native surface updates","buildRunId":"build-one"},
                    {"id":"request-two","projectId":"preview","status":"succeeded","title":"Focused layout checks","buildRunId":"build-one"},
                    {"id":"waiting","projectId":"preview","taskId":"preview-task","status":"ready","title":"Check narrow layouts","command":"cargo test","createdAt":now},
                    {"id":"planned","projectId":"preview","taskId":"preview-task","status":"planned","title":"Prepare design review","command":"cargo check","createdAt":now}],
                "buildRuns":[{"id":"build-one","projectId":"preview","ownerTaskId":"preview-task","requestIds":["request-one","request-two"],
                    "status":"succeeded","exitCode":0,"finishedAt":now,"command":"cargo test","cwd":"/synthetic/preview","targetKey":"preview-only",
                    "summary":"Focused checks completed.\n  Synthetic report for disclosure review.\n\nNo live build was run."},
                    {"id":"failed-build","projectId":"preview","ownerTaskId":"preview-task","requestIds":[],"status":"failed","exitCode":1,"finishedAt":now-86_400_000,"command":"cargo test","summary":"Synthetic failure report. No live command ran."}]});
            let controls = div().w_full().flex().flex_col().gap(design::ui_scaled_px(8.))
                .child("Synthetic controls")
                .child(Input::new(&self.name).aria_label("Synthetic workspace name"))
                .child(Select::new(&self.choice))
                .child(div().flex().gap(design::ui_scaled_px(8.))
                    .child(Button::new("preview-cancel").compact().label("Cancel"))
                    .child(Button::new("preview-save").compact().primary().outline().label("Save")));
            let mut exchanges = div().w_full().flex().flex_col().gap(design::ui_scaled_px(4.));
            for (index, initially_open) in [(0, true), (1, false)] {
                let open = window.use_keyed_state(SharedString::from(format!("preview-exchange-group-open-{index}")), cx, |_, _| initially_open);
                let expanded = *open.read(cx);
                let mut details = crate::chat::agent_exchange::timeline(&format!("preview-group-{index}"));
                for (member, direction, status, body) in [
                    (0, crate::chat::agent_exchange::Direction::Incoming, "completed", "Asked: review native widget shapes. Synthetic example only."),
                    (1, crate::chat::agent_exchange::Direction::Outgoing, "completed", "Replied: checked control and card geometry. Synthetic example only."),
                    (2, crate::chat::agent_exchange::Direction::Incoming, "queued", "Asked: verify the narrow-pane layout. Synthetic example only."),
                ] {
                    let exchange = crate::chat::agent_exchange::Exchange {
                        direction, peer:"Codex".into(), status:status.into(), timestamp:Some(now-30_000+member*10_000), steps:vec![],
                    };
                    details = details.child(crate::chat::agent_exchange::render_step(exchange,
                        format!("preview-group-{index}-member-{member}"), palette, div().child(body).into_any_element()));
                }
                let summary = crate::chat::agent_exchange::Exchange {
                    direction:crate::chat::agent_exchange::Direction::Mixed,
                    peer:"Codex".into(), status:"queued".into(), timestamp:Some(now), steps:vec![],
                };
                exchanges = exchanges.child(crate::chat::agent_exchange::render_group(summary, 3,
                    format!("preview-group-{index}"), expanded, palette, details.into_any_element(),
                    move |_, _, cx| open.update(cx, |value, cx| { *value = !*value; cx.notify(); })));
            }
            root = root.child(div().id("design-system-overview").flex_1().min_h_0().overflow_y_scroll()
                .p(design::ui_scaled_px(20.)).flex().flex_row().items_start().gap(design::ui_scaled_px(24.))
                .child(div().w(design::ui_scaled_px(400.)).flex_shrink_0().flex().flex_col().gap(design::ui_scaled_px(20.))
                    .child(controls)
                    .child(exchanges)
                    .child(crate::plans::render_plan_card(&plan, palette, 1., false)))
                .child(div().flex_1().min_w_0().child(crate::collaboration::build_queue::content(&queue, "preview", palette))));
        }
        root
    }
}

pub(super) fn open_if_requested(cx: &mut App) -> bool {
    let Ok(mode) = std::env::var("MONITTER_GPUI_DESIGN_SYSTEM_PREVIEW") else { return false; };
    if mode == "surface" {
        crate::surface_preview::open(cx);
        return true;
    }
    let mode = if matches!(mode.as_str(), "overview" | "light" | "plans" | "plans-light" | "model" | "model-light" | "permissions" | "permissions-light") { mode } else { "overview".into() };
    let _ = monitter_gpui_desktop::fonts::register(cx);
    Theme::change(if mode.ends_with("light") { ThemeMode::Light } else { ThemeMode::Dark }, None, cx);
    let palette = design::palette(&json!({"theme":if mode.ends_with("light") {"light"} else {"dark"}}),
        if mode.ends_with("light") {WindowAppearance::Light} else {WindowAppearance::Dark});
    Theme::update(cx, |theme| {
        theme.radius = design::ui_scaled_px(design::radius::CONTROL);
        theme.radius_lg = design::ui_scaled_px(design::radius::CARD);
        theme.font_family = "IBM Plex Sans".into();
        theme.font_size = design::ui_scaled_px(14.);
        theme.colors.primary = rgb(palette.accent).into();
        theme.colors.background = rgb(palette.paper).into();
        theme.colors.foreground = rgb(palette.ink).into();
        theme.colors.muted_foreground = rgb(palette.muted).into();
        theme.colors.input = rgb(palette.line).into();
    });
    let (window, _) = gpui_kit::open_window(WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, size(px(1200.), px(820.)), cx))),
        titlebar: Some(TitlebarOptions { title: Some("Monitter · Synthetic design preview".into()), ..Default::default() }),
        ..Default::default()
    }, cx, |window, cx| {
        window.on_window_should_close(cx, |_, cx| { cx.quit(); true });
        let picker = if mode.starts_with("permissions") { Some(crate::composer_footer::permission_picker_preview(window, cx)) } else { mode.starts_with("model").then(|| crate::composer_footer::model_picker_preview(window, cx)) };
        let preview = cx.new(|cx| Preview {
            picker,
            mode,
            name: cx.new(|cx| InputState::new(window, cx).default_value("Synthetic workspace")),
            choice: cx.new(|cx| SelectState::new(vec![SharedString::from("Workspace"), SharedString::from("Agents & integrations"), SharedString::from("Security & access")], Some(component::IndexPath::new(0)), window, cx)),
        });
        preview
    }).expect("synthetic design preview window");
    crate::visual_export::install(cx);
    crate::visual_export::schedule(window, cx);
    cx.activate(true);
    true
}
