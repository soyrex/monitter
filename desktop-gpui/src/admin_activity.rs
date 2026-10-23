//! Read-only view of the current session's admin activity log.
use gpui_kit::{
    assets::IconName,
    component::{ActiveTheme as _, Icon, Selectable, Sizable as _, Theme, ThemeStyled as _, button::*, popover::Popover},
    prelude::*,
    *,
};
use monitter_lib::experimental_desktop::DesktopService;
use serde_json::{Value, json};
use std::{collections::HashSet, time::Duration};

#[derive(Clone, Debug, PartialEq)]
pub struct ActivityEntry {
    pub id: String,
    pub purpose: String,
    pub task_id: Option<String>,
    pub pane_title: Option<String>,
    pub model: Option<String>,
    pub status: String,
    pub created_at: Option<i64>,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub detail: String,
}
impl ActivityEntry {
    fn from_value(value: &Value) -> Option<Self> {
        Some(Self {
            id: value["id"].as_str()?.to_owned(),
            purpose: value["purpose"].as_str().unwrap_or("admin_turn").to_owned(),
            task_id: value["taskId"].as_str().map(str::to_owned),
            pane_title: value["paneTitle"].as_str().map(str::to_owned),
            model: value["model"].as_str().map(str::to_owned),
            status: value["status"].as_str().unwrap_or("queued").to_owned(),
            created_at: value["createdAt"].as_i64(),
            started_at: value["startedAt"].as_i64(),
            finished_at: value["finishedAt"].as_i64(),
            detail: value["detail"].as_str().unwrap_or("").to_owned(),
        })
    }
    fn purpose_title(&self) -> &'static str {
        match self.purpose.as_str() {
            "autoname" | "rename_all" => "Workspace naming",
            "notification_summary" => "Notification summary",
            _ => "Admin turn",
        }
    }
    fn tint(&self, theme: &Theme) -> Hsla {
        match self.status.as_str() {
            "queued" => theme.muted_foreground,
            "running" => theme.primary,
            "succeeded" => theme.success,
            "failed" | "timeout" => theme.danger,
            _ => theme.warning,
        }
    }
    fn status_icon(&self) -> IconName {
        match self.status.as_str() {
            "queued" => IconName::CircleDot,
            "running" => IconName::LoaderCircle,
            "succeeded" => IconName::CircleCheck,
            "failed" => IconName::CircleX,
            "timeout" => IconName::Clock,
            _ => IconName::CircleQuestionMark,
        }
    }
    fn purpose_label(&self) -> String {
        format!("{} · {}", self.purpose_title(), self.status)
    }
    fn time_label(&self) -> String {
        self.created_at.map(format_time).unwrap_or_else(|| "Time unavailable".into())
    }
}

fn format_time(ms: i64) -> String {
    crate::timestamps::local_datetime(ms).map(|time| time.format("%H:%M:%S").to_string())
        .unwrap_or_else(|| "Time unavailable".into())
}
fn intro(detail: &str, limit: usize) -> String {
    let compact = detail.trim().to_owned();
    let mut chars = compact.chars();
    let mut excerpt: String = chars.by_ref().take(limit).collect();
    if chars.next().is_some() {
        if let Some(boundary) = excerpt.rfind(char::is_whitespace) { excerpt.truncate(boundary); }
        excerpt.push('…');
    }
    excerpt
}
fn friendly_command_error(error: String) -> String {
    if error.contains("unknown command") || error.contains("Unknown command") {
        "Admin activity is unavailable in this backend version.".into()
    } else { error }
}

#[derive(Clone, Debug)]
pub enum AdminActivityEvent { FocusTask(String) }
#[derive(IntoElement)]
struct ActivityTrigger { open: bool }
impl Selectable for ActivityTrigger {
    fn selected(mut self, selected: bool) -> Self { self.open = selected; self }
    fn is_selected(&self) -> bool { self.open }
}
impl RenderOnce for ActivityTrigger {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        div().id("admin-activity-trigger").debug_selector(|| "admin-activity-trigger".into())
        .child(Button::new("admin-activity-button").accessibility_label("Admin activity")
                .accessibility_id("admin-activity-button").ghost().compact().selected(self.open)
                .tooltip("Admin activity · current session log")
                .icon(Icon::new(IconName::Bot).with_size(px(18.)).text_color(cx.theme().muted_foreground)))
    }
}

