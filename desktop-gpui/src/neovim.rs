//! Native Neovim grid rendering and queued private backend IPC.
use crate::design::ui_scaled_px;
use futures::{FutureExt as _, StreamExt as _};
use gpui_kit::component::{
    ActiveTheme,
    button::{Button, ButtonVariants},
};
use gpui_kit::{prelude::*, *};
use monitter_gpui_desktop::neovim_model::{Cell, CmdlineChunk, NeovimModel};
use monitter_lib::experimental_desktop::DesktopService;
use serde_json::{Value, json};
use std::{ops::Range, time::Duration};

/// Emitted by a `NeovimSession` when the model advanced, the layout changed,
/// or the editor exited/failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NeovimSessionEvent {
    /// A flushed redraw and/or a window-layout change is available.
    Updated,
    /// The editor process exited (`snapshot["exited"] == true`).
    Exited,
    /// The backend could not be reached.
    Error,
}

fn session_is_missing(error: &str) -> bool {
    error == "Neovim session no longer exists."
}

// Keep animated editors responsive, but bound idle RPC and hidden lifecycle work.
fn neovim_poll_delay(visible: bool, idle_reads: u32) -> Duration {
    if !visible {
        return Duration::from_secs(1);
    }
    Duration::from_millis((20u64 << idle_reads.min(4)).min(250))
}

fn has_gui_panes(pane_count: usize) -> bool {
    pane_count > 0
}

fn to_rgb(v: i64) -> u32 {
    // Neovim colours arrive as 0xRRGGBB or 0xAARRGGBB; drop any alpha.
    (v as u32) & 0x00FF_FFFF
}

#[derive(Clone)]
struct Appearance {
    family: String,
    size: f32,
    weight: f32,
    line_ratio: f32,
}
impl Appearance {
    fn from_snapshot(snapshot: &Value) -> Self {
        let settings = &snapshot["settings"];
        let family = field(settings, "terminalFont");
        Self {
            family: if family.trim().is_empty() {
                "Menlo".into()
            } else {
                family
            },
            size: settings["terminalFontSize"]
                .as_f64()
                .filter(|v| v.is_finite())
                .unwrap_or(14.)
                .clamp(8., 32.) as f32,
            weight: settings["terminalFontWeight"]
                .as_f64()
                .filter(|v| v.is_finite())
                .unwrap_or(400.)
                .clamp(300., 700.) as f32,
            line_ratio: settings["terminalLineHeight"]
                .as_f64()
                .filter(|v| v.is_finite())
                .unwrap_or(1.)
                .clamp(1., 2.5) as f32,
        }
    }
    fn cell_height(&self) -> f32 {
        let height: f32 = ui_scaled_px((self.size * 1.4 * self.line_ratio).ceil()).into();
        height
    }
}

