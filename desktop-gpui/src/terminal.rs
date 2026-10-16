//! Retained native VT terminal. Hiding this entity never closes a PTY.
//! One FIFO serializes reads, resize requests, input and explicit closes.
use crate::design::ui_scaled_px;
use futures::{FutureExt as _, StreamExt as _};
use gpui_kit::{
    component::{Disableable, Sizable, button::*},
    prelude::*,
    *,
};
use monitter_gpui_desktop::terminal_model::TerminalModel;
use monitter_lib::experimental_desktop::DesktopService;
use serde_json::{Value, json};
use std::{
    collections::{HashMap, VecDeque},
    ops::Range,
    time::Duration,
};

fn terminal_context_snapshot(snapshot: &Value) -> Value {
    json!({"settings": snapshot["settings"], "hosts": snapshot["hosts"]})
}
fn terminal_context_changed(previous: &Value, snapshot: &Value) -> bool {
    previous["settings"] != snapshot["settings"] || previous["hosts"] != snapshot["hosts"]
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
#[derive(Clone, Copy)]
struct Palette {
    name: &'static str,
    background: u32,
    foreground: u32,
    cursor: u32,
    ansi: [u32; 16],
}
// Shared product color data, painted natively; no DOM or frontend runtime.
const PALETTES: &[Palette] = &[
    Palette {
        name: "Monitter",
        background: 0x090b0d,
        foreground: 0xe5e7eb,
        cursor: 0x77b58b,
        ansi: [
            0x090b0d, 0xe05f65, 0x77b58b, 0xd8b15d, 0x5c9ded, 0xb980d9, 0x5cbec9, 0xd8dadd,
            0x596168, 0xff777d, 0x8dcc9f, 0xedc873, 0x79b7ff, 0xd39bed, 0x77d7df, 0xffffff,
        ],
    },
    Palette {
        name: "Catppuccin Mocha",
        background: 0x1E1E2E,
        foreground: 0xCDD6F4,
        cursor: 0xF5E0DC,
        ansi: [
            0x45475A, 0xF38BA8, 0xA6E3A1, 0xF9E2AF, 0x89B4FA, 0xF5C2E7, 0x94E2D5, 0xA6ADC8,
            0x585B70, 0xF37799, 0x89D88B, 0xEBD391, 0x74A8FC, 0xF2AEDE, 0x6BD7CA, 0xBAC2DE,
        ],
    },
    Palette {
        name: "Dracula",
        background: 0x282A36,
        foreground: 0xF8F8F2,
        cursor: 0xF8F8F2,
        ansi: [
            0x21222C, 0xFF5555, 0x50FA7B, 0xF1FA8C, 0xBD93F9, 0xFF79C6, 0x8BE9FD, 0xF8F8F2,
            0x6272A4, 0xFF6E6E, 0x69FF94, 0xFFFFA5, 0xD6ACFF, 0xFF92DF, 0xA4FFFF, 0xFFFFFF,
        ],
    },
    Palette {
        name: "Gruvbox Dark",
        background: 0x282828,
        foreground: 0xEBDBB2,
        cursor: 0xEBDBB2,
        ansi: [
            0x282828, 0xCC241D, 0x98971A, 0xD79921, 0x458588, 0xB16286, 0x689D6A, 0xA89984,
            0x928374, 0xFB4934, 0xB8BB26, 0xFABD2F, 0x83A598, 0xD3869B, 0x8EC07C, 0xEBDBB2,
        ],
    },
    Palette {
        name: "Molokai",
        background: 0x121212,
        foreground: 0xBBBBBB,
        cursor: 0xBBBBBB,
        ansi: [
            0x121212, 0xFA2573, 0x98E123, 0xDFD460, 0x1080D0, 0x8700FF, 0x43A8D0, 0xBBBBBB,
            0x555555, 0xF6669D, 0xB1E05F, 0xFFF26D, 0x00AFFF, 0xAF87FF, 0x51CEFF, 0xFFFFFF,
        ],
    },
    Palette {
        name: "Nord",
        background: 0x2E3440,
        foreground: 0xD8DEE9,
        cursor: 0xECEFF4,
        ansi: [
            0x3B4252, 0xBF616A, 0xA3BE8C, 0xEBCB8B, 0x81A1C1, 0xB48EAD, 0x88C0D0, 0xE5E9F0,
            0x4C566A, 0xBF616A, 0xA3BE8C, 0xEBCB8B, 0x81A1C1, 0xB48EAD, 0x8FBCBB, 0xECEFF4,
        ],
    },
    Palette {
        name: "One Dark",
        background: 0x21252B,
        foreground: 0xABB2BF,
        cursor: 0xABB2BF,
        ansi: [
            0x21252B, 0xE06C75, 0x98C379, 0xE5C07B, 0x61AFEF, 0xC678DD, 0x56B6C2, 0xABB2BF,
            0x767676, 0xE06C75, 0x98C379, 0xE5C07B, 0x61AFEF, 0xC678DD, 0x56B6C2, 0xABB2BF,
        ],
    },
    Palette {
        name: "Solarized Dark",
        background: 0x001E27,
        foreground: 0x708284,
        cursor: 0x708284,
        ansi: [
            0x002831, 0xD11C24, 0x738A05, 0xA57706, 0x2176C7, 0xC61C6F, 0x259286, 0xEAE3CB,
            0x001E27, 0xBD3613, 0x475B62, 0x536870, 0x708284, 0x5956BA, 0x819090, 0xFCF4DC,
        ],
    },
    Palette {
        name: "Wombat",
        background: 0x171717,
        foreground: 0xDEDACF,
        cursor: 0xBBBBBB,
        ansi: [
            0x000000, 0xFF615A, 0xB1E969, 0xEBD99C, 0x5DA9F6, 0xE86AFF, 0x82FFF7, 0xDEDACF,
            0x313131, 0xF58C80, 0xDDF88F, 0xEEE5B2, 0xA5C7FF, 0xDDAAFF, 0xB7FFF9, 0xFFFFFF,
        ],
    },
    Palette {
        name: "XTerm",
        background: 0x000000,
        foreground: 0xFFFFFF,
        cursor: 0xFFFFFF,
        ansi: [
            0x000000, 0xCD0000, 0x00CD00, 0xCDCD00, 0x0000EE, 0xCD00CD, 0x00CDCD, 0xE5E5E5,
            0x7F7F7F, 0xFF0000, 0x00FF00, 0xFFFF00, 0x5C5CFF, 0xFF00FF, 0x00FFFF, 0xFFFFFF,
        ],
    },
];
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct CellPoint {
    row: u16,
    col: u16,
}
struct Selection {
    anchor: CellPoint,
    end: CellPoint,
    text: String,
}
impl Selection {
    fn ordered(&self) -> (CellPoint, CellPoint) {
        if self.anchor <= self.end {
            (self.anchor, self.end)
        } else {
            (self.end, self.anchor)
        }
    }
    fn contains(&self, row: u16, col: u16) -> bool {
        let (start, end) = self.ordered();
        CellPoint { row, col } >= start && CellPoint { row, col } < end
    }
}
#[derive(Clone, Copy)]
enum MouseKind {
    Press,
    Release,
    Motion,
    WheelUp,
    WheelDown,
}
// Xterm DEC mouse encodings: https://invisible-island.net/xterm/ctlseqs/ctlseqs.html
fn mouse_bytes(
    mode: vt100::MouseProtocolMode,
    encoding: vt100::MouseProtocolEncoding,
    kind: MouseKind,
    button: Option<u8>,
    cell: CellPoint,
    modifiers: Modifiers,
) -> Option<Vec<u8>> {
    use vt100::{MouseProtocolEncoding as Encoding, MouseProtocolMode as Mode};
    if mode == Mode::None || modifiers.shift {
        return None;
    }
    let mut code = match kind {
        MouseKind::Press => button?,
        MouseKind::Release if mode == Mode::Press => return None,
        MouseKind::Release => {
            if encoding == Encoding::Sgr {
                button?
            } else {
                3
            }
        }
        MouseKind::Motion
            if mode == Mode::AnyMotion || (mode == Mode::ButtonMotion && button.is_some()) =>
        {
            32 + button.unwrap_or(3)
        }
        MouseKind::Motion => return None,
        MouseKind::WheelUp => 64,
        MouseKind::WheelDown => 65,
    };
    if modifiers.alt {
        code += 8;
    }
    if modifiers.control {
        code += 16;
    }
    let x = u32::from(cell.col) + 1;
    let y = u32::from(cell.row) + 1;
    match encoding {
        Encoding::Sgr => Some(
            format!(
                "\x1b[<{code};{x};{y}{}",
                if matches!(kind, MouseKind::Release) {
                    'm'
                } else {
                    'M'
                }
            )
            .into_bytes(),
        ),
        Encoding::Default if x <= 223 && y <= 223 => Some(vec![
            27,
            b'[',
            b'M',
            code + 32,
            (x + 32) as u8,
            (y + 32) as u8,
        ]),
        Encoding::Utf8 if x <= 2015 && y <= 2015 => {
            let mut bytes = vec![27, b'[', b'M'];
            for value in [u32::from(code) + 32, x + 32, y + 32] {
                let mut buf = [0; 4];
                bytes.extend_from_slice(char::from_u32(value)?.encode_utf8(&mut buf).as_bytes());
            }
            Some(bytes)
        }
        _ => None,
    }
}

fn field(v: &Value, k: &str) -> String {
    v.get(k)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}
fn merge_adopted_sessions(
    mut sessions: Vec<Value>,
    adopted: &mut HashMap<String, Value>,
) -> Vec<Value> {
    for (id, session) in adopted.drain() {
        if !sessions.iter().any(|listed| field(listed, "id") == id) {
            sessions.push(session);
        }
    }
    sessions
}
fn terminal_toolbar_button(
    id: impl Into<SharedString>,
    label: impl Into<SharedString>,
    accessibility_label: impl Into<SharedString>,
    tooltip: impl Into<SharedString>,
    selected: bool,
    palette: crate::design::Palette,
    interface_font_ratio: f32,
    cx: &mut Context<TerminalPane>,
) -> Button {
    let transparent = rgba(0x00000000);
    Button::new(id.into())
        .with_size(gpui_kit::component::Size::Small)
        .compact()
        .h(px(25.))
        .min_h(px(25.))
        .min_w(px(0.))
        .px(px(7.))
        .py(px(2.))
        .rounded(px(5.))
        .border_1()
        .border_color(if selected {
            rgb(palette.line)
        } else {
            transparent
        })
        .font_family(crate::design::FONT_MONO)
        .text_size(px(10. * interface_font_ratio))
        .custom(
            ButtonCustomVariant::new(cx)
                .color(if selected {
                    rgb(palette.soft).into()
                } else {
                    transparent.into()
                })
                .foreground(rgb(if selected { palette.ink } else { palette.muted }).into())
                .hover(crate::design::surface_color(palette.soft).into())
                .active(crate::design::surface_color(palette.soft).into()),
        )
        .accessibility_label(accessibility_label)
        .tooltip(tooltip)
        .child(
            div()
                .max_w(px(160.))
                .truncate()
                .font_family(crate::design::FONT_MONO)
                .text_size(px(10. * interface_font_ratio))
                .line_height(relative(1.2))
                .child(label.into()),
        )
}
fn rows(v: &Value, k: &str) -> Vec<Value> {
    v.get(k)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}
#[derive(Clone)]
enum Operation {
    List,
    Open { target: Value, cols: u16, rows: u16 },
    Read { id: String, after: u64 },
    Write { id: String, data: Vec<u8> },
    Resize { id: String, cols: u16, rows: u16 },
    Close { id: String },
}
impl Operation {
    fn invoke(&self, service: &DesktopService) -> Result<Value, String> {
        let (command, args) = match self {
            Self::List => ("list_terminals", json!({})),
            Self::Open { target, cols, rows } => (
                "open_terminal",
                json!({"target":target,"cols":cols,"rows":rows}),
            ),
            Self::Read { id, after } => ("read_terminal", json!({"id":id,"afterSeq":after})),
            Self::Write { id, data } => {
                service.write_terminal_input(id, data)?;
                return Ok(Value::Null);
            }
            Self::Resize { id, cols, rows } => {
                ("resize_terminal", json!({"id":id,"cols":cols,"rows":rows}))
            }
            Self::Close { id } => ("close_terminal", json!({"id":id})),
        };
        service.invoke(command, args)
    }
    fn session(&self) -> Option<&str> {
        match self {
            Self::Read { id, .. }
            | Self::Write { id, .. }
            | Self::Resize { id, .. }
            | Self::Close { id } => Some(id),
            _ => None,
        }
    }
}

// Pure FIFO policy: switching the visible session never retargets queued input.
// An explicit close may bypass a paused queue, but must not retry another
// session's failed write without the owner's explicit Retry action.
fn next_operation(
    queue: &mut VecDeque<Operation>,
    busy: bool,
    paused: bool,
    close_bypass: bool,
) -> Option<Operation> {
    if busy
        || (paused && (!close_bypass || !matches!(queue.front(), Some(Operation::Close { .. }))))
    {
        return None;
    }
    queue.pop_front()
}
fn retain_failed_operation(queue: &mut VecDeque<Operation>, operation: Operation) -> bool {
    let closing = operation.session().is_some_and(|id| {
        queue
            .iter()
            .any(|queued| matches!(queued, Operation::Close { id: closing } if closing == id))
    });
    if closing {
        false
    } else {
        queue.push_front(operation);
        true
    }
}
fn queue_explicit_close(queue: &mut VecDeque<Operation>, id: String, paused: bool) -> bool {
    let discards_failed = paused && queue.front().and_then(Operation::session) == Some(id.as_str());
    queue.retain(|operation| operation.session() != Some(id.as_str()));
    queue.push_front(Operation::Close { id });
    paused && !discards_failed
}

/// Positive backend exit evidence for one retained pinned terminal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum TerminalEvent { Exited { session_id: String }, SessionClosed { session_id: String } }
impl EventEmitter<TerminalEvent> for TerminalPane {}

pub(super) fn remove_exited_terminal_pane(dock: &Entity<crate::docking::DockWorkspace>, session_id: &str, window: &mut Window, cx: &mut App) -> bool {
    let Ok(key)=crate::terminal_workspace::terminal_pane_id(session_id) else {return false;};
    dock.update(cx,|dock,cx|dock.remove(&key,window,cx))
}

fn pinned_exit_id(mode: &PaneMode, sessions: &[Value]) -> Option<String> {
    let PaneMode::Pinned { session_id } = mode else { return None; };
    sessions.iter().find(|session| field(session,"id") == *session_id && session["status"].as_str() == Some("exited")).map(|_|session_id.clone())
}

fn take_exit_notification(mode: &PaneMode, id: &str, reported: &mut bool, queue: &mut VecDeque<Operation>) -> Option<TerminalEvent> {
    if *reported || !matches!(mode, PaneMode::Pinned { session_id } if session_id==id) {return None;}
    *reported=true;
    queue.clear();
    Some(TerminalEvent::Exited {session_id:id.to_owned()})
}

// One lifecycle query per backend, regardless of how many pinned tabs exist.
// Registry entries are weak; pane release cancels the last monitor task.
#[derive(Default)]
struct ExitMonitors(HashMap<String, WeakEntity<TerminalExitMonitor>>);
impl Global for ExitMonitors {}
struct TerminalSessions(Result<Vec<Value>, String>);
struct TerminalExitMonitor {
    watchers: std::collections::HashSet<EntityId>,
    wake: futures::channel::mpsc::Sender<()>,
    _task: Task<()>,
}
impl EventEmitter<TerminalSessions> for TerminalExitMonitor {}
impl TerminalExitMonitor {
    fn shared(service: DesktopService, cx: &mut App) -> Entity<Self> {
        if cx.try_global::<ExitMonitors>().is_none() {
            cx.set_global(ExitMonitors::default());
        }
        let key = service.server_instance_id().to_owned();
        if let Some(monitor) = cx
            .global::<ExitMonitors>()
            .0
            .get(&key)
            .and_then(WeakEntity::upgrade)
        {
            return monitor;
        }
        let entity = cx.new(|cx: &mut Context<Self>| {
            let (wake, mut wakes) = futures::channel::mpsc::channel(1);
            let task = cx.spawn(async move |monitor, cx| {
                loop {
                    let Ok(active) = monitor.update(cx, |monitor, _| !monitor.watchers.is_empty())
                    else {
                        break;
                    };
                    if !active {
                        if wakes.next().await.is_none() {
                            break;
                        }
                        continue;
                    }
                    cx.background_executor().timer(Duration::from_secs(1)).await;
                    let Ok(active) = monitor.update(cx, |monitor, _| !monitor.watchers.is_empty())
                    else {
                        break;
                    };
                    if !active {
                        continue;
                    }
                    let service = service.clone();
                    let result = cx
                        .background_executor()
                        .spawn(async move { service.invoke("list_terminals", json!({})) })
                        .await;
                    let sessions = result.and_then(|value| {
                        value
                            .as_array()
                            .cloned()
                            .ok_or_else(|| "Terminal list response is invalid.".to_owned())
                    });
                    if monitor
                        .update(cx, |_, cx| cx.emit(TerminalSessions(sessions)))
                        .is_err()
                    {
                        break;
                    }
                }
            });
            Self {
                watchers: Default::default(),
                wake,
                _task: task,
            }
        });
        let registry = &mut cx.global_mut::<ExitMonitors>().0;
        registry.retain(|_, monitor| monitor.upgrade().is_some());
        registry.insert(key, entity.downgrade());
        entity
    }
    fn watch(&mut self, pane: EntityId, active: bool) {
        if active {
            self.watchers.insert(pane);
            let _ = self.wake.try_send(());
        } else {
            self.watchers.remove(&pane);
        }
    }
}

pub struct TerminalPane {
    service: DesktopService,
    snapshot: Value,
    mode: PaneMode,
    visible: bool,
    initial_read_pending: bool,
    task_id: Option<String>,
    sessions: Vec<Value>,
    list_in_flight: bool,
    adopted_during_list: HashMap<String, Value>,
    models: HashMap<String, TerminalModel>,
    selected: Option<String>,
    focus: FocusHandle,
    focus_requested: bool,
    queue: VecDeque<Operation>,
    busy: bool,
    paused: bool,
    close_bypass: bool,
    error: Option<String>,
    cols: u16,
    rows: u16,
    cell_width: Pixels,
    grid_bounds: Option<Bounds<Pixels>>,
    appearance: Appearance,
    palette: usize,
    selection: Option<Selection>,
    selecting: bool,
    protocol_button: Option<MouseButton>,
    last_mouse_cell: Option<CellPoint>,
    wheel_remainder: f32,
    marked: String,
    marked_selection: Range<usize>,
    _poll: Task<()>,
    exit_monitor: Option<Entity<TerminalExitMonitor>>,
    exit_monitor_error: Option<String>,
    poll_wake: futures::channel::mpsc::Sender<()>,
    exit_reported: bool,
}
#[derive(Clone, PartialEq, Eq)]
enum PaneMode {
    Manager,
    Pinned { session_id: String },
}
pub(crate) struct PinnedSession {
    session_id: String,
    session: Value,
}
impl TryFrom<Value> for PinnedSession {
    type Error = String;

    fn try_from(session: Value) -> Result<Self, Self::Error> {
        let session_id = session
            .get("id")
            .and_then(Value::as_str)
            .ok_or("Pinned terminal session has no session ID.")?
            .to_owned();
        crate::terminal_workspace::terminal_pane_id(&session_id)?;
        match session.get("status").and_then(Value::as_str) {
            Some("running" | "exited") => Ok(Self {
                session_id,
                session,
            }),
            _ => Err("Pinned terminal session has an unsupported status.".into()),
        }
    }
}
impl PaneMode {
    fn allows(&self, operation: &Operation) -> bool {
        match self {
            Self::Manager => true,
            Self::Pinned { session_id } => match operation {
                Operation::Read { id, .. }
                | Operation::Write { id, .. }
                | Operation::Resize { id, .. } => id == session_id,
                // A workspace tab represents visibility, not PTY ownership.
                // Session closure remains an explicit manager-pane action.
                Operation::List | Operation::Open { .. } | Operation::Close { .. } => false,
            },
        }
    }
}
fn operation_allowed(mode: &PaneMode, operation: &Operation) -> bool {
    mode.allows(operation)
}
fn remove_queued_reads(queue: &mut VecDeque<Operation>, session_id: Option<&str>) {
    queue.retain(|operation| match operation {
        Operation::Read { id, .. } => session_id.is_some_and(|pinned| id != pinned),
        _ => true,
    });
}
fn should_poll(visible: bool, paused: bool, busy: bool) -> bool {
    visible && !paused && !busy
}
fn terminal_read_due(initial_read_pending: bool, status: &str) -> bool {
    initial_read_pending || status == "running"
}
fn schedule_initial_read(
    queue: &mut VecDeque<Operation>,
    mode: &PaneMode,
    pending: bool,
    visible: bool,
    paused: bool,
    busy: bool,
    after: u64,
) -> bool {
    if !pending || !should_poll(visible, paused, busy) {
        return false;
    }
    let PaneMode::Pinned { session_id } = mode else {
        return false;
    };
    if queue
        .iter()
        .any(|operation| matches!(operation, Operation::Read { id, .. } if id == session_id))
    {
        return false;
    }
    queue.push_back(Operation::Read {
        id: session_id.clone(),
        after,
    });
    true
}
fn initial_operations(mode: &PaneMode) -> VecDeque<Operation> {
    match mode {
        PaneMode::Manager => VecDeque::from([Operation::List]),
        PaneMode::Pinned { .. } => VecDeque::new(),
    }
}
impl TerminalPane {
    pub(super) fn palette_sessions(&self) -> &[Value] { &self.sessions }
    /// Search the retained terminal screen and scrollback without moving its viewport.
    pub(super) fn palette_search_text(&self, session_id: &str) -> String {
        const LIMIT: usize = 512 * 1024;
        let Some(model) = self.models.get(session_id) else { return String::new(); };
        let mut screen = model.parser.screen().clone();
        screen.set_scrollback(usize::MAX);
        let oldest = screen.scrollback();
        let mut text = String::new();
        // Start with the live screen so a large history retains recent output.
        for offset in (0..=oldest).step_by(usize::from(screen.size().0).max(1)) {
            screen.set_scrollback(offset);
            for ch in screen.contents().chars().chain(std::iter::once('\n')) {
                if text.len() + ch.len_utf8() > LIMIT { return text; }
                text.push(ch);
            }
        }
        // Include the oldest partial page when history is not a screen multiple.
        if oldest % usize::from(screen.size().0).max(1) != 0 {
            screen.set_scrollback(oldest);
            for ch in screen.contents().chars() {
                if text.len() + ch.len_utf8() > LIMIT { break; }
                text.push(ch);
            }
        }
        text
    }

    pub(crate) fn foreground_surface(&self) -> Rgba {
        rgb(PALETTES[self.palette].foreground)
    }

    pub fn new(
        service: DesktopService,
        snapshot: &Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut pane = Self::new_inner(
            service,
            snapshot,
            window,
            PaneMode::Manager,
            true,
            false,
            cx,
        );
        pane.pump(cx);
        pane
    }
    pub(crate) fn new_pinned(
        service: DesktopService,
        snapshot: &Value,
        session: PinnedSession,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let session_id = session.session_id.clone();
        let mut pane = Self::new_inner(
            service,
            snapshot,
            window,
            PaneMode::Pinned {
                session_id: session_id.clone(),
            },
            false,
            true,
            cx,
        );
        let cols = pane.cols;
        let rows = pane.rows;
        pane.install_session_projection(session.session, session_id, cols, rows, false);
        pane
    }
    fn new_inner(
        service: DesktopService,
        snapshot: &Value,
        _window: &mut Window,
        mode: PaneMode,
        visible: bool,
        initial_read_pending: bool,
        cx: &mut Context<Self>,
    ) -> Self {
        let (poll_wake, mut wakes) = futures::channel::mpsc::channel(1);
        let poll = cx.spawn(async move |pane, cx| {
            loop {
                let Ok(active) = pane.update(cx, |pane, _| pane.visible && !pane.paused && !pane.exit_reported) else { break; };
                if active {
                    let timer = cx.background_executor().timer(Duration::from_millis(60)).fuse();
                    let wake = wakes.next().fuse();
                    futures::pin_mut!(timer, wake);
                    if let futures::future::Either::Left((signal, _)) = futures::future::select(wake, timer).await {
                        if signal.is_none() { break; }
                    }
                } else if wakes.next().await.is_none() { break; }
                if pane
                    .update(cx, |pane, cx| {
                        if should_poll(pane.visible, pane.paused, pane.busy) {
                            if let Some(id) = pane.selected.clone() {
                                let status = pane
                                    .models
                                    .get(&id)
                                    .map(|model| model.status.clone())
                                    .unwrap_or_default();
                                let after = pane
                                    .models
                                    .get(&id)
                                    .map(|model| model.seq)
                                    .unwrap_or_default();
                                if terminal_read_due(pane.initial_read_pending, &status) {
                                    if pane.initial_read_pending {
                                    schedule_initial_read(
                                        &mut pane.queue,
                                        &pane.mode,
                                        pane.initial_read_pending,
                                        pane.visible,
                                        pane.paused,
                                        pane.busy,
                                        after,
                                    );
                                    } else if !pane.queue.iter().any(
                                        |op| matches!(op,Operation::Read{id:queued,..} if queued==&id),
                                    )
                                {
                                    pane.queue.push_back(Operation::Read { id, after });
                                }
                            }
                        }
                        }
                        if !pane.paused && !pane.busy {
                            pane.pump(cx);
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        let exit_monitor = if matches!(mode, PaneMode::Pinned { .. }) {
            let monitor = TerminalExitMonitor::shared(service.clone(), cx);
            let pane_id = cx.entity_id();
            monitor.update(cx, |monitor, _| monitor.watch(pane_id, true));
            cx.subscribe(
                &monitor,
                |pane, _, sessions: &TerminalSessions, cx| match &sessions.0 {
                    Ok(sessions) => {
                        if pane.exit_monitor_error.take().is_some() {
                            cx.notify();
                        }
                        if let Some(id) = pinned_exit_id(&pane.mode, sessions) {
                            pane.report_exit(&id, cx);
                        }
                    }
                    Err(error) => {
                        if !pane.exit_reported && pane.exit_monitor_error.as_ref() != Some(error) {
                            pane.exit_monitor_error = Some(error.clone());
                            cx.notify();
                        }
                    }
                },
            )
            .detach();
            cx.on_release(move |pane, cx| {
                if let Some(monitor) = &pane.exit_monitor {
                    monitor.update(cx, |monitor, _| monitor.watch(pane_id, false));
                }
            })
            .detach();
            Some(monitor)
        } else {
            None
        };
        let queue = initial_operations(&mode);
        cx.observe_global::<crate::terminal_theme::Selection>(|pane, cx| {
            let index = cx.global::<crate::terminal_theme::Selection>().0;
            if pane.palette != index { pane.palette = index; cx.notify(); }
        }).detach();
        let pane = Self {
            service,
            snapshot: terminal_context_snapshot(snapshot),
            mode,
            visible,
            initial_read_pending,
            task_id: None,
            sessions: vec![],
            list_in_flight: false,
            adopted_during_list: HashMap::new(),
            models: HashMap::new(),
            selected: None,
            focus: cx.focus_handle(),
            focus_requested: false,
            queue,
            busy: false,
            paused: false,
            close_bypass: false,
            error: None,
            cols: 80,
            rows: 24,
            cell_width: ui_scaled_px(8.4),
            grid_bounds: None,
            appearance: Appearance::from_snapshot(snapshot),
            palette: crate::terminal_theme::current().palette_index,
            selection: None,
            selecting: false,
            protocol_button: None,
            last_mouse_cell: None,
            wheel_remainder: 0.,
            marked: String::new(),
            marked_selection: 0..0,
            _poll: poll,
            exit_monitor,
            exit_monitor_error: None,
            poll_wake,
            exit_reported: false,
        };
        pane
    }
    fn report_exit(&mut self, id: &str, cx: &mut Context<Self>) {
        // Discard work for the exited process, without queuing a PTY Close.
        if let Some(event) =
            take_exit_notification(&self.mode, id, &mut self.exit_reported, &mut self.queue)
        {
            if let Some(monitor) = &self.exit_monitor {
                let pane_id = cx.entity_id();
                monitor.update(cx, |monitor, _| monitor.watch(pane_id, false));
            }
            let _ = self.poll_wake.try_send(());
            cx.emit(event);
        }
    }
    fn report_known_exit(&mut self, cx: &mut Context<Self>) {
        let exited=pinned_exit_id(&self.mode,&self.sessions).or_else(|| {
            let PaneMode::Pinned {session_id}=&self.mode else {return None;};
            self.models.get(session_id).filter(|model|model.status=="exited").map(|_|session_id.clone())
        });
        if let Some(id)=exited {self.report_exit(&id,cx);}
    }
    fn operation_allowed(&self, operation: &Operation) -> bool {
        operation_allowed(&self.mode, operation)
    }
    pub(crate) fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        if self.visible == visible {
            return;
        }
        self.visible = visible;
        let _ = self.poll_wake.try_send(());
        if !visible {
            let pinned_id = match &self.mode {
                PaneMode::Manager => None,
                PaneMode::Pinned { session_id } => Some(session_id.as_str()),
            };
            remove_queued_reads(&mut self.queue, pinned_id);
        } else if self.initial_read_pending {
            let after = self
                .selected
                .as_ref()
                .and_then(|id| self.models.get(id))
                .map(|model| model.seq)
                .unwrap_or_default();
            schedule_initial_read(
                &mut self.queue,
                &self.mode,
                self.initial_read_pending,
                self.visible,
                self.paused,
                self.busy,
                after,
            );
        }
        cx.notify();
        self.pump(cx);
    }
    fn install_session(
        &mut self,
        session: Value,
        cols: u16,
        rows: u16,
        request_focus: bool,
    ) -> Result<(), String> {
        let id = field(&session, "id");
        if id.trim().is_empty() {
            return Err("Terminal open response has no session ID.".into());
        }
        self.install_session_projection(session, id, cols, rows, request_focus);
        Ok(())
    }
    fn install_session_projection(
        &mut self,
        session: Value,
        id: String,
        cols: u16,
        rows: u16,
        request_focus: bool,
    ) {
        self.sessions.retain(|listed| field(listed, "id") != id);
        let mut model = TerminalModel::new(rows, cols);
        if let Some(status) = session.get("status").and_then(Value::as_str) {
            model.status = status.to_owned();
        }
        model.exit_code = session
            .get("exitCode")
            .and_then(Value::as_i64)
            .and_then(|code| i32::try_from(code).ok());
        self.sessions.push(session);
        self.models.insert(id.clone(), model);
        self.selected = Some(id);
        self.selection = None;
        self.selecting = false;
        self.focus_requested = request_focus;
        self.marked.clear();
    }
    pub fn set_context(
        &mut self,
        snapshot: &Value,
        task_id: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let context_changed = terminal_context_changed(&self.snapshot, snapshot);
        let task_changed = matches!(self.mode, PaneMode::Manager) && self.task_id != task_id;
        if !context_changed && !task_changed { return; }
        if context_changed {
            self.snapshot = terminal_context_snapshot(snapshot);
            self.appearance = Appearance::from_snapshot(snapshot);
        }
        if task_changed { self.task_id = task_id; }
        cx.notify();
    }
    /// Adopt a terminal session opened by the owning workspace flow.
    /// This applies the normal open-result projection without issuing another
    /// backend command or creating a second PTY.
    pub(crate) fn accept_opened_session(
        &mut self,
        session: Value,
        cols: u16,
        rows: u16,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        if !matches!(self.mode, PaneMode::Manager) {
            return Err("A pinned terminal cannot adopt another session.".into());
        }
        let id = field(&session, "id");
        let adopted_session = session.clone();
        self.apply(
            Operation::Open {
                target: Value::Null,
                cols,
                rows,
            },
            session,
        )?;
        if self.list_in_flight {
            self.adopted_during_list.insert(id, adopted_session);
        }
        cx.notify();
        Ok(())
    }
    fn enqueue(&mut self, op: Operation, cx: &mut Context<Self>) {
        if !self.operation_allowed(&op) {
            self.error = Some("This terminal pane cannot access that session.".into());
            cx.notify();
            return;
        }
        self.queue.push_back(op);
        self.pump(cx);
    }
    fn pump(&mut self, cx: &mut Context<Self>) {
        let mode = self.mode.clone();
        self.queue.retain(|operation| mode.allows(operation));
        let Some(op) = next_operation(&mut self.queue, self.busy, self.paused, self.close_bypass)
        else {
            return;
        };
        if matches!(op, Operation::Close { .. }) {
            self.close_bypass = false;
        }
        if self.initial_read_pending
            && matches!(&op, Operation::Read { id, .. }
                if matches!(&self.mode, PaneMode::Pinned { session_id } if id == session_id))
        {
            self.initial_read_pending = false;
        }
        if matches!(op, Operation::List) {
            self.list_in_flight = true;
        }
        self.busy = true;
        let service = self.service.clone();
        let request = op.clone();
        let worker = cx
            .background_executor()
            .spawn(async move { request.invoke(&service) });
        cx.spawn(async move |pane,cx|{
            let result=worker.await;
            let _ = pane.update(cx, |pane, cx| {
                pane.busy = false;
                let is_list = matches!(op, Operation::List);
                let closed_session=match &op {Operation::Close {id}=>Some(id.clone()),_=>None};
                if !pane.operation_allowed(&op) {
                    pane.error = Some("Terminal response did not match this pinned session.".into());
                    pane.paused = true;
                    pane.pump(cx);
                    cx.notify();
                    return;
                }
                let changed = match result {
                    Err(error) => {
                        if is_list {
                            pane.list_in_flight = false;
                            pane.adopted_during_list.clear();
                        }
                        if retain_failed_operation(&mut pane.queue, op) {
                            pane.error = Some(format!(
                                "{error} Pending operations are retained. Retrying a failed write may repeat partially written input."
                            ));
                            pane.paused = true;
                        }
                        true
                    }
                    Ok(value) => match pane.apply(op, value) {
                        Ok(updated) => {
                            if let Some(session_id)=closed_session {cx.emit(TerminalEvent::SessionClosed {session_id});}
                            updated
                        },
                        Err(error) => {
                            if is_list {
                                pane.list_in_flight = false;
                                pane.adopted_during_list.clear();
                            }
                            pane.error = Some(error);
                            pane.paused = true;
                            true
                        }
                    },
                };
                pane.report_known_exit(cx);
                if pane.paused { let _ = pane.poll_wake.try_send(()); }
                pane.pump(cx);
                if changed {
                    cx.notify();
                }
            });
        }).detach();
    }
    fn apply(&mut self, op: Operation, value: Value) -> Result<bool, String> {
        let mut changed = true;
        match op {
            Operation::List => {
                let sessions = value
                    .as_array()
                    .cloned()
                    .ok_or("Terminal list response is invalid.")?;
                self.sessions = merge_adopted_sessions(sessions, &mut self.adopted_during_list);
                self.list_in_flight = false;
                for session in &self.sessions {
                    self.models
                        .entry(field(session, "id"))
                        .or_insert_with(|| TerminalModel::new(self.rows, self.cols));
                }
            }
            Operation::Open { cols, rows, .. } => {
                self.install_session(value, cols, rows, true)?;
            }
            Operation::Read { id, .. } => {
                if value.get("session").is_some_and(|session|field(session,"id")!=id) {
                    return Err("Terminal read response did not match its session identity.".into());
                }
                let model = self
                    .models
                    .get_mut(&id)
                    .ok_or("Terminal screen model is missing.")?;
                changed = model.apply_read(&value)?;
                let response = model.take_responses();
                if !response.is_empty() {
                    self.queue.push_back(Operation::Write {
                        id: id.clone(),
                        data: response,
                    });
                }
                if let Some(session) = value.get("session") {
                    if let Some(existing) = self
                        .sessions
                        .iter_mut()
                        .find(|session| field(session, "id") == id)
                    {
                        changed |= *existing != *session;
                        *existing = session.clone();
                    }
                }
            }
            Operation::Resize { id, cols, rows } => {
                if let Some(model) = self.models.get_mut(&id) {
                    model.resize(rows, cols);
                }
            }
            Operation::Close { id } => {
                self.adopted_during_list.remove(&id);
                self.sessions.retain(|session| field(session, "id") != id);
                self.models.remove(&id);
                if self.selected.as_ref() == Some(&id) {
                    self.selected = None;
                    self.marked.clear();
                }
            }
            Operation::Write { .. } => {
                changed = false;
            }
        }
        Ok(changed)
    }
    fn open(&mut self, local: bool, cx: &mut Context<Self>) {
        let target = if !local {
            let Some(task) = self.task_id.clone() else {
                self.error = Some("Select a chat before opening its terminal.".into());
                cx.notify();
                return;
            };
            json!({"taskId":task})
        } else {
            let host = rows(&self.snapshot, "hosts")
                .into_iter()
                .find(|host| field(host, "kind") == "local");
            let Some(host) = host else {
                self.error = Some("No local host is configured.".into());
                cx.notify();
                return;
            };
            json!({"hostId":field(&host,"id")})
        };
        self.enqueue(
            Operation::Open {
                target,
                cols: self.cols,
                rows: self.rows,
            },
            cx,
        );
    }
    /// Open a task-owned shell, optionally delivering the supplied command
    /// through the backend's existing `TerminalTarget.command` field.
    /// `None` and the empty string both request an ordinary task terminal.
    /// Non-empty command text is passed through without trimming or rewriting.
    pub(crate) fn open_command(
        &mut self,
        task_id: &str,
        command: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        if task_id.trim().is_empty() {
            self.error = Some("Select a chat before opening its terminal.".into());
            cx.notify();
            return;
        }
        let mut target = json!({"taskId": task_id});
        if let Some(command) = command.filter(|command| !command.is_empty()) {
            target["command"] = json!(command);
        }
        self.enqueue(
            Operation::Open {
                target,
                cols: self.cols,
                rows: self.rows,
            },
            cx,
        );
    }
    fn bytes(&mut self, data: Vec<u8>, cx: &mut Context<Self>) {
        let Some(id) = self.selected.clone() else {
            return;
        };
        if data.is_empty() {
            return;
        }
        if let Some(model) = self.models.get_mut(&id) {
            model.parser.screen_mut().set_scrollback(0);
        }
        self.selection = None;
        // Native typed input keeps arbitrary terminal protocol bytes intact.
        // Chunk only at the existing backend's 64 KiB write limit.
        for chunk in data.chunks(65_536) {
            self.queue.push_back(Operation::Write {
                id: id.clone(),
                data: chunk.to_vec(),
            });
        }
        self.pump(cx);
    }
    fn paste(&mut self, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            if let Some(model) = self.selected.as_ref().and_then(|id| self.models.get(id)) {
                let data = model.paste_bytes(&text);
                self.bytes(data, cx);
            }
        }
    }
    fn copy(&mut self, cx: &mut Context<Self>) {
        let selected = self
            .selection
            .as_ref()
            .filter(|selection| !selection.text.is_empty())
            .map(|selection| selection.text.clone());
        let text = selected.or_else(|| {
            self.selected
                .as_ref()
                .and_then(|id| self.models.get(id))
                .map(|model| model.parser.screen().contents())
        });
        if let Some(text) = text {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }
    fn key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.focus.is_focused(window) {
            return;
        }
        let key = &event.keystroke.key;
        let mods = event.keystroke.modifiers;
        if mods.platform {
            if key == "v" {
                self.paste(cx);
                cx.stop_propagation();
            } else if key == "c" {
                self.copy(cx);
                cx.stop_propagation();
            }
            return;
        }
        // Native committed-text callbacks handle ordinary Unicode and IME.
        // Never send a printable key here and again from replace_text.
        if event.prefer_character_input
            || (!mods.control && !mods.alt && (key.chars().count() == 1 || key == "space"))
            || (!self.marked.is_empty() && !mods.control)
        {
            return;
        }
        if let Some(model) = self.selected.as_ref().and_then(|id| self.models.get(id)) {
            if let Some(data) = model.key_bytes(key, mods.shift, mods.alt, mods.control) {
                self.bytes(data, cx);
                cx.stop_propagation();
            }
        }
    }
    fn resize_grid(&mut self, bounds: Bounds<Pixels>, cell_width: Pixels, cx: &mut Context<Self>) {
        self.grid_bounds = Some(bounds);
        self.cell_width = cell_width;
        let width: f32 = bounds.size.width.into();
        let height: f32 = bounds.size.height.into();
        let cell: f32 = cell_width.into();
        let cols = (width / cell.max(1.)).floor().clamp(10., 500.) as u16;
        let rows = (height / self.appearance.cell_height())
            .floor()
            .clamp(4., 300.) as u16;
        self.cols = cols;
        self.rows = rows;
        if let Some(id) = self.selected.clone() {
            let current = self
                .models
                .get(&id)
                .map(|model| model.parser.screen().size());
            if current!=Some((rows,cols))&&!self.queue.iter().any(|op|matches!(op,Operation::Resize{id:queued,cols:c,rows:r} if queued==&id&&*c==cols&&*r==rows)){
                self.enqueue(Operation::Resize{id,cols,rows},cx);
            }
        }
    }
    fn cell_at(&self, position: Point<Pixels>, boundary: bool, clamp: bool) -> Option<CellPoint> {
        let bounds = self.grid_bounds?;
        if !clamp && !bounds.contains(&position) {
            return None;
        }
        let screen = self
            .selected
            .as_ref()
            .and_then(|id| self.models.get(id))?
            .parser
            .screen();
        let (height, width) = screen.size();
        let x: f32 = (position.x - bounds.origin.x).into();
        let y: f32 = (position.y - bounds.origin.y).into();
        let cell_width: f32 = self.cell_width.into();
        let column = x / cell_width;
        let col = if boundary {
            column.round()
        } else {
            column.floor()
        };
        let row = (y / self.appearance.cell_height())
            .floor()
            .clamp(0., f32::from(height.saturating_sub(1))) as u16;
        let mut col = col.clamp(
            0.,
            f32::from(if boundary {
                width
            } else {
                width.saturating_sub(1)
            }),
        ) as u16;
        if boundary
            && screen
                .cell(row, col)
                .is_some_and(|cell| cell.is_wide_continuation())
        {
            // A native selection boundary cannot split a double-width scalar.
            col = if column < f32::from(col) {
                col.saturating_sub(1)
            } else {
                col.saturating_add(1).min(width)
            };
        }
        Some(CellPoint { row, col })
    }
    fn scroll(&mut self, delta: i32, cx: &mut Context<Self>) {
        if let Some(model) = self
            .selected
            .as_ref()
            .and_then(|id| self.models.get_mut(id))
        {
            let old = model.parser.screen().scrollback();
            let next = if delta >= 0 {
                old.saturating_add(delta as usize)
            } else {
                old.saturating_sub(delta.unsigned_abs() as usize)
            };
            model.parser.screen_mut().set_scrollback(next);
            self.selection = None;
            self.selecting = false;
            cx.notify();
        }
    }
    fn live(&mut self, cx: &mut Context<Self>) {
        if let Some(model) = self
            .selected
            .as_ref()
            .and_then(|id| self.models.get_mut(id))
        {
            model.parser.screen_mut().set_scrollback(0);
        }
        self.selection = None;
        self.selecting = false;
        cx.notify();
    }
    fn report_mouse(
        &mut self,
        kind: MouseKind,
        button: Option<MouseButton>,
        position: Point<Pixels>,
        modifiers: Modifiers,
        cx: &mut Context<Self>,
    ) -> bool {
        if modifiers.shift {
            return false;
        }
        let Some(cell) = self.cell_at(position, false, matches!(kind, MouseKind::Release)) else {
            return false;
        };
        let Some(model) = self.selected.as_ref().and_then(|id| self.models.get(id)) else {
            return false;
        };
        let screen = model.parser.screen();
        if screen.scrollback() != 0
            || screen.mouse_protocol_mode() == vt100::MouseProtocolMode::None
        {
            return false;
        }
        let button_code = button.and_then(|button| match button {
            MouseButton::Left => Some(0),
            MouseButton::Middle => Some(1),
            MouseButton::Right => Some(2),
            _ => None,
        });
        if matches!(kind, MouseKind::Motion) && self.last_mouse_cell == Some(cell) {
            return true;
        }
        let packet = mouse_bytes(
            screen.mouse_protocol_mode(),
            screen.mouse_protocol_encoding(),
            kind,
            button_code,
            cell,
            modifiers,
        );
        if let Some(bytes) = packet {
            self.last_mouse_cell = Some(cell);
            self.bytes(bytes, cx);
        }
        // A requested app protocol owns its supported event stream even if
        // this particular event/mode has no report (e.g. X10 release).
        true
    }
    fn update_selection(&mut self, cx: &mut Context<Self>) {
        if let (Some(selection), Some(model)) = (
            &mut self.selection,
            self.selected.as_ref().and_then(|id| self.models.get(id)),
        ) {
            let (start, end) = selection.ordered();
            selection.text = model
                .parser
                .screen()
                .contents_between(start.row, start.col, end.row, end.col);
        }
        cx.notify();
    }
    fn mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        self.last_mouse_cell = None;
        if self.report_mouse(
            MouseKind::Press,
            Some(event.button),
            event.position,
            event.modifiers,
            cx,
        ) {
            self.protocol_button = Some(event.button);
            self.selecting = false;
        } else if event.button == MouseButton::Left {
            if let Some(cell) = self.cell_at(event.position, true, false) {
                self.selection = Some(Selection {
                    anchor: cell,
                    end: cell,
                    text: String::new(),
                });
                self.selecting = true;
                self.protocol_button = None;
            }
        }
        cx.stop_propagation();
        cx.notify();
    }
    fn mouse_up(&mut self, event: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        // A splitter or tab drag may release over/outside this grid. Do not
        // consume a release for a press that this terminal never owned.
        let owns_press = self.protocol_button == Some(event.button)
            || (event.button == MouseButton::Left && self.selecting);
        if !owns_press {
            return;
        }
        if self.protocol_button.take().is_some() {
            // Finish an app-owned press even if Shift was pressed mid-drag.
            let mut modifiers = event.modifiers;
            modifiers.shift = false;
            self.report_mouse(
                MouseKind::Release,
                Some(event.button),
                event.position,
                modifiers,
                cx,
            );
        } else if self.selecting {
            if let Some(cell) = self.cell_at(event.position, true, true) {
                if let Some(selection) = &mut self.selection {
                    selection.end = cell;
                }
            }
            self.update_selection(cx);
        }
        self.selecting = false;
        self.last_mouse_cell = None;
        cx.stop_propagation();
    }
    fn mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.selecting {
            if let Some(cell) = self.cell_at(event.position, true, true) {
                if let Some(selection) = &mut self.selection {
                    selection.end = cell;
                }
            }
            self.update_selection(cx);
        } else {
            self.report_mouse(
                MouseKind::Motion,
                self.protocol_button.or(event.pressed_button),
                event.position,
                event.modifiers,
                cx,
            );
        }
    }
    fn wheel(&mut self, event: &ScrollWheelEvent, _: &mut Window, cx: &mut Context<Self>) {
        let delta = match event.delta {
            ScrollDelta::Pixels(point) => {
                let y: f32 = point.y.into();
                y / self.appearance.cell_height()
            }
            ScrollDelta::Lines(point) => point.y,
        };
        self.wheel_remainder += delta;
        let steps = self.wheel_remainder.trunc().clamp(-100., 100.) as i32;
        self.wheel_remainder -= steps as f32;
        if steps != 0 {
            let kind = if steps > 0 {
                MouseKind::WheelUp
            } else {
                MouseKind::WheelDown
            };
            if self.report_mouse(kind, None, event.position, event.modifiers, cx) {
                for _ in 1..steps.unsigned_abs() {
                    self.report_mouse(kind, None, event.position, event.modifiers, cx);
                }
            } else {
                self.scroll(steps, cx);
            }
        }
        cx.stop_propagation();
    }
    fn close(&mut self, cx: &mut Context<Self>) {
        if !matches!(self.mode, PaneMode::Manager) {
            return;
        }
        let Some(id) = self.selected.clone() else {
            return;
        };
        self.adopted_during_list.remove(&id);
        // Explicit close is the only user action allowed to abandon queued
        // input for this PTY. In-flight operations finish before the close.
        self.paused = queue_explicit_close(&mut self.queue, id, self.paused);
        self.close_bypass = true;
        if !self.paused {
            self.error = None;
        }
        self.pump(cx);
    }
}

impl EntityInputHandler for TerminalPane {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        actual: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let units: Vec<_> = self.marked.encode_utf16().collect();
        let start = range.start.min(units.len());
        let end = range.end.max(start).min(units.len());
        *actual = Some(start..end);
        String::from_utf16(&units[start..end]).ok()
    }
    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.marked_selection.clone(),
            reversed: false,
        })
    }
    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        (!self.marked.is_empty()).then(|| 0..self.marked.encode_utf16().count())
    }
    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.marked.clear();
        self.marked_selection = 0..0;
        cx.notify();
    }
    fn replace_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.marked.clear();
        self.marked_selection = 0..0;
        self.bytes(text.as_bytes().to_vec(), cx);
        cx.notify();
    }
    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        selection: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut units: Vec<_> = self.marked.encode_utf16().collect();
        let range = range.unwrap_or(0..units.len());
        let start = range.start.min(units.len());
        let end = range.end.max(start).min(units.len());
        units.splice(start..end, text.encode_utf16());
        self.marked = String::from_utf16_lossy(&units);
        let length = self.marked.encode_utf16().count();
        self.marked_selection = selection
            .map(|r| r.start.min(length)..r.end.min(length))
            .unwrap_or(length..length);
        cx.notify();
    }
    fn bounds_for_range(
        &mut self,
        _: Range<usize>,
        bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let (row, col) = self
            .selected
            .as_ref()
            .and_then(|id| self.models.get(id))
            .map(|m| m.parser.screen().cursor_position())
            .unwrap_or((0, 0));
        Some(Bounds {
            origin: point(
                bounds.origin.x + self.cell_width * col as f32,
                bounds.origin.y + px(self.appearance.cell_height() * row as f32),
            ),
            size: size(self.cell_width, px(self.appearance.cell_height())),
        })
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
impl Focusable for TerminalPane {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

fn color(value: vt100::Color, default: u32, palette: &Palette) -> u32 {
    match value {
        vt100::Color::Default => default,
        vt100::Color::Rgb(r, g, b) => ((r as u32) << 16) | ((g as u32) << 8) | (b as u32),
        vt100::Color::Idx(index) if index < 16 => palette.ansi[index as usize],
        vt100::Color::Idx(index) if index < 232 => {
            let n = (index - 16) as u32;
            let component = |x: u32| if x == 0 { 0 } else { 55 + 40 * x };
            (component(n / 36) << 16) | (component((n / 6) % 6) << 8) | component(n % 6)
        }
        vt100::Color::Idx(index) => {
            let v = 8 + 10 * (index as u32 - 232);
            (v << 16) | (v << 8) | v
        }
    }
}
fn terminal_run(
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
struct TerminalGrid {
    pane: Entity<TerminalPane>,
}
impl IntoElement for TerminalGrid {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}
impl Element for TerminalGrid {
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
        let run = terminal_run(&appearance, "M", 0xdddddd, false, false, false);
        let measure = window.text_system().shape_line(
            "M".into(),
            ui_scaled_px(appearance.size),
            &[run],
            None,
        );
        // x_for_index already reflects the scaled font size; only the fallback
        // minimum is a nominal UI dimension.
        let width = measure.x_for_index(1).max(ui_scaled_px(1.));
        self.pane
            .update(cx, |pane, cx| pane.resize_grid(bounds, width, cx));
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
        self.pane.update(cx, |pane, cx| {
            let focus = pane.focus.clone();
            let appearance = &pane.appearance;
            let palette = PALETTES[pane.palette];
            let cell_height = appearance.cell_height();
            window.handle_input(
                &focus,
                ElementInputHandler::new(bounds, self.pane.clone()),
                cx,
            );
            window.with_content_mask(Some(ContentMask::new(bounds)), |window| {
                // The pane supplies the shared translucent surface. Default
                // terminal cells leave it visible; ANSI backgrounds still paint.
                if let Some(model) = pane.selected.as_ref().and_then(|id| pane.models.get(id)) {
                    let screen = model.parser.screen();
                    let (rows, cols) = screen.size();
                    for row in 0..rows {
                        for col in 0..cols {
                            let Some(cell) = screen.cell(row, col) else {
                                continue;
                            };
                            if cell.is_wide_continuation() {
                                continue;
                            }
                            let origin = point(
                                bounds.origin.x + *width * col as f32,
                                bounds.origin.y + px(cell_height * row as f32),
                            );
                            let mut fg = color(cell.fgcolor(), palette.foreground, &palette);
                            let mut bg = color(cell.bgcolor(), palette.background, &palette);
                            if cell.inverse() {
                                std::mem::swap(&mut fg, &mut bg);
                            }
                            if cell.dim() {
                                fg = (((fg >> 16) & 255) * 2 / 3) << 16
                                    | (((fg >> 8) & 255) * 2 / 3) << 8
                                    | (fg & 255) * 2 / 3;
                            }
                            if cell.inverse() || cell.bgcolor() != vt100::Color::Default {
                                window.paint_quad(fill(
                                    Bounds {
                                        origin,
                                        size: size(
                                            *width * if cell.is_wide() { 2. } else { 1. },
                                            px(cell_height),
                                        ),
                                    },
                                    rgb(bg),
                                ));
                            }
                            if !cell.has_contents() {
                                continue;
                            }
                            let text = cell.contents();
                            let run = terminal_run(
                                appearance,
                                text,
                                fg,
                                cell.bold(),
                                cell.italic(),
                                cell.underline(),
                            );
                            let line = window.text_system().shape_line(
                                text.to_owned().into(),
                                ui_scaled_px(appearance.size),
                                &[run],
                                None,
                            );
                            let _ = line.paint(
                                origin,
                                px(cell_height),
                                TextAlign::Left,
                                None,
                                window,
                                cx,
                            );
                        }
                    }
                    if let Some(selection) = &pane.selection {
                        for row in 0..rows {
                            for col in 0..cols {
                                if selection.contains(row, col) {
                                    let origin = point(
                                        bounds.origin.x + *width * col as f32,
                                        bounds.origin.y + px(cell_height * row as f32),
                                    );
                                    window.paint_quad(fill(
                                        Bounds {
                                            origin,
                                            size: size(*width, px(cell_height)),
                                        },
                                        rgba((palette.ansi[8] << 8) | 0x99),
                                    ));
                                }
                            }
                        }
                    }
                    let (row, col) = screen.cursor_position();
                    let origin = point(
                        bounds.origin.x + *width * col as f32,
                        bounds.origin.y + px(cell_height * row as f32),
                    );
                    if screen.scrollback() == 0 && !screen.hide_cursor() && focus.is_focused(window)
                    {
                        window.paint_quad(fill(
                            Bounds {
                                origin,
                                size: size(*width, px(cell_height)),
                            },
                            rgba((palette.cursor << 8) | 0x65),
                        ));
                    }
                    if !pane.marked.is_empty() {
                        let run = terminal_run(
                            appearance,
                            &pane.marked,
                            palette.foreground,
                            false,
                            false,
                            true,
                        );
                        let line = window.text_system().shape_line(
                            pane.marked.clone().into(),
                            ui_scaled_px(appearance.size),
                            &[run],
                            None,
                        );
                        let _ =
                            line.paint(origin, px(cell_height), TextAlign::Left, None, window, cx);
                    }
                }
            });
        });
    }
}

impl Render for TerminalPane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.focus_requested {
            window.focus(&self.focus, cx);
            self.focus_requested = false;
        }
        let palette = crate::design::palette(&self.snapshot["settings"], window.appearance());
        let interface_font_ratio = self.snapshot["settings"]["interfaceFontSize"]
            .as_f64()
            .filter(|size| size.is_finite())
            .unwrap_or(14.)
            .clamp(8., 32.) as f32
            / 14.;
        let grid = div()
            .id("terminal-screen")
            .flex_1()
            .min_h(px(0.))
            .min_w_0()
            .w_full()
            .key_context("MonitterNativeTerminal")
            .track_focus(&self.focus)
            .cursor(CursorStyle::IBeam)
            .on_action(
                cx.listener(|pane, _: &gpui_kit::component::input::Copy, _, cx| pane.copy(cx)),
            )
            .on_action(
                cx.listener(|pane, _: &gpui_kit::component::input::Paste, _, cx| pane.paste(cx)),
            )
            .on_key_down(cx.listener(Self::key))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::mouse_down))
            .on_mouse_down(MouseButton::Middle, cx.listener(Self::mouse_down))
            .on_mouse_down(MouseButton::Right, cx.listener(Self::mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::mouse_up))
            .on_mouse_up(MouseButton::Middle, cx.listener(Self::mouse_up))
            .on_mouse_up(MouseButton::Right, cx.listener(Self::mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::mouse_up))
            .on_mouse_up_out(MouseButton::Middle, cx.listener(Self::mouse_up))
            .on_mouse_up_out(MouseButton::Right, cx.listener(Self::mouse_up))
            .on_mouse_move(cx.listener(Self::mouse_move))
            .on_scroll_wheel(cx.listener(Self::wheel))
            .child(TerminalGrid { pane: cx.entity() });
        if let PaneMode::Pinned { session_id } = &self.mode {
            let state = if let Some(error) = &self.error {
                Some((format!("Terminal error: {error}"), true))
            } else if !self
                .sessions
                .iter()
                .any(|session| field(session, "id") == *session_id)
            {
                Some(("Terminal session is unavailable.".to_owned(), false))
            } else if let Some(model) = self.models.get(session_id) {
                (model.status == "exited").then(|| {
                    (
                        format!(
                            "Exited{}.",
                            model
                                .exit_code
                                .map(|code| format!(" with status {code}"))
                                .unwrap_or_default()
                        ),
                        false,
                    )
                })
            } else {
                Some(("Terminal session is unavailable.".to_owned(), false))
            };
            let mut screen = div()
                .relative()
                .flex()
                .flex_col()
                .size_full()
                .min_w_0()
                .min_h_0()
                .overflow_hidden()
                .p(px(14. * interface_font_ratio))
                .child(grid);
            if let Some((message, error)) = state {
                screen = screen.child(
                    div()
                        .absolute()
                        .right(px(10.))
                        .bottom(px(8.))
                        .px(px(6.))
                        .py(px(3.))
                        .rounded(px(4.))
                        .bg(crate::design::surface_color(palette.soft))
                        .text_size(px(11. * interface_font_ratio))
                        .line_height(relative(1.2))
                        .text_color(rgb(if error { 0xdd6666 } else { palette.muted }))
                        .child(message),
                );
            }
            return screen;
        }
        let mut controls = div()
            .id("terminal-session-controls")
            .flex()
            .flex_none()
            .items_center()
            .gap(px(4.))
            .h(px(29.))
            .min_w_0()
            .overflow_x_scroll()
            .child(
                terminal_toolbar_button(
                    "terminal-new-task",
                    "Chat shell",
                    "New terminal for selected chat",
                    "Open a terminal for the selected chat",
                    false,
                    palette,
                    interface_font_ratio,
                    cx,
                )
                .disabled(self.task_id.is_none() || self.paused)
                .on_click(cx.listener(|pane, _, _, cx| pane.open(false, cx))),
            )
            .child(
                terminal_toolbar_button(
                    "terminal-new-local",
                    "New local",
                    "New local terminal",
                    "Open a local terminal",
                    false,
                    palette,
                    interface_font_ratio,
                    cx,
                )
                .disabled(self.paused)
                .on_click(cx.listener(|pane, _, _, cx| pane.open(true, cx))),
            )
            .child(
                terminal_toolbar_button(
                    "terminal-list",
                    "Reload",
                    "Reload terminal sessions",
                    "Reload terminal sessions",
                    false,
                    palette,
                    interface_font_ratio,
                    cx,
                )
                .disabled(self.paused)
                .on_click(cx.listener(|pane, _, _, cx| pane.enqueue(Operation::List, cx))),
            );
        for session in self.sessions.clone() {
            let id = field(&session, "id");
            let chosen = self.selected.as_ref() == Some(&id);
            controls = controls.child(
                terminal_toolbar_button(
                    SharedString::from(format!("terminal-select-{id}")),
                    field(&session, "title"),
                    format!("Select terminal {}", field(&session, "title")),
                    "Select this terminal session",
                    chosen,
                    palette,
                    interface_font_ratio,
                    cx,
                )
                .on_click(cx.listener(move |pane, _, window, cx| {
                    pane.selected = Some(id.clone());
                    pane.selection = None;
                    pane.selecting = false;
                    pane.protocol_button = None;
                    pane.last_mouse_cell = None;
                    pane.marked.clear();
                    window.focus(&pane.focus, cx);
                    cx.notify();
                })),
            );
        }
        let mut pane = div()
            .flex()
            .flex_col()
            .size_full()
            .min_w_0()
            .min_h_0()
            .gap(px(4.))
            .p(px(8.))
            .child(controls);
        if let Some(error) = self.error.as_ref().or(self.exit_monitor_error.as_ref()) {
            pane = pane.child(
                div()
                    .flex_none()
                    .text_size(px(10. * interface_font_ratio))
                    .line_height(relative(1.25))
                    .text_color(rgb(0xd66b70))
                    .child(error.clone()),
            );
        }
        if self.paused {
            pane = pane.child(
                terminal_toolbar_button(
                    "terminal-retry",
                    "Retry",
                    "Retry retained terminal operation",
                    "Retry the paused terminal operation",
                    false,
                    palette,
                    interface_font_ratio,
                    cx,
                )
                .on_click(cx.listener(|pane, _, _, cx| {
                    pane.paused = false;
                    let _ = pane.poll_wake.try_send(());
                    pane.error = None;
                    pane.pump(cx);
                    cx.notify();
                })),
            );
        }
        if let Some(id) = &self.selected {
            if let Some(model) = self.models.get(id) {
                pane = pane.child(
                    div()
                        .flex_none()
                        .text_size(px(9. * interface_font_ratio))
                        .line_height(relative(1.2))
                        .text_color(rgb(if model.status == "running" {
                            palette.muted
                        } else {
                            palette.accent
                        }))
                        .child(format!(
                            "{} · {}×{}{}{}",
                            model.status,
                            self.cols,
                            self.rows,
                            model
                                .exit_code
                                .map(|code| format!(" · exit {code}"))
                                .unwrap_or_default(),
                            if model.truncated {
                                " · Earlier output was truncated; screen reset to retained output."
                            } else {
                                ""
                            }
                        )),
                );
            }
            let copy_label = if self
                .selection
                .as_ref()
                .is_some_and(|selection| !selection.text.is_empty())
            {
                "Copy selection"
            } else {
                "Copy screen"
            };
            let actions = div()
                .id("terminal-actions")
                .flex()
                .flex_none()
                .items_center()
                .gap(px(4.))
                .h(px(29.))
                .min_w_0()
                .overflow_x_scroll()
                .child(
                    terminal_toolbar_button(
                        "terminal-copy",
                        copy_label,
                        copy_label,
                        "Copy selected text or the visible terminal screen",
                        false,
                        palette,
                        interface_font_ratio,
                        cx,
                    )
                    .on_click(cx.listener(|pane, _, _, cx| pane.copy(cx))),
                )
                .child(
                    terminal_toolbar_button(
                        "terminal-paste",
                        "Paste",
                        "Paste into terminal",
                        "Paste clipboard text into the terminal",
                        false,
                        palette,
                        interface_font_ratio,
                        cx,
                    )
                    .on_click(cx.listener(|pane, _, _, cx| pane.paste(cx))),
                )
                .child(
                    terminal_toolbar_button(
                        "terminal-history-up",
                        "Earlier",
                        "Scroll to earlier terminal output",
                        "Scroll up through terminal output",
                        false,
                        palette,
                        interface_font_ratio,
                        cx,
                    )
                    .on_click(cx.listener(|pane, _, _, cx| {
                        pane.scroll(i32::from(pane.rows.saturating_sub(1)), cx)
                    })),
                )
                .child(
                    terminal_toolbar_button(
                        "terminal-history-down",
                        "Later",
                        "Scroll to later terminal output",
                        "Scroll down through terminal output",
                        false,
                        palette,
                        interface_font_ratio,
                        cx,
                    )
                    .on_click(cx.listener(|pane, _, _, cx| {
                        pane.scroll(-i32::from(pane.rows.saturating_sub(1)), cx)
                    })),
                )
                .child(
                    terminal_toolbar_button(
                        "terminal-history-live",
                        "Live",
                        "Follow live terminal output",
                        "Return to live terminal output",
                        false,
                        palette,
                        interface_font_ratio,
                        cx,
                    )
                    .on_click(cx.listener(|pane, _, _, cx| pane.live(cx))),
                )
                .child(
                    terminal_toolbar_button(
                        "terminal-palette",
                        PALETTES[self.palette].name,
                        format!("Terminal palette: {}", PALETTES[self.palette].name),
                        "Change terminal color palette",
                        false,
                        palette,
                        interface_font_ratio,
                        cx,
                    )
                    .on_click(cx.listener(|pane, _, _, cx| {
                        let theme = crate::terminal_theme::theme_for_palette_index((pane.palette + 1) % PALETTES.len());
                        if let Some(theme) = crate::terminal_theme::choose(theme.id) {
                            cx.set_global(crate::terminal_theme::Selection(theme.palette_index));
                        }
                        cx.notify();
                    })),
                )
                .child(
                    terminal_toolbar_button(
                        "terminal-close",
                        "Close",
                        "Close terminal",
                        "Close this terminal session",
                        false,
                        palette,
                        interface_font_ratio,
                        cx,
                    )
                    .on_click(cx.listener(|pane, _, _, cx| pane.close(cx))),
                );
            pane = pane.child(actions).child(grid);
        } else {
            pane = pane.child(
                div()
                    .flex_1()
                    .min_h(px(0.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(11. * interface_font_ratio))
                    .text_color(rgb(palette.muted))
                    .child("Select a session or open a new terminal."),
            );
        }
        pane
    }
}

