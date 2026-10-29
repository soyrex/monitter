//! One native catalog shared by both keyboard aliases. Enumeration is local;
//! execution revalidates the fresh catalog in Desktop::execute_palette.
use crate::workspace_windows::DesktopContextExt as _;
use super::*;
use command_palette::{EntryKind as K, PaletteEntry as E};
fn entry(
    id: impl Into<String>,
    label: impl Into<String>,
    subtitle: impl Into<String>,
    kind: K,
    disabled: bool,
    checked: Option<bool>,
) -> E {
    E {
        id: id.into(),
        label: label.into(),
        subtitle: subtitle.into(),
        kind,
        disabled,
        checked,
        keywords: String::new(),
        keyboard_hint: None,
        search_text: String::new(),
        open_priority: 2,
    }
}
#[cfg(target_os = "macos")]
const WINDOW_CENTER_PRESETS: &[(&str, u8, u8)] = &[
    ("window-center-80-70", 80, 70),
    ("window-center-85-75", 85, 75),
    ("window-center-90-80", 90, 80),
    ("window-center-95-85", 95, 85),
];
const TOGGLES: &[(&str, &str, &str, bool)] = &[
    ("tools", "Show tool activity", "showToolActivity", true),
    (
        "reasoning",
        "Show reasoning summaries",
        "showReasoningSummaries",
        true,
    ),
    (
        "focus-mouse",
        "Focus follows mouse",
        "focusFollowsMouse",
        false,
    ),
    (
        "active-pane-border",
        "Highlight active pane",
        "showActivePaneBorder",
        true,
    ),
    ("dim-panes", "Dim inactive panes", "dimInactivePanes", true),
    ("enter", "Enter to send", "sendWithEnter", false),
    (
        "compress-tools",
        "Compress tool calls",
        "compressToolCalls",
        false,
    ),
    (
        "tint-messages",
        "Tint my messages",
        "tintUserMessages",
        false,
    ),
    (
        "show-tab-close",
        "Show tab close buttons",
        "showTabCloseButtons",
        true,
    ),
    ("auto-hide-tabs", "Auto-hide tabs", "autoHideTabs", false),
];
const SETTINGS: &[(&str, &str, &str)] = &[
    ("appearance", "Appearance settings", "appearance"),
    ("typography-settings", "Fonts & text", "typography"),
    ("behaviour-settings", "Keyboard & focus", "behaviour"),
    ("session-lock-settings", "Session lock", "security"),
    (
        "conversation-settings",
        "Conversation settings",
        "conversation",
    ),
    ("environment-secrets", "Environment & Secrets", "secrets"),
    (
        "pane-appearance-settings",
        "Layout & panes",
        "interface",
    ),
    (
        "appearance-detail-settings",
        "Advanced appearance settings",
        "appearance",
    ),
    ("agents-directory", "Browse agents", "agents"),
    ("hosts", "Manage hosts", "agents"),
];
const CREATION_COMMANDS: &[(&str, &str)] = &[
    ("new-task", "New chat"),
    ("new-terminal", "New terminal"),
    ("new-neovim", "New Neovim pane"),
    ("neovim-files", "Show Neovim files"),
    ("new-agent", "New agent"),
    ("new-channel", "New channel"),
    ("new-project", "New project"),
    ("ideas", "Idea List"),
    ("new-browser", "New browser tab"),
    ("balance-panes", "Balance panes"),
    ("archived", "Archived chats"),
];
const LAYOUTS: &[(&str, &str)] = &[
    ("single", "One pane"),
    ("columns", "Two columns"),
    ("columns-3", "3 columns"),
    ("columns-4", "4 columns"),
    ("grid", "2 × 2 grid"),
    ("grid-3x2", "3 × 2 grid"),
    ("grid-4x2", "4 × 2 grid"),
];
fn toggle_patch(id: &str, settings: &Value) -> Option<Value> {
    let (_, _, key, default) = TOGGLES.iter().find(|(key, _, _, _)| *key == id)?;
    Some(json!({*key:!settings[*key].as_bool().unwrap_or(*default)}))
}
// Content is collected when opening/refreshing the palette, never on a keystroke.
const SEARCH_TEXT_LIMIT: usize = 512 * 1024;
fn append_search_text(output: &mut String, text: &str) {
    let available = SEARCH_TEXT_LIMIT.saturating_sub(output.len());
    let mut end = text.len().min(available);
    while !text.is_char_boundary(end) { end -= 1; }
    output.push_str(&text[..end]);
    if output.len() < SEARCH_TEXT_LIMIT { output.push('\n'); }
}
/// The live layout supersedes its saved workspace copy. Retained pane entities
/// alone are not evidence that a chat is still open.
pub(super) fn rename_all_targets(view: &Desktop, cx: &App) -> Vec<Value> {
    let mut layouts = Vec::new();
    if let Ok(layout) = view.dock.read(cx).serialize(cx) { layouts.push(layout); }
    if let Some(set) = &view.named_workspaces {
        layouts.extend(set.items.iter().filter(|w| w.id != set.active_id)
            .filter_map(|w| w.layout.clone()));
    }
    rename_all_chat_targets(layouts.iter().flat_map(|layout|
        docking::DockWorkspace::saved_ids(layout).unwrap_or_default()), &view.snapshot)
}
fn rename_all_chat_targets(ids: impl IntoIterator<Item = String>, snapshot: &Value) -> Vec<Value> {
    let internal = crate::workspace_visibility::internal_task_ids(snapshot);
    let mut seen = std::collections::HashSet::new();
    ids.into_iter().filter_map(|pane| {
        let id = pane.strip_prefix("chat:")?;
        if id.is_empty() || internal.contains(id) || !seen.insert(id.to_owned()) { return None; }
        let task = snapshot["tasks"].as_array()?.iter().find(|task| task["id"].as_str() == Some(id))?;
        if task["archived"].as_bool() == Some(true) { return None; }
        Some(json!({"taskId":id}))
    }).collect()
}
fn open_pane_entries(view: &Desktop, cx: &App) -> Vec<E> {
    let mut panes = Vec::new();
    let active = view.named_workspaces.as_ref().map(|set| set.active_id.as_str()).unwrap_or("");
    let active_name = view.named_workspaces.as_ref().and_then(|set| set.items.iter().find(|w| w.id == active)).map(|w| w.name.as_str()).unwrap_or("Current workspace");
    let mut layouts = Vec::new();
    if let Ok(layout) = view.dock.read(cx).serialize(cx) {
        layouts.push((String::new(), active_name.to_owned(), layout));
    }
    if let Some(set) = &view.named_workspaces {
        for workspace in &set.items {
            if workspace.id != active {
                if let Some(layout) = &workspace.layout { layouts.push((workspace.id.clone(), workspace.name.clone(), layout.clone())); }
            }
        }
    }
    let browsers = view.browser.as_ref().map(|b| b.read(cx).metadata_snapshot(cx));
    let chats = view.chat_panes.borrow();
    for (workspace, workspace_name, layout) in layouts {
        for id in docking::DockWorkspace::saved_ids(&layout).unwrap_or_default() {
            let mut label = view.dock.read(cx).pane_title(&id, cx).map(|s| s.to_string()).unwrap_or_else(|| id.clone());
            let mut text = String::new();
            let mut keywords = String::new();
            let kind = if let Some(task_id) = id.strip_prefix("chat:") {
                if let Some(task) = view.snapshot["tasks"].as_array().and_then(|tasks| tasks.iter().find(|t| t["id"].as_str() == Some(task_id))) {
                    label = field(task, "title");
                    let agent_name = view.snapshot["agents"].as_array()
                        .and_then(|agents| task["agentId"].as_str().and_then(|id| agents.iter().find(|agent| agent["id"].as_str() == Some(id))))
                        .map(|agent| field(agent, "name"))
                        .unwrap_or_default();
                    keywords = format!("{} {} {} {} {}", field(task, "provider"), field(task, "model"), field(task, "cwd"), agent_name, field(task, "archivedAgentName"));
                }
                if let Some(transcript) = chats.get(task_id).and_then(|chat| chat.read(cx).transcripts.get(task_id)) {
                    for message in transcript.messages.iter().rev() {
                        append_search_text(&mut text, message["text"].as_str().unwrap_or(""));
                        if text.len() >= SEARCH_TEXT_LIMIT { break; }
                    }
                } else if let Some(messages) = view.snapshot["messages"].as_array() {
                    for message in messages.iter().rev().filter(|m| m["taskId"].as_str() == Some(task_id)) {
                        append_search_text(&mut text, message["text"].as_str().unwrap_or(""));
                        if text.len() >= SEARCH_TEXT_LIMIT { break; }
                    }
                }
                K::Task
            } else if let Some(session) = id.strip_prefix("terminal:") {
                if let Some(metadata) = view.palette_terminal_sessions.iter().find(|row| row["id"].as_str() == Some(session)) {
                    keywords = format!("{} {}", field(metadata, "title"), field(metadata, "cwd"));
                }
                for pane in view.terminal.iter().chain(view.terminal_panes.values()) {
                    if let Some(metadata) = pane.read(cx).palette_sessions().iter().find(|row| row["id"].as_str() == Some(session)) {
                        keywords = format!("{} {}", field(metadata, "title"), field(metadata, "cwd"));
                    }
                    let content = pane.read(cx).palette_search_text(session);
                    if !content.is_empty() { text = content; break; }
                }
                K::Terminal
            } else if let Some((session, win)) = neovim_workspace::parse_id(&id) {
                if let Some(session) = view.neovim_sessions.get(session) { text = session.read(cx).palette_search_text(win); }
                K::Pane
            } else if let Some(browser_id) = id.strip_prefix("browser:").and_then(|id| id.parse::<u64>().ok()) {
                if let Some(tab) = browsers.as_ref().and_then(|metadata| metadata.tabs.iter().find(|tab| tab.id == browser_id)) {
                    label = tab.title.clone();
                    keywords = format!("{} {}", tab.actual_url.as_deref().unwrap_or_default(), tab.requested_url);
                    append_search_text(&mut text, tab.actual_url.as_deref().unwrap_or(&tab.requested_url));
                }
                K::Pane
            } else { K::Pane };
            let row_id = if workspace.is_empty() { format!("pane:{id}") } else { format!("workspace-pane:{workspace}:{id}") };
            let mut row = entry(row_id, if label.trim().is_empty() { id } else { label }, format!("Open · {workspace_name}"), kind, view.preferences_restore || view.preferences_restoring, None);
            row.keywords = keywords;
            row.search_text = text;
            row.open_priority = if workspace.is_empty() { 0 } else { 1 };
            panes.push(row);
        }
    }
    panes
}
pub(super) fn entries(view: &Desktop, cx: &App) -> Vec<E> {
    let blocked = view.busy || view.service.is_none();
    let settings = &view.snapshot["settings"];
    let mut result = vec![];
    result.push(entry(
        "rename-all", "Rename all chat panes",
        "Check open chats across workspaces with System One; name changed topics in one admin request",
        K::Command,
        blocked || view.preferences_restore || view.preferences_restoring || rename_all_targets(view, cx).is_empty(),
        None,
    ));
    #[cfg(target_os = "macos")]
    for (id, width, height) in WINDOW_CENTER_PRESETS {
        result.push(entry(
            *id, format!("Window: Center at {width}% × {height}%"),
            "Resize and center on the current screen, excluding the Dock and menu bar",
            K::Command, false, None,
        ));
    }
    for (id, label, _) in Desktop::palette_routes() {
        result.push(entry(
            format!("route:{id}"),
            label,
            "Open native workspace surface",
            K::Route,
            view.service.is_none(),
            None,
        ));
    }
    let agents = rows(&view.snapshot, "agents");
    let mut tasks = rows(&view.snapshot, "tasks");
    tasks.sort_by_key(|task| std::cmp::Reverse(task["updatedAt"].as_i64().unwrap_or(0)));
    for task in tasks {
        if task["channelId"].as_str().is_some() {
            continue;
        }
        let agent = agents
            .iter()
            .find(|agent| field(agent, "id") == field(&task, "agentId"));
        if agent.is_some_and(|agent| agent["internal"].as_bool() == Some(true)) {
            continue;
        }
        // Deleted-agent archives remain real retained chats.
        if agent.is_none() && task["archived"].as_bool() != Some(true) {
            continue;
        }
        let archived = task["archived"].as_bool() == Some(true);
        let mut row = entry(
            format!("task:{}", field(&task, "id")),
            field(&task, "title"),
            format!(
                "{}{}",
                if archived {
                    "Restore archived chat · "
                } else {
                    ""
                },
                agent
                    .map(|a| field(a, "name"))
                    .unwrap_or_else(|| field(&task, "archivedAgentName"))
            ),
            if archived { K::ArchivedTask } else { K::Task },
            view.service.is_none() || (archived && view.busy),
            None,
        );
        row.keywords = format!(
            "{} {} {} {}",
            field(&task, "provider"),
            field(&task, "model"),
            field(&task, "cwd"),
            field(&task, "archivedAgentName")
        );
        result.push(row);
    }
    for (collection, kind, prefix) in [
        ("agents", K::Agent, "agent"),
        ("channels", K::Channel, "channel"),
        ("projects", K::Project, "project"),
    ] {
        for item in rows(&view.snapshot, collection) {
            if item["internal"].as_bool() == Some(true) {
                continue;
            }
            result.push(entry(
                format!("{prefix}:{}", field(&item, "id")),
                field(&item, "name"),
                field(&item, "description"),
                kind,
                view.service.is_none(),
                None,
            ));
        }
    }
    if view.draft_creation.is_some() {
        result.push(entry(
            "draft:retained",
            "New chat",
            "Retained native draft composer",
            K::Draft,
            blocked,
            None,
        ));
    }
    let open_panes = open_pane_entries(view, cx);
    let open_tasks: HashSet<_> = open_panes.iter().filter_map(|row| {
        row.id.strip_prefix("pane:chat:").map(str::to_owned)
            .or_else(|| row.id.strip_prefix("workspace-pane:")?.split_once(":")?.1.strip_prefix("chat:").map(str::to_owned))
    }).collect();
    result.retain(|row| !row.id.strip_prefix("task:").is_some_and(|id| open_tasks.contains(id)));
    let open_terminals: HashSet<_> = open_panes.iter().filter_map(|row| {
        row.id.strip_prefix("pane:terminal:").map(str::to_owned)
            .or_else(|| row.id.strip_prefix("workspace-pane:")?.split_once(':')?.1.strip_prefix("terminal:").map(str::to_owned))
    }).collect();
    result.extend(open_panes);
    let mut sessions = std::collections::BTreeMap::new();
    for session in &view.palette_terminal_sessions {
        sessions.insert(field(session, "id"), session.clone());
    }
    for pane in view.terminal.iter().chain(view.terminal_panes.values()) {
        for session in pane.read(cx).palette_sessions() {
            sessions.insert(field(session, "id"), session.clone());
        }
    }
    for (id, session) in sessions {
        if !id.is_empty() && !open_terminals.contains(&id) {
            result.push(entry(
                format!("terminal:{id}"),
                field(&session, "title"),
                field(&session, "cwd"),
                K::Terminal,
                view.service.is_none(),
                None,
            ));
        }
    }
    if let Some(workspaces) = &view.named_workspaces {
        for workspace in &workspaces.items {
            result.push(entry(
                format!("workspace:{}", workspace.id),
                workspace.name.clone(),
                "Switch named workspace",
                K::Workspace,
                view.preferences_restore || view.preferences_restoring,
                Some(workspaces.active_id == workspace.id),
            ));
        }
    }
    for (id, label, _) in SETTINGS {
        result.push(entry(
            *id,
            *label,
            "Open native preferences",
            K::Command,
            false,
            None,
        ));
    }
    for &(id, label) in CREATION_COMMANDS {
        let mut row = entry(id, label, "Workspace command", K::Command, blocked, None);
        if id == "new-terminal" {
            row.disabled |= view.empty_terminal_pending;
        }
        if id == "balance-panes" {
            row.keyboard_hint = Some("⌘⌥=".into());
        }
        if id == "ideas" {
            row.keywords = "ideas project notes shelf".into();
            row.subtitle = "Open a pane beside your agents".into();
        }
        result.push(row);
    }
    for &(id, label) in LAYOUTS {
        result.push(entry(
            format!("layout:{id}"),
            label,
            "Arrange retained native panes",
            K::Command,
            blocked,
            None,
        ));
    }
    for (id, label, key, default) in TOGGLES {
        result.push(entry(
            *id,
            *label,
            "Saved conversation or workspace setting",
            K::Command,
            blocked,
            Some(settings[*key].as_bool().unwrap_or(*default)),
        ));
    }
    result.push(entry(
        "steer-busy",
        "Steer busy agents when supported",
        "Saved behavior setting",
        K::Command,
        blocked,
        Some(field(settings, "busyMessageMode") == "steer"),
    ));
    for (id, label, view_kind) in [
        ("standard", "Standard sidebar view", SidebarView::Agents),
        ("activity", "Activity sidebar view", SidebarView::Activity),
        ("projects", "Projects sidebar view", SidebarView::Projects),
    ] {
        result.push(entry(
            format!("sidebar:{id}"),
            label,
            "Sidebar view",
            K::Command,
            false,
            Some(view.sidebar_view == view_kind),
        ));
    }
    for (prefix, values, key, default) in [
        ("theme", vec!["light", "dark", "system"], "theme", "system"),
        (
            "window-surface",
            vec!["opaque", "translucent", "glass"],
            "windowSurface",
            "opaque",
        ),
        (
            "density",
            vec!["tight", "normal", "spacious"],
            "interfaceDensity",
            "normal",
        ),
        (
            "tab-style",
            vec!["classic", "modern"],
            "tabStyle",
            "classic",
        ),
        (
            "shortcut-mode",
            vec!["standard", "vim"],
            "shortcutMode",
            "standard",
        ),
    ] {
        for value in values {
            result.push(entry(
                format!("{prefix}:{value}"),
                format!("{}: {value}", prefix.replace('-', " ")),
                "Saved workspace setting",
                K::Command,
                blocked,
                Some(settings[key].as_str().unwrap_or(default) == value),
            ));
        }
    }
    let appearance = view
        .preferences
        .as_ref()
        .map(|preferences| &preferences.data.appearance);
    for (id, label) in settings::native_appearance::PRESETS {
        for (prefix, scope) in [
            ("apply-theme", "Apply theme"),
            ("app-theme-light", "Light app theme"),
            ("app-theme-dark", "Dark app theme"),
        ] {
            let checked = appearance.map(|a| match prefix {
                "app-theme-light" => a.light_palette == *id,
                "app-theme-dark" => a.dark_palette == *id,
                _ => a.light_palette == *id && a.dark_palette == *id,
            });
            result.push(entry(
                format!("{prefix}:{id}"),
                format!("{scope}: {label}"),
                "Private native appearance preference",
                K::Command,
                view.busy || appearance.is_none(),
                checked,
            ));
        }
    }
    for (id, label) in [
        ("monitter", "Live workspace palette"),
        ("reference", "Warm paper palette"),
    ] {
        result.push(entry(
            format!("native-palette:{id}"),
            label,
            "Private native appearance preference",
            K::Command,
            view.busy || appearance.is_none(),
            Some(view.native_theme_preset == id),
        ));
    }
    for (id, label) in [
        ("scale-up", "Increase interface scale"),
        ("scale-down", "Decrease interface scale"),
        ("scale-reset", "Reset interface scale to 125%"),
        ("contrast-reset", "Reset theme contrast"),
        ("contrast-down", "Decrease theme contrast"),
        ("contrast-up", "Increase theme contrast"),
        ("accent-reset", "Use theme accent colours"),
        ("tint-reset", "Reset surface tint to 5%"),
        ("tint-down", "Decrease surface tint"),
        ("tint-up", "Increase surface tint"),
        ("border-opacity-reset", "Reset border opacity to 100%"),
        ("border-opacity-down", "Decrease border opacity"),
        ("border-opacity-up", "Increase border opacity"),
    ] {
        let boundary = appearance.is_some_and(|a| match id {
            "scale-up" => a.interface_scale >= 200,
            "scale-down" => a.interface_scale <= 80,
            "contrast-up" => a.contrast >= 100,
            "contrast-down" => a.contrast == 0,
            "tint-up" => a.surface_tint >= 50,
            "tint-down" => a.surface_tint == 0,
            "border-opacity-up" => a.border_opacity >= 100,
            "border-opacity-down" => a.border_opacity == 0,
            _ => false,
        });
        result.push(entry(
            id,
            label,
            "Private native appearance preference",
            K::Command,
            view.busy || appearance.is_none() || boundary,
            None,
        ));
    }
    for (id, label) in [
        ("inactive-opacity-down", "Decrease inactive pane opacity"),
        ("inactive-opacity-up", "Increase inactive pane opacity"),
    ] {
        let opacity = settings["inactivePaneOpacity"].as_f64().unwrap_or(0.6);
        result.push(entry(
            id,
            label,
            format!("Current {}%", (opacity * 100.).round()),
            K::Command,
            blocked
                || if id.ends_with("down") {
                    opacity <= 0.1
                } else {
                    opacity >= 0.9
                },
            None,
        ));
    }
    for mode in native_motion::MotionPreference::ALL {
        result.push(entry(
            format!("motion:{}", mode.id()),
            format!("Motion: {}", mode.label()),
            "Private native motion preference",
            K::Command,
            view.busy,
            Some(native_motion::current() == mode),
        ));
    }
    for theme in terminal_theme::THEMES {
        result.push(entry(
            format!("terminal-theme:{}", theme.id),
            format!("Terminal theme: {}", theme.label),
            "Private native terminal preference",
            K::Command,
            view.busy,
            Some(terminal_theme::current().id == theme.id),
        ));
    }
    if let Some(task) = rows(&view.snapshot, "tasks")
        .iter()
        .find(|t| Some(field(t, "id")) == view.selected)
    {
        let running = field(task, "status") == "running";
        result.push(entry(
            "archive",
            "Archive current chat",
            field(task, "title"),
            K::Command,
            blocked || running,
            None,
        ));
        if running {
            result.push(entry(
                "stop",
                "Stop current chat",
                field(task, "title"),
                K::Command,
                blocked,
                None,
            ));
        }
        result.push(entry(
            "autoname",
            "Auto-name current pane",
            field(task, "title"),
            K::Command,
            blocked,
            None,
        ));
        let pane = view.chat_panes.borrow().get(&field(task, "id")).cloned();
        result.push(entry(
            "detail",
            "Show run detail",
            "Toggle current chat details",
            K::Command,
            pane.is_none(),
            pane.map(|pane| pane.read(cx).secondary_sidebar_state().visible),
        ));
    }
    result
}