/// One attached Neovim session: owns the `NeovimModel`, polls redraw batches,
/// and maintains the window-layout projection used to reconcile dock panes.
pub struct NeovimSession {
    service: DesktopService,
    id: String,
    pub snapshot: Value,
    model: NeovimModel,
    workspace: Value,
    palette: Option<crate::design::Palette>,
    seq: u64,
    error: Option<String>,
    sizes: std::collections::HashMap<i64, (u16, u16)>,
    queue: std::collections::VecDeque<(&'static str, Value)>,
    frame: Vec<Value>,
    last_focus: Option<i64>,
    pub pending_focus: Option<i64>,
    focus_revision: u64,
    gui_panes: std::collections::HashSet<EntityId>,
    visible_panes: std::collections::HashSet<EntityId>,
    wake: futures::channel::mpsc::Sender<()>,
    _poll: Task<()>,
}
impl EventEmitter<NeovimSessionEvent> for NeovimSession {}
fn empty_layout() -> Value {
    json!({"layout":null,"windows":[],"exited":false})
}
impl NeovimSession {
    pub fn new(service: DesktopService, id: String, cx: &mut Context<Self>) -> Self {
        let pane_service = service.clone();
        let poll_id = id.clone();
        let (wake, mut wakes) = futures::channel::mpsc::channel(1);
        let poll = cx.spawn(async move |this, cx| {
            let mut idle_reads = 0u32;
            loop {
                let Ok((after, commands, exited, focus_revision)) = this.update(cx, |s, _| {
                    (
                        s.seq,
                        s.queue.drain(..).collect::<Vec<_>>(),
                        s.snapshot["exited"] == true,
                        s.focus_revision,
                    )
                }) else {
                    break;
                };
                if exited {
                    break;
                }
                let had_commands = !commands.is_empty();
                let service = service.clone();
                let id = poll_id.clone();
                let result = cx
                    .background_executor()
                    .spawn(async move {
                        let mut error = None;
                        let mut focused = false;
                        for (name, args) in commands {
                            if let Err(e) = service.invoke(name, args) {
                                error = Some(e);
                            } else if name == "neovim_focus" {
                                focused = true;
                            }
                        }
                        (
                            service.invoke("read_neovim", json!({"id":id,"after":after})),
                            error,
                            focused.then_some(focus_revision),
                        )
                    })
                    .await;
                let active = result
                    .0
                    .as_ref()
                    .ok()
                    .is_some_and(|response| response["seq"].as_u64().unwrap_or(after) != after);
                if this
                    .update(cx, |s, cx| {
                        if result.2 == Some(s.focus_revision) {
                            s.pending_focus = None;
                        }
                        if let Some(error) = result.1 {
                            s.error = Some(error);
                            cx.notify();
                        }
                        match result.0 {
                            Ok(response) => s.consume(response, cx),
                            Err(error) => s.consume_read_error(error, cx),
                        }
                    })
                    .is_err()
                {
                    break;
                }
                idle_reads = if active || had_commands {
                    0
                } else {
                    idle_reads.saturating_add(1)
                };
                let Ok((has_panes, visible)) = this.update(cx, |s, _| {
                    (has_gui_panes(s.gui_panes.len()), !s.visible_panes.is_empty())
                }) else {
                    break;
                };
                if !has_panes {
                    // The GUI released every pane for this session. Keep the
                    // backend editor alive, but park this client poller until
                    // a saved workspace recreates a pane and wakes us.
                    if wakes.next().await.is_none() {
                        break;
                    }
                    idle_reads = 0;
                    continue;
                }
                let delay = neovim_poll_delay(visible, idle_reads);
                let timer = cx.background_executor().timer(delay).fuse();
                let wake = wakes.next().fuse();
                futures::pin_mut!(timer, wake);
                if let futures::future::Either::Left((signal, _)) =
                    futures::future::select(wake, timer).await
                {
                    if signal.is_none() {
                        break;
                    }
                    idle_reads = 0;
                }
            }
        });
        Self {
            service: pane_service,
            id,
            snapshot: empty_layout(),
            model: NeovimModel::new(),
            workspace: Value::Null,
            palette: None,
            seq: 0,
            error: None,
            sizes: Default::default(),
            queue: Default::default(),
            frame: Vec::new(),
            last_focus: None,
            pending_focus: None,
            focus_revision: 0,
            gui_panes: Default::default(),
            visible_panes: Default::default(),
            wake,
            _poll: poll,
        }
    }
    fn set_pane_visible(&mut self, pane: EntityId, visible: bool) {
        let changed = if visible {
            self.visible_panes.insert(pane)
        } else {
            self.visible_panes.remove(&pane)
        };
        if changed {
            let _ = self.wake.try_send(());
        }
    }
    fn retain_gui_pane(&mut self, pane: EntityId) {
        if self.gui_panes.insert(pane) {
            let _ = self.wake.try_send(());
        }
    }
    fn release_gui_pane(&mut self, pane: EntityId) {
        if self.gui_panes.remove(&pane) {
            // A hidden pane may have no visibility transition left to wake the
            // poll loop. Wake it so it can park immediately when this was the
            // final GUI view for the session.
            let _ = self.wake.try_send(());
        }
        self.set_pane_visible(pane, false);
    }
    pub fn set_workspace(&mut self, snapshot: &Value, cx: &mut Context<Self>) {
        let workspace = json!({"projects": snapshot["projects"], "hosts": snapshot["hosts"]});
        let changed =
            self.workspace != workspace || self.snapshot["settings"] != snapshot["settings"];
        self.workspace = workspace;
        self.snapshot["settings"] = snapshot["settings"].clone();
        if changed {
            cx.notify();
        }
    }
    pub fn set_theme(&mut self, palette: crate::design::Palette, settings: &Value, cx: &mut Context<Self>) {
        if self.palette != Some(palette) || self.snapshot["settings"] != *settings {
            self.palette = Some(palette);
            self.snapshot["settings"] = settings.clone();
            cx.notify();
        }
    }
    /// Retained rendered window text and filename metadata; never reads files.
    pub(super) fn palette_search_text(&self, win: i64) -> String {
        const LIMIT: usize = 512 * 1024;
        let mut text = String::new();
        if let Some(row) = self.snapshot["windows"].as_array()
            .and_then(|rows| rows.iter().find(|row| row["id"].as_i64() == Some(win))) {
            for key in ["name", "cwd", "filetype"] {
                if let Some(value) = row[key].as_str() {
                    for ch in value.chars().chain(std::iter::once('\n')) {
                        if text.len() + ch.len_utf8() > LIMIT { return text; }
                        text.push(ch);
                    }
                }
            }
        }
        for position in self.model.windows.values().filter(|position| position.win == win) {
            if let Some(grid) = self.model.grid(position.grid) {
                for row in grid.cells.chunks(usize::from(grid.cols).max(1)) {
                    for ch in row.iter().flat_map(|cell| cell.text.chars()).chain(std::iter::once('\n')) {
                        if text.len() + ch.len_utf8() > LIMIT { return text; }
                        text.push(ch);
                    }
                }
            }
        }
        text
    }
    pub fn model(&self) -> &NeovimModel {
        &self.model
    }
    fn consume(&mut self, mut response: Value, cx: &mut Context<Self>) {
        if response["gap"] == true {
            self.model.reset();
            self.frame.clear();
            self.sizes.clear();
        }
        self.seq = response["seq"].as_u64().unwrap_or(self.seq);
        let mut flushed = false;
        if let Some(batches) = response["events"].as_array() {
            for batch in batches {
                if let Some(events) = batch["events"].as_array() {
                    for event in events {
                        self.frame.push(event.clone());
                        if event[0] == "flush" {
                            flushed |= self
                                .model
                                .apply_redraw(&Value::Array(std::mem::take(&mut self.frame)));
                        }
                    }
                }
            }
        }
        if self.frame.len() > 32768 {
            self.frame.clear();
            self.error = Some("Neovim redraw frame exceeded the UI limit.".into());
        }
        if let Some(error) = response["error"].as_str() {
            self.error = Some(error.to_owned());
        }
        // Metadata is refreshed asynchronously and can trail an input batch.
        // A flushed cursor event identifies the window currently receiving
        // Neovim input, including navigation performed by mappings/plugins.
        if let Some(win) = self
            .model
            .cursor
            .as_ref()
            .and_then(|cursor| self.model.window_for_grid.get(&cursor.grid))
        {
            response["currentWindow"] = json!(win);
        }
        let changed = self.snapshot["layout"] != response["layout"]
            || self.snapshot["windows"] != response["windows"]
            || self.snapshot["currentWindow"] != response["currentWindow"]
            || self.snapshot["exited"] != response["exited"];
        response["settings"] = self.snapshot["settings"].clone();
        self.snapshot = response;
        if flushed || changed {
            cx.emit(NeovimSessionEvent::Updated);
            cx.notify();
        }
    }
    fn consume_read_error(&mut self, error: String, cx: &mut Context<Self>) {
        if session_is_missing(&error) {
            // This is an authoritative registry miss, not a transport failure.
            // Feed it through normal reconciliation so orphan panes disappear.
            self.queue.clear();
            self.pending_focus = None;
            self.consume(json!({"layout": null, "windows": [], "exited": true}), cx);
        } else {
            self.error = Some(error);
            cx.notify();
        }
    }