#[cfg(test)]
mod native_terminal_tests {
    use super::*;
    use ::core::prelude::v1::test;

    #[::core::prelude::v1::test]
    fn terminal_context_excludes_transcripts_and_detects_host_and_style_changes() {
        let snapshot = json!({"hosts":[{"id":"local","kind":"local"}],"settings":{"terminalFontSize":14},"tasks":[{"messages":[{"text":"large transcript"}]}]});
        let context = terminal_context_snapshot(&snapshot);
        assert!(context.get("tasks").is_none());
        let mut next = snapshot.clone();
        next["tasks"][0]["messages"][0]["text"] = json!("new token");
        assert!(!terminal_context_changed(&context, &next));
        next["settings"]["terminalFontSize"] = json!(16);
        assert!(terminal_context_changed(&context, &next));
        next = snapshot.clone();
        next["hosts"][0]["id"] = json!("replacement");
        assert!(terminal_context_changed(&context, &next));
    }

    #[cfg(feature = "visual-export")]
    #[gpui_kit::test]
    fn exit_monitor_is_shared_and_registry_does_not_keep_it_alive(cx: &mut TestAppContext) {
        let service = DesktopService::disconnected_for_ui_fixture();
        let first = cx.update(|cx| TerminalExitMonitor::shared(service.clone(), cx));
        let second = cx.update(|cx| TerminalExitMonitor::shared(service, cx));
        assert_eq!(first.entity_id(), second.entity_id());
        let one = cx.new(|_| ()).entity_id();
        let two = cx.new(|_| ()).entity_id();
        first.update(cx, |monitor, _| {
            monitor.watch(one, true);
            monitor.watch(two, true);
            monitor.watch(one, false);
            assert_eq!(monitor.watchers.len(), 1);
            monitor.watch(two, false);
            assert!(monitor.watchers.is_empty());
        });
        let weak = first.downgrade();
        drop(first);
        drop(second);
        // Neither its waiting task nor the global registry owns the entity.
        assert!(weak.upgrade().is_none());
    }