pub struct AdminActivity {
    service: Option<DesktopService>,
    open: bool,
    loading: bool,
    requested: bool,
    entries: Vec<ActivityEntry>,
    error: Option<String>,
    expanded: HashSet<String>,
    poll_generation: u64,
    request_generation: u64,
}
impl EventEmitter<AdminActivityEvent> for AdminActivity {}
impl AdminActivity {
    pub fn new(service: Option<DesktopService>) -> Self {
        Self { service, open: false, loading: false, requested: false, entries: vec![], error: None,
            expanded: HashSet::new(), poll_generation: 0, request_generation: 0 }
    }
    pub fn connect(&mut self, service: DesktopService, cx: &mut Context<Self>) {
        self.service = Some(service);
        if self.open { self.refresh(cx); }
    }
    fn set_open(&mut self, open: bool, cx: &mut Context<Self>) {
        self.open = open;
        if open {
            self.refresh(cx);
            self.start_polling(cx);
        } else {
            self.poll_generation = self.poll_generation.wrapping_add(1);
        }
        cx.notify();
    }
    fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.loading { return; }
        self.loading = true;
        self.requested = true;
        self.error = None;
        self.request_generation = self.request_generation.wrapping_add(1);
        let generation = self.request_generation;
        let Some(service) = self.service.clone() else {
            self.loading = false;
            self.error = Some("Admin activity is unavailable while the backend is disconnected.".into());
            cx.notify();
            return;
        };
        let work = cx.background_executor().spawn(async move {
            service.invoke("read_admin_activity", json!({}))
        });
        cx.spawn(async move |view, cx| {
            let result = work.await;
            let _ = view.update(cx, |view, cx| {
                if generation != view.request_generation { return; }
                view.loading = false;
                match result {
                    Ok(value) => {
                        view.entries = value["entries"].as_array().into_iter().flatten()
                            .filter_map(ActivityEntry::from_value).take(200).collect();
                        view.expanded.retain(|id| view.entries.iter().any(|entry| &entry.id == id));
                        view.error = None;
                    }
                    Err(error) => view.error = Some(friendly_command_error(error)),
                }
                cx.notify();
            });
        }).detach();
        cx.notify();
    }
    fn start_polling(&mut self, cx: &mut Context<Self>) {
        if !self.open { return; }
        self.poll_generation = self.poll_generation.wrapping_add(1);
        let generation = self.poll_generation;
        cx.spawn(async move |view, cx| loop {
            cx.background_executor().timer(Duration::from_secs(5)).await;
            let should_continue = view.update(cx, |view, cx| {
                if !view.open || view.poll_generation != generation { return false; }
                if !view.loading { view.refresh(cx); }
                true
            }).unwrap_or(false);
            if !should_continue { break; }
        }).detach();
    }
    fn toggle_expanded(&mut self, id: &str, cx: &mut Context<Self>) {
        if !self.expanded.remove(id) { self.expanded.insert(id.to_owned()); }
        cx.notify();
    }
    fn content(&mut self, max_height: Pixels, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let font = theme.font_size * (12. / 14.);
        let mut list = div().id("admin-activity-list").debug_selector(|| "admin-activity-list".into())
            .w_full().max_h(max_height).overflow_y_scroll().flex().flex_col()
            .font_family(theme.font_family.clone()).text_size(font);
        let header = div().w_full().px(px(16.)).pt(px(16.)).pb(px(14.)).border_b_1().border_color(theme.border)
            .flex().items_center().justify_between()
            .child(div().flex().flex_col().gap(px(2.))
                .child(div().font_weight(FontWeight::SEMIBOLD).text_size(px(15.)).child("Admin activity"))
                .child(div().text_size(px(12.)).text_color(theme.muted_foreground).child("Recent log · current session only")))
            .child(if self.loading { "Updating…".into_any_element() } else { Button::new("admin-activity-refresh").icon(IconName::RefreshCw).ghost().compact().size(px(32.)).rounded(px(8.)).tooltip("Refresh activity").on_click(cx.listener(|view,_,_,cx|view.refresh(cx))).into_any_element() });
        list = list.child(header);
        if let Some(error) = &self.error {
            list = list.child(div().px(px(14.)).py(px(13.)).text_color(theme.danger).child(error.clone()));
        } else if self.entries.is_empty() {
            let message = if self.loading { "Loading current session activity…" }
                else if self.requested { "No admin activity in this session yet." }
                else { "Open to load current session activity." };
            list = list.child(div().px(px(14.)).py(px(16.)).text_color(theme.muted_foreground).child(message));
        } else {
            for (index, entry) in self.entries.iter().enumerate() {
                let id = entry.id.clone();
                let row_id = id.clone();
                let expanded = self.expanded.contains(&entry.id);
                let tint = entry.tint(theme);
                let mut dot_bg = tint; dot_bg.a = 0.14;
                let mut dot_border = tint; dot_border.a = 0.38;
                let hover_bg = theme.button_hover;
                let line = div().id(SharedString::from(format!("admin-activity-row-{}", entry.id)))
                    .debug_selector({let selector=format!("admin-activity-row-{}",entry.id);move || selector.clone()})
                    .w_full().flex().items_stretch().gap(px(12.)).px(px(16.)).pt(px(14.)).pb(px(12.))
                    .border_b_1().border_color(theme.border).hover(move |style| style.bg(hover_bg))
                    .cursor_pointer()
                    .on_click(cx.listener(move |view,_,_,cx|view.toggle_expanded(&row_id,cx)))
                    .child(div().relative().w(px(20.)).flex_shrink_0().self_stretch().flex().justify_center()
                        .when(index + 1 < self.entries.len(), |rail| rail.child(div().absolute().top(px(11.)).bottom(px(-10.)).w(px(1.)).bg(theme.border)))
                        .child(div().self_start().w(px(20.)).h(px(20.)).rounded_full().border_1().border_color(dot_border).bg(dot_bg).flex().items_center().justify_center()
                            .child(Icon::new(entry.status_icon()).with_size(px(12.)).text_color(tint))))
                    .child(div().flex_1().min_w_0().flex().flex_col().gap(px(4.))
                        .child(div().flex().items_center().justify_between().gap(px(6.))
                            .child(div().font_weight(FontWeight::SEMIBOLD).text_size(px(14.)).truncate().child(entry.purpose_title()))
                            .child(div().flex_shrink_0().font_family(crate::design::FONT_MONO).text_size(px(12.)).text_color(theme.muted_foreground).child(entry.time_label())))
                        .child(div().flex().items_center().gap(px(6.)).text_size(px(12.)).text_color(tint)
                            .child(entry.status.clone())
                            .when_some(entry.model.clone(), |row, model| row.child(div().text_color(theme.muted_foreground).child(format!("· {model}")))))
                        .when_some(entry.pane_title.clone(), |row, title| row.child(div().text_size(px(12.)).text_color(theme.muted_foreground).truncate().child(title)))
                        .when(!expanded && !entry.detail.is_empty(), |row| row.child(div().text_size(px(13.)).line_height(relative(1.5)).text_color(theme.foreground).child(intro(&entry.detail, 180))))
                        .when(expanded, |row| row.child(div().mt(px(4.)).flex().flex_col().gap(px(3.)).text_size(font * 0.82).text_color(theme.muted_foreground)
                            .child(format!("Purpose: {}", entry.purpose_title()))
                            .child(format!("Created: {}", timestamp_label(entry.created_at)))
                            .child(format!("Started: {}", timestamp_label(entry.started_at)))
                            .child(format!("Finished: {}", timestamp_label(entry.finished_at)))
                            .child(format!("Task ID: {}", entry.task_id.as_deref().unwrap_or("Unavailable")))
                            .child(format!("Entry ID: {}", entry.id))
                            .when(!entry.detail.is_empty(), |box_| box_.child(div().pt(px(3.)).text_color(theme.foreground).child(entry.detail.clone())))))
                        .child(div().flex().items_center().gap(px(8.))
                            .child(Button::new(SharedString::from(format!("admin-activity-expand-{id}")))
                                .icon(if expanded { IconName::ChevronUp } else { IconName::ChevronDown }).ghost().compact().size(px(25.))
                                .tooltip(if expanded { "Show less" } else { "Show details" })
                                .on_click(cx.listener(move |view,_,_,cx| {cx.stop_propagation();view.toggle_expanded(&id,cx);})))
                            .when_some(entry.task_id.clone(), |row, task_id| row.child(Button::new(SharedString::from(format!("admin-activity-focus-{task_id}")))
                                .debug_selector({let selector=format!("admin-activity-focus-{task_id}");move || selector.clone()})
                                .icon(IconName::ArrowUpRight).ghost().compact().text_size(font * 0.82).child("Focus pane")
                                .on_click(cx.listener(move |view,_,_,cx| {cx.stop_propagation();cx.emit(AdminActivityEvent::FocusTask(task_id.clone()));view.set_open(false,cx);}))))));
                list = list.child(line);
            }
        }
        div().debug_selector(|| "admin-activity-surface".into()).w(px(440.)).popover_style(cx).overflow_hidden()
            .font_family(theme.font_family.clone()).text_size(font).child(list).into_any_element()
    }
}
fn timestamp_label(value: Option<i64>) -> String { value.map(crate::timestamps::datetime_label).unwrap_or_else(|| "Unavailable".into()) }
impl Render for AdminActivity {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let owner = cx.entity();
        let content_owner = owner.clone();
        let max_height = window.bounds().size.height * 0.8;
        Popover::new("admin-activity-popover").appearance(false).anchor(Anchor::TopRight)
            .open(self.open).overlay_closable(true)
            .trigger(ActivityTrigger { open: self.open })
            .on_open_change(move |open,_,cx| owner.update(cx,|view,cx|view.set_open(*open,cx)))
            .content(move |_,_,cx| content_owner.update(cx,|view,cx|view.content(max_height,cx)))
    }
}