    fn enqueue(&mut self, name: &'static str, mut args: Value, cx: &mut Context<Self>) {
        if self.queue.len() >= 512 {
            self.error = Some("Neovim input queue is full; wait for the editor.".into());
            cx.notify();
            return;
        }
        args["id"] = json!(self.id);
        self.queue.push_back((name, args));
        let _ = self.wake.try_send(());
    }
    pub fn focus_window(&mut self, win: i64, cx: &mut Context<Self>) {
        if self.last_focus != Some(win) {
            self.last_focus = Some(win);
            self.pending_focus = Some(win);
            self.focus_revision = self.focus_revision.wrapping_add(1);
            self.enqueue("neovim_focus", json!({"windowId":win}), cx);
        }
    }
    pub fn open_file(&mut self, win: i64, path: &str, cx: &mut Context<Self>) {
        self.focus_window(win, cx);
        self.enqueue("neovim_open_file", json!({"windowId":win,"path":path}), cx);
        self.last_focus = None;
    }
    pub fn command(&mut self, win: i64, command: &str, cx: &mut Context<Self>) {
        self.focus_window(win, cx);
        self.enqueue("neovim_command", json!({"command":command}), cx);
        self.last_focus = None;
    }
    pub fn close_window(&mut self, win: i64, cx: &mut Context<Self>) {
        self.enqueue("neovim_close_window", json!({"windowId":win}), cx);
        self.last_focus = None;
    }
    pub fn input(&mut self, win: i64, text: &str, cx: &mut Context<Self>) {
        if !text.is_empty() {
            self.focus_window(win, cx);
            self.error = None;
            self.enqueue("neovim_input", json!({"keys":text}), cx);
        }
    }
    fn paste(&mut self, win: i64, text: &str, cx: &mut Context<Self>) {
        self.focus_window(win, cx);
        self.enqueue("neovim_paste", json!({"text":text}), cx);
    }
    fn resize(&mut self, win: i64, cols: u16, rows: u16, cx: &mut Context<Self>) {
        let Some(grid) = self.model.windows.get(&win).map(|w| w.grid) else {
            return;
        };
        if self.sizes.get(&grid) == Some(&(cols, rows)) {
            return;
        }
        self.sizes.insert(grid, (cols, rows));
        self.enqueue(
            "neovim_resize",
            json!({"grid":grid,"cols":cols,"rows":rows}),
            cx,
        );
    }
    fn mouse(
        &mut self,
        win: i64,
        cell: CellPoint,
        button: &str,
        action: &str,
        cx: &mut Context<Self>,
    ) {
        let Some(grid) = self.model.windows.get(&win).map(|w| w.grid) else {
            return;
        };
        self.last_focus = Some(win);
        self.enqueue("neovim_mouse",json!({"windowId":win,"grid":grid,"row":cell.row,"col":cell.col,"button":button,"action":action,"modifier":""}),cx);
    }
}

/// A dock pane that paints one Neovim window grid from the shared session model.
pub struct NeovimPane {
    session: Entity<NeovimSession>,
    win: i64,
    focus: FocusHandle,
    marked: String,
    appearance: Appearance,
    grid_bounds: Option<Bounds<Pixels>>,
    cell_width: Pixels,
    files: Entity<crate::neovim_files::NeovimFilesPane>,
    files_visible: bool,
}

/// An owned, renderable copy of one window's grid frame, decoupled from the
/// session borrow so painting never needs a mutable `cx` while reading.
struct RenderFrame {
    cols: u16,
    rows: u16,
    cells: Vec<RenderCell>,
    cursor: Option<(i64, i64)>,
    default_bg: u32,
}
struct RenderCell {
    text: String,
    fg: u32,
    bg: u32,
    bold: bool,
    italic: bool,
    underline: bool,
}

fn render_frame(model: &NeovimModel, win: i64) -> Option<RenderFrame> {
    let grid_id = model.windows.get(&win)?.grid;
    let grid = model.grid(grid_id)?;
    if grid.cells.len() != usize::from(grid.cols) * usize::from(grid.rows) {
        return None;
    }
    let default_fg = model.default_fg.map(to_rgb).unwrap_or(0xdddddd);
    let default_bg = model.default_bg.map(to_rgb).unwrap_or(0x0a0a0a);
    let cursor = model
        .cursor
        .as_ref()
        .filter(|c| c.grid == grid_id)
        .map(|c| (c.row, c.col));
    let mut cells = Vec::with_capacity(grid.cells.len());
    for cell in &grid.cells {
        cells.push(resolve_cell(cell, model, default_fg, default_bg));
    }
    Some(RenderFrame {
        cols: grid.cols,
        rows: grid.rows,
        cells,
        cursor,
        default_bg,
    })
}

fn resolve_cell(cell: &Cell, model: &NeovimModel, default_fg: u32, default_bg: u32) -> RenderCell {
    let mut fg = default_fg;
    let mut bg = default_bg;
    let mut bold = false;
    let mut italic = false;
    let mut underline = false;
    if let Some(hl) = model.highlights.get(&cell.hl_id) {
        if let Some(f) = hl.fg {
            fg = to_rgb(f);
        }
        if let Some(b) = hl.bg {
            bg = to_rgb(b);
        }
        bold = hl.bold;
        italic = hl.italic;
        underline = hl.underline;
        if hl.reverse {
            std::mem::swap(&mut fg, &mut bg);
        }
    }
    RenderCell {
        text: cell.text.clone(),
        fg,
        bg,
        bold,
        italic,
        underline,
    }
}

impl NeovimPane {
    pub fn new(
        session: Entity<NeovimSession>,
        win: i64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let pane_id = cx.entity_id();
        session.update(cx, |session, _| session.retain_gui_pane(pane_id));
        cx.on_release(move |pane, cx| {
            pane.session
                .update(cx, |session, _| session.release_gui_pane(pane_id));
        })
        .detach();
        let appearance = Appearance::from_snapshot(&session.read(cx).snapshot);
        cx.observe(&session, |_, _, cx| cx.notify()).detach();
        let service = session.read(cx).service.clone();
        let id = session.read(cx).id.clone();
        let path = session.read(cx).snapshot["cwd"]
            .as_str()
            .unwrap_or("")
            .to_owned();
        let files =
            cx.new(|cx| crate::neovim_files::NeovimFilesPane::new(service, id, path, window, cx));
        cx.subscribe_in(
            &files,
            window,
            |pane, _, event: &crate::neovim_files::NeovimFilesEvent, window, cx| match event {
                crate::neovim_files::NeovimFilesEvent::OpenFile(path) => {
                    pane.session
                        .update(cx, |session, cx| session.open_file(pane.win, path, cx));
                    pane.focus.focus(window, cx);
                }
                crate::neovim_files::NeovimFilesEvent::Collapse => {
                    pane.files_visible = false;
                    pane.focus.focus(window, cx);
                    cx.notify();
                }
            },
        )
        .detach();
        let focus = cx.focus_handle();
        cx.on_focus(&focus, window, |pane, _, cx| {
            // Focus is an event, not a render side effect. Neovim can change
            // its current window itself, so the previous request is no proof
            // that this newly focused native pane still owns editor input.
            pane.session.update(cx, |session, cx| {
                session.last_focus = None;
                session.focus_window(pane.win, cx);
            });
            cx.notify();
        })
        .detach();
        Self {
            session,
            win,
            focus,
            marked: String::new(),
            appearance,
            grid_bounds: None,
            cell_width: ui_scaled_px(8.4),
            files,
            files_visible: true,
        }
    }