impl Desktop {
    fn palette_settings_category(
        &mut self,
        category: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.window.native_settings.is_none() {
            self.window.native_settings = Some(settings::NativeSettings::new(
                &self.snapshot["settings"],
                window,
                cx,
            ));
        }
        if let Some(state) = &mut self.window.native_settings {
            state.select_category(category);
        }
        self.show_preferences(window, cx);
    }
    pub(super) fn palette_channels(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.switch_surface(Sidebar::Channels, window, cx);
        if self.window.channels.is_none() {
            if let Some(service) = self.service.clone() {
                let channels = cx.new(|cx| channels::ChannelPane::new(service, window, cx));
                self.window.channel_subscription =
                    Some(cx.subscribe_workspace(&channels, window, |view, _, event, _, cx| match event {
                        channels::ChannelEvent::Refresh => view.refresh(cx),
                        channels::ChannelEvent::HistoryPage {
                            channel,
                            before,
                            page,
                        } => view.accept_channel_page(channel, before.as_deref(), page.clone(), cx),
                    }));
                self.window.channels = Some(channels);
            }
        }
        if let Some(pane) = &self.window.channels {
            pane.update(cx, |pane, cx| {
                pane.sync(
                    self.snapshot.clone(),
                    self.revision.as_deref().unwrap_or_default(),
                    false,
                    &[],
                    &self.window.channel_cursors,
                    cx,
                )
            });
        }
    }
    pub(super) fn preferred_project_for_current_pane(&self) -> Option<String> {
        let task_id = self.window.selected.as_deref()?;
        rows(&self.snapshot, "tasks").into_iter().find(|task| field(task, "id") == task_id)
            .and_then(|task| task["projectId"].as_str().map(str::to_owned))
    }
    fn palette_projects(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.switch_surface(Sidebar::Projects, window, cx);
        if self.window.workspace.is_none() {
            if let Some(service) = self.service.clone() {
                let preferred_project = self.preferred_project_for_current_pane();
                self.window.workspace = Some(cx.new(|cx| {
                    workspace::WorkspacePane::new(service, self.snapshot.clone(), preferred_project, window, cx)
                }));
            }
        }
    }
    pub(super) fn execute_catalog_entry(
        &mut self,
        row: &E,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let id = row.id.as_str();
        if let Some((workspace, pane)) = id.strip_prefix("workspace-pane:").and_then(|id| id.split_once(':')) {
            if let Some(item) = self.named_workspaces.as_mut().and_then(|set| set.items.iter_mut().find(|item| item.id == workspace)) {
                if let Some(layout) = &mut item.layout {
                    crate::workspace_visibility::remember_focus(layout, Some(pane));
                }
            }
            self.switch_named_workspace(workspace.to_owned(), window, cx);
            return true;
        }
        if let Some(pane) = id.strip_prefix("pane:") {
            self.window.dock.update(cx, |dock, cx| {
                dock.set_shown(true, window, cx);
                dock.reveal_and_focus(pane, window, cx)
            });
            return true;
        }
        if let Some(session_id) = id.strip_prefix("terminal:") {
            let session = self.window.palette_terminal_sessions
                .iter()
                .cloned()
                .chain(
                    self.window.terminal
                        .iter()
                        .chain(self.window.terminal_panes.values())
                        .flat_map(|pane| pane.read(cx).palette_sessions().to_vec()),
                )
                .find(|session| field(session, "id") == session_id);
            if let Some(session) = session {
                if let Err(error) = self.open_terminal_pane(session, None, window, cx) {
                    self.window.error = Some(error);
                }
            }
            return true;
        }
        if let Some(mode) = id.strip_prefix("motion:") {
            if let Some(mode) = native_motion::MotionPreference::parse(mode) {
                native_motion::set_preference(mode, cx);
            }
            return true;
        }
        if let Some(theme) = id.strip_prefix("terminal-theme:") {
            if let Some(theme) = terminal_theme::choose(theme) {
                cx.set_global(terminal_theme::Selection(theme.palette_index));
                cx.notify();
            }
            return true;
        }
        if let Some(workspace) = id.strip_prefix("workspace:") {
            self.switch_named_workspace(workspace.into(), window, cx);
            return true;
        }
        if id == "draft:retained" {
            self.open_chat_draft(None, window, cx);
            return true;
        }
        if let Some(project) = id.strip_prefix("project:") {
            let value = rows(&self.snapshot, "projects")
                .into_iter()
                .find(|p| field(p, "id") == project);
            self.palette_projects(window, cx);
            if let (Some(pane), Some(value)) = (&self.window.workspace, value) {
                pane.update(cx, |pane, cx| pane.select(value, window, cx));
            }
            return true;
        }
        if let Some(channel) = id.strip_prefix("channel:") {
            self.palette_channels(window, cx);
            if let Some(pane) = &self.window.channels {
                pane.update(cx, |pane, cx| pane.select(Some(channel.into()), window, cx));
            }
            return true;
        }
        if let Some(agent) = id.strip_prefix("agent:") {
            self.palette_settings_category("agents", window, cx);
            if self.window.agent_editor.is_none() {
                if let Some(service) = self.service.clone() {
                    self.window.agent_editor = Some(cx.new(|cx| {
                        agents::AgentHostPane::new(
                            service,
                            self.snapshot.as_ref().clone(),
                            window,
                            cx,
                        )
                    }));
                }
            }
            if let Some(pane) = &self.window.agent_editor {
                pane.update(cx, |pane, cx| pane.select_agent(agent, window, cx));
            }
            return true;
        }
        #[cfg(target_os = "macos")]
        if let Some((_, width, height)) = WINDOW_CENTER_PRESETS.iter().find(|(key, _, _)| *key == id) {
            if let Err(error) = window_state::center_at_percent(window, *width, *height, cx) {
                self.window.error = Some(error);
                cx.notify();
            }
            return true;
        }
        if let Some((_, _, category)) = SETTINGS.iter().find(|(key, _, _)| *key == id) {
            self.palette_settings_category(category, window, cx);
            if id == "hosts" || id == "agents-directory" {
                if self.window.agent_editor.is_none() {
                    if let Some(service) = self.service.clone() {
                        self.window.agent_editor = Some(cx.new(|cx| {
                            agents::AgentHostPane::new(
                                service,
                                self.snapshot.as_ref().clone(),
                                window,
                                cx,
                            )
                        }));
                    }
                }
                if let Some(pane) = &self.window.agent_editor {
                    pane.update(cx, |pane, cx| {
                        if id == "hosts" {
                            pane.show_hosts(cx)
                        } else {
                            pane.show_agents(cx)
                        }
                    });
                }
            }
            return true;
        }
        if let Some(patch) = toggle_patch(id, &self.snapshot["settings"]) {
            self.patch_settings(patch, cx);
            return true;
        }
        if let Some(view) = id.strip_prefix("sidebar:") {
            self.window.sidebar_view = match view {
                "projects" => SidebarView::Projects,
                "activity" => SidebarView::Activity,
                _ => SidebarView::Agents,
            };
            cx.notify();
            return true;
        }
        for (prefix, key) in [
            ("window-surface:", "windowSurface"),
            ("density:", "interfaceDensity"),
            ("tab-style:", "tabStyle"),
            ("shortcut-mode:", "shortcutMode"),
        ] {
            if let Some(value) = id.strip_prefix(prefix) {
                self.patch_settings(json!({key:value}), cx);
                return true;
            }
        }
        if let Some(preset) = id.strip_prefix("layout:") {
            if let Err(error) = self.window.dock
                .update(cx, |dock, cx| dock.apply_layout_preset(preset, window, cx))
            {
                self.window.error = Some(error);
            } else {
                self.queue_preferences_save(cx);
            }
            return true;
        }
        for (prefix, scope) in [
            ("apply-theme:", 0),
            ("app-theme-light:", 1),
            ("app-theme-dark:", 2),
        ] {
            if let Some(preset) = id.strip_prefix(prefix) {
                if let Some(preferences) = &mut self.preferences {
                    if scope != 2 {
                        preferences.data.appearance.light_palette = preset.into();
                    }
                    if scope != 1 {
                        preferences.data.appearance.dark_palette = preset.into();
                    }
                    preferences.data.appearance.warm_paper = false;
                }
                self.native_theme_preset = "monitter".into();
                self.window.native_settings = None;
                self.queue_preferences_save(cx);
                return true;
            }
        }
        if [
            "scale-up",
            "scale-down",
            "scale-reset",
            "contrast-reset",
            "contrast-down",
            "contrast-up",
            "accent-reset",
            "tint-reset",
            "tint-down",
            "tint-up",
            "border-opacity-reset",
            "border-opacity-down",
            "border-opacity-up",
        ]
        .contains(&id)
        {
            if let Some(preferences) = &mut self.preferences {
                let a = &mut preferences.data.appearance;
                match id {
                    "scale-up" => a.interface_scale = (a.interface_scale + 5).min(200),
                    "scale-down" => a.interface_scale = a.interface_scale.saturating_sub(5).max(80),
                    "scale-reset" => a.interface_scale = 125,
                    "contrast-up" => a.contrast = (a.contrast + 10).min(100),
                    "contrast-down" => a.contrast = a.contrast.saturating_sub(10),
                    "contrast-reset" => a.contrast = 0,
                    "tint-up" => a.surface_tint = (a.surface_tint + 5).min(50),
                    "tint-down" => a.surface_tint = a.surface_tint.saturating_sub(5),
                    "tint-reset" => a.surface_tint = 5,
                    "border-opacity-up" => a.border_opacity = (a.border_opacity + 5).min(100),
                    "border-opacity-down" => a.border_opacity = a.border_opacity.saturating_sub(5),
                    "border-opacity-reset" => a.border_opacity = 100,
                    "accent-reset" => a.custom_accent = None,
                    _ => {}
                }
            }
            if id.starts_with("scale-") {
                if let Some(preferences) = &self.preferences {
                    design::set_interface_scale(preferences.data.appearance.interface_scale);
                }
            }
            self.window.native_settings = None;
            self.queue_preferences_save(cx);
            cx.notify();
            return true;
        }
        match id {
            "detail" => {
                if let Some(task) = &self.window.selected {
                    if let Some(pane) = self.chat_panes.borrow().get(task).cloned() {
                        pane.update(cx, |pane, cx| pane.toggle_companion(cx));
                    }
                }
            }
            "new-task" => self.open_chat_draft(None, window, cx),
            "neovim-files" => {
                if let Err(error) = self.show_neovim_files(window, cx) { self.window.error = Some(error); }
            }
            "new-neovim" => {
                if let Err(error) = self.open_neovim_pane(None, window, cx) {
                    self.window.error = Some(error);
                }
            }
            "new-terminal" => {
                let pane = self.window.dock.update(cx, |dock, cx| dock.add_focused_empty_tab(window, cx));
                match pane {
                    Ok(pane) => {
                        if let Err(error) = self.empty_pane_action(
                            &pane,
                            empty_pane::EmptyPaneAction::Terminal,
                            window,
                            cx,
                        ) {
                            self.window.error = Some(error);
                        }
                    }
                    Err(error) => self.window.error = Some(error),
                }
            }
            "new-browser" => {
                self.ensure_browser(window, cx);
                match self
                    .create_browser_tab(window, cx)
                    .and_then(|id| self.open_browser_tab(id, window, cx))
                {
                    Ok(()) => {}
                    Err(error) => self.window.error = Some(error),
                }
            }
            "new-agent" => {
                self.palette_settings_category("agents", window, cx);
                if self.window.agent_editor.is_none() {
                    if let Some(service) = self.service.clone() {
                        self.window.agent_editor = Some(cx.new(|cx| {
                            agents::AgentHostPane::new(
                                service,
                                self.snapshot.as_ref().clone(),
                                window,
                                cx,
                            )
                        }));
                    }
                }
                if let Some(pane) = &self.window.agent_editor {
                    pane.update(cx, |pane, cx| pane.select_agent("", window, cx));
                }
            }
            "new-project" => {
                self.palette_projects(window, cx);
                if let Some(pane) = &self.window.workspace {
                    pane.update(cx, |pane, cx| pane.select(json!({}), window, cx));
                }
            }
            "ideas" => {
                if let Err(error) = self.open_ideas(window, cx) {
                    self.window.error = Some(error);
                }
            }
            "new-channel" => {
                self.palette_channels(window, cx);
                if let Some(pane) = &self.window.channels {
                    pane.update(cx, |pane, cx| pane.create(window, cx));
                }
            }
            "balance-panes" => {
                self.window.dock
                    .update(cx, |dock, cx| dock.equalize_workspace(window, cx));
                self.queue_preferences_save(cx);
            }
            "archived" => self.switch_surface(Sidebar::Archived, window, cx),
            "steer-busy" => {
                let steer = field(&self.snapshot["settings"], "busyMessageMode") == "steer";
                self.patch_settings(
                    json!({"busyMessageMode":if steer {"queue"}else{"steer"}}),
                    cx,
                );
            }
            "inactive-opacity-down" | "inactive-opacity-up" => {
                let opacity = self.snapshot["settings"]["inactivePaneOpacity"]
                    .as_f64()
                    .unwrap_or(0.6);
                let next =
                    (opacity + if id.ends_with("down") { -0.05 } else { 0.05 }).clamp(0.1, 0.9);
                self.patch_settings(json!({"inactivePaneOpacity":(next*100.).round()/100.}), cx);
            }
            "rename-all" => {
                let targets = rename_all_targets(self, cx);
                if !targets.is_empty() {
                    self.command("rename_all", json!({"targets":targets}), cx);
                }
            }
            "stop" | "archive" | "autoname" => {
                if let Some(task) = self.window.selected.clone() {
                    match id {
                        "stop" => self.command("cancel_task", json!({"taskId":task}), cx),
                        "archive" => self.command(
                            "set_task_archived",
                            json!({"taskId":task,"archived":true}),
                            cx,
                        ),
                        _ => self.command("autoname", json!({"target":{"taskId":task}}), cx),
                    }
                }
            }
            _ => return false,
        }
        cx.notify();
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[::core::prelude::v1::test]
    fn rename_all_chat_targets_deduplicate_and_exclude_private_closed_or_nonchat_panes() {
        let snapshot = json!({"agents":[{"id":"admin","internal":true},{"id":"worker"}],
            "tasks":[{"id":"a","agentId":"worker"},{"id":"b","agentId":"worker"},
                {"id":"old","archived":true},{"id":"hidden","agentId":"admin"},
                {"id":"closed","agentId":"worker"}]});
        let ids = ["chat:a","terminal:x","chat:a","chat:old","chat:hidden","browser:1",
            "chat:missing","chat:","chat:b"].map(str::to_owned);
        assert_eq!(rename_all_chat_targets(ids, &snapshot), vec![json!({"taskId":"a"}),json!({"taskId":"b"})]);
    }
    #[::core::prelude::v1::test]
    fn palette_static_creation_layout_settings_and_toggle_inventory_is_complete_unique() {
        let mut ids = std::collections::HashSet::new();
        for &(id, _) in CREATION_COMMANDS {
            assert!(ids.insert(id.to_string()));
        }
        for &(id, _) in LAYOUTS {
            assert!(ids.insert(format!("layout:{id}")));
        }
        for &(id, _, _) in SETTINGS {
            assert!(ids.insert(id.to_string()));
        }
        for &(id, _, _, _) in TOGGLES {
            assert!(ids.insert(id.to_string()));
        }
        let expected = [
            "new-task",
            "new-terminal",
            "new-agent",
            "new-channel",
            "new-project",
            "ideas",
            "new-browser",
            "new-neovim",
            "neovim-files",
            "balance-panes",
            "archived",
            "layout:single",
            "layout:columns",
            "layout:columns-3",
            "layout:columns-4",
            "layout:grid",
            "layout:grid-3x2",
            "layout:grid-4x2",
            "appearance",
            "typography-settings",
            "behaviour-settings",
            "session-lock-settings",
            "conversation-settings",
            "environment-secrets",
            "pane-appearance-settings",
            "appearance-detail-settings",
            "agents-directory",
            "hosts",
            "tools",
            "reasoning",
            "focus-mouse",
            "active-pane-border",
            "dim-panes",
            "enter",
            "compress-tools",
            "tint-messages",
            "show-tab-close",
            "auto-hide-tabs",
        ];
        assert_eq!(ids, expected.into_iter().map(str::to_owned).collect());
        assert_eq!(SETTINGS.iter().find(|(id, _, _)| *id == "pane-appearance-settings").unwrap().2, "interface");
        assert_eq!(SETTINGS.iter().find(|(id, _, _)| *id == "session-lock-settings").unwrap().2, "security");
        let appearance_ids = settings::native_appearance::PRESETS
            .iter()
            .flat_map(|(id, _)| {
                [
                    format!("apply-theme:{id}"),
                    format!("app-theme-light:{id}"),
                    format!("app-theme-dark:{id}"),
                ]
            })
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(appearance_ids.len(), 30);
        assert_eq!(terminal_theme::THEMES.len(), 10);
        assert_eq!(native_motion::MotionPreference::ALL.len(), 4);
    }
    #[::core::prelude::v1::test]
    fn palette_toggle_dispatch_inverts_the_actual_setting_and_honors_defaults() {
        let mut settings = json!({});
        for &(id, _, key, default) in TOGGLES {
            let patch = toggle_patch(id, &settings).unwrap();
            assert_eq!(patch[key], json!(!default));
            settings[key] = json!(!default);
            assert_eq!(toggle_patch(id, &settings).unwrap()[key], json!(default));
        }
        assert!(toggle_patch("not-a-command", &settings).is_none());
    }
}
