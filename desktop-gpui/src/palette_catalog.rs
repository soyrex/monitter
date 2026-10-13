//! One native catalog shared by both keyboard aliases. Enumeration is local;
//! execution revalidates the fresh catalog in Desktop::execute_palette.
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
    }
}
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
    ("new-agent", "New agent"),
    ("new-channel", "New channel"),
    ("new-project", "New project"),
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
pub(super) fn entries(view: &Desktop, cx: &App) -> Vec<E> {
    let blocked = view.busy || view.service.is_none();
    let settings = &view.snapshot["settings"];
    let mut result = vec![];
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
    if let Ok(layout) = view.dock.read(cx).serialize(cx) {
        if let Ok(ids) = docking::DockWorkspace::saved_ids(&layout) {
            for id in ids {
                let kind = if id.starts_with("terminal:") {
                    K::Terminal
                } else {
                    K::Pane
                };
                let label = view
                    .dock
                    .read(cx)
                    .pane_title(&id, cx)
                    .map(|title| title.to_string())
                    .filter(|title| !title.trim().is_empty())
                    .unwrap_or_else(|| id.clone());
                result.push(entry(
                    format!("pane:{id}"),
                    label,
                    "Focus open native pane",
                    kind,
                    false,
                    None,
                ));
            }
        }
    }
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
        if !id.is_empty() {
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
        if self.native_settings.is_none() {
            self.native_settings = Some(settings::NativeSettings::new(
                &self.snapshot["settings"],
                window,
                cx,
            ));
        }
        if let Some(state) = &mut self.native_settings {
            state.select_category(category);
        }
        self.show_preferences(window, cx);
    }
    fn palette_channels(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.switch_surface(Sidebar::Channels, window, cx);
        if self.channels.is_none() {
            if let Some(service) = self.service.clone() {
                let channels = cx.new(|cx| channels::ChannelPane::new(service, window, cx));
                self.channel_subscription =
                    Some(cx.subscribe(&channels, |view, _, event, cx| match event {
                        channels::ChannelEvent::Refresh => view.refresh(cx),
                        channels::ChannelEvent::HistoryPage {
                            channel,
                            before,
                            page,
                        } => view.accept_channel_page(channel, before.as_deref(), page.clone(), cx),
                    }));
                self.channels = Some(channels);
            }
        }
        if let Some(pane) = &self.channels {
            pane.update(cx, |pane, cx| {
                pane.sync(
                    self.snapshot.clone(),
                    self.revision.as_deref().unwrap_or_default(),
                    false,
                    &[],
                    &self.channel_cursors,
                    cx,
                )
            });
        }
    }
    fn palette_projects(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.switch_surface(Sidebar::Projects, window, cx);
        if self.workspace.is_none() {
            if let Some(service) = self.service.clone() {
                self.workspace = Some(cx.new(|cx| {
                    workspace::WorkspacePane::new(service, self.snapshot.clone(), window, cx)
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
        if let Some(pane) = id.strip_prefix("pane:") {
            self.dock.update(cx, |dock, cx| {
                dock.set_shown(true, window, cx);
                dock.focus(pane, window, cx)
            });
            return true;
        }
        if let Some(session_id) = id.strip_prefix("terminal:") {
            let session = self
                .palette_terminal_sessions
                .iter()
                .cloned()
                .chain(
                    self.terminal
                        .iter()
                        .chain(self.terminal_panes.values())
                        .flat_map(|pane| pane.read(cx).palette_sessions().to_vec()),
                )
                .find(|session| field(session, "id") == session_id);
            if let Some(session) = session {
                if let Err(error) = self.open_terminal_pane(session, None, window, cx) {
                    self.error = Some(error);
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
            if let (Some(pane), Some(value)) = (&self.workspace, value) {
                pane.update(cx, |pane, cx| pane.select(value, window, cx));
            }
            return true;
        }
        if let Some(channel) = id.strip_prefix("channel:") {
            self.palette_channels(window, cx);
            if let Some(pane) = &self.channels {
                pane.update(cx, |pane, cx| pane.select(Some(channel.into()), window, cx));
            }
            return true;
        }
        if let Some(agent) = id.strip_prefix("agent:") {
            self.palette_settings_category("agents", window, cx);
            if self.agent_editor.is_none() {
                if let Some(service) = self.service.clone() {
                    self.agent_editor = Some(cx.new(|cx| {
                        agents::AgentHostPane::new(
                            service,
                            self.snapshot.as_ref().clone(),
                            window,
                            cx,
                        )
                    }));
                }
            }
            if let Some(pane) = &self.agent_editor {
                pane.update(cx, |pane, cx| pane.select_agent(agent, window, cx));
            }
            return true;
        }
        if let Some((_, _, category)) = SETTINGS.iter().find(|(key, _, _)| *key == id) {
            self.palette_settings_category(category, window, cx);
            if id == "hosts" || id == "agents-directory" {
                if self.agent_editor.is_none() {
                    if let Some(service) = self.service.clone() {
                        self.agent_editor = Some(cx.new(|cx| {
                            agents::AgentHostPane::new(
                                service,
                                self.snapshot.as_ref().clone(),
                                window,
                                cx,
                            )
                        }));
                    }
                }
                if let Some(pane) = &self.agent_editor {
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
            self.sidebar_view = match view {
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
            if let Err(error) = self
                .dock
                .update(cx, |dock, cx| dock.apply_layout_preset(preset, window, cx))
            {
                self.error = Some(error);
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
                self.native_settings = None;
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
            self.native_settings = None;
            self.queue_preferences_save(cx);
            cx.notify();
            return true;
        }
        match id {
            "detail" => {
                if let Some(task) = &self.selected {
                    if let Some(pane) = self.chat_panes.borrow().get(task).cloned() {
                        pane.update(cx, |pane, cx| pane.toggle_companion(cx));
                    }
                }
            }
            "new-task" => self.open_chat_draft(None, window, cx),
            "new-neovim" => {
                if let Err(error) = self.open_neovim_pane(None, window, cx) {
                    self.error = Some(error);
                }
            }
            "new-terminal" => {
                let pane = self.dock.update(cx, |dock, cx| {
                    let group = dock
                        .focused_group_node(window, cx)
                        .ok_or("No focused native pane.")?;
                    dock.add_empty_tab(group, window, cx)
                });
                match pane {
                    Ok(pane) => {
                        if let Err(error) = self.empty_pane_action(
                            &pane,
                            empty_pane::EmptyPaneAction::Terminal,
                            window,
                            cx,
                        ) {
                            self.error = Some(error);
                        }
                    }
                    Err(error) => self.error = Some(error),
                }
            }
            "new-browser" => {
                self.ensure_browser(window, cx);
                match self
                    .create_browser_tab(window, cx)
                    .and_then(|id| self.open_browser_tab(id, window, cx))
                {
                    Ok(()) => {}
                    Err(error) => self.error = Some(error),
                }
            }
            "new-agent" => {
                self.palette_settings_category("agents", window, cx);
                if self.agent_editor.is_none() {
                    if let Some(service) = self.service.clone() {
                        self.agent_editor = Some(cx.new(|cx| {
                            agents::AgentHostPane::new(
                                service,
                                self.snapshot.as_ref().clone(),
                                window,
                                cx,
                            )
                        }));
                    }
                }
                if let Some(pane) = &self.agent_editor {
                    pane.update(cx, |pane, cx| pane.select_agent("", window, cx));
                }
            }
            "new-project" => {
                self.palette_projects(window, cx);
                if let Some(pane) = &self.workspace {
                    pane.update(cx, |pane, cx| pane.select(json!({}), window, cx));
                }
            }
            "new-channel" => {
                self.palette_channels(window, cx);
                if let Some(pane) = &self.channels {
                    pane.update(cx, |pane, cx| pane.create(window, cx));
                }
            }
            "balance-panes" => {
                self.dock
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
            "stop" | "archive" | "autoname" => {
                if let Some(task) = self.selected.clone() {
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
            "new-browser",
            "new-neovim",
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
