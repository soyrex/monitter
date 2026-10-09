//! Retained native keyboard selector. Entries are supplied by the owner root;
//! selection emits intent and never invokes a backend or creates a task/session.
use crate::design::ui_scaled_px as px;
use gpui_kit::{
    component::{
        ActiveTheme,
        button::*,
        input::{Input, InputEvent, InputState},
    },
    prelude::*,
    *,
};
use std::collections::{HashMap, HashSet};

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
    pub disabled: bool,
    pub checked: Option<bool>,
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
fn matches(entry: &PaletteEntry, query: &str) -> bool {
    let text = format!(
        "{} {} {} {}",
        entry.label,
        entry.subtitle,
        entry.kind.label(),
        entry.keywords
    )
    .to_lowercase();
    query
        .to_lowercase()
        .split_whitespace()
        .all(|token| text.contains(token))
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
    visible: Vec<usize>,
    selected: Option<usize>,
    mode: PaletteMode,
    queries: HashMap<PaletteMode, String>,
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
                    let value = palette.query.read(cx).value().to_string();
                    palette.queries.insert(palette.mode, value);
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
            visible: vec![],
            selected: None,
            mode: PaletteMode::Switch,
            queries: HashMap::new(),
            open: false,
            previous_focus: None,
            scroll: UniformListScrollHandle::default(),
            _input_events: events,
        }
    }
    /// Host supplies the real current set for Cmd+K (Switch) or Cmd+P (Commands).
    /// Reopening retains each mode's query. No built-in/fabricated entries exist.
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
        self.mode = mode;
        self.entries = entries;
        self.selected = None;
        let value = self.queries.get(&mode).cloned().unwrap_or_default();
        self.query
            .update(cx, |input, cx| input.set_value(value, window, cx));
        self.refilter(cx);
        self.query.update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
        Ok(())
    }
    /// Refresh the supplied entries without replacing the query or selected ID.
    /// The root must revalidate the selected ID before executing any authority.
    pub fn set_entries(
        &mut self,
        entries: Vec<PaletteEntry>,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        validate_entries(&entries)?;
        let selected = self
            .selected
            .and_then(|index| self.entries.get(index))
            .map(|entry| entry.id.clone());
        self.entries = entries;
        self.selected =
            selected.and_then(|id| self.entries.iter().position(|entry| entry.id == id));
        self.refilter(cx);
        Ok(())
    }
    pub fn is_open(&self) -> bool {
        self.open
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
        self.visible = self
            .entries
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| matches(entry, &query).then_some(index))
            .collect();
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
        if !["up", "down", "escape"].contains(&key) || self.composing(window, cx) {
            return;
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
        let entries = self.entries.clone();
        let visible = self.visible.clone();
        let selected = self.selected;
        let background = cx.theme().background;
        let foreground = cx.theme().foreground;
        let muted = cx.theme().muted_foreground;
        let highlighted = cx.theme().muted;
        let items = uniform_list("palette-results", visible.len(), move |range, _, _| {
            range
                .map(|position| {
                    let index = visible[position];
                    let entry = &entries[index];
                    let weak = weak.clone();
                    let id = entry.id.clone();
                    div()
                        .id(SharedString::from(format!("palette-entry-{id}")))
                        .h(px(62.))
                        .px_3()
                        .py_2()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .when(selected == Some(index), |row| row.bg(highlighted))
                        .text_color(if entry.disabled { muted } else { foreground })
                        .child(format!(
                            "{}{}",
                            match entry.checked {
                                Some(true) => "✓ ",
                                Some(false) => "○ ",
                                None => "",
                            },
                            entry.label
                        ))
                        .child(div().text_sm().text_color(muted).child(format!(
                            "{} · {}{}",
                            entry.kind.label(),
                            entry.subtitle,
                            if entry.disabled {
                                " · unavailable"
                            } else {
                                ""
                            }
                        )))
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
        .h(px(372.))
        .w_full()
        .track_scroll(&self.scroll);
        div()
            .id("native-command-palette")
            .w(px(640.))
            .max_w_full()
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .rounded_lg()
            .bg(background)
            .text_color(foreground)
            .capture_key_down(cx.listener(Self::key))
            .child(
                div()
                    .flex()
                    .justify_between()
                    .child(if self.mode == PaletteMode::Switch {
                        "Switch workspace item"
                    } else {
                        "Workspace commands"
                    })
                    .child(Button::new("palette-dismiss").label("Escape").on_click(
                        cx.listener(|palette, _, window, cx| palette.dismiss(window, cx)),
                    )),
            )
            .child(Input::new(&self.query))
            .when(self.visible.is_empty(), |panel| {
                panel.child("No matching locally available items.")
            })
            .child(items)
            .child(
                div()
                    .text_sm()
                    .text_color(muted)
                    .child("↑ ↓ select · Enter choose · Escape dismiss"),
            )
            .into_any_element()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn entry(id: &str, label: &str, disabled: bool) -> PaletteEntry {
        PaletteEntry {
            id: id.into(),
            label: label.into(),
            subtitle: "Local agent".into(),
            kind: EntryKind::Task,
            keywords: "codex work".into(),
            disabled,
            checked: None,
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
}