    pub(crate) fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        let pane_id = cx.entity_id();
        self.session
            .update(cx, |session, _| session.set_pane_visible(pane_id, visible));
    }

    pub fn show_files(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.files_visible = true;
        self.files.focus_handle(cx).focus(window, cx);
        cx.notify();
    }

    /// Whether this pane's window hosts the visible cursor (and therefore the
    /// cmdline/messages/popup overlays).
    fn is_host(&self, cx: &App) -> bool {
        let model = self.session.read(cx).model();
        model
            .cursor
            .as_ref()
            .and_then(|c| model.window_for_grid.get(&c.grid).copied())
            == Some(self.win)
    }

    /// Notify the backend of the pane's computed grid size (best-effort, once).
    fn send_resize(&mut self, cx: &mut Context<Self>) {
        let Some(bounds) = self.grid_bounds else {
            return;
        };
        let cell: f32 = self.cell_width.into();
        let width: f32 = bounds.size.width.into();
        let height: f32 = bounds.size.height.into();
        let cols = (width / cell.max(1.)).floor().clamp(10., 500.) as u16;
        let rows = (height / self.appearance.cell_height())
            .floor()
            .clamp(4., 300.) as u16;
        self.session
            .update(cx, |session, cx| session.resize(self.win, cols, rows, cx));
    }

