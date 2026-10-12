//! Task-scoped native slash-command suggestions for the composer.
//!
//! The parent owns the text input and command execution. It calls `set_input`
//! as the draft changes, forwards unmodified Up/Down/Enter/Escape key events
//! to `handle_key`, renders `render_menu` above the composer, and subscribes to
//! `SlashPaletteEvent::Selected`. Selection emits the untouched input so the
//! parent can validate arguments and execute through its existing authority.
//! Provider suggestions come only from `get_task_slash_commands`.

use crate::design::ui_scaled_px as px;
use futures::channel::oneshot;
use gpui_kit::{prelude::*, *};
use monitter_lib::experimental_desktop::DesktopService;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{Arc, LazyLock, Mutex},
};

const MAX_CANDIDATES: usize = 100;
const MAX_NAME_BYTES: usize = 256;
const MAX_DESCRIPTION_BYTES: usize = 2048;
const WORKER_STACK_SIZE: usize = 8 * 1024 * 1024;

pub enum PaletteAction {
    Previous,
    Next,
    Confirm,
    Dismiss,
}
const MAX_COMMAND_ICON_INKS: usize = 16;
const COMMAND_ICON_PATH: &str =
    "M15 6v12a3 3 0 1 0 3-3H6a3 3 0 1 0 3 3V6a3 3 0 1 0-3 3h12a3 3 0 1 0-3-3";

static COMMAND_ICONS: LazyLock<Mutex<HashMap<u32, Arc<Image>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn command_icon(ink: u32) -> Arc<Image> {
    if let Ok(mut icons) = COMMAND_ICONS.lock() {
        if let Some(icon) = icons.get(&ink) {
            return icon.clone();
        }
        let svg = format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"14\" height=\"14\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"#{ink:06x}\" stroke-width=\"2\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><path d=\"{COMMAND_ICON_PATH}\"/></svg>"
        );
        let image = Arc::new(Image::from_bytes(ImageFormat::Svg, svg.into_bytes()));
        if icons.len() >= MAX_COMMAND_ICON_INKS {
            if let Some(oldest) = icons.keys().next().copied() {
                icons.remove(&oldest);
            }
        }
        icons.insert(ink, image.clone());
        image
    } else {
        let svg = format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"14\" height=\"14\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"#{ink:06x}\" stroke-width=\"2\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><path d=\"{COMMAND_ICON_PATH}\"/></svg>"
        );
        Arc::new(Image::from_bytes(ImageFormat::Svg, svg.into_bytes()))
    }
}

fn source_icon(source: &str, provider: &str, ink: u32) -> AnyElement {
    let container = div()
        .w(px(18.))
        .h(px(18.))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center();
    if source == "monitter" {
        container
            .child(img(command_icon(ink)).w(px(14.)).h(px(14.)))
            .into_any_element()
    } else if let Some(image) = crate::branding::provider_for_ink(provider, "", ink) {
        container
            .child(img(image).w(px(14.)).h(px(14.)))
            .into_any_element()
    } else {
        let glyph = provider
            .chars()
            .find(|character| character.is_ascii_alphanumeric())
            .map(|character| character.to_ascii_uppercase().to_string())
            .unwrap_or_else(|| "?".to_owned());
        container
            .text_size(px(12.))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(rgb(ink))
            .child(glyph)
            .into_any_element()
    }
}

/// A display-only slash command. The palette never dispatches this command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SlashPaletteItem {
    /// Command name without the leading slash. Provider names may be nested,
    /// such as `memory show`.
    pub name: String,
    pub description: String,
    pub input_hint: Option<String>,
    /// `monitter`, `codex`, or `acp`.
    pub source: String,
    pub provider: String,
}

impl SlashPaletteItem {
    pub fn label(&self) -> String {
        format!("/{}", self.name)
    }

    fn id(&self) -> String {
        format!("{}:{}:{}", self.source, self.provider, self.name)
    }
}

/// Selection is intent only; the owner decides whether to navigate, execute a
/// Monitter action, or send a provider command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SlashPaletteEvent {
    Selected {
        item: SlashPaletteItem,
        input: String,
    },
    Submit {
        item: SlashPaletteItem,
        input: String,
    },
}

