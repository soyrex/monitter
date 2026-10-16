//! Retained native keyboard selector. Entries are supplied by the owner root;
//! selection emits intent and never invokes a backend or creates a task/session.
use crate::design::ui_scaled_px as px;
use gpui_kit::assets::IconName;
use gpui_kit::{
    component::{
        ActiveTheme, Icon, Selectable as _, Sizable as _,
        button::*,
        input::{Enter as InputEnter, Escape as InputEscape, Input, InputEvent, InputState},
    },
    prelude::*,
    *,
};
use std::collections::{HashMap, HashSet};
#[path = "palette_search.rs"]
mod palette_search;
use palette_search::PaletteSearch;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PaletteMode {
    Switch,
    Commands,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    Task,
    ArchivedTask,
    Draft,
    Terminal,
    Channel,
    Agent,
    Project,
    Route,
    Command,
    Pane,
    Workspace,
}
impl EntryKind {
    fn label(self) -> &'static str {
        match self {
            Self::Task => "Chat",
            Self::ArchivedTask => "Archived chat",
            Self::Draft => "Draft chat",
            Self::Terminal => "Terminal",
            Self::Channel => "Channel",
            Self::Agent => "Agent",
            Self::Project => "Project",
            Self::Route => "Workspace",
            Self::Command => "Command",
            Self::Pane => "Pane",
            Self::Workspace => "Workspace",
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaletteEntry {
    pub id: String,
    pub label: String,
    pub subtitle: String,
    pub kind: EntryKind,
    pub keywords: String,
    pub search_text: String,
    pub open_priority: u8,
    pub disabled: bool,
    pub checked: Option<bool>,
    pub keyboard_hint: Option<String>,
}
#[derive(Debug, Clone)]
pub enum PaletteEvent {
    Selected(PaletteEntry),
    Cancelled,
}

fn validate_entries(entries: &[PaletteEntry]) -> Result<(), String> {
    let mut ids = HashSet::new();
    for entry in entries {
        if entry.id.is_empty()
            || entry.id.len() > 256
            || entry.label.trim().is_empty()
            || !ids.insert(&entry.id)
        {
            return Err("Palette entries require unique stable IDs and nonempty labels.".into());
        }
    }
    Ok(())
}
/// Filters are independent of which shortcut opened the menu.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PaletteCategory {
    #[default]
    All,
    Commands,
    Sessions,
    Panes,
    Terminals,
    Agents,
    Projects,
    Channels,
    Workspaces,
}
impl PaletteCategory {
    const ALL: [Self; 9] = [
        Self::All,
        Self::Commands,
        Self::Sessions,
        Self::Panes,
        Self::Terminals,
        Self::Agents,
        Self::Projects,
        Self::Channels,
        Self::Workspaces,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Commands => "Commands",
            Self::Sessions => "Sessions",
            Self::Panes => "Panes",
            Self::Terminals => "Terminals",
            Self::Agents => "Agents",
            Self::Projects => "Projects",
            Self::Channels => "Channels",
            Self::Workspaces => "Workspaces",
        }
    }
    pub fn includes_entry(self, entry: &PaletteEntry) -> bool {
        if self == Self::Panes {
            entry.kind == EntryKind::Pane
                || entry.id.starts_with("pane:")
                || entry.open_priority < 2
        } else {
            self.includes(entry.kind)
        }
    }
    pub fn includes(self, kind: EntryKind) -> bool {
        match self {
            Self::All => true,
            Self::Commands => kind == EntryKind::Command,
            Self::Sessions => matches!(
                kind,
                EntryKind::Task | EntryKind::ArchivedTask | EntryKind::Draft
            ),
            Self::Panes => kind == EntryKind::Pane,
            Self::Terminals => kind == EntryKind::Terminal,
            Self::Agents => kind == EntryKind::Agent,
            Self::Projects => kind == EntryKind::Project,
            Self::Channels => kind == EntryKind::Channel,
            Self::Workspaces => matches!(kind, EntryKind::Workspace | EntryKind::Route),
        }
    }
}