    fn key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.focus.is_focused(window) {
            return;
        }
        let mods = event.keystroke.modifiers;
        if mods.platform {
            if event.keystroke.key == "v" {
                if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                    self.session
                        .update(cx, |s, cx| s.paste(self.win, &text, cx));
                }
                cx.stop_propagation();
            }
            return;
        }
        let printable = event.prefer_character_input
            || (!mods.control
                && !mods.alt
                && (event.keystroke.key.chars().count() == 1 || event.keystroke.key == "space"));
        if printable {
            // Delivered through the committed-text (IME) path instead.
            return;
        }
        if let Some(notation) = nvim_key(&event.keystroke) {
            self.send_input(&notation, cx);
            cx.stop_propagation();
        }
    }

    fn send_input(&mut self, text: &str, cx: &mut Context<Self>) {
        self.session
            .update(cx, |session, cx| session.input(self.win, text, cx));
    }

    fn mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        if !event.modifiers.shift {
            if let Some(cell) = self.cell_at(event.position, cx) {
                self.session
                    .update(cx, |s, cx| s.mouse(self.win, cell, "left", "press", cx));
            }
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn mouse_up(&mut self, event: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        if !event.modifiers.shift {
            if let Some(cell) = self.cell_at(event.position, cx) {
                self.session
                    .update(cx, |s, cx| s.mouse(self.win, cell, "left", "release", cx));
            }
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn cell_at(&self, position: Point<Pixels>, cx: &App) -> Option<CellPoint> {
        let bounds = self.grid_bounds?;
        if !bounds.contains(&position) {
            return None;
        }
        let grid = render_frame(self.session.read(cx).model(), self.win)?;
        let x: f32 = (position.x - bounds.origin.x).into();
        let y: f32 = (position.y - bounds.origin.y).into();
        let cell: f32 = self.cell_width.into();
        let col = (x / cell.max(1.))
            .floor()
            .clamp(0., f32::from(grid.cols.saturating_sub(1))) as u16;
        let row = (y / self.appearance.cell_height())
            .floor()
            .clamp(0., f32::from(grid.rows.saturating_sub(1))) as u16;
        Some(CellPoint { row, col })
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct CellPoint {
    row: u16,
    col: u16,
}

/// A GPUI `Element` that paints the Neovim window grid from the shared model.
struct NeovimGrid {
    pane: Entity<NeovimPane>,
    win: Option<i64>,
}
impl IntoElement for NeovimGrid {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}
impl Element for NeovimGrid {
    type RequestLayoutState = ();
    type PrepaintState = Pixels;
    fn id(&self) -> Option<ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }
    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size = Size::full();
        (window.request_layout(style, [], cx), ())
    }
    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Pixels {
        let appearance = self.pane.read(cx).appearance.clone();
        let run = neovim_run(&appearance, "M", 0xdddddd, false, false, false);
        let measure = window.text_system().shape_line(
            "M".into(),
            ui_scaled_px(appearance.size),
            &[run],
            None,
        );
        let width = measure.x_for_index(1).max(ui_scaled_px(1.));
        if self.win.is_none() {
            self.pane.update(cx, |pane, cx| {
                pane.grid_bounds = Some(bounds);
                pane.cell_width = width;
                pane.send_resize(cx);
            });
        }
        width
    }
    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        width: &mut Pixels,
        window: &mut Window,
        cx: &mut App,
    ) {
        // Copy the frame out of the session first so no immutable session
        // borrow is held while painting with a mutable `cx`.
        let (frame, appearance) = self.pane.update(cx, |pane, cx| {
            (
                render_frame(pane.session.read(cx).model(), self.win.unwrap_or(pane.win)),
                pane.appearance.clone(),
            )
        });
        let Some(frame) = frame else { return };
        let focus = self.pane.read(cx).focus_handle(cx);
        let cell_height = appearance.cell_height();
        if self.win.is_none() {
            window.handle_input(
                &focus,
                ElementInputHandler::new(bounds, self.pane.clone()),
                cx,
            );
        }
        window.with_content_mask(Some(ContentMask::new(bounds)), |window| {
            // The dock paints the shared translucent pane surface once. Leave
            // the editor's normal background clear so its padding and grid
            // meet seamlessly. Floating windows still need their own backing.
            if self.win.is_some() {
                window.paint_quad(fill(bounds, rgb(frame.default_bg)));
            }
            for row in 0..frame.rows {
                for col in 0..frame.cols {
                    let cell = &frame.cells[row as usize * frame.cols as usize + col as usize];
                    if cell.text.is_empty() {
                        continue;
                    }
                    let origin = point(
                        bounds.origin.x + *width * col as f32,
                        bounds.origin.y + px(cell_height * row as f32),
                    );
                    let is_wide = text_width(&cell.text) == 2;
                    if cell.bg != frame.default_bg {
                        let w = *width * if is_wide { 2. } else { 1. };
                        window.paint_quad(fill(
                            Bounds {
                                origin,
                                size: size(w, px(cell_height)),
                            },
                            rgb(cell.bg),
                        ));
                    }
                    let run = neovim_run(
                        &appearance,
                        &cell.text,
                        cell.fg,
                        cell.bold,
                        cell.italic,
                        cell.underline,
                    );
                    let line = window.text_system().shape_line(
                        cell.text.clone().into(),
                        ui_scaled_px(appearance.size),
                        &[run],
                        None,
                    );
                    let _ = line.paint(origin, px(cell_height), TextAlign::Left, None, window, cx);
                }
            }
            if let Some((crow, ccol)) = frame.cursor {
                if crow >= 0 && ccol >= 0 && crow < frame.rows as i64 && ccol < frame.cols as i64 {
                    let origin = point(
                        bounds.origin.x + *width * ccol as f32,
                        bounds.origin.y + px(cell_height * crow as f32),
                    );
                    if focus.is_focused(window) {
                        window.paint_quad(fill(
                            Bounds {
                                origin,
                                size: size(*width, px(cell_height)),
                            },
                            rgba((0xdddddd << 8) | 0x55),
                        ));
                    }
                }
            }
        });
    }
}

fn text_width(text: &str) -> u8 {
    let mut width = 0u8;
    for ch in text.chars() {
        width = width.saturating_add(if is_wide_char(ch) { 2 } else { 1 });
    }
    width
}

/// Rough East Asian Width approximation for wide CJK/fullwidth glyphs.
fn is_wide_char(ch: char) -> bool {
    let c = ch as u32;
    (0x1100..=0x115F).contains(&c)
        || (0x2E80..=0xA4CF).contains(&c)
        || (0xAC00..=0xD7A3).contains(&c)
        || (0xF900..=0xFAFF).contains(&c)
        || (0xFE30..=0xFE4F).contains(&c)
        || (0xFF00..=0xFF60).contains(&c)
        || (0xFFE0..=0xFFE6).contains(&c)
        || (0x1F300..=0x1F64F).contains(&c)
        || (0x1F900..=0x1F9FF).contains(&c)
}

/// Map a GPUI keystroke to Neovim input notation.
fn nvim_key(keystroke: &Keystroke) -> Option<String> {
    let mods = keystroke.modifiers;
    let key = match keystroke.key.as_str() {
        "enter" => "CR",
        "escape" => "Esc",
        "backspace" => "BS",
        "delete" => "Del",
        "tab" => "Tab",
        "up" => "Up",
        "down" => "Down",
        "left" => "Left",
        "right" => "Right",
        "home" => "Home",
        "end" => "End",
        "pageup" => "PageUp",
        "pagedown" => "PageDown",
        "space" => "Space",
        k if k.chars().count() == 1 || k.starts_with('f') => k,
        _ => return None,
    };
    let mut prefixes = String::new();
    if mods.control {
        prefixes.push_str("C-");
    }
    if mods.alt {
        prefixes.push_str("M-");
    }
    if mods.shift && key.chars().count() > 1 {
        prefixes.push_str("S-");
    }
    Some(format!("<{prefixes}{key}>"))
}

fn neovim_run(
    appearance: &Appearance,
    text: &str,
    fg: u32,
    bold: bool,
    italic: bool,
    underline: bool,
) -> TextRun {
    let mut font = font(appearance.family.clone());
    font.weight = FontWeight(if bold {
        (appearance.weight + 300.).min(900.)
    } else {
        appearance.weight
    });
    if italic {
        font.style = FontStyle::Italic;
    }
    TextRun {
        len: text.len(),
        font,
        color: rgb(fg).into(),
        background_color: None,
        underline: underline.then(|| UnderlineStyle {
            color: Some(rgb(fg).into()),
            thickness: ui_scaled_px(1.),
            wavy: false,
        }),
        strikethrough: None,
    }
}

impl Focusable for NeovimPane {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for NeovimPane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mode_context = {
            let session = self.session.read(cx);
            let model = session.model();
            monitter_gpui_desktop::neovim_keys::context(&model.mode, model.cmdline_visible)
        };
        let palette = self.session.read(cx).palette.unwrap_or_else(|| {
            crate::design::palette(
                &self.session.read(cx).snapshot["settings"],
                window.appearance(),
            )
        });
        let snapshot = self.session.read(cx).snapshot.clone();
        self.files.update(cx, |files, cx| {
            files.set_theme(snapshot["settings"].clone(), palette, cx)
        });
        let interface_font_ratio = snapshot["settings"]["interfaceFontSize"]
            .as_f64()
            .filter(|size| size.is_finite())
            .unwrap_or(14.)
            .clamp(8., 32.) as f32
            / 14.;
        let is_host = self.focus.is_focused(window);

        let mut screen = div()
            .id("neovim-screen")
            .when(palette.dark, |screen| {
                screen.bg(rgb(0x000000).opacity(0.20))
            })
            .pt(px(20.))
            .pr(px(20.))
            .flex_1()
            .min_h(px(0.))
            .min_w_0()
            .w_full()
            .relative()
            .cursor(CursorStyle::IBeam)
            .on_mouse_down(MouseButton::Left, cx.listener(Self::mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::mouse_up))
            .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, _, cx| {
                let delta = match event.delta {
                    ScrollDelta::Pixels(p) => {
                        let y: f32 = p.y.into();
                        y / this.appearance.cell_height()
                    }
                    ScrollDelta::Lines(p) => p.y,
                };
                if let Some(cell) = this.cell_at(event.position, cx) {
                    for _ in 0..delta.abs().ceil().min(12.) as usize {
                        this.session.update(cx, |s, cx| {
                            s.mouse(
                                this.win,
                                cell,
                                "wheel",
                                if delta > 0. { "up" } else { "down" },
                                cx,
                            )
                        });
                    }
                }
                cx.stop_propagation();
            }))
            .child(NeovimGrid {
                pane: cx.entity(),
                win: None,
            });

        let session = self.session.read(cx);
        let model = session.model();
        if let Some(owner) = model.windows.get(&self.win) {
            let mut floats = model
                .windows
                .values()
                .filter(|w| {
                    w.floating
                        && !w.hidden
                        && (w.anchor_grid == owner.grid || (w.anchor_grid == 1 && is_host))
                })
                .collect::<Vec<_>>();
            floats.sort_by_key(|w| w.zindex);
            for float in floats {
                if let Some(grid) = model.grid(float.grid).filter(|g| !g.cells.is_empty()) {
                    let col = (float.start_col - owner.start_col).max(0) as f32;
                    let row = (float.start_row - owner.start_row).max(0) as f32;
                    screen = screen.child(
                        div()
                            .absolute()
                            .left(self.cell_width * col)
                            .top(px(self.appearance.cell_height() * row))
                            .w(self.cell_width * f32::from(grid.cols))
                            .h(px(self.appearance.cell_height() * f32::from(grid.rows)))
                            .child(NeovimGrid {
                                pane: cx.entity(),
                                win: Some(float.win),
                            }),
                    );
                }
            }
        }

        if is_host {
            // Messages (last few, oldest first).
            let messages = self.session.read(cx).model().messages.clone();
            if !messages.is_empty() {
                let text = messages
                    .iter()
                    .rev()
                    .take(3)
                    .map(|m| chunks_text(&m.chunks))
                    .filter(|t| !t.trim().is_empty())
                    .collect::<Vec<_>>()
                    .join("\n");
                if !text.trim().is_empty() {
                    screen = screen.child(
                        div()
                            .id("neovim-messages")
                            .absolute()
                            .left(px(10.))
                            .top(px(8.))
                            .max_w(px(560.))
                            .px(px(8.))
                            .py(px(5.))
                            .rounded(px(6.))
                            .bg(rgba((0x000000 << 8) | 0xB8))
                            .text_size(px(11. * interface_font_ratio))
                            .line_height(relative(1.25))
                            .text_color(rgb(0xeeeeee))
                            .child(text),
                    );
                }
            }

            // Popupmenu, when visible.
            let popup = self.session.read(cx).model().popupmenu.clone();
            if let Some(popup) = popup.filter(|p| p.visible) {
                let items = popup
                    .items
                    .iter()
                    .enumerate()
                    .map(|(i, item)| {
                        if i == popup.selected as usize {
                            div()
                                .flex()
                                .items_center()
                                .w_full()
                                .px(px(6.))
                                .bg(rgb(palette.accent))
                                .text_color(rgb(0x0a0a0a))
                                .child(item.word.clone())
                        } else {
                            div()
                                .flex()
                                .items_center()
                                .w_full()
                                .px(px(6.))
                                .child(item.word.clone())
                        }
                    })
                    .collect::<Vec<_>>();
                screen = screen.child(
                    div()
                        .id("neovim-popup")
                        .absolute()
                        .left(px(10.))
                        .top(px(10.))
                        .min_w(px(180.))
                        .bg(rgba((0x111111 << 8) | 0xD8))
                        .text_color(rgb(0xeeeeee))
                        .border_1()
                        .border_color(rgb(palette.line))
                        .rounded(px(6.))
                        .text_size(px(12. * interface_font_ratio))
                        .line_height(relative(1.35))
                        .children(items),
                );
            }
        }

        let error = self.session.read(cx).error.clone().map(|error| {
            if error.contains("E37:") || error.contains("E89:") || error.contains("E162:") {
                "Unsaved changes. Use :write to save before closing, or :q! to discard.".to_owned()
            } else {
                error
            }
        });
        let editor = div()
            .flex()
            .flex_col()
            .flex_1()
            .h_full()
            .min_w_0()
            .min_h_0()
            .children(error.map(|error| {
                div()
                    .px(px(8.))
                    .py(px(4.))
                    .text_color(rgb(0xff8888))
                    .child(error)
            }))
            .children(
                (!self.marked.is_empty()).then(|| div().px(px(8.)).child(self.marked.clone())),
            )
            .child(screen)
            .child({
                let session = self.session.read(cx);
                let command = (session.snapshot["currentWindow"].as_i64() == Some(self.win))
                    .then(|| {
                        session
                            .model
                            .cmdlines
                            .iter()
                            .max_by_key(|(level, _)| *level)
                            .map(|(_, command)| command_text(command))
                    })
                    .flatten();
                let mut footer = div()
                    .id("neovim-status-footer")
                    .flex_shrink_0()
                    .w_full()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .border_t_1()
                    .border_color(rgb(palette.line))
                    .px(px(10.))
                    .py(px(5.))
                    .bg(crate::design::surface_color(palette.panel))
                    .text_color(rgb(palette.muted))
                    .text_size((cx.theme().font_size - px(4.)).max(px(8.)));
                let command_active = command.is_some();
                if let Some(command) = command {
                    footer = footer.child(
                        div()
                            .min_w_0()
                            .overflow_hidden()
                            .border_1()
                            .border_color(rgb(palette.line).opacity(0.))
                            .px(px(8.))
                            .py(px(4.))
                            .text_color(rgb(palette.ink))
                            .child(command),
                    );
                } else {
                    let row = snapshot["windows"]
                        .as_array()
                        .and_then(|windows| {
                            windows
                                .iter()
                                .find(|row| row["id"].as_i64() == Some(self.win))
                        })
                        .unwrap_or(&Value::Null);
                    let cwd = row["cwd"]
                        .as_str()
                        .filter(|cwd| !cwd.is_empty())
                        .or_else(|| snapshot["cwd"].as_str())
                        .unwrap_or("");
                    let language = row["filetype"]
                        .as_str()
                        .filter(|language| !language.is_empty())
                        .unwrap_or("Plain text");
                    let project = project_name(&session.workspace, cwd);
                    for (index, label) in [
                        cwd.to_owned(),
                        language.to_owned(),
                        project,
                        mode_label(&session.model.mode).to_owned(),
                    ]
                    .into_iter()
                    .enumerate()
                    {
                        if !label.is_empty() {
                            footer = footer.child(
                                div()
                                    .rounded(px(4.))
                                    .border_1()
                                    .border_color(rgb(palette.line).opacity(0.))
                                    .px(px(8.))
                                    .py(px(4.))
                                    .min_w_0()
                                    .truncate()
                                    .when(index == 0, |item| item.flex_1())
                                    .when(index != 0, |item| item.flex_shrink_0())
                                    .when(index == 2, |item| item.max_w(px(160.)))
                                    .child(label),
                            );
                        }
                    }
                }
                footer.when(!command_active, |footer| {
                    footer.child(
                        Button::new("neovim-toggle-files")
                            .ghost()
                            .accessibility_label("Toggle Neovim file browser")
                            .child("Files")
                            .on_click(cx.listener(|pane, _, window, cx| {
                                if pane.files_visible {
                                    pane.files_visible = false;
                                    pane.focus.focus(window, cx);
                                    cx.notify();
                                } else {
                                    pane.show_files(window, cx);
                                }
                            })),
                    )
                })
            });
        div()
            .id("neovim-editor-pane")
            .track_focus(&self.focus)
            .key_context(mode_context)
            .on_key_down(cx.listener(Self::key))
            .size_full()
            .min_w_0()
            .min_h_0()
            .flex()
            .when(self.files_visible, |pane| {
                pane.child(
                    div()
                        .id("neovim-integrated-files")
                        .w(px(240.))
                        .max_w(relative(0.4))
                        .h_full()
                        .flex_shrink_0()
                        .min_w_0()
                        .overflow_hidden()
                        .border_r_1()
                        .border_color(rgb(palette.line))
                        .child(self.files.clone()),
                )
            })
            .child(editor)
    }
}