impl EventEmitter<SlashPaletteEvent> for SlashPalette {}

#[derive(Clone, Debug, PartialEq, Eq)]
struct LookupKey {
    task_id: String,
    revision: String,
    generation: u64,
}

pub struct SlashPalette {
    service: DesktopService,
    snapshot: Value,
    composed_palette: Option<crate::design::Palette>,
    revision: String,
    task_id: String,
    generation: u64,
    input: String,
    open: bool,
    items: Vec<SlashPaletteItem>,
    visible: Vec<usize>,
    selected: Option<usize>,
    loading: bool,
    error: Option<String>,
    lookup_key: Option<LookupKey>,
}

impl Render for SlashPalette {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.render_menu(window, cx)
    }
}

impl SlashPalette {
    pub fn new(service: DesktopService) -> Self {
        Self {
            service,
            snapshot: Value::Null,
            composed_palette: None,
            revision: String::new(),
            task_id: String::new(),
            generation: 0,
            input: String::new(),
            open: false,
            items: Vec::new(),
            visible: Vec::new(),
            selected: None,
            loading: false,
            error: None,
            lookup_key: None,
        }
    }

    /// Refresh task scope. A changed task or canonical revision invalidates
    /// outstanding requests before any response can be applied.
    pub fn set_context(
        &mut self,
        snapshot: &Value,
        revision: impl ToString,
        task_id: impl Into<String>,
        cx: &mut Context<Self>,
    ) {
        let revision = revision.to_string();
        let task_id = task_id.into();
        if self.revision == revision && self.task_id == task_id {
            self.snapshot = serde_json::json!({
                "settings": snapshot["settings"],
                "tasks": snapshot["tasks"],
            });
            self.refilter(cx);
            return;
        }

        self.snapshot = serde_json::json!({
            "settings": snapshot["settings"],
            "tasks": snapshot["tasks"],
        });
        self.revision = revision;
        self.task_id = task_id;
        self.generation = self.generation.wrapping_add(1);
        self.items.clear();
        self.visible.clear();
        self.selected = None;
        self.loading = false;
        self.error = None;
        self.lookup_key = None;
        if self.open {
            self.refresh_local_items();
            self.begin_lookup(cx);
            self.refilter(cx);
        } else {
            cx.notify();
        }
    }

    /// A publication outside this task advances the canonical revision without
    /// changing its command scope. Keep an existing lookup valid; substantive
    /// task/context updates still invalidate it through `generation`.
    pub fn advance_revision(&mut self, revision: &str) {
        self.revision = revision.to_owned();
        if let Some(key) = self.lookup_key.as_mut() {
            key.revision = self.revision.clone();
        }
    }