    #[cfg(feature = "visual-export")]
    #[gpui_kit::test]
    fn hidden_terminal_wakes_on_reveal_and_monitor_failures_never_signal_exit(
        cx: &mut TestAppContext,
    ) {
        cx.update(gpui_kit::init);
        let mut retained = None;
        let (_, visual) = cx.add_window_view(|window, cx| {
            let pane = cx.new(|cx| {
                TerminalPane::new_inner(
                    DesktopService::disconnected_for_ui_fixture(),
                    &json!({}),
                    window,
                    PaneMode::Pinned {
                        session_id: "owned".into(),
                    },
                    false,
                    true,
                    cx,
                )
            });
            let monitor = pane.read(cx).exit_monitor.as_ref().unwrap().clone();
            monitor.update(cx, |monitor, cx| {
                monitor._task = cx.spawn(async move |_, _| {})
            });
            pane.update(cx, |pane, cx| pane._poll = cx.spawn(async move |_, _| {}));
            retained = Some(pane.clone());
            gpui_kit::component::Root::new(pane, window, cx)
        });
        let pane = retained.unwrap();
        let (sender, mut receiver) = futures::channel::mpsc::channel(1);
        visual.update(|_, cx| {
            pane.update(cx, |pane, cx| {
                pane.poll_wake = sender;
                assert!(receiver.next().now_or_never().is_none());
                pane.set_visible(true, cx);
                assert_eq!(receiver.next().now_or_never(), Some(Some(())));
            })
        });
        let monitor = visual.update(|_, cx| pane.read(cx).exit_monitor.as_ref().unwrap().clone());
        visual.update(|_, cx| {
            monitor.update(cx, |_, cx| {
                cx.emit(TerminalSessions(Err("Backend disconnected".into())))
            })
        });
        visual.run_until_parked();
        visual.update(|_, cx| {
            assert_eq!(
                pane.read(cx).exit_monitor_error.as_deref(),
                Some("Backend disconnected")
            );
            assert!(!pane.read(cx).exit_reported);
        });
        visual.update(|_, cx| {
            monitor.update(cx, |_, cx| {
                cx.emit(TerminalSessions(Ok(vec![
                    json!({"id":"owned","status":"exited"}),
                ])))
            })
        });
        visual.run_until_parked();
        visual.update(|_, cx| {
            assert!(pane.read(cx).exit_reported);
            assert!(pane.read(cx).exit_monitor_error.is_none());
            assert!(monitor.read(cx).watchers.is_empty());
        });
    }