fn command_text(command: &monitter_gpui_desktop::neovim_model::Cmdline) -> String {
    let content = chunks_text(&command.content);
    let mut pos = command.pos.min(content.len());
    while !content.is_char_boundary(pos) {
        pos -= 1;
    }
    format!(
        "{}{}{}{}▏{}",
        command.firstc,
        command.prompt,
        " ".repeat(command.indent as usize),
        &content[..pos],
        &content[pos..]
    )
}

fn mode_label(mode: &str) -> &str {
    match mode {
        "normal" => "Normal",
        "insert" => "Insert",
        "replace" => "Replace",
        "visual" => "Visual",
        "visual_select" => "Visual select",
        "cmdline_normal" | "cmdline_insert" | "cmdline_replace" => "Command",
        "terminal" => "Terminal",
        "operator" => "Operator pending",
        "" => "Normal",
        other => other,
    }
}

fn project_name(workspace: &Value, cwd: &str) -> String {
    let path = std::path::Path::new(cwd);
    if !path.is_absolute() {
        return String::new();
    }
    workspace["projects"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|project| {
            project["workspaces"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(move |root| {
                    let host = root["hostId"].as_str()?;
                    let local = workspace["hosts"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .any(|row| row["id"].as_str() == Some(host) && row["kind"] == "local");
                    let root = std::path::Path::new(root["cwd"].as_str()?);
                    (local && root.is_absolute() && path.starts_with(root))
                        .then(|| (root.components().count(), field(project, "name")))
                })
        })
        .max_by_key(|(depth, _)| *depth)
        .map(|(_, name)| name)
        .unwrap_or_default()
}

fn chunks_text(chunks: &[CmdlineChunk]) -> String {
    chunks.iter().map(|chunk| chunk.text.as_str()).collect()
}

// -------------------------------------------------------------------------
// EntityInputHandler: committed text (IME / printable keys) is forwarded as
// nvim input. The remaining methods are minimal so the pane can host input.
// -------------------------------------------------------------------------
impl EntityInputHandler for NeovimPane {
    fn text_for_range(
        &mut self,
        _: Range<usize>,
        actual: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        *actual = Some(0..0);
        Some(String::new())
    }
    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: 0..0,
            reversed: false,
        })
    }
    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        (!self.marked.is_empty()).then(|| 0..self.marked.encode_utf16().count())
    }
    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.marked.clear();
        cx.notify();
    }
    fn replace_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.focus.is_focused(window) {
            return;
        }
        self.marked.clear();
        self.send_input(&text.replace('<', "<LT>"), cx);
        cx.notify();
    }
    fn replace_and_mark_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        text: &str,
        _: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.marked = text.to_owned();
        cx.notify();
    }
    fn bounds_for_range(
        &mut self,
        _: Range<usize>,
        bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        Some(bounds)
    }
    fn character_index_for_point(
        &mut self,
        _: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        Some(0)
    }
}