fn entry_icon(kind: EntryKind) -> IconName {
    match kind {
        EntryKind::Task | EntryKind::Draft => IconName::MessageSquare,
        EntryKind::ArchivedTask => IconName::Archive,
        EntryKind::Terminal => IconName::SquareTerminal,
        EntryKind::Channel => IconName::Radio,
        EntryKind::Agent => IconName::Bot,
        EntryKind::Project => IconName::Folder,
        EntryKind::Pane => IconName::PanelsTopLeft,
        EntryKind::Route | EntryKind::Workspace => IconName::LayoutGrid,
        EntryKind::Command => IconName::Command,
    }
}

fn bounded_text(value: &str) -> String {
    value.chars().take(1024).collect::<String>().to_lowercase()
}
fn subsequence(text: &str, term: &str) -> bool {
    let mut remaining = term.chars();
    let mut next = remaining.next();
    for ch in text.chars() {
        if next == Some(ch) {
            next = remaining.next();
        }
        if next.is_none() {
            return true;
        }
    }
    next.is_none()
}
/// Tauri's ranking: exact title, prefix, substring, title words, descriptor,
/// then ordered subsequences. Bounds keep local typing work predictable.
fn match_score(entry: &PaletteEntry, query: &str) -> Option<u8> {
    let normalized = query.chars().take(256).collect::<String>().to_lowercase();
    let normalized = normalized.trim();
    let terms: Vec<_> = normalized.split_whitespace().take(32).collect();
    if terms.is_empty() {
        return Some(0);
    }
    let label = bounded_text(&entry.label);
    let descriptor = format!(
        "{} {} {}",
        bounded_text(&entry.subtitle),
        bounded_text(&entry.keywords),
        entry.kind.label().to_lowercase()
    );
    if label == normalized {
        Some(0)
    } else if label.starts_with(normalized) {
        Some(1)
    } else if label.contains(normalized) {
        Some(2)
    } else if terms.iter().all(|term| label.contains(term)) {
        Some(3)
    } else if descriptor.contains(normalized) || terms.iter().all(|term| descriptor.contains(term))
    {
        Some(4)
    } else {
        let text = format!("{label} {descriptor}");
        terms
            .iter()
            .all(|term| subsequence(&text, term))
            .then_some(5)
    }
}
fn matches(entry: &PaletteEntry, query: &str) -> bool {
    match_score(entry, query).is_some()
}
#[cfg(test)]
fn ranked_entries(entries: &[PaletteEntry], category: PaletteCategory, query: &str) -> Vec<usize> {
    let mut search = PaletteSearch::default();
    search.update(entries);
    search.ranked(entries, category, query)
}
fn moved_selection(
    entries: &[PaletteEntry],
    visible: &[usize],
    current: Option<usize>,
    forward: bool,
) -> Option<usize> {
    let enabled: Vec<_> = visible
        .iter()
        .copied()
        .filter(|index| !entries[*index].disabled)
        .collect();
    if enabled.is_empty() {
        return None;
    }
    let selected =
        current.and_then(|index| enabled.iter().position(|candidate| *candidate == index));
    Some(match selected {
        None => {
            if forward {
                enabled[0]
            } else {
                *enabled.last().unwrap()
            }
        }
        Some(position) => {
            enabled[if forward {
                (position + 1) % enabled.len()
            } else {
                (position + enabled.len() - 1) % enabled.len()
            }]
        }
    })
}