    /// Track the owner's unsent text. Opening and provider lookup begin only
    /// when the input contains a slash command token at its start.
    /// Explicit submission checks the native editor source. Preserve selected
    /// suggestions and dismissed state when the cached draft already matches.
    pub fn sync_input(&mut self, input: impl Into<String>, cx: &mut Context<Self>) {
        let input = input.into();
        if self.input != input { self.set_input(input, cx); }
    }
    pub fn set_input(&mut self, input: impl Into<String>, cx: &mut Context<Self>) {
        let input = input.into();
        if self.input != input {
            self.error = None;
        }
        self.input = input;
        self.open = is_slash_input(&self.input);
        if self.open {
            self.refresh_local_items();
            self.begin_lookup(cx);
            self.refilter(cx);
        } else {
            self.selected = None;
            self.visible.clear();
            cx.notify();
        }
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn is_loading(&self) -> bool {
        self.loading
    }

    /// Use the owning chat pane's fully composed design tokens. Passing `None`
    /// falls back to the palette built from the current snapshot and window
    /// appearance.
    pub fn set_composed_palette(
        &mut self,
        palette: Option<crate::design::Palette>,
        cx: &mut Context<Self>,
    ) {
        if self.composed_palette != palette {
            self.composed_palette = palette;
            cx.notify();
        }
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Dismiss suggestions while leaving draft contents to the owner.
    pub fn dismiss(&mut self, cx: &mut Context<Self>) {
        self.open = false;
        self.selected = None;
        self.visible.clear();
        cx.notify();
    }

    /// Resolve a typed single-slash command when the user explicitly sends.
    /// Unknown commands and pending provider lookups are consumed so the
    /// slash text cannot silently fall through as a model prompt. `//` remains
    /// literal user text and returns false for normal submission.
    pub fn submit_typed_input(&mut self, cx: &mut Context<Self>) -> bool {
        if !is_slash_input(&self.input) {
            return false;
        }
        if let Some(item) = resolve_input(&self.items, &self.input) {
            let input = self.input.clone();
            self.open = false;
            self.visible.clear();
            self.selected = None;
            self.error = None;
            cx.emit(SlashPaletteEvent::Submit { item, input });
            cx.notify();
            return true;
        }

        if self.loading {
            self.open = true;
            self.error = Some("Provider commands are still loading; wait and try again.".into());
            self.refilter(cx);
            return true;
        }
        if self.error.is_some() && self.lookup_key.is_some() && !self.task_id.is_empty() {
            self.open = true;
            self.error = Some(
                "Provider command lookup failed; this command was not sent. Retrying lookup…"
                    .into(),
            );
            self.lookup_key = None;
            self.begin_lookup(cx);
            self.refilter(cx);
            return true;
        }

        {
            self.open = true;
            self.error = Some(
                "This command is not available. Start with // to send a literal slash message."
                    .into(),
            );
            self.refilter(cx);
        }
        true
    }

    /// Handle navigation/selection keys while the palette is open. The owner
    /// should call this from the focused composer after preserving IME input.
    /// Returns true when the key was consumed.
    pub fn handle_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) -> bool {
        if !self.open
            || event.keystroke.modifiers.control
            || event.keystroke.modifiers.platform
            || event.keystroke.modifiers.alt
            || event.keystroke.modifiers.shift
        {
            return false;
        }
        match event.keystroke.key.as_str() {
            "up" | "down" => {
                self.move_selection(event.keystroke.key == "down");
                cx.notify();
                true
            }
            "escape" => {
                self.dismiss(cx);
                true
            }
            "enter" => {
                self.confirm(cx);
                true
            }
            _ => false,
        }
    }

    /// Native text inputs dispatch bound actions before raw key events.
    pub fn handle_action(&mut self, action: PaletteAction, cx: &mut Context<Self>) -> bool {
        if !self.open {
            return false;
        }
        match action {
            PaletteAction::Previous => {
                self.move_selection(false);
                cx.notify();
            }
            PaletteAction::Next => {
                self.move_selection(true);
                cx.notify();
            }
            PaletteAction::Confirm => self.confirm(cx),
            PaletteAction::Dismiss => self.dismiss(cx),
        }
        true
    }

    /// Render the suggestion list. The owner places this above the composer
    /// and retains focus in its native text input.
    pub fn render_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        if !self.open {
            return div().into_any_element();
        }

        let palette = self.composed_palette.unwrap_or_else(|| {
            crate::design::palette(&self.snapshot["settings"], window.appearance())
        });
        let font_ratio = self.snapshot["settings"]["interfaceFontSize"]
            .as_f64()
            .unwrap_or(crate::design::INTERFACE_FONT_SIZE as f64)
            .clamp(8., 32.) as f32
            / crate::design::INTERFACE_FONT_SIZE;
        let panel = rgb(palette.panel).opacity(0.94);
        let active_background = rgb(mix_rgb(palette.panel, palette.accent, 0.13));
        let entries = self.items.clone();
        let visible = self.visible.clone();
        let selected = self.selected;
        let weak = cx.weak_entity();
        let mut menu = div()
            .id("slash-command-menu")
            .role(Role::Menu)
            .aria_label("Available slash commands")
            .w_full()
            .max_h(px(270.))
            .overflow_hidden()
            .flex()
            .flex_col()
            .rounded_t(px(10.))
            .rounded_bl(px(0.))
            .rounded_br(px(0.))
            .mb(-px(10.))
            .border_1()
            .border_color(rgb(palette.line))
            .text_color(rgb(palette.ink))
            .font_family(crate::design::FONT_SANS)
            .bg(panel);

        let mut header = div()
            .w_full()
            .flex()
            .items_center()
            .gap(px(7.))
            .px(px(11.))
            .pt(px(9.))
            .pb(px(7.))
            .text_color(rgb(palette.muted))
            .font_family(crate::design::FONT_MONO)
            .text_size(px(10. * font_ratio))
            .child("COMMANDS");
        if self.loading {
            header = header.child(
                div()
                    .ml_auto()
                    .text_size(px(10. * font_ratio))
                    .child("Loading…"),
            );
        }
        menu = menu.child(header);

        let mut options = div()
            .id("slash-command-options")
            .w_full()
            .max_h(px(225.))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .pb(px(10.));

        for index in visible.into_iter().take(MAX_CANDIDATES) {
            let Some(item) = entries.get(index).cloned() else {
                continue;
            };
            let row_weak = weak.clone();
            let source_label = source_label(&item.source);
            let icon = source_icon(&item.source, &item.provider, palette.ink);
            let description = item
                .input_hint
                .as_ref()
                .map(|hint| format!("{} · {}", item.description, hint))
                .unwrap_or_else(|| item.description.clone());
            options = options.child(
                div()
                    .id(SharedString::from(format!("slash-command-{}", item.id())))
                    .role(Role::MenuItem)
                    .aria_label(format!(
                        "{}, {}. {}",
                        item.label(),
                        source_label,
                        description
                    ))
                    .aria_selected(selected == Some(index))
                    .w_full()
                    .min_h(px(42.))
                    .px(px(11.))
                    .py(px(7.))
                    .flex()
                    .items_center()
                    .gap(px(9.))
                    .cursor_pointer()
                    .when(selected == Some(index), |row| row.bg(active_background))
                    .on_click(move |_, _, cx| {
                        let _ = row_weak.update(cx, |palette, cx| {
                            palette.selected = Some(index);
                            palette.confirm(cx);
                        });
                    })
                    .child(icon)
                    .child(
                        div()
                            .min_w(px(76.))
                            .max_w(px(170.))
                            .flex_shrink_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .font_family(crate::design::FONT_MONO)
                            .text_size(px(12. * font_ratio))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(item.label()),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_size(px(12. * font_ratio))
                            .text_color(rgb(palette.muted))
                            .child(description),
                    )
                    .child(
                        div()
                            .ml_auto()
                            .flex_shrink_0()
                            .whitespace_nowrap()
                            .text_size(px(9. * font_ratio))
                            .text_color(rgb(palette.muted))
                            .font_family(crate::design::FONT_MONO)
                            .child(source_label),
                    ),
            );
        }
        menu = menu.child(options);

        if self.visible.is_empty() {
            if !self.loading {
                menu = menu.child(
                    div()
                        .id("slash-command-empty-status")
                        .role(Role::Status)
                        .aria_label(
                            self.error
                                .as_deref()
                                .unwrap_or("No matching commands")
                                .to_owned(),
                        )
                        .px(px(11.))
                        .pt(px(9.))
                        .pb(px(18.))
                        .text_size(px(12. * font_ratio))
                        .text_color(rgb(palette.muted))
                        .child(
                            self.error
                                .as_deref()
                                .unwrap_or("No matching commands")
                                .to_owned(),
                        ),
                );
            }
        }
        if !self.visible.is_empty() {
            if let Some(error) = &self.error {
                menu = menu.child(
                    div()
                        .id("slash-command-provider-error")
                        .role(Role::Status)
                        .aria_label(format!("Provider commands unavailable: {error}"))
                        .px(px(11.))
                        .pt(px(5.))
                        .pb(px(8.))
                        .text_size(px(11. * font_ratio))
                        .text_color(rgb(0xbd655b))
                        .child(format!("Provider commands unavailable: {error}")),
                );
            }
        }
        menu.into_any_element()
    }