    #[::core::prelude::v1::test]
    fn terminal_exit_evidence_matches_only_the_pinned_session_and_never_missing_metadata() {
        let mode=PaneMode::Pinned {session_id:"owned".into()};
        for sessions in [vec![],vec![json!({"id":"other","status":"exited"})],vec![json!({"id":"owned","status":"running"})],vec![json!({"id":"owned"})]] {
            assert_eq!(pinned_exit_id(&mode,&sessions),None);
        }
        let sessions=vec![json!({"id":"other","status":"running"}),json!({"id":"owned","status":"exited","exitCode":0})];
        assert_eq!(pinned_exit_id(&mode,&sessions).as_deref(),Some("owned"));
        assert_eq!(pinned_exit_id(&PaneMode::Manager,&sessions),None);
    }
    #[::core::prelude::v1::test]
    fn terminal_exit_notification_is_single_identity_bound_and_never_kills_a_pty() {
        let mode=PaneMode::Pinned {session_id:"owned".into()};
        let mut reported=false;
        let mut queue=VecDeque::from([Operation::Read {id:"owned".into(),after:5},write("owned",b"pending")]);
        let original=queue.len();
        assert!(take_exit_notification(&mode,"other",&mut reported,&mut queue).is_none());
        assert!(take_exit_notification(&PaneMode::Manager,"owned",&mut reported,&mut queue).is_none());
        assert!(!reported);
        assert_eq!(queue.len(),original);
        assert_eq!(take_exit_notification(&mode,"owned",&mut reported,&mut queue),Some(TerminalEvent::Exited {session_id:"owned".into()}));
        assert!(queue.is_empty());
        assert!(take_exit_notification(&mode,"owned",&mut reported,&mut queue).is_none());
        assert!(!queue.iter().any(|op|matches!(op,Operation::Close {..})));
    }
    #[::core::prelude::v1::test]
    fn pinned_initializer_queues_no_list_or_open_and_fences_every_session_operation() {
        let pinned = PaneMode::Pinned {
            session_id: "real-session-a".into(),
        };
        assert!(initial_operations(&pinned).is_empty());
        for operation in [
            Operation::Read {
                id: "real-session-a".into(),
                after: 0,
            },
            write("real-session-a", b"input"),
            Operation::Resize {
                id: "real-session-a".into(),
                cols: 80,
                rows: 24,
            },
        ] {
            assert!(operation_allowed(&pinned, &operation));
        }
        for operation in [
            Operation::List,
            Operation::Open {
                target: Value::Null,
                cols: 80,
                rows: 24,
            },
            Operation::Read {
                id: "real-session-b".into(),
                after: 0,
            },
            write("real-session-b", b"wrong session"),
            Operation::Close {
                id: "real-session-a".into(),
            },
            Operation::Resize {
                id: "real-session-b".into(),
                cols: 80,
                rows: 24,
            },
            Operation::Close {
                id: "real-session-b".into(),
            },
        ] {
            assert!(!operation_allowed(&pinned, &operation));
        }
        assert_eq!(
            initial_operations(&PaneMode::Manager)
                .iter()
                .map(|operation| matches!(operation, Operation::List))
                .collect::<Vec<_>>(),
            [true]
        );
    }