impl crate::Desktop {
    pub(super) fn ensure_admin_activity(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Entity<AdminActivity> {
        if let Some(activity) = &self.admin_activity { return activity.clone(); }
        let activity = cx.new(|_| AdminActivity::new(self.service.clone()));
        let subscription = cx.subscribe_in(&activity, window, |view, _, event, window, cx| {
            match event {
                AdminActivityEvent::FocusTask(task_id) => {
                    if !crate::rows(&view.snapshot, "tasks").iter().any(|task| crate::field(task, "id") == *task_id) { return; }
                    view.navigate_sidebar_task(task_id.clone(), window, cx);
                    if let Some(service) = view.service.clone() {
                        view.ensure_chat_pane(task_id, service, window, cx);
                    }
                }
            }
        });
        self.admin_activity_subscription = Some(subscription);
        self.admin_activity = Some(activity.clone());
        activity
    }
}

#[cfg(feature = "visual-export")]
pub fn open_preview_if_requested(cx: &mut App) -> bool {
    let Ok(mode) = std::env::var("MONITTER_GPUI_ADMIN_ACTIVITY_PREVIEW") else { return false; };
    let _ = monitter_gpui_desktop::fonts::register(cx);
    gpui_kit::component::Theme::change(gpui_kit::component::ThemeMode::Dark, None, cx);
    struct Preview { activity: Entity<AdminActivity>, expanded: bool }
    impl Render for Preview {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            div().size_full().bg(cx.theme().background).text_color(cx.theme().foreground)
                .font_family(cx.theme().font_family.clone()).text_size(cx.theme().font_size).p(px(24.))
                .flex().flex_col().gap(px(12.))
                .child(div().font_weight(FontWeight::SEMIBOLD).child("Admin activity · Preview synthetic"))
                .child(div().text_color(cx.theme().muted_foreground).child("Synthetic examples · no live session or backend"))
                .child(self.activity.update(cx,|view,cx|view.content(window.bounds().size.height * 0.8,cx)))
        }
    }
    let expanded = mode == "expanded";
    let (window, _) = gpui_kit::open_window(WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None,size(px(520.),px(700.)),cx))),
        titlebar: Some(TitlebarOptions { title: Some("Monitter Admin Activity Preview".into()), ..Default::default() }),
        ..Default::default()
    },cx,|_,cx| {
        let service = DesktopService::disconnected_for_ui_fixture();
        let activity = cx.new(|_| AdminActivity::new(Some(service)));
        activity.update(cx,|view,cx| {
            view.entries = [
                ("queued","autoname","Workspace name refresh",None,None,None,"Waiting for an available admin slot."),
                ("running","notification_summary","Notification summary",Some(1_760_000_000_000),None,None,"Preparing a concise summary for the latest notification."),
                ("succeeded","admin_turn","Review current session state",Some(1_760_000_010_000),Some(1_760_000_010_400i64),Some(1_760_000_011_200),"The requested session review finished successfully."),
                ("failed","admin_turn","Inspect workspace startup",Some(1_760_000_020_000),Some(1_760_000_020_200i64),Some(1_760_000_022_000),"The admin request failed because the current backend did not return a usable response."),
            ].into_iter().enumerate().map(|(index,(status,purpose,title,started,_,finished,detail))| ActivityEntry {
                id:format!("preview-{index}"),purpose:purpose.into(),task_id:(index != 0).then(||format!("preview-task-{index}")),
                pane_title:Some(title.into()),model:Some("GPT-5.6 Luna".into()),status:status.into(),created_at:Some(1_760_000_000_000+index as i64*10_000),
                started_at:started,finished_at:finished,detail:detail.into(),
            }).collect();
            if expanded { view.expanded.insert("preview-2".into()); }
            view.open = true; cx.notify();
        });
        cx.subscribe(&activity,|_,_:&AdminActivityEvent,_|{}).detach();
        cx.new(|_|Preview {activity,expanded})
    }).expect("admin activity preview window");
    crate::visual_export::install(cx);
    crate::visual_export::schedule(window,cx);
    cx.activate(true);
    true
}