    fn refresh_local_items(&mut self) {
        let task = self.task_id_is_current();
        let task_id = self.task_id.clone();
        let task_running = task
            && self.snapshot["tasks"].as_array().is_some_and(|tasks| {
                tasks.iter().any(|value| {
                    value["id"].as_str() == Some(task_id.as_str())
                        && value["status"].as_str() == Some("running")
                })
            });
        let provider = self.snapshot["tasks"]
            .as_array()
            .and_then(|tasks| {
                tasks
                    .iter()
                    .find(|value| value["id"].as_str() == Some(task_id.as_str()))
            })
            .and_then(|value| value["provider"].as_str())
            .unwrap_or("codex")
            .to_owned();

        let mut local = vec![
            local_item(
                "new",
                "Clear this chat context or start a new chat",
                &provider,
            ),
            local_item("settings", "Open Monitter preferences", &provider),
            local_item("terminal", "Open or run a command in a terminal", &provider),
        ];
        if task {
            local.push(local_item("clear", "Clear this chat context", &provider));
            local.push(local_item(
                "project",
                "Choose the project for this chat",
                &provider,
            ));
            local.push(local_item(
                "rename",
                "Generate a title from recent content",
                &provider,
            ));
        }
        if task_running {
            local.push(local_item("stop", "Stop this running task", &provider));
        }

        // Preserve backend items while replacing only the fixed Monitter set.
        let mut providers = std::mem::take(&mut self.items);
        providers.retain(|item| item.source != "monitter");
        local.extend(providers);
        self.items = local.into_iter().take(MAX_CANDIDATES).collect();
    }