    #[::core::prelude::v1::test]
    fn hidden_pinned_pane_removes_poll_reads_but_keeps_queued_mutations_in_order() {
        let mut queue = VecDeque::from([
            Operation::Read {
                id: "pinned".into(),
                after: 8,
            },
            write("pinned", b"pending input"),
            Operation::Resize {
                id: "pinned".into(),
                cols: 90,
                rows: 30,
            },
            Operation::Close {
                id: "pinned".into(),
            },
        ]);

        remove_queued_reads(&mut queue, Some("pinned"));

        assert_eq!(queue.len(), 3);
        assert!(
            matches!(queue.pop_front(), Some(Operation::Write { id, data }) if id == "pinned" && data.as_slice() == b"pending input")
        );
        assert!(
            matches!(queue.pop_front(), Some(Operation::Resize { id, cols: 90, rows: 30 }) if id == "pinned")
        );
        assert!(matches!(queue.pop_front(), Some(Operation::Close { id }) if id == "pinned"));
        assert!(queue.is_empty());
        assert!(!should_poll(false, false, false));
        assert!(!should_poll(true, true, false));
        assert!(!should_poll(true, false, true));
        assert!(should_poll(true, false, false));
    }

    #[::core::prelude::v1::test]
    fn pinned_session_preflight_accepts_running_or_exited_canonical_sessions_only() {
        let running = PinnedSession::try_from(serde_json::json!({
            "id":"550e8400-e29b-41d4-a716-446655440000",
            "status":"running"
        }))
        .unwrap();
        assert_eq!(running.session_id, "550e8400-e29b-41d4-a716-446655440000");
        let exited = PinnedSession::try_from(serde_json::json!({
            "id":"550e8400-e29b-41d4-a716-446655440000",
            "status":"exited"
        }));
        assert!(exited.is_ok());
        assert!(
            PinnedSession::try_from(serde_json::json!({
                "id":"00000000-0000-0000-0000-000000000000",
                "status":"running"
            }))
            .is_err()
        );
        assert!(
            PinnedSession::try_from(serde_json::json!({
                "id":"550e8400-e29b-41d4-a716-446655440000",
                "status":"restarting"
            }))
            .is_err()
        );
    }