#[cfg(all(test, feature = "visual-export"))]
mod tests {
    use super::{ActivityEntry, AdminActivity, AdminActivityEvent};
    use gpui_kit::{AppContext as _, Modifiers, TestAppContext, Render, Window, Context, IntoElement};
    use std::{cell::RefCell, rc::Rc};

    struct Fixture { activity: gpui::Entity<AdminActivity> }
    impl Render for Fixture {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            self.activity.update(cx, |activity, cx| {
                activity.content(window.bounds().size.height * 0.8, cx)
            })
        }
    }

    #[gpui_kit::test]
    fn timeline_row_expands_and_focus_emits_once_without_collapsing(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let mut activity = None;
        let (_, visual) = cx.add_window_view(|window, cx| {
            let view = cx.new(|_| AdminActivity::new(None));
            activity = Some(view.clone());
            view.update(cx, |view, _| {
                view.entries = vec![ActivityEntry {
                    id: "entry-one".into(),
                    purpose: "admin_turn".into(),
                    task_id: Some("task-one".into()),
                    pane_title: Some("Review current task".into()),
                    model: Some("GPT-5.6 Luna".into()),
                    status: "succeeded".into(),
                    created_at: Some(1_760_000_000_000),
                    started_at: Some(1_760_000_000_100),
                    finished_at: Some(1_760_000_000_700),
                    detail: "A sufficiently detailed result for the activity timeline renderer test.".into(),
                }];
            });
            let fixture = cx.new(|_| Fixture { activity: view });
            gpui_kit::component::Root::new(fixture, window, cx)
        });
        let activity = activity.expect("fixture activity entity");
        let events = Rc::new(RefCell::new(Vec::new()));
        visual.update(|_, cx| {
            let events = events.clone();
            cx.subscribe(&activity, move |_, event: &AdminActivityEvent, _| events.borrow_mut().push(event.clone())).detach();
        });
        visual.run_until_parked();
        visual.update(|window, cx| window.draw(cx).clear(cx));

        let row = visual.debug_bounds("admin-activity-row-entry-one").expect("timeline row rendered");
        visual.simulate_click(row.center(), Modifiers::none());
        visual.run_until_parked();
        visual.update(|window, cx| window.draw(cx).clear(cx));
        assert!(activity.read_with(visual, |view, _| view.expanded.contains("entry-one")));

        let focus = visual.debug_bounds("admin-activity-focus-task-one").expect("focus action rendered");
        visual.simulate_click(focus.center(), Modifiers::none());
        visual.run_until_parked();
        assert!(activity.read_with(visual, |view, _| view.expanded.contains("entry-one")), "focus action must not collapse the row");
        assert_eq!(events.borrow().len(), 1);
        assert!(matches!(events.borrow().first(), Some(AdminActivityEvent::FocusTask(task_id)) if task_id == "task-one"));
    }
}