    fn task_id_is_current(&self) -> bool {
        !self.task_id.is_empty()
            && self.snapshot["tasks"].as_array().is_some_and(|tasks| {
                tasks
                    .iter()
                    .any(|value| value["id"].as_str() == Some(self.task_id.as_str()))
            })
    }

    fn begin_lookup(&mut self, cx: &mut Context<Self>) {
        if self.task_id.is_empty() {
            return;
        }
        let key = LookupKey {
            task_id: self.task_id.clone(),
            revision: self.revision.clone(),
            generation: self.generation,
        };
        if self.lookup_key.as_ref() == Some(&key) {
            return;
        }
        self.lookup_key = Some(key.clone());
        self.loading = true;
        self.error = None;
        let service = self.service.clone();
        let task_id = key.task_id.clone();
        let (sender, receiver) = oneshot::channel();
        let started = std::thread::Builder::new()
            .name("monitter-slash-commands".into())
            .stack_size(WORKER_STACK_SIZE)
            .spawn(move || {
                let _ = sender
                    .send(service.invoke("get_task_slash_commands", json!({"taskId": task_id})));
            });
        if let Err(error) = started {
            self.loading = false;
            self.error = Some(format!("Could not start command lookup: {error}"));
            cx.notify();
            return;
        }

        cx.spawn(async move |entity, cx| {
            let result = receiver
                .await
                .map_err(|_| "Command lookup worker ended without a result.".to_owned())
                .and_then(|result| result);
            let _ = entity.update(cx, |palette, cx| {
                if palette.generation != key.generation || palette.task_id != key.task_id {
                    return;
                }
                palette.loading = false;
                match result {
                    Ok(value) => match parse_provider_items(&value) {
                        Ok(items) => {
                            palette.items.retain(|item| item.source == "monitter");
                            palette.items.extend(items);
                            palette.items.truncate(MAX_CANDIDATES);
                            palette.error = None;
                        }
                        Err(error) => palette.error = Some(error),
                    },
                    Err(error) => palette.error = Some(error),
                }
                palette.refilter(cx);
            });
        })
        .detach();
        cx.notify();
    }

    fn refilter(&mut self, cx: &mut Context<Self>) {
        let Some(query) = slash_query(&self.input) else {
            self.visible.clear();
            self.selected = None;
            cx.notify();
            return;
        };
        self.visible = self
            .items
            .iter()
            .enumerate()
            .filter_map(|(index, item)| matches_query(item, &query).then_some(index))
            .take(MAX_CANDIDATES)
            .collect();
        if !self
            .selected
            .is_some_and(|index| self.visible.contains(&index))
        {
            self.selected = self.visible.first().copied();
        }
        cx.notify();
    }