    #[::core::prelude::v1::test]
    fn pinned_initial_read_waits_until_visible_and_reads_exited_scrollback_once() {
        let mode = PaneMode::Pinned {
            session_id: "pinned".into(),
        };
        let mut queue = VecDeque::new();
        let pending = true;

        assert!(!schedule_initial_read(
            &mut queue, &mode, pending, false, false, false, 17,
        ));
        assert!(queue.is_empty());

        assert!(schedule_initial_read(
            &mut queue, &mode, pending, true, false, false, 17,
        ));
        assert!(!schedule_initial_read(
            &mut queue, &mode, pending, true, false, false, 17,
        ));
        assert!(matches!(
            queue.front(),
            Some(Operation::Read { id, after: 17 }) if id == "pinned"
        ));

        // If hidden before dispatch, the pending flag remains set, so a later
        // visibility transition may queue the same first read exactly once.
        remove_queued_reads(&mut queue, Some("pinned"));
        assert!(queue.is_empty());
        assert!(schedule_initial_read(
            &mut queue, &mode, pending, true, false, false, 17,
        ));
        assert_eq!(queue.len(), 1);
    }

    #[::core::prelude::v1::test]
    fn in_flight_list_keeps_a_session_adopted_after_request_start() {
        let mut adopted = HashMap::from([(
            "new-session".to_owned(),
            serde_json::json!({"id":"new-session", "title":"New terminal"}),
        )]);
        let listed = vec![serde_json::json!({"id":"old-session"})];

        let merged = merge_adopted_sessions(listed, &mut adopted);

        assert_eq!(
            merged
                .iter()
                .map(|session| field(session, "id"))
                .collect::<Vec<_>>(),
            ["old-session", "new-session"]
        );
        assert!(adopted.is_empty());
    }