/// Read a string field with a default, matching the rest of the tree.
fn field(v: &Value, key: &str) -> String {
    v.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ::core::prelude::v1::test;

    #[test]
    fn idle_editor_polling_is_bounded_and_resets_after_activity() {
        assert_eq!(neovim_poll_delay(false, 0), Duration::from_secs(1));
        assert_eq!(neovim_poll_delay(true, 0), Duration::from_millis(20));
        assert_eq!(
            neovim_poll_delay(true, u32::MAX),
            Duration::from_millis(250)
        );
        assert!(!has_gui_panes(0));
        assert!(has_gui_panes(1));
    }

    #[cfg(feature = "visual-export")]
    #[gpui_kit::test]
    fn hidden_editor_input_wakes_immediately_and_visibility_is_aggregated(cx: &mut TestAppContext) {
        let session = cx.new(|cx| {
            let mut session = NeovimSession::new(
                DesktopService::disconnected_for_ui_fixture(),
                "wake".into(),
                cx,
            );
            session._poll = cx.spawn(async move |_, _| {});
            session
        });
        let first = cx.new(|_| ()).entity_id();
        let second = cx.new(|_| ()).entity_id();
        let (sender, mut receiver) = futures::channel::mpsc::channel(1);
        session.update(cx, |session, cx| {
            session.wake = sender;
            assert!(receiver.next().now_or_never().is_none());
            assert_eq!(
                neovim_poll_delay(!session.visible_panes.is_empty(), 0),
                Duration::from_secs(1)
            );
            session.input(1, "x", cx);
            // The waiting future becomes ready without advancing a timer.
            assert_eq!(receiver.next().now_or_never(), Some(Some(())));
            assert_eq!(
                session
                    .queue
                    .iter()
                    .map(|(name, _)| *name)
                    .collect::<Vec<_>>(),
                vec!["neovim_focus", "neovim_input"]
            );
            session.set_pane_visible(first, true);
            session.set_pane_visible(second, true);
            session.set_pane_visible(first, false);
            assert_eq!(session.visible_panes.len(), 1);
            session.set_pane_visible(second, false);
            assert!(session.visible_panes.is_empty());
        });
    }

    // Dispatch through the actual native render/focus tree, without a backend,
    // so moving the focus handle away from the key handler is observable.
    #[cfg(feature = "visual-export")]
    #[gpui_kit::test]
    fn integrated_sidebar_preserves_editor_key_routing(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let mut retained = None;
        let (_, visual) = cx.add_window_view(|window, cx| {
            let session = cx.new(|cx| {
                let mut session = NeovimSession::new(
                    DesktopService::disconnected_for_ui_fixture(),
                    "focus-fixture".into(),
                    cx,
                );
                // Cancel polling: inspect the real input queue locally.
                session._poll = cx.spawn(async move |_, _| {});
                session
            });
            let pane = cx.new(|cx| NeovimPane::new(session, 1001, window, cx));
            let focus = pane.read(cx).focus.clone();
            focus.focus(window, cx);
            retained = Some(pane.clone());
            gpui_kit::component::Root::new(pane, window, cx)
        });
        let pane = retained.unwrap();
        visual.run_until_parked();
        visual.update(|_, cx| {
            let session = pane.read(cx).session.clone();
            session.update(cx, |session, _| session.queue.clear())
        });
        visual.simulate_keystrokes("escape");
        visual.update(|_, cx| {
            assert!(
                pane.read(cx)
                    .session
                    .read(cx)
                    .queue
                    .iter()
                    .any(|(name, args)| *name == "neovim_input" && args["keys"] == "<Esc>")
            );
            let session = pane.read(cx).session.clone();
            session.update(cx, |session, _| session.queue.clear());
        });
        visual.update(|window, cx| pane.update(cx, |pane, cx| pane.show_files(window, cx)));
        visual.run_until_parked();
        visual.update(|window, cx| {
            assert!(pane.read(cx).focus.contains_focused(window, cx));
            assert!(!pane.read(cx).focus.is_focused(window));
        });
        visual.simulate_keystrokes("escape");
        visual.update(|_, cx| {
            assert!(
                !pane
                    .read(cx)
                    .session
                    .read(cx)
                    .queue
                    .iter()
                    .any(|(name, _)| *name == "neovim_input")
            )
        });
        visual.update(|_, cx| {
            let files = pane.read(cx).files.clone();
            files.update(cx, |_, cx| {
                cx.emit(crate::neovim_files::NeovimFilesEvent::OpenFile(
                    "/fixture/file.rs".into(),
                ));
            })
        });
        visual.update(|window, cx| {
            assert!(pane.read(cx).focus.is_focused(window));
            assert!(
                pane.read(cx)
                    .session
                    .read(cx)
                    .queue
                    .iter()
                    .any(|(name, args)| *name == "neovim_open_file"
                        && args["windowId"] == 1001
                        && args["path"] == "/fixture/file.rs")
            );
        });
    }

    #[cfg(feature = "visual-export")]
    #[gpui_kit::test]
    fn missing_session_reconciles_but_transient_errors_preserve_editor(cx: &mut TestAppContext) {
        let session = cx.new(|cx| {
            let mut session = NeovimSession::new(
                DesktopService::disconnected_for_ui_fixture(), "gone".into(), cx);
            session._poll = cx.spawn(async move |_, _| {});
            session.snapshot = json!({"layout": ["leaf", 1001], "windows": [{"id":1001}], "exited":false});
            session
        });
        let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let capture = events.clone();
        let _subscription = cx.update(|cx| {
            cx.subscribe(&session, move |_, event: &NeovimSessionEvent, _| {
                capture.borrow_mut().push(*event)
            })
        });
        session.update(cx, |session, cx| {
            session.consume_read_error("Backend disconnected".into(), cx);
            assert_eq!(session.snapshot["exited"], false);
            assert_eq!(session.snapshot["layout"], json!(["leaf",1001]));
            session.pending_focus = Some(1001);
            session.enqueue("neovim_input", json!({"keys":"x"}), cx);
            session.consume_read_error("Neovim session no longer exists.".into(), cx);
            assert_eq!(session.snapshot["exited"], true);
            assert!(session.snapshot["windows"].as_array().unwrap().is_empty());
            assert!(session.queue.is_empty());
            assert!(session.pending_focus.is_none());
        });
        cx.run_until_parked();
        assert_eq!(*events.borrow(), vec![NeovimSessionEvent::Updated]);
    }

    #[test]
    fn command_caret_uses_vim_byte_position_and_prompt() {
        let command = monitter_gpui_desktop::neovim_model::Cmdline {
            firstc: "/".into(),
            prompt: "find ".into(),
            indent: 2,
            content: vec![CmdlineChunk {
                text: "éclair".into(),
                hl_id: 0,
            }],
            pos: 2,
        };
        assert_eq!(command_text(&command), "/find   é▏clair");
        let command = monitter_gpui_desktop::neovim_model::Cmdline { pos: 1, ..command };
        assert_eq!(command_text(&command), "/find   ▏éclair");
    }

    #[test]
    fn project_uses_deepest_local_ancestor_with_path_boundaries() {
        let workspace = json!({"hosts": [{"id":"local","kind":"local"}, {"id":"ssh","kind":"ssh"}],
        "projects": [
            {"name":"Outer", "workspaces":[{"hostId":"local", "cwd":"/code/app"}]},
            {"name":"Inner", "workspaces":[{"hostId":"local", "cwd":"/code/app/nested"}]},
            {"name":"Remote", "workspaces":[{"hostId":"ssh", "cwd":"/code/app/nested/src"}]}
        ]});
        assert_eq!(project_name(&workspace, "/code/app/nested/src"), "Inner");
        assert_eq!(project_name(&workspace, "/code/app/src"), "Outer");
        assert_eq!(project_name(&workspace, "/code/application"), "");
        assert_eq!(project_name(&workspace, "app/src"), "");
    }

    #[test]
    fn wide_and_narrow_characters() {
        assert_eq!(text_width("a"), 1);
        assert_eq!(text_width("中"), 2);
        assert_eq!(text_width("ab"), 2);
        assert!(is_wide_char('中'));
        assert!(!is_wide_char('a'));
    }

    #[test]
    fn colors_mask_alpha() {
        assert_eq!(to_rgb(0xFF_00AABB), 0x00AABB);
    }

    #[test]
    fn missing_session_is_distinct_from_transport_and_editor_errors() {
        assert!(session_is_missing("Neovim session no longer exists."));
        for error in ["Backend disconnected", "Neovim stream closed", "No write since last change", "request timed out"] {
            assert!(!session_is_missing(error));
        }
    }

    #[test]
    fn empty_layout_is_quiet() {
        let layout = empty_layout();
        assert_eq!(layout["exited"], false);
        assert!(layout["layout"].is_null());
    }
}