    fn move_selection(&mut self, forward: bool) {
        if self.visible.is_empty() {
            self.selected = None;
            return;
        }
        let position = self
            .selected
            .and_then(|selected| self.visible.iter().position(|index| *index == selected));
        self.selected = Some(match position {
            None if forward => self.visible[0],
            None => *self.visible.last().unwrap(),
            Some(position) => {
                self.visible[if forward {
                    (position + 1) % self.visible.len()
                } else {
                    (position + self.visible.len() - 1) % self.visible.len()
                }]
            }
        });
    }

    fn confirm(&mut self, cx: &mut Context<Self>) {
        let Some(item) = self
            .selected
            .and_then(|index| self.items.get(index))
            .cloned()
        else {
            return;
        };
        let input = self.input.clone();
        self.open = false;
        self.visible.clear();
        self.selected = None;
        cx.emit(SlashPaletteEvent::Selected { item, input });
        cx.notify();
    }
}

fn local_item(name: &str, description: &str, provider: &str) -> SlashPaletteItem {
    SlashPaletteItem {
        name: name.to_owned(),
        description: description.to_owned(),
        input_hint: None,
        source: "monitter".into(),
        provider: provider.to_owned(),
    }
}

fn source_label(source: &str) -> &'static str {
    match source {
        "monitter" => "MONITTER",
        "codex" => "CODEX",
        _ => "ACP PROVIDER",
    }
}

fn mix_rgb(base: u32, color: u32, amount: f32) -> u32 {
    let channel = |shift: u32| {
        let base = ((base >> shift) & 0xff) as f32;
        let color = ((color >> shift) & 0xff) as f32;
        (base * (1. - amount) + color * amount).round() as u32
    };
    (channel(16) << 16) | (channel(8) << 8) | channel(0)
}

fn is_slash_input(input: &str) -> bool {
    let trimmed = input.trim_start();
    trimmed.starts_with('/') && !trimmed.starts_with("//")
}

fn slash_query(input: &str) -> Option<String> {
    let trimmed = input.trim_start();
    let command = trimmed.strip_prefix('/')?;
    if command.starts_with('/') {
        return None;
    }
    Some(
        command
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .to_lowercase(),
    )
}

fn matches_query(item: &SlashPaletteItem, query: &str) -> bool {
    item.name
        .split_whitespace()
        .next()
        .is_some_and(|name| name.to_lowercase().starts_with(query))
}

/// Return the argument text after a case-insensitive exact command prefix.
/// A command name must end at whitespace or input end, so `/modeling` does not
/// match `/model`; the returned suffix preserves the user's exact spacing.
pub fn typed_suffix<'a>(input: &'a str, item: &SlashPaletteItem) -> Option<&'a str> {
    let trimmed_start = input.trim_start();
    let command = item.label();
    let Some(prefix) = trimmed_start.get(..command.len()) else {
        return None;
    };
    if !prefix.eq_ignore_ascii_case(&command) {
        return None;
    }
    let Some(remainder) = trimmed_start.get(command.len()..) else {
        return None;
    };
    if remainder.is_empty() {
        Some("")
    } else if remainder.chars().next().is_some_and(char::is_whitespace) {
        Some(remainder.trim_start())
    } else {
        None
    }
}

fn resolve_input(items: &[SlashPaletteItem], input: &str) -> Option<SlashPaletteItem> {
    let mut best: Option<&SlashPaletteItem> = None;
    for item in items {
        if typed_suffix(input, item).is_none() {
            continue;
        }
        if best.is_none_or(|current| item.name.len() > current.name.len()) {
            best = Some(item);
        }
    }
    best.cloned()
}