    #[::core::prelude::v1::test]
    fn later_authoritative_list_can_remove_the_one_time_adoption() {
        let mut adopted = HashMap::from([(
            "new-session".to_owned(),
            serde_json::json!({"id":"new-session", "title":"New terminal"}),
        )]);
        let _in_flight =
            merge_adopted_sessions(vec![serde_json::json!({"id":"old-session"})], &mut adopted);

        let refreshed =
            merge_adopted_sessions(vec![serde_json::json!({"id":"old-session"})], &mut adopted);

        assert_eq!(refreshed.len(), 1);
        assert_eq!(field(&refreshed[0], "id"), "old-session");
    }

    fn write(id: &str, bytes: &[u8]) -> Operation {
        Operation::Write {
            id: id.into(),
            data: bytes.into(),
        }
    }
    fn assert_write(operation: Option<Operation>, id: &str, bytes: &[u8]) {
        match operation {
            Some(Operation::Write { id: actual, data }) => {
                assert_eq!(actual, id);
                assert_eq!(data, bytes);
            }
            _ => panic!("expected queued write"),
        }
    }
    #[::core::prelude::v1::test]
    fn fifo_serializes_binary_writes_reads_and_resize_across_visible_session_switch() {
        let mut queue = VecDeque::from([
            write("a", &[0x80, 0xff]),
            Operation::Read {
                id: "a".into(),
                after: 4,
            },
            Operation::Resize {
                id: "a".into(),
                cols: 90,
                rows: 30,
            },
            write("b", b"next"),
        ]);
        assert!(next_operation(&mut queue, true, false, false).is_none());
        assert_eq!(queue.len(), 4);
        assert_write(
            next_operation(&mut queue, false, false, false),
            "a",
            &[0x80, 0xff],
        );
        assert!(
            matches!(next_operation(&mut queue,false,false,false),Some(Operation::Read {id,after:4}) if id=="a")
        );
        assert!(
            matches!(next_operation(&mut queue,false,false,false),Some(Operation::Resize {id,cols:90,rows:30}) if id=="a")
        );
        assert_write(
            next_operation(&mut queue, false, false, false),
            "b",
            b"next",
        );
        assert!(queue.is_empty());
    }
    #[::core::prelude::v1::test]
    fn failure_retains_front_and_only_explicit_retry_releases_it() {
        let mut queue = VecDeque::from([write("a", b"first"), write("a", b"second")]);
        let failed = next_operation(&mut queue, false, false, false).unwrap();
        assert!(retain_failed_operation(&mut queue, failed));
        assert!(next_operation(&mut queue, false, true, false).is_none());
        assert_eq!(queue.len(), 2);
        assert_write(
            next_operation(&mut queue, false, false, false),
            "a",
            b"first",
        );
        assert_write(
            next_operation(&mut queue, false, false, false),
            "a",
            b"second",
        );
    }
    #[::core::prelude::v1::test]
    fn unrelated_close_must_not_implicitly_retry_failed_input() {
        let mut queue = VecDeque::from([
            write("a", b"possibly partial"),
            write("b", b"abandoned"),
            Operation::List,
        ]);
        let paused = queue_explicit_close(&mut queue, "b".into(), true);
        assert!(paused);
        assert!(next_operation(&mut queue, true, paused, true).is_none());
        assert!(
            matches!(next_operation(&mut queue,false,paused,true),Some(Operation::Close{id}) if id=="b")
        );
        assert!(next_operation(&mut queue, false, paused, false).is_none());
        assert_write(
            next_operation(&mut queue, false, false, false),
            "a",
            b"possibly partial",
        );
        assert!(matches!(
            next_operation(&mut queue, false, false, false),
            Some(Operation::List)
        ));
    }
    #[::core::prelude::v1::test]
    fn explicit_close_discards_only_target_and_suppresses_its_inflight_failure() {
        let mut queue = VecDeque::from([
            write("a", b"failed"),
            write("b", b"preserved"),
            Operation::Read {
                id: "a".into(),
                after: 1,
            },
        ]);
        let paused = queue_explicit_close(&mut queue, "a".into(), true);
        assert!(!paused);
        assert!(!retain_failed_operation(
            &mut queue,
            write("a", b"in flight")
        ));
        assert!(
            matches!(next_operation(&mut queue,false,paused,true),Some(Operation::Close{id}) if id=="a")
        );
        assert_write(
            next_operation(&mut queue, false, paused, false),
            "b",
            b"preserved",
        );
        assert!(queue.is_empty());
    }
    #[::core::prelude::v1::test]
    fn failed_close_waits_for_retry_instead_of_spinning() {
        let mut queue = VecDeque::new();
        assert!(queue_explicit_close(&mut queue, "b".into(), true));
        let closing = next_operation(&mut queue, false, true, true).unwrap();
        assert!(retain_failed_operation(&mut queue, closing));
        // The pane consumes the close bypass when dispatching it.
        assert!(next_operation(&mut queue, false, true, false).is_none());
        assert!(
            matches!(next_operation(&mut queue,false,false,false),Some(Operation::Close {id}) if id=="b")
        );
    }
    #[::core::prelude::v1::test]
    fn sgr_mouse_has_exact_modifiers_coordinates_and_release() {
        let point = CellPoint { row: 40, col: 120 };
        let modifiers = Modifiers {
            control: true,
            alt: true,
            ..Modifiers::default()
        };
        assert_eq!(
            mouse_bytes(
                vt100::MouseProtocolMode::PressRelease,
                vt100::MouseProtocolEncoding::Sgr,
                MouseKind::Press,
                Some(0),
                point,
                modifiers
            )
            .unwrap(),
            b"\x1b[<24;121;41M"
        );
        assert_eq!(
            mouse_bytes(
                vt100::MouseProtocolMode::PressRelease,
                vt100::MouseProtocolEncoding::Sgr,
                MouseKind::Release,
                Some(2),
                point,
                Modifiers::default()
            )
            .unwrap(),
            b"\x1b[<2;121;41m"
        );
    }
    #[::core::prelude::v1::test]
    fn legacy_mouse_keeps_raw_coordinate_bytes_and_mode_filters() {
        let point = CellPoint { row: 40, col: 120 };
        assert_eq!(
            mouse_bytes(
                vt100::MouseProtocolMode::Press,
                vt100::MouseProtocolEncoding::Default,
                MouseKind::Press,
                Some(0),
                point,
                Modifiers::default()
            )
            .unwrap(),
            vec![27, b'[', b'M', 32, 153, 73]
        );
        assert!(
            mouse_bytes(
                vt100::MouseProtocolMode::Press,
                vt100::MouseProtocolEncoding::Default,
                MouseKind::Release,
                Some(0),
                point,
                Modifiers::default()
            )
            .is_none()
        );
        assert!(
            mouse_bytes(
                vt100::MouseProtocolMode::ButtonMotion,
                vt100::MouseProtocolEncoding::Sgr,
                MouseKind::Motion,
                None,
                point,
                Modifiers::default()
            )
            .is_none()
        );
        assert!(
            mouse_bytes(
                vt100::MouseProtocolMode::AnyMotion,
                vt100::MouseProtocolEncoding::Sgr,
                MouseKind::Motion,
                None,
                point,
                Modifiers {
                    shift: true,
                    ..Modifiers::default()
                }
            )
            .is_none()
        );
    }
    #[::core::prelude::v1::test]
    fn selection_is_direction_independent_and_end_exclusive() {
        let selection = Selection {
            anchor: CellPoint { row: 1, col: 4 },
            end: CellPoint { row: 0, col: 2 },
            text: String::new(),
        };
        assert!(!selection.contains(0, 1));
        assert!(selection.contains(0, 2));
        assert!(selection.contains(1, 3));
        assert!(!selection.contains(1, 4));
        let mut model = TerminalModel::new(2, 10);
        model.parser.process(b"abcdef\r\nghijkl");
        assert_eq!(
            model.parser.screen().contents_between(0, 2, 1, 4),
            "cdef\nghij"
        );
    }
    #[::core::prelude::v1::test]
    fn display_settings_and_palette_colors_are_native_inputs() {
        let appearance = Appearance::from_snapshot(
            &json!({"settings":{"terminalFont":"Test Mono","terminalFontSize":18,"terminalFontWeight":500,"terminalLineHeight":1.5}}),
        );
        assert_eq!(appearance.family, "Test Mono");
        assert_eq!(appearance.size, 18.);
        assert_eq!(appearance.weight, 500.);
        crate::design::set_interface_scale(100);
        assert_eq!(appearance.cell_height(), 38.);
        crate::design::set_interface_scale(125);
        assert_eq!(appearance.cell_height(), 47.5);
        crate::design::set_interface_scale(crate::design::INTERFACE_SCALE_DEFAULT);
        assert_eq!(color(vt100::Color::Idx(196), 0, &PALETTES[0]), 0xff0000);
        assert_eq!(color(vt100::Color::Idx(255), 0, &PALETTES[0]), 0xeeeeee);
        assert_eq!(
            color(vt100::Color::Default, PALETTES[0].foreground, &PALETTES[0]),
            PALETTES[0].foreground
        );
    }
}

#[cfg(all(test,feature="visual-export"))]
#[path="terminal_exit_tests.rs"]
mod terminal_exit_tests;
