//! Named, client-local pane layouts. Agent and PTY ownership stays with the backend.
use crate::{
    Desktop, Sidebar,
    design::{self, ui_scaled_px as px},
};
use gpui_kit::{
    assets::IconName,
    component::{
        ActiveTheme as _, Disableable, Icon, Sizable,
        button::*,
        input::{Input, InputEvent, InputState},
        popover::Popover,
    },
    prelude::*,
    *,
};
use monitter_lib::experimental_desktop::{
    NativePreferenceData, NativeWorkspace, NativeWorkspaceSet,
};
use rand_core::{OsRng, RngCore};
use serde_json::{Value, json};

#[derive(Clone)]
struct WorkspaceTabDrag {
    id: String,
    name: String,
    icon: IconName,
    color: Rgba,
    ink: Rgba,
    background: Rgba,
}
impl Render for WorkspaceTabDrag {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div().h(px(32.)).px(px(12.)).flex().items_center().gap(px(8.))
            .rounded(px(12.)).border_1().border_color(self.color)
            .bg(self.background).text_color(self.ink)
            .font_family(cx.theme().font_family.clone())
            .text_size((cx.theme().font_size - px(4.)).max(px(8.)))
            .child(Icon::new(self.icon.clone()).with_size(px(16.)))
            .child(self.name.clone())
    }
}
fn reorderable_tab(
    tab: Stateful<Div>,
    drag: WorkspaceTabDrag,
    on_drop: impl Fn(&WorkspaceTabDrag, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let target = drag.id.clone();
    let color = drag.color;
    tab.on_drag(drag, |drag, _, _, cx| {
        cx.stop_propagation();
        cx.new(|_| drag.clone())
    })
    .can_drop(move |value, _, _| value.downcast_ref::<WorkspaceTabDrag>()
        .is_some_and(|source| source.id != target))
    .drag_over::<WorkspaceTabDrag>(move |style, _, _, _|
        style.border_color(color).bg(color.opacity(0.18)))
    .on_drop(on_drop)
}

fn new_id() -> String {
    let mut bytes = [0u8; 16];
    OsRng.fill_bytes(&mut bytes);
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    )
}
pub(super) fn empty_layout() -> Value {
    json!({"version":1,"center":{"panel_name":"StackPanel","children":[{"panel_name":"TabPanel","children":[{"panel_name":"MonitterNativePane","children":[],"info":{"panel":{"id":format!("empty:{}", new_id())}}}],"info":{"tabs":{"active_index":0}}}],"info":{"stack":{"sizes":[600.0],"axis":0}}}})
}
pub fn loaded_workspaces(data: &NativePreferenceData) -> NativeWorkspaceSet {
    if let Some(mut set) = data.workspaces.clone() {
        for item in &mut set.items {
            if item.layout.is_none() {
                item.layout = Some(empty_layout());
            }
        }
        return set;
    }
    {
        let id = new_id();
        NativeWorkspaceSet {
            active_id: id.clone(),
            items: vec![NativeWorkspace {
                id,
                name: "Monitter".into(),
                color: "#38BFD2".into(),
                icon: "folder".into(),
                layout: data.layout.clone(),
            }],
        }
    }
}
fn icon(name: &str) -> IconName {
    match name {
        "code" => IconName::Code,
        "terminal" => IconName::Terminal,
        "globe" => IconName::Globe,
        "sparkles" => IconName::Star,
        _ => IconName::Folder,
    }
}
pub struct WorkspaceEditor {
    id: String,
    name: Entity<InputState>,
    color: Entity<InputState>,
    icon: String,
    error: Option<String>,
    confirm_remove: bool,
    _subscriptions: Vec<Subscription>,
}
impl Desktop {
    pub(super) fn capture_workspace_layout(&mut self, layout: Value) {
        if let Some(set) = &mut self.named_workspaces {
            if let Some(item) = set.items.iter_mut().find(|item| item.id == set.active_id) {
                item.layout = Some(layout);
            }
        }
    }
    fn workspace_ready(&self) -> bool {
        self.preferences.is_some()
            && !self.preferences_busy
            && !self.preferences_blocked
            && !self.preferences_restore
            && !self.preferences_restoring
            && !self.preferences_restore_suppressed
            && !self.closing
            && !self.close_pending
    }
    fn switch_named_workspace(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        if !self.workspace_ready()
            || self
                .named_workspaces
                .as_ref()
                .is_none_or(|set| set.active_id == id)
        {
            return;
        }
        if self
            .named_workspaces
            .as_ref()
            .is_none_or(|set| !set.items.iter().any(|item| item.id == id))
        {
            return;
        }
        self.capture_native_drafts(cx);
        match self.dock.read(cx).serialize(cx) {
            Ok(layout) => self.capture_workspace_layout(layout),
            Err(error) => {
                self.preferences_error = Some(error);
                cx.notify();
                return;
            }
        }
        self.workspaces_dirty = true;
        self.preferences_dirty = true;
        self.workspace_switch = Some(id);
        self.workspace_editor = None;
        self.selected = None;
        self.dock_current = None;
        self.sidebar = Sidebar::Chats;
        self.route_touched = true;
        self.preferences_restore = true;
        self.restore_preferences(window, cx);
        cx.notify();
    }
    fn add_named_workspace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.workspace_ready() {
            return;
        }
        let Some(set) = &mut self.named_workspaces else {
            return;
        };
        if set.items.len() >= 24 {
            return;
        }
        let id = new_id();
        let index = set.items.len();
        let colors = ["#38BFD2", "#41C968", "#A78BFA", "#F59E42", "#EF709C"];
        set.items.push(NativeWorkspace {
            id: id.clone(),
            name: format!("Workspace {}", index + 1),
            color: colors[index % colors.len()].into(),
            icon: "folder".into(),
            layout: Some(empty_layout()),
        });
        self.switch_named_workspace(id.clone(), window, cx);
        // The retained editor is separate from layout restoration.
        self.edit_named_workspace(&id, window, cx);
    }
    fn edit_named_workspace(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(item) = self
            .named_workspaces
            .as_ref()
            .and_then(|set| set.items.iter().find(|item| item.id == id))
        else {
            return;
        };
        let name = cx.new(|cx| InputState::new(window, cx).default_value(item.name.clone()));
        let color = cx.new(|cx| InputState::new(window, cx).default_value(item.color.clone()));
        let subscriptions = [&name, &color]
            .into_iter()
            .map(|input| {
                cx.subscribe(input, |view, _, event, cx| {
                    if matches!(event, InputEvent::PressEnter { .. }) {
                        view.save_workspace_editor(cx);
                    }
                })
            })
            .collect();
        self.workspace_editor = Some(WorkspaceEditor {
            id: id.into(),
            name,
            color,
            icon: item.icon.clone(),
            error: None,
            confirm_remove: false,
            _subscriptions: subscriptions,
        });
        cx.notify();
    }
    fn save_workspace_editor(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = &mut self.workspace_editor else {
            return;
        };
        let name = editor.name.read(cx).value().trim().to_owned();
        let color = editor.color.read(cx).value().trim().to_owned();
        if name.is_empty() || name.chars().count() > 80 || name.chars().any(char::is_control) {
            editor.error = Some("Name must contain 1–80 characters.".into());
            cx.notify();
            return;
        }
        if color.len() != 7
            || !color.starts_with('#')
            || !color[1..].bytes().all(|b| b.is_ascii_hexdigit())
        {
            editor.error = Some("Enter a colour in #RRGGBB format.".into());
            cx.notify();
            return;
        }
        if let Some(item) = self
            .named_workspaces
            .as_mut()
            .and_then(|set| set.items.iter_mut().find(|item| item.id == editor.id))
        {
            item.name = name;
            item.color = color.to_ascii_uppercase();
            item.icon = editor.icon.clone();
        }
        self.workspace_editor = None;
        self.workspaces_dirty = true;
        self.queue_preferences_save(cx);
    }
    fn reorder_named_workspace(&mut self, source: &str, target: &str, cx: &mut Context<Self>) {
        if !self.workspace_ready() { return; }
        if self.named_workspaces.as_mut().is_some_and(|set|
            monitter_gpui_desktop::workspace_order::reorder(set, source, target)) {
            self.workspaces_dirty = true;
            self.queue_preferences_save(cx);
            cx.notify();
        }
    }
    fn remove_named_workspace(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if !self.workspace_ready() { return; }
        let Some(set) = self.named_workspaces.as_ref() else { return; };
        if set.items.len() <= 1 { return; }
        let Some(index) = set.items.iter().position(|item| item.id == id) else { return; };
        if set.active_id == id {
            let neighbor = set.items[if index + 1 < set.items.len() { index + 1 } else { index - 1 }].id.clone();
            // Commit removal only after the neighbour's layout restores successfully.
            self.pending_workspace_removal = Some(id.into());
            self.switch_named_workspace(neighbor, window, cx);
            if self.workspace_switch.is_none() { self.pending_workspace_removal = None; }
        } else {
            monitter_gpui_desktop::workspace_order::remove(self.named_workspaces.as_mut().unwrap(), id);
            self.workspace_editor = None;
            self.workspaces_dirty = true;
            self.queue_preferences_save(cx);
        }
        cx.notify();
    }
    pub(super) fn render_workspace_tabs(
        &mut self,
        palette: design::Palette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let usage_width = if self.sidebar_collapsed {
            design::compact_sidebar_width_design(design::interface_scale())
        } else { self.current_sidebar_width(cx) };
        if let Some(usage) = &self.sidebar_usage {
            usage.update(cx, |usage, cx| usage.set_composed_palette(palette, usage_width, cx));
        }
        let ready = self.workspace_ready();
        let tern =
            design::tile_theme() == monitter_lib::experimental_desktop::NativeTileTheme::Tern;
        let mut tabs = div()
            .id("named-workspace-tabs")
            .flex()
            .items_center()
            .gap(px(5.))
            .flex_1()
            .min_w_0()
            .overflow_x_scroll();
        if let Some(set) = &self.named_workspaces {
            for item in &set.items {
                let active = item.id == set.active_id;
                let id = item.id.clone();
                let edit_id = id.clone();
                let menu_id = id.clone();
                let is_open = self
                    .workspace_editor
                    .as_ref()
                    .is_some_and(|editor| editor.id == id);
                let owner = cx.entity();
                let menu_owner = owner.clone();
                let key_id = id.clone();
                let focus = window
                    .use_keyed_state(
                        SharedString::from(format!("workspace-focus-{id}")),
                        cx,
                        |_, cx| cx.focus_handle(),
                    )
                    .read(cx)
                    .clone();
                let click_focus = focus.clone();
                let color = rgb(design::parse_color(&item.color, palette.accent));
                let selection_chrome = crate::chrome_motion::selection_chrome(
                    SharedString::from(format!("workspace-selection-motion-{id}")), active,
                    color, px(if tern { 12. } else { 5. }), window, cx,
                );
                let drop_id = id.clone();
                let drag = WorkspaceTabDrag {
                    id: id.clone(), name: item.name.clone(), icon: icon(&item.icon),
                    color, ink: rgb(palette.ink), background: rgb(palette.panel),
                };
                let trigger = Button::new(SharedString::from(format!("edit-workspace-{id}")))
                    .ghost()
                    .compact()
                    .w(px(22.))
                    .h(px(22.))
                    .p_0()
                    .icon(icon(&item.icon))
                    .accessibility_label(format!("Edit {} workspace", item.name))
                    .tooltip("Edit workspace")
                    .disabled(!ready && !is_open)
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(|_, _, cx| cx.stop_propagation());
                let editor_focus = self
                    .workspace_editor
                    .as_ref()
                    .filter(|editor| editor.id == id)
                    .map(|editor| editor.name.read(cx).focus_handle(cx));
                let menu = Popover::new(SharedString::from(format!("workspace-edit-menu-{id}")))
                    .trigger(trigger)
                    .anchor(Anchor::TopLeft)
                    .open(is_open)
                    .when_some(editor_focus.as_ref(), |menu, focus| menu.track_focus(focus))
                    .overlay_closable(true)
                    .on_open_change(move |open, window, cx| {
                        owner.update(cx, |view, cx| {
                            if *open {
                                if !view
                                    .workspace_editor
                                    .as_ref()
                                    .is_some_and(|editor| editor.id == menu_id)
                                {
                                    view.edit_named_workspace(&menu_id, window, cx);
                                    if let Some(editor) = &view.workspace_editor {
                                        editor.name.read(cx).focus_handle(cx).focus(window, cx);
                                    }
                                }
                            } else if view
                                .workspace_editor
                                .as_ref()
                                .is_some_and(|editor| editor.id == menu_id)
                            {
                                view.workspace_editor = None;
                                cx.notify();
                            }
                        });
                    })
                    .content(move |_, _, cx| {
                        menu_owner.update(cx, |view, cx| view.render_workspace_editor(palette, cx))
                    });
                tabs = tabs.child(
                    div()
                        .id(SharedString::from(format!("workspace-pill-{id}")))
                        .relative()
                        .flex_shrink_0()
                        .h(px(32.))
                        .max_w(px(300.))
                        .px(px(12.))
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .rounded(px(if tern { 12. } else { 5. }))
                        .border_1()
                        .border_color(rgb(palette.line).opacity(0.))
                        .text_color(rgb(if active { palette.ink } else { palette.muted }))
                        .cursor_pointer()
                        .track_focus(&focus)
                        .tab_index(0)
                        .focus(|style| style.border_color(color))
                        .hover(|style| style.bg(color.opacity(0.10)))
                        .when(ready, |tab| reorderable_tab(tab, drag,
                            cx.listener(move |view, drag: &WorkspaceTabDrag, _, cx| {
                                view.reorder_named_workspace(&drag.id, &drop_id, cx);
                                cx.stop_propagation();
                            })))
                        .role(Role::Tab)
                        .aria_selected(active)
                        .aria_label(format!(
                            "{} workspace{}",
                            item.name,
                            if active { ", selected" } else { "" }
                        ))
                        .child(div().absolute().inset_0().child(selection_chrome))
                        .child(menu)
                        .child(div().min_w_0().truncate()
                            .text_size((cx.theme().font_size - px(4.)).max(px(8.)))
                            .child(item.name.clone()))
                        .child(div().size(px(8.)).rounded_full().bg(color))
                        .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                            click_focus.focus(window, cx);
                            cx.stop_propagation();
                        })
                        .on_key_down(cx.listener(move |view, event: &KeyDownEvent, window, cx| {
                            if event.keystroke.key == "f2" {
                                view.edit_named_workspace(&key_id, window, cx);
                                cx.stop_propagation();
                            } else if event.keystroke.key == "enter"
                                || event.keystroke.key == "space"
                            {
                                view.switch_named_workspace(key_id.clone(), window, cx);
                                cx.stop_propagation();
                            }
                        }))
                        .on_click(cx.listener(move |view, event: &ClickEvent, window, cx| {
                            if event.click_count() == 2 {
                                view.edit_named_workspace(&edit_id, window, cx);
                            } else {
                                view.switch_named_workspace(id.clone(), window, cx);
                            }
                        })),
                );
            }
        }
        tabs = tabs.child(
            Button::new("add-workspace")
                .ghost()
                .compact()
                .flex_shrink_0()
                .label("+")
                .accessibility_label("New workspace")
                .tooltip("New workspace")
                .disabled(
                    !ready
                        || self
                            .named_workspaces
                            .as_ref()
                            .is_none_or(|set| set.items.len() >= 24),
                )
                .on_click(cx.listener(|view, _, window, cx| view.add_named_workspace(window, cx))),
        );
        let bar = div()
            .w_full()
            .flex_shrink_0()
            .pl(gpui_kit::px(
                if window.is_fullscreen() || (tern && !self.sidebar_is_hidden()) {
                    10.
                } else {
                    82.
                },
            ))
            .pr(px(10.))
            // macOS controls are 14pt tall with a 22pt top inset: their
            // centre is 29pt. Keep the whole row on that native centre line,
            // including the usage rings, independently of interface scale.
            .h(gpui_kit::px(58.))
            .py_0()
            .flex()
            .items_center()
            .gap(px(5.))
            .on_mouse_down(MouseButton::Left, |event, window, _| {
                if event.click_count == 2 {
                    window.titlebar_double_click();
                } else {
                    window.start_window_move();
                }
            })
            .when(tern, |bar| {
                let label = if self.sidebar_is_hidden() {
                    "Show full sidebar"
                } else if self.sidebar_collapsed {
                    "Hide sidebar"
                } else {
                    "Use icon sidebar"
                };
                bar.child(
                    Button::new("cycle-main-sidebar")
                        .track_focus(&self.sidebar_toggle_focus)
                        .ghost()
                        .compact()
                        .flex_shrink_0()
                        .icon(IconName::PanelLeft)
                        .accessibility_label(label)
                        .tooltip(label)
                        .on_click(
                            cx.listener(|view, _, window, cx| view.toggle_sidebar(window, cx)),
                        ),
                )
            })
            .child(tabs)
            .children(self.workspace_usage_rings.clone());

        div()
            .flex_shrink_0()
            .w_full()
            .flex()
            .flex_col()
            // Reclaim the row's empty lower half without moving its controls.
            // The dock's two half-insets then supply exactly the density gutter.
            .when(tern, |row| row.mb((px(32.) - gpui_kit::px(58.)) / 2.))
            .font_family(cx.theme().font_family.clone())
            .text_size(cx.theme().font_size)
            .text_color(rgb(palette.ink))
            .child(bar)
    }
    fn render_workspace_editor(
        &mut self,
        palette: design::Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if let Some(editor) = &self.workspace_editor {
            let can_remove = self.workspace_ready()
                && self.named_workspaces.as_ref().is_some_and(|set| set.items.len() > 1);
            if editor.confirm_remove {
                let id = editor.id.clone();
                return div().id("workspace-remove-confirmation")
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .w(px(320.)).flex().flex_col().gap(px(10.))
                    .text_color(rgb(palette.ink)).font_family(cx.theme().font_family.clone())
                    .text_size(cx.theme().font_size)
                    .child("Remove this workspace?")
                    .child("Its chats and running agents will remain available.")
                    .child(div().flex().justify_end().gap(px(5.))
                        .child(Button::new("cancel-workspace-removal").compact().label("Cancel")
                            .on_click(cx.listener(|view, _, _, cx| {
                                if let Some(editor) = &mut view.workspace_editor { editor.confirm_remove = false; }
                                cx.notify();
                            })))
                        .child(Button::new("confirm-workspace-removal").compact().label("Remove workspace")
                            .disabled(!can_remove)
                            .on_click(cx.listener(move |view, _, window, cx|
                                view.remove_named_workspace(&id, window, cx)))))
                    .into_any_element();
            }
            let mut icons = div().flex().gap(px(5.));
            for choice in ["folder", "code", "terminal", "globe", "sparkles"] {
                let selected = editor.icon == choice;
                icons = icons.child(
                    Button::new(SharedString::from(format!("workspace-icon-{choice}")))
                        .compact()
                        .icon(icon(choice))
                        .accessibility_label(format!("Workspace icon: {choice}"))
                        .tooltip(choice)
                        .when(selected, |button| button.primary())
                        .on_click(cx.listener(move |view, _, _, cx| {
                            if let Some(editor) = &mut view.workspace_editor {
                                editor.icon = choice.into();
                            }
                            cx.notify();
                        })),
                );
            }
            let mut colors = div().flex().gap(px(5.));
            for value in ["#38BFD2", "#41C968", "#A78BFA", "#F59E42", "#EF709C"] {
                colors = colors.child(
                    Button::new(SharedString::from(format!("workspace-color-{value}")))
                        .compact()
                        .label("●")
                        .text_color(rgb(design::parse_color(value, palette.accent)))
                        .accessibility_label(format!("Workspace colour {value}"))
                        .tooltip(value)
                        .on_click(cx.listener(move |view, _, window, cx| {
                            if let Some(editor) = &view.workspace_editor {
                                editor
                                    .color
                                    .update(cx, |input, cx| input.set_value(value, window, cx));
                            }
                        })),
                );
            }
            div()
                .id("workspace-editor")
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(|_, _, cx| cx.stop_propagation())
                .flex()
                .flex_col()
                .gap(px(5.))
                .w(px(320.))
                .max_w_full()
                .max_h(px(450.))
                .overflow_y_scroll()
                .font_family(cx.theme().font_family.clone())
                .text_size(cx.theme().font_size)
                .text_color(rgb(palette.ink))
                .on_key_down(cx.listener(|view, event: &KeyDownEvent, _, cx| {
                    if event.keystroke.key == "escape" {
                        view.workspace_editor = None;
                        cx.stop_propagation();
                        cx.notify();
                    } else if event.keystroke.key == "enter" {
                        view.save_workspace_editor(cx);
                        cx.stop_propagation();
                    }
                }))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(10.))
                        .child(
                            div()
                                .w_full()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .gap(px(5.))
                                .child("Workspace name")
                                .child(Input::new(&editor.name).aria_label("Workspace name")),
                        )
                        .child(
                            div()
                                .w_full()
                                .flex()
                                .flex_col()
                                .gap(px(5.))
                                .child("Colour (#RRGGBB)")
                                .child(Input::new(&editor.color).aria_label("Workspace colour")),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .flex_wrap()
                        .gap(px(10.))
                        .child(icons)
                        .child(colors),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_end()
                        .gap(px(5.))
                        .child(
                            Button::new("save-workspace")
                                .compact()
                                .label("Save")
                                .on_click(
                                    cx.listener(|view, _, _, cx| view.save_workspace_editor(cx)),
                                ),
                        )
                        .child(
                            Button::new("cancel-workspace-edit")
                                .compact()
                                .label("Cancel")
                                .on_click(cx.listener(|view, _, _, cx| {
                                    view.workspace_editor = None;
                                    cx.notify();
                                })),
                        ),
                )
                .children(
                    editor
                        .error
                        .as_ref()
                        .map(|error| div().text_color(rgb(0xd35e5e)).child(error.clone())),
                )
                .child(Button::new("remove-workspace").ghost().compact()
                    .label("Remove workspace…").disabled(!can_remove)
                    .tooltip(if can_remove { "Remove this workspace" } else { "Keep at least one workspace" })
                    .on_click(cx.listener(|view, _, _, cx| {
                        if let Some(editor) = &mut view.workspace_editor { editor.confirm_remove = true; }
                        cx.notify();
                    })))
                .into_any_element()
        } else {
            div().into_any_element()
        }
    }
}
#[cfg(test)]
mod tests {
    use super::{empty_layout, loaded_workspaces, new_id};
    use monitter_lib::experimental_desktop::{NativePreferenceData, NativeWorkspace};
    use serde_json::json;
    use gpui_kit::{AppContext as _, Context, Render, IntoElement, TestAppContext, Window,
        MouseButton, Modifiers, SharedString, ParentElement as _, Styled as _,
        InteractiveElement as _, StatefulInteractiveElement as _, div, point, rgb};
    struct DragFixture {
        set: monitter_lib::experimental_desktop::NativeWorkspaceSet,
        clicks: usize,
    }
    impl Render for DragFixture {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let mut row = div().w(gpui_kit::px(600.)).h(gpui_kit::px(80.)).flex().gap(gpui_kit::px(5.));
            for item in &self.set.items {
                let id = item.id.clone();
                let drag = super::WorkspaceTabDrag { id: id.clone(), name: item.name.clone(),
                    icon: gpui_kit::assets::IconName::Folder, color: rgb(0x38bfd2),
                    ink: rgb(0xffffff), background: rgb(0x111111) };
                let tab = div().id(SharedString::from(format!("drag-fixture-{id}")))
                    .w(gpui_kit::px(160.)).h(gpui_kit::px(32.)).flex_shrink_0()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(|view, _, _, cx| { view.clicks += 1; cx.notify(); }));
                row = row.child(super::reorderable_tab(tab, drag, cx.listener(
                    move |view, drag: &super::WorkspaceTabDrag, _, cx| {
                        monitter_gpui_desktop::workspace_order::reorder(&mut view.set, &drag.id, &id);
                        cx.stop_propagation(); cx.notify();
                    })));
            }
            row
        }
    }
    #[gpui::test]
    fn workspace_drag_reorders_both_directions_without_clicking(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let (view, cx) = cx.add_window_view(|_, _| DragFixture {
            set: monitter_lib::experimental_desktop::NativeWorkspaceSet {
                active_id: "b".into(), items: ["a", "b", "c"].into_iter().map(|id| NativeWorkspace {
                    id: id.into(), name: id.into(), icon: "folder".into(), color: "#38BFD2".into(), layout: None,
                }).collect(),
            }, clicks: 0,
        });
        cx.run_until_parked(); cx.update(|window, cx| window.draw(cx).clear(cx));
        for (from, to, expected) in [(80., 410., vec!["b", "c", "a"]), (410., 80., vec!["a", "b", "c"])] {
            let start = point(gpui_kit::px(from), gpui_kit::px(16.));
            let end = point(gpui_kit::px(to), gpui_kit::px(16.));
            cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::none());
            cx.simulate_mouse_move(point(gpui_kit::px(from + 10.), gpui_kit::px(16.)), Some(MouseButton::Left), Modifiers::none());
            cx.simulate_mouse_move(end, Some(MouseButton::Left), Modifiers::none());
            cx.simulate_mouse_up(end, MouseButton::Left, Modifiers::none());
            cx.run_until_parked();
            cx.update(|window, cx| {
                let state = view.read(cx);
                assert_eq!(state.set.items.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(), expected);
                assert_eq!(state.set.active_id, "b"); assert_eq!(state.clicks, 0);
                window.draw(cx).clear(cx);
            });
        }
        cx.simulate_click(point(gpui_kit::px(80.), gpui_kit::px(16.)), Modifiers::none());
        cx.update(|_, cx| assert_eq!(view.read(cx).clicks, 1));
    }
    #[test]
    fn legacy_layout_becomes_default_workspace_without_losing_panes() {
        let layout = empty_layout();
        let data = NativePreferenceData {
            layout: Some(layout.clone()),
            ..Default::default()
        };
        let set = loaded_workspaces(&data);
        assert_eq!(set.items.len(), 1);
        assert_eq!(set.items[0].layout, Some(layout));
        assert_eq!(set.active_id, set.items[0].id);
    }
    #[test]
    fn missing_named_layout_does_not_inherit_legacy_panes() {
        let mut set = loaded_workspaces(&NativePreferenceData::default());
        set.items[0].layout = None;
        let data = NativePreferenceData {
            layout: Some(json!({"legacy": true})),
            workspaces: Some(set),
            ..Default::default()
        };
        let loaded = loaded_workspaces(&data);
        let layout = loaded.items[0].layout.as_ref().unwrap();
        assert!(layout.get("legacy").is_none());
        let ids = crate::docking::DockWorkspace::saved_ids(layout).unwrap();
        assert_eq!(ids.len(), 1);
        assert!(ids[0].starts_with("empty:"));
    }
    #[test]
    fn workspace_records_keep_distinct_layouts_and_selected_identity() {
        let mut set = loaded_workspaces(&NativePreferenceData::default());
        set.items.push(NativeWorkspace {
            id: new_id(),
            name: "Second".into(),
            color: "#41C968".into(),
            icon: "code".into(),
            layout: Some(empty_layout()),
        });
        set.active_id = set.items[1].id.clone();
        set.items[0].layout = Some(empty_layout());
        let data = NativePreferenceData {
            workspaces: Some(set.clone()),
            ..Default::default()
        };
        assert_eq!(loaded_workspaces(&data), set);
        assert_ne!(set.items[0].id, set.items[1].id);
    }
}
