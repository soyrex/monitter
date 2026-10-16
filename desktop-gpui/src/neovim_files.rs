//! Native, virtualized directory navigation for a live Neovim session.
use crate::design::{self, ui_scaled_px as px};
use gpui_kit::{
    component::button::{Button, ButtonVariants},
    prelude::*,
    *,
};
use monitter_lib::experimental_desktop::DesktopService;
use serde_json::{Value, json};
use std::sync::Arc;

#[derive(Clone, Debug)]
pub enum NeovimFilesEvent {
    OpenFile(String),
    Collapse,
}
#[derive(Clone)]
struct Entry {
    name: String,
    path: String,
    is_directory: bool,
    is_symlink: bool,
}
struct Listing {
    path: String,
    parent: Option<String>,
    entries: Vec<Entry>,
    truncated: bool,
}

fn parse_listing(value: Value) -> Result<Listing, String> {
    let string = |value: &Value, key: &str| {
        value[key]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| format!("Invalid directory response: missing {key}"))
    };
    let path = string(&value, "path")?;
    let entries = value["entries"]
        .as_array()
        .ok_or("Invalid directory response: missing entries")?
        .iter()
        .map(|entry| {
            Ok(Entry {
                name: string(entry, "name")?,
                path: string(entry, "path")?,
                is_directory: entry["isDirectory"]
                    .as_bool()
                    .ok_or("Invalid directory response: missing isDirectory")?,
                is_symlink: entry["isSymlink"].as_bool().unwrap_or(false),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(Listing {
        path,
        parent: value["parent"].as_str().map(str::to_owned),
        entries,
        truncated: value["truncated"].as_bool().unwrap_or(false),
    })
}

pub struct NeovimFilesPane {
    service: DesktopService,
    session_id: String,
    path: String,
    parent: Option<String>,
    entries: Vec<Entry>,
    visible: Arc<Vec<Entry>>,
    selected: Option<usize>,
    show_hidden: bool,
    loading: bool,
    error: Option<String>,
    truncated: bool,
    generation: u64,
    focus: FocusHandle,
    scroll: UniformListScrollHandle,
    settings: Value,
    palette: Option<design::Palette>,
}
impl EventEmitter<NeovimFilesEvent> for NeovimFilesPane {}
impl NeovimFilesPane {
    pub fn new(
        service: DesktopService,
        session_id: String,
        initial_path: String,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut pane = Self {
            service,
            session_id,
            path: initial_path,
            parent: None,
            entries: Vec::new(),
            visible: Arc::new(Vec::new()),
            selected: None,
            show_hidden: false,
            loading: false,
            error: None,
            truncated: false,
            generation: 0,
            focus: cx.focus_handle(),
            scroll: UniformListScrollHandle::default(),
            settings: Value::Null,
            palette: None,
        };
        pane.refresh(cx);
        pane
    }
    pub fn set_theme(&mut self, settings: Value, palette: design::Palette, cx: &mut Context<Self>) {
        if self.settings != settings || self.palette != Some(palette) {
            self.settings = settings;
            self.palette = Some(palette);
            cx.notify();
        }
    }
    pub fn set_settings(&mut self, settings: Value, cx: &mut Context<Self>) {
        if self.settings != settings {
            self.settings = settings;
            cx.notify();
        }
    }
    fn filter(&mut self) {
        self.visible = Arc::new(
            self.entries
                .iter()
                .filter(|entry| self.show_hidden || !entry.name.starts_with('.'))
                .cloned()
                .collect(),
        );
        self.selected = (!self.visible.is_empty()).then_some(0);
        self.scroll.scroll_to_item(0, ScrollStrategy::Top);
    }
    fn refresh(&mut self, cx: &mut Context<Self>) {
        self.generation += 1;
        let generation = self.generation;
        self.loading = true;
        self.error = None;
        let service = self.service.clone();
        let args = json!({"id": self.session_id, "path": self.path});
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    service
                        .invoke("neovim_list_directory", args)
                        .and_then(parse_listing)
                })
                .await;
            let _ = this.update(cx, |pane, cx| {
                if generation != pane.generation {
                    return;
                }
                pane.loading = false;
                match result {
                    Ok(mut listing) => {
                        listing.entries.sort_by(|a, b| {
                            b.is_directory
                                .cmp(&a.is_directory)
                                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
                                .then_with(|| a.name.cmp(&b.name))
                        });
                        pane.path = listing.path;
                        pane.parent = listing.parent;
                        pane.entries = listing.entries;
                        pane.truncated = listing.truncated;
                        pane.filter();
                    }
                    Err(error) => {
                        pane.parent = std::path::Path::new(&pane.path)
                            .parent()
                            .map(|parent| parent.to_string_lossy().into_owned());
                        pane.error = Some(error);
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn open(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.loading || self.error.is_some() {
            return;
        }
        let Some(entry) = self.visible.get(index).cloned() else {
            return;
        };
        self.selected = Some(index);
        if entry.is_directory {
            self.path = entry.path;
            self.refresh(cx);
        } else {
            cx.emit(NeovimFilesEvent::OpenFile(entry.path));
            cx.notify();
        }
    }
    fn up(&mut self, cx: &mut Context<Self>) {
        if self.loading {
            return;
        }
        if let Some(parent) = self.parent.clone() {
            self.path = parent;
            self.refresh(cx);
        }
    }
    fn key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.focus.is_focused(window)
            || event.keystroke.modifiers.platform
            || event.keystroke.modifiers.control
            || event.keystroke.modifiers.alt
        {
            return;
        }
        match event.keystroke.key.as_str() {
            "up" | "down" if !self.loading && !self.visible.is_empty() => {
                let current = self.selected.unwrap_or(0);
                self.selected = Some(if event.keystroke.key == "up" {
                    current.saturating_sub(1)
                } else {
                    (current + 1).min(self.visible.len() - 1)
                });
                self.scroll
                    .scroll_to_item(self.selected.unwrap(), ScrollStrategy::Nearest);
                cx.notify();
            }
            "enter" => {
                if let Some(index) = self.selected {
                    self.open(index, cx);
                }
            }
            "backspace" => self.up(cx),
            _ => return,
        }
        window.prevent_default();
        cx.stop_propagation();
    }
}
impl Focusable for NeovimFilesPane {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl Render for NeovimFilesPane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = self.palette.unwrap_or_else(|| design::palette(&self.settings, window.appearance()));
        let rows = self.visible.clone();
        let weak = cx.weak_entity();
        let selected = self.selected;
        let list = uniform_list(
            "neovim-directory-entries",
            rows.len(),
            move |range, _, _| {
                range
                    .map(|index| {
                        let entry = &rows[index];
                        let weak = weak.clone();
                        div()
                            .id(index)
                            .h(px(30.))
                            .w_full()
                            .px_3()
                            .flex()
                            .items_center()
                            .gap_2()
                            .cursor_pointer()
                            .when(selected == Some(index), |row| {
                                row.bg(design::surface_color(palette.soft))
                            })
                            .hover(|row| row.bg(design::surface_color(palette.soft)))
                            .child(
                                div()
                                    .text_color(rgb(palette.muted))
                                    .child(if entry.is_directory { "▸" } else { "·" }),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .child(entry.name.clone()),
                            )
                            .when(entry.is_symlink, |row| {
                                row.child(div().text_color(rgb(palette.muted)).child("↗"))
                            })
                            .on_click(move |_, window, cx| {
                                let _ = weak.update(cx, |pane, cx| {
                                    pane.focus.focus(window, cx);
                                    pane.open(index, cx);
                                });
                            })
                            .into_any_element()
                    })
                    .collect()
            },
        )
        .h_full()
        .w_full()
        .track_scroll(&self.scroll);
        let message = if self.loading {
            Some("Loading directory…".to_string())
        } else if let Some(error) = &self.error {
            Some(format!("Unable to list directory: {error}"))
        } else if self.visible.is_empty() {
            Some("No visible files".to_string())
        } else {
            None
        };
        div()
            .id("neovim-files-pane")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key))
            .size_full()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .bg(crate::surface_blend::background(
                crate::surface_blend::Area::Sidebar,
                palette.sidebar,
            ))
            .text_color(rgb(palette.ink))
            .text_size(px(12.))
            .child(
                div()
                    .px_3()
                    .py_2()
                    .border_b_1()
                    .border_color(rgb(palette.line))
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().flex_1().min_w_0().truncate().child(self.path.clone()))
                    .child(
                        Button::new("neovim-files-collapse")
                            .ghost()
                            .accessibility_label("Collapse Neovim file browser")
                            .text_color(rgb(palette.muted))
                            .child("‹")
                            .on_click(
                                cx.listener(|_, _, _, cx| cx.emit(NeovimFilesEvent::Collapse)),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .gap_3()
                    .px_3()
                    .py_2()
                    .text_color(rgb(palette.muted))
                    .child(
                        div()
                            .id("directory-up")
                            .cursor_pointer()
                            .when(self.parent.is_none() || self.loading, |button| {
                                button.opacity(0.4)
                            })
                            .child("↑ Up")
                            .on_click(cx.listener(|pane, _, window, cx| {
                                pane.focus.focus(window, cx);
                                pane.up(cx);
                            })),
                    )
                    .child(
                        div()
                            .id("directory-refresh")
                            .cursor_pointer()
                            .child("Refresh")
                            .on_click(cx.listener(|pane, _, window, cx| {
                                pane.focus.focus(window, cx);
                                pane.refresh(cx);
                            })),
                    )
                    .child(
                        div()
                            .id("directory-hidden")
                            .cursor_pointer()
                            .child(if self.show_hidden {
                                "Hide dotfiles"
                            } else {
                                "Show dotfiles"
                            })
                            .on_click(cx.listener(|pane, _, window, cx| {
                                pane.focus.focus(window, cx);
                                pane.show_hidden = !pane.show_hidden;
                                pane.filter();
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .when_some(message.clone(), |area, message| {
                        area.child(div().p_3().text_color(rgb(palette.muted)).child(message))
                    })
                    .when(message.is_none(), |area| area.child(list)),
            )
            .when(self.truncated, |pane| {
                pane.child(
                    div()
                        .px_3()
                        .py_2()
                        .text_color(rgb(palette.muted))
                        .child("Directory listing limited to 4096 entries"),
                )
            })
    }
}