pub struct CommandPalette {
    query: Entity<InputState>,
    entries: Vec<PaletteEntry>,
    search: PaletteSearch,
    generation: u64,
    async_content: HashMap<String, String>,
    visible: Vec<usize>,
    selected: Option<usize>,
    mode: PaletteMode,
    category: PaletteCategory,
    category_scroll: ScrollHandle,
    open: bool,
    previous_focus: Option<FocusHandle>,
    scroll: UniformListScrollHandle,
    _input_events: Subscription,
}
impl EventEmitter<PaletteEvent> for CommandPalette {}
impl Focusable for CommandPalette {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.query.read(cx).focus_handle(cx)
    }
}
impl CommandPalette {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let query =
            cx.new(|cx| InputState::new(window, cx).placeholder("Search locally available items…"));
        let events = cx.subscribe_in(
            &query,
            window,
            |palette, _, event, window, cx| match event {
                InputEvent::Change => {
                    palette.selected = None;
                    palette.refilter(cx);
                }
                InputEvent::PressEnter {
                    secondary: false,
                    shift: false,
                } => {
                    if !palette.composing(window, cx) {
                        palette.confirm(window, cx);
                    }
                }
                _ => {}
            },
        );
        Self {
            query,
            entries: vec![],
            search: PaletteSearch::default(),
            generation: 0,
            async_content: HashMap::new(),
            visible: vec![],
            selected: None,
            mode: PaletteMode::Switch,
            category: PaletteCategory::All,
            category_scroll: ScrollHandle::new(),
            open: false,
            previous_focus: None,
            scroll: UniformListScrollHandle::default(),
            _input_events: events,
        }
    }
    /// Both shortcut aliases open the same retained query and category.
    /// Host supplies the real current catalog; the palette emits intent only.
    pub fn open(
        &mut self,
        mode: PaletteMode,
        entries: Vec<PaletteEntry>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        validate_entries(&entries)?;
        if !self.open {
            self.previous_focus = window.focused(cx);
        }
        self.open = true;
        self.generation = self.generation.wrapping_add(1);
        self.async_content.clear();
        self.mode = mode;
        self.search.update(&entries);
        self.entries = entries;
        self.selected = None;
        self.refilter(cx);
        self.query.update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
        Ok(())
    }
    /// Refresh the supplied entries without replacing the query or selected ID.
    /// The root must revalidate the selected ID before executing any authority.
    pub fn set_entries(
        &mut self,
        mut entries: Vec<PaletteEntry>,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        validate_entries(&entries)?;
        let selected = self
            .selected
            .and_then(|index| self.entries.get(index))
            .map(|entry| entry.id.clone());
        self.async_content
            .retain(|id, _| entries.iter().any(|entry| &entry.id == id));
        for entry in &mut entries {
            if let Some(text) = self.async_content.get(&entry.id) {
                entry.search_text.clone_from(text);
            }
        }
        self.search.update(&entries);
        self.entries = entries;
        self.selected =
            selected.and_then(|id| self.entries.iter().position(|entry| entry.id == id));
        self.refilter(cx);
        Ok(())
    }
    /// Identifies this opening so late browser snapshots cannot overwrite a newer catalog.
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn set_search_text(
        &mut self,
        id: &str,
        generation: u64,
        text: String,
        cx: &mut Context<Self>,
    ) {
        if !self.open || generation != self.generation {
            return;
        }
        let Some(entry) = self.entries.iter_mut().find(|entry| entry.id == id) else {
            return;
        };
        self.async_content.insert(id.to_owned(), text.clone());
        if entry.search_text == text {
            return;
        }
        entry.search_text = text;
        let followed_top = self.selected == self.visible.first().copied();
        self.search.update(&self.entries);
        // An asynchronously discovered pane should become the default result,
        // while a selection moved further down the list remains stable.
        if followed_top {
            self.selected = None;
        }
        self.refilter(cx);
        cx.notify();
    }
    pub fn is_open(&self) -> bool {
        self.open
    }
    pub fn category(&self) -> PaletteCategory {
        self.category
    }
    pub fn set_category(&mut self, category: PaletteCategory, cx: &mut Context<Self>) {
        self.category = category;
        self.selected = None;
        self.refilter(cx);
    }
    pub fn visible_entries(&self) -> impl Iterator<Item = &PaletteEntry> {
        self.visible.iter().map(|index| &self.entries[*index])
    }
    pub fn mode(&self) -> PaletteMode {
        self.mode
    }
    pub fn dismiss(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.open {
            return;
        }
        self.close(window, cx);
        cx.emit(PaletteEvent::Cancelled);
    }
    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open = false;
        if let Some(focus) = self.previous_focus.take() {
            window.focus(&focus, cx);
        }
        cx.notify();
    }
    fn composing(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.query.update(cx, |input, cx| {
            input.marked_text_range(window, cx).is_some()
        })
    }
    fn refilter(&mut self, cx: &mut Context<Self>) {
        let query = self.query.read(cx).value().to_string();
        self.visible = self.search.ranked(&self.entries, self.category, &query);
        if !self
            .selected
            .is_some_and(|index| self.visible.contains(&index) && !self.entries[index].disabled)
        {
            self.selected = moved_selection(&self.entries, &self.visible, None, true);
        }
        self.reveal();
        cx.notify();
    }
    fn reveal(&self) {
        if let Some(position) = self.selected.and_then(|index| {
            self.visible
                .iter()
                .position(|candidate| *candidate == index)
        }) {
            self.scroll
                .scroll_to_item(position, ScrollStrategy::Nearest);
        }
    }
    fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.open {
            return;
        }
        if let Some(entry) = self
            .selected
            .and_then(|index| self.entries.get(index))
            .filter(|entry| !entry.disabled)
            .cloned()
        {
            self.close(window, cx);
            cx.emit(PaletteEvent::Selected(entry));
        }
    }
    fn key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.open
            || event.keystroke.modifiers.control
            || event.keystroke.modifiers.platform
            || event.keystroke.modifiers.alt
            || event.keystroke.modifiers.shift
        {
            return;
        }
        let key = event.keystroke.key.as_str();

        if !["up", "down", "enter", "escape"].contains(&key) {
            return;
        }
        if self.composing(window, cx) {
            if key == "enter" || key == "escape" {
                // Guard before InputState commits/cancels marked text and
                // redispatches a confirmation/dismissal with no marked range.
                // Native IME handling precedes this ordinary key fallback.
                window.prevent_default();
                cx.stop_propagation();
            }
            return;
        }
        if key == "enter" {
            return; // InputState emits the normal confirmation event.
        }
        match key {
            "escape" => self.dismiss(window, cx),
            "up" | "down" => {
                self.selected =
                    moved_selection(&self.entries, &self.visible, self.selected, key == "down");
                self.reveal();
                cx.notify();
            }
            _ => {}
        }
        cx.stop_propagation();
    }
}
impl Render for CommandPalette {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.open {
            return div().into_any_element();
        }
        let weak = cx.weak_entity();
        // Rows need presentation metadata only; content snapshots can be large.
        let entries: Vec<_> = self
            .entries
            .iter()
            .map(|entry| PaletteEntry {
                id: entry.id.clone(),
                label: entry.label.clone(),
                subtitle: entry.subtitle.clone(),
                kind: entry.kind,
                keywords: String::new(),
                search_text: String::new(),
                open_priority: entry.open_priority,
                disabled: entry.disabled,
                checked: entry.checked,
                keyboard_hint: entry.keyboard_hint.clone(),
            })
            .collect();
        let snippets = self.search.snippets().clone();
        let visible = self.visible.clone();
        let selected = self.selected;
        let background = cx.theme().background;
        let foreground = cx.theme().foreground;
        let muted = cx.theme().muted_foreground;
        let accent = cx.theme().primary;
        let border = cx.theme().border;
        let highlighted = cx.theme().muted;
        let items = uniform_list("palette-results", visible.len(), move |range, _, _| {
            range
                .map(|position| {
                    let index = visible[position];
                    let entry = &entries[index];
                    let weak = weak.clone();
                    let id = entry.id.clone();
                    let active = selected == Some(index);
                    div()
                        .id(SharedString::from(format!("palette-entry-{id}")))
                        .debug_selector({
                            let id = id.clone();
                            move || format!("palette-entry-{id}").into()
                        })
                        .w_full()
                        .min_w_0()
                        .h(px(58.))
                        .px_3()
                        .py_2()
                        .flex()
                        .items_center()
                        .gap_3()
                        .rounded(px(7.))
                        .border_l_2()
                        .border_color(if active { accent } else { accent.opacity(0.) })
                        .when(active, |row| row.bg(highlighted))
                        .when(!entry.disabled, |row| {
                            row.cursor_pointer().hover(|style| style.bg(highlighted))
                        })
                        .opacity(if entry.disabled { 0.5 } else { 1. })
                        .text_color(foreground)
                        .child(
                            div()
                                .w(px(22.))
                                .h(px(22.))
                                .flex_shrink_0()
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(
                                    Icon::new(entry_icon(entry.kind))
                                        .size(px(19.))
                                        .text_color(if active { accent } else { muted }),
                                ),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .gap_1()
                                .child(
                                    div()
                                        .text_size(px(13.))
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_ellipsis()
                                        .child(entry.label.clone()),
                                )
                                .child(
                                    div()
                                        .text_size(px(11.))
                                        .text_color(muted)
                                        .text_ellipsis()
                                        .child(format!(
                                            "{} · {}{}{}",
                                            entry.kind.label(),
                                            entry.subtitle,
                                            snippets
                                                .get(&entry.id)
                                                .map(|snippet| format!(" · {snippet}"))
                                                .unwrap_or_default(),
                                            if entry.disabled {
                                                " · unavailable"
                                            } else {
                                                ""
                                            }
                                        )),
                                ),
                        )
                        .when_some(entry.checked, |row, checked| {
                            row.child(
                                div()
                                    .text_color(if checked { accent } else { muted })
                                    .child(if checked { "✓" } else { "○" }),
                            )
                        })
                        .when_some(entry.keyboard_hint.clone(), |row, hint| {
                            row.child(
                                div()
                                    .text_size(px(10.))
                                    .text_color(muted)
                                    .px_1()
                                    .py(px(2.))
                                    .rounded(px(4.))
                                    .border_1()
                                    .border_color(border)
                                    .child(hint),
                            )
                        })
                        .when(active, |row| row.child(div().text_color(muted).child("↵")))
                        .on_click(move |_, window, cx| {
                            let _ = weak.update(cx, |palette, cx| {
                                palette.selected = palette
                                    .entries
                                    .iter()
                                    .position(|entry| entry.id == id && !entry.disabled);
                                palette.confirm(window, cx);
                            });
                        })
                })
                .collect::<Vec<_>>()
        })
        .h(px(348.))
        .w_full()
        .track_scroll(&self.scroll);
        let items = ScrollEdgeFade::new(
            items,
            self.scroll.0.borrow().base_handle.clone(),
            Axis::Vertical,
            gpui_kit::px(20.),
        );
        let categories = div()
            .id("palette-categories")
            .flex()
            .gap_1()
            .px_3()
            .py_2()
            .overflow_x_scroll()
            .track_scroll(&self.category_scroll)
            .children(PaletteCategory::ALL.into_iter().map(|category| {
                Button::new(SharedString::from(format!(
                    "palette-filter-{}",
                    category.label()
                )))
                .label(category.label())
                .small()
                .ghost()
                .rounded_full()
                .selected(self.category == category)
                .toggled(self.category == category)
                .accessibility_label(format!(
                    "{} filter{}",
                    category.label(),
                    if self.category == category {
                        ", selected"
                    } else {
                        ""
                    }
                ))
                .tooltip(format!("Show {}", category.label().to_lowercase()))
                .debug_selector(move || format!("palette-filter-{}", category.label()).into())
                .on_click(cx.listener(move |palette, _, window, cx| {
                    palette.set_category(category, cx);
                    palette
                        .query
                        .update(cx, |input, cx| input.focus(window, cx));
                }))
            }));
        let categories = ScrollEdgeFade::new(
            categories,
            self.category_scroll.clone(),
            Axis::Horizontal,
            gpui_kit::px(16.),
        );
        div()
            .id("native-command-palette")
            .w(px(660.))
            .max_w_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .rounded(px(13.))
            .border_1()
            .border_color(border)
            .shadow_lg()
            .bg(background)
            .text_color(foreground)
            .capture_action(cx.listener(|palette, _: &InputEscape, window, cx| {
                // Bound input actions run before low-level key listeners.
                // Stop Escape before it clears the input's marked text and
                // propagates as an ordinary palette dismissal.
                if palette.composing(window, cx) {
                    cx.stop_propagation();
                }
            }))
            .capture_action(cx.listener(|palette, _: &InputEnter, window, cx| {
                if palette.composing(window, cx) {
                    cx.stop_propagation();
                }
            }))
            .capture_key_down(cx.listener(Self::key))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px_3()
                    .py_2()
                    .border_b_1()
                    .border_color(border)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .text_color(muted)
                            .text_size(px(11.))
                            .child(Icon::new(IconName::Command).size(px(15.)))
                            .child("COMMAND MENU"),
                    )
                    .child(
                        Button::new("palette-dismiss")
                            .icon(IconName::X)
                            .small()
                            .ghost()
                            .accessibility_label("Close command menu")
                            .tooltip("Close (Esc)")
                            .on_click(
                                cx.listener(|palette, _, window, cx| palette.dismiss(window, cx)),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .border_b_1()
                    .border_color(border)
                    .child(Icon::new(IconName::Search).size(px(18.)).text_color(muted))
                    .child(Input::new(&self.query).appearance(false).flex_1()),
            )
            .child(categories)
            .when(self.visible.is_empty(), |panel| {
                panel.child(
                    div()
                        .h(px(348.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_color(muted)
                        .text_size(px(12.))
                        .child("No matching items"),
                )
            })
            .when(!self.visible.is_empty(), |panel| {
                panel.child(div().px_2().child(items))
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px_3()
                    .py_2()
                    .border_t_1()
                    .border_color(border)
                    .text_size(px(10.))
                    .text_color(muted)
                    .child(format!("{} results", self.visible.len()))
                    .child("↑ ↓ Navigate    ↵ Select    Esc Close"),
            )
            .into_any_element()
    }
}
#[cfg(test)]
mod tests {
    use super::{
        CommandPalette, EntryKind, PaletteCategory, PaletteEntry, PaletteMode, matches,
        moved_selection, ranked_entries, validate_entries,
    };
    use gpui_kit::{AppContext as _, EntityInputHandler as _, Modifiers, TestAppContext};
    fn entry(id: &str, label: &str, disabled: bool) -> PaletteEntry {
        PaletteEntry {
            id: id.into(),
            label: label.into(),
            subtitle: "Local agent".into(),
            kind: EntryKind::Task,
            keywords: "codex work".into(),
            search_text: String::new(),
            open_priority: 2,
            disabled,
            checked: None,
            keyboard_hint: None,
        }
    }
    #[::core::prelude::v1::test]
    fn query_matches_all_tokens_case_insensitively_across_fields() {
        let entry = entry("t", "Résumé review", false);
        assert!(matches(&entry, "RÉSUMÉ codex"));
        assert!(matches(&entry, "agent review"));
        assert!(!matches(&entry, "review missing"));
        assert!(matches(&entry, "   "));
    }
    #[::core::prelude::v1::test]
    fn keyboard_wraps_and_skips_unavailable_entries() {
        let entries = vec![
            entry("a", "One", false),
            entry("b", "Unavailable", true),
            entry("c", "Three", false),
        ];
        let visible = vec![0, 1, 2];
        assert_eq!(moved_selection(&entries, &visible, Some(0), true), Some(2));
        assert_eq!(moved_selection(&entries, &visible, Some(2), true), Some(0));
        assert_eq!(moved_selection(&entries, &visible, Some(0), false), Some(2));
        assert_eq!(moved_selection(&entries, &[1], None, true), None);
        assert_eq!(moved_selection(&entries, &[], None, true), None);
    }
    #[::core::prelude::v1::test]
    fn duplicate_or_empty_entry_ids_cannot_emit_ambiguous_intents() {
        assert!(validate_entries(&[entry("a", "One", false), entry("a", "Two", false)]).is_err());
        assert!(validate_entries(&[entry("", "Missing ID", false)]).is_err());
        assert!(validate_entries(&[entry("a", " ", false)]).is_err());
    }
    #[::core::prelude::v1::test]
    fn palette_search_ranks_titles_before_descriptors_and_fuzzy_matches() {
        let mut descriptor = entry("descriptor", "Open editor", false);
        descriptor.keywords = "terminal".into();
        let entries = vec![
            descriptor,
            entry(
                "fuzzy",
                "The external remote interactive native alpha list",
                false,
            ),
            entry("contains", "New terminal", false),
            entry("prefix", "Terminal tools", false),
            entry("exact", "Terminal", false),
        ];
        assert_eq!(
            ranked_entries(&entries, PaletteCategory::All, "terminal"),
            vec![4, 3, 2, 0, 1]
        );
        assert!(matches(&entry("unicode", "Résumé review", false), "ré rv"));
    }
    #[::core::prelude::v1::test]
    fn palette_categories_cover_content_kinds_without_losing_all() {
        for category in PaletteCategory::ALL {
            if category != PaletteCategory::All {
                assert!(
                    !category.includes(EntryKind::Command) || category == PaletteCategory::Commands
                );
            }
        }
        assert!(PaletteCategory::Sessions.includes(EntryKind::Draft));
        assert!(PaletteCategory::Sessions.includes(EntryKind::ArchivedTask));
        assert!(PaletteCategory::Panes.includes(EntryKind::Pane));
        assert!(!PaletteCategory::Panes.includes(EntryKind::Task));
        assert!(PaletteCategory::Workspaces.includes(EntryKind::Route));
        assert!(PaletteCategory::Workspaces.includes(EntryKind::Workspace));
    }

    #[::core::prelude::v1::test]
    fn palette_terminal_pane_appears_in_panes_and_terminals() {
        let mut pane = entry("pane:terminal:one", "Terminal pane", false);
        pane.kind = EntryKind::Terminal;
        let mut session = entry("terminal:one", "Terminal session", false);
        session.kind = EntryKind::Terminal;
        let entries = vec![pane, session];
        assert_eq!(
            ranked_entries(&entries, PaletteCategory::Panes, ""),
            vec![0]
        );
        assert_eq!(
            ranked_entries(&entries, PaletteCategory::Terminals, ""),
            vec![0, 1]
        );
    }

    #[gpui_kit::test]
    fn browser_snapshots_are_scoped_to_opening_and_survive_catalog_refresh(
        cx: &mut TestAppContext,
    ) {
        cx.update(gpui_kit::init);
        let (palette, cx) = cx.add_window_view(|window, cx| CommandPalette::new(window, cx));
        cx.update(|window, cx| {
            palette.update(cx, |palette, cx| {
                let catalog = vec![entry("browser", "Browser", false)];
                palette
                    .open(PaletteMode::Switch, catalog.clone(), window, cx)
                    .unwrap();
                let first = palette.generation();
                palette.set_search_text("browser", first, "snapshotneedle".into(), cx);
                assert_eq!(palette.entries[0].search_text, "snapshotneedle");
                palette.set_entries(catalog.clone(), cx).unwrap();
                assert_eq!(palette.entries[0].search_text, "snapshotneedle");
                assert_eq!(
                    palette
                        .search
                        .ranked(&palette.entries, PaletteCategory::All, "snapshotneedle"),
                    vec![0]
                );
                palette
                    .open(PaletteMode::Switch, catalog.clone(), window, cx)
                    .unwrap();
                let second = palette.generation();
                assert_ne!(first, second);
                palette.set_search_text("browser", first, "staleafterreopen".into(), cx);
                assert!(palette.entries[0].search_text.is_empty());
                palette.set_search_text("browser", second, "currentneedle".into(), cx);
                palette.close(window, cx);
                palette.set_search_text("browser", second, "staleafterclose".into(), cx);
                assert_eq!(palette.entries[0].search_text, "currentneedle");
                palette
                    .open(PaletteMode::Switch, catalog.clone(), window, cx)
                    .unwrap();
                let third = palette.generation();
                palette.set_search_text("browser", third, "removedneedle".into(), cx);
                palette.set_entries(vec![], cx).unwrap();
                palette.set_search_text("browser", third, "missingneedle".into(), cx);
                assert!(palette.entries.is_empty());
                assert!(palette.async_content.is_empty());
                palette.set_entries(catalog, cx).unwrap();
                assert!(palette.entries[0].search_text.is_empty());
                assert!(
                    palette
                        .search
                        .ranked(&palette.entries, PaletteCategory::All, "removedneedle")
                        .is_empty()
                );
            });
        });
    }

    #[gpui_kit::test]
    fn changing_query_selects_best_match_instead_of_previous_matching_row(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let (palette, cx) = cx.add_window_view(|window, cx| CommandPalette::new(window, cx));
        cx.update(|window, cx| {
            palette.update(cx, |palette, cx| {
                palette
                    .open(
                        PaletteMode::Switch,
                        vec![
                            entry("first", "Alpha needle", false),
                            entry("best", "Needle", false),
                        ],
                        window,
                        cx,
                    )
                    .unwrap();
                palette.query.update(cx, |input, cx| {
                    input.set_value("alpha", window, cx);
                    // Programmatic set_value deliberately does not emit Change.
                    cx.emit(super::InputEvent::Change);
                });
            });
        });
        cx.run_until_parked();
        cx.update(|window, cx| {
            palette.update(cx, |palette, cx| {
                assert_eq!(palette.selected, Some(0));
                palette.query.update(cx, |input, cx| {
                    input.set_value("needle", window, cx);
                    cx.emit(super::InputEvent::Change);
                });
            });
        });
        cx.run_until_parked();
        cx.update(|_, cx| {
            palette.update(cx, |palette, cx| {
                assert_eq!(palette.visible, vec![1, 0]);
                assert_eq!(palette.selected, Some(1));
                // A background refresh still preserves an explicit lower-ranked selection.
                palette.selected = Some(0);
                palette.set_entries(palette.entries.clone(), cx).unwrap();
                assert_eq!(palette.selected, Some(0));
            });
        });
    }

    #[gpui_kit::test]
    fn palette_shortcut_aliases_retain_query_category_and_restore_focus(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let (palette, cx) = cx.add_window_view(|window, cx| CommandPalette::new(window, cx));
        let original_focus = cx.update(|window, cx| {
            let focus = cx.focus_handle();
            window.focus(&focus, cx);
            focus
        });
        let mut terminal = entry("terminal", "Terminal", false);
        terminal.kind = EntryKind::Terminal;
        let mut disabled = terminal.clone();
        disabled.id = "unavailable".into();
        disabled.disabled = true;
        let entries = vec![disabled, terminal, entry("chat", "Chat", false)];
        cx.update(|window, cx| {
            palette.update(cx, |palette, cx| {
                palette
                    .open(PaletteMode::Switch, entries.clone(), window, cx)
                    .unwrap();
                palette
                    .query
                    .update(cx, |input, cx| input.set_value("codex", window, cx));
            })
        });
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let filter = cx
            .debug_bounds("palette-filter-Terminals")
            .expect("native category pill is drawn");
        cx.simulate_click(filter.center(), Modifiers::none());
        cx.run_until_parked();
        cx.update(|window, cx| {
            palette.update(cx, |palette, cx| {
                assert_eq!(palette.category(), PaletteCategory::Terminals);
                assert_eq!(palette.visible_entries().count(), 2);
                assert_eq!(
                    palette.selected.map(|ix| palette.entries[ix].id.as_str()),
                    Some("terminal")
                );
                palette
                    .open(PaletteMode::Commands, entries.clone(), window, cx)
                    .unwrap();
                assert_eq!(palette.query.read(cx).value().as_str(), "codex");
                assert_eq!(palette.category(), PaletteCategory::Terminals);
            })
        });
        cx.simulate_keystrokes("down");
        cx.update(|_, cx| {
            palette.update(cx, |palette, _| {
                assert_eq!(
                    palette.selected.map(|ix| palette.entries[ix].id.as_str()),
                    Some("terminal")
                );
            })
        });
        // A navigation key may legitimately commit/cancel the input's marked
        // text. Begin a fresh composition for each key so a subsequent ordinary
        // Escape is not mistaken for an Escape during composition.
        for key in ["down", "enter", "escape"] {
            cx.update(|window, cx| {
                palette.update(cx, |palette, cx| {
                    palette.query.update(cx, |input, cx| {
                        input.set_value("", window, cx);
                        input.replace_and_mark_text_in_range(None, "co", Some(2..2), window, cx);
                    });
                    assert!(palette.composing(window, cx));
                });
            });
            cx.run_until_parked();
            cx.update(|window, cx| window.draw(cx).clear(cx));
            cx.simulate_keystrokes(key);
            cx.update(|window, cx| {
                palette.update(cx, |palette, cx| {
                    assert!(palette.is_open(), "{key} during IME cannot close the menu");
                    assert_eq!(
                        palette.selected.map(|ix| palette.entries[ix].id.as_str()),
                        Some("terminal"),
                        "IME keys cannot navigate palette results"
                    );
                    palette
                        .query
                        .update(cx, |input, cx| input.unmark_text(window, cx));
                });
            });
        }
        cx.simulate_keystrokes("escape");
        cx.update(|window, cx| {
            assert!(!palette.read(cx).is_open());
            assert!(original_focus.is_focused(window));
        });
    }
}