fn parse_provider_items(value: &Value) -> Result<Vec<SlashPaletteItem>, String> {
    let commands = value
        .as_array()
        .ok_or_else(|| "The command service returned an invalid catalog.".to_owned())?;
    let mut items = Vec::new();
    for command in commands.iter().take(MAX_CANDIDATES) {
        let name = command["name"].as_str().unwrap_or_default().trim();
        let description = command["description"].as_str().unwrap_or_default().trim();
        let source = command["source"].as_str().unwrap_or_default();
        let provider = command["provider"].as_str().unwrap_or_default();
        let input_hint = command["inputHint"]
            .as_str()
            .filter(|hint| !hint.trim().is_empty())
            .map(|hint| hint.chars().take(512).collect::<String>());
        if name.is_empty()
            || name.len() > MAX_NAME_BYTES
            || description.len() > MAX_DESCRIPTION_BYTES
            || source == "monitter"
            || !matches!(source, "codex" | "acp")
            || provider.is_empty()
            || provider.len() > 64
            || name.starts_with('/')
            || name.split_whitespace().count() > 8
            || !name.split_whitespace().all(|segment| {
                !segment.is_empty()
                    && segment.chars().all(|character| {
                        character.is_ascii_alphanumeric() || "-_:.".contains(character)
                    })
            })
        {
            continue;
        }
        items.push(SlashPaletteItem {
            name: name.to_owned(),
            description: description.to_owned(),
            input_hint,
            source: source.to_owned(),
            provider: provider.to_owned(),
        });
    }
    Ok(items)
}

#[cfg(test)]
mod tests {
    use super::{
        MAX_CANDIDATES, SlashPaletteItem, is_slash_input, matches_query, parse_provider_items,
        resolve_input, slash_query, typed_suffix,
    };
    use serde_json::{Value, json};

    fn item(name: &str) -> SlashPaletteItem {
        SlashPaletteItem {
            name: name.into(),
            description: String::new(),
            input_hint: None,
            source: "codex".into(),
            provider: "codex".into(),
        }
    }

    #[::core::prelude::v1::test]
    fn slash_filter_matches_first_command_token_case_insensitively() {
        assert!(is_slash_input("  /Model reasoning"));
        assert_eq!(slash_query("  /Model reasoning"), Some("model".into()));
        assert_eq!(slash_query("//literal slash"), None);
        assert_eq!(slash_query("ordinary text"), None);
        assert!(matches_query(&item("model"), "mod"));
        assert!(matches_query(&item("memory show"), "mem"));
        assert!(!matches_query(&item("model"), "memory"));
    }

    #[::core::prelude::v1::test]
    fn typed_suffix_requires_an_exact_command_boundary_and_preserves_arguments() {
        let model = item("model");
        assert_eq!(
            typed_suffix("/MODEL  gpt-5 high", &model),
            Some("gpt-5 high")
        );
        assert_eq!(typed_suffix("  /model", &model), Some(""));
        assert_eq!(typed_suffix("/modeling x", &model), None);

        let nested = item("memory show");
        assert_eq!(typed_suffix("/memory show  key", &nested), Some("key"));
        assert_eq!(typed_suffix("/memory shown key", &nested), None);
    }

    #[::core::prelude::v1::test]
    fn explicit_send_resolves_the_longest_exact_command_and_rejects_prefix_collisions() {
        let commands = [item("memory"), item("memory show"), item("model")];
        assert_eq!(
            resolve_input(&commands, "/MEMORY show key").map(|item| item.name),
            Some("memory show".into())
        );
        assert_eq!(
            resolve_input(&commands, "/modeling x").map(|item| item.name),
            None
        );
        assert_eq!(resolve_input(&commands, "//model"), None);
    }

    #[::core::prelude::v1::test]
    fn provider_catalog_rejects_invalid_sources_and_bounds_candidates() {
        let mut commands = (0..(MAX_CANDIDATES + 10))
            .map(|index| {
                json!({
                    "name": format!("command{index}"),
                    "description": "provider command",
                    "source": "codex",
                    "provider": "codex"
                })
            })
            .collect::<Vec<_>>();
        commands.push(json!({
            "name":"forged", "description":"bad source", "source":"monitter", "provider":"codex"
        }));
        let items = parse_provider_items(&Value::Array(commands)).unwrap();
        assert_eq!(items.len(), MAX_CANDIDATES);
        assert!(items.iter().all(|item| item.source == "codex"));
    }
}
