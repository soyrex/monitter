//! Pure Neovim UI redraw model.
//!
//! This module is the rendering-free authority for the Neovim UI protocol
//! emitted by `nvim_ui_attach` calls (see `runtime/doc/api-ui-events.txt`
//! in Neovim). It accepts a batched `serde_json::Value` "redraw" payload and
//! produces a small, allocation-bounded snapshot suitable for any UI backend
//! (GPUI here; SvelteKit elsewhere) to read. There are no I/O, no async, no
//! GPUI/GPUI-Kit dependencies: the file is intentionally a pure data model so
//! it can be unit-tested in isolation and reused by the renderer and tests.
//!
//! Coverage of the protocol:
//! - `ext_linegrid`: `grid_resize`, `grid_line`, `grid_clear`,
//!   `grid_destroy`, `grid_cursor_goto`, `grid_scroll`.
//! - `ext_multigrid`: `win_pos`, `win_float_pos`, `win_hide`, `win_close`.
//! - `ext_cmdline`: `cmdline_show`, `cmdline_pos`, `cmdline_hide`,
//!   `cmdline_special_char`, `cmdline_block_show`, `cmdline_block_append`,
//!   `cmdline_block_hide`.
//! - `ext_popupmenu`: `popupmenu_show`, `popupmenu_select`,
//!   `popupmenu_hide`.
//! - `ext_messages`: `msg_show`, `msg_clear`, `msg_showmode`,
//!   `msg_showcmd`, `msg_ruler`, `msg_history_show`.
//! - Highlights and defaults: `hl_attr_define`, `default_colors_set`,
//!   `hl_group_set`, plus mode state from `mode_change`.
//!
//! Bounded allocation: dimensions and per-event counts are clamped against
//! `MAX_*` constants. The model never panics on hostile or oversized input.
use serde_json::Value;
use std::collections::HashMap;

/// Maximum columns a single grid may declare.
pub const MAX_GRID_COLS: usize = 1024;
/// Maximum rows a single grid may declare.
pub const MAX_GRID_ROWS: usize = 1024;
/// Maximum simultaneously tracked grids.
pub const MAX_GRIDS: usize = 64;
/// Aggregate cells the model can hold across all live grids in one connection.
/// `64 * 1024 * 1024` is 67M cells; real UIs use far less. We cap the running
/// total so a hostile or pathological Neovim can't OOM the renderer by
/// spawning many large grids (e.g. via `win_float_pos`).
pub const MAX_TOTAL_GRID_CELLS: usize = 2 * 1024 * 1024;
/// Maximum highlight id the model will retain.
pub const MAX_HL_ID: u64 = 1 << 16;
/// Maximum nested cmdline levels.
pub const MAX_CMDLINE_LEVELS: usize = 8;
/// Maximum chunks inside a single cmdline_show payload.
pub const MAX_CMDLINE_CHUNKS: usize = 1024;
/// Maximum stored visible messages.
pub const MAX_MESSAGES: usize = 256;
/// Maximum popupmenu items in a single show.
pub const MAX_POPUP_ITEMS: usize = 256;
/// Maximum cells inside a single `grid_line` payload.
pub const MAX_CELLS_PER_LINE: usize = 4096;
/// Maximum decoded byte budget per single redraw batch.
pub const MAX_EVENT_BYTES: usize = 4 * 1024 * 1024;
/// Maximum bytes retained per styled text chunk (cmdline / message /
/// showmode / showcmd / ruler). Neovim emits multi-line text; 64 KiB is
/// enough for any plausible screenful and rejects hostile payloads.
pub const MAX_CHUNK_TEXT_BYTES: usize = 64 * 1024;

/// Truncate a styled text chunk so the renderer cannot be asked to retain
/// arbitrarily long strings. Returns the input by value when it is within
/// the budget; otherwise the truncated prefix with a `…` ellipsis appended
/// so consumers can see the chunk was clipped.
fn cap_chunk_text(text: &str) -> String {
    if text.len() <= MAX_CHUNK_TEXT_BYTES {
        return text.to_owned();
    }
    // Find a safe UTF-8 boundary at or below the limit so we never split
    // a code point mid-byte.
    let mut cut = MAX_CHUNK_TEXT_BYTES;
    while cut > 0 && !text.is_char_boundary(cut) {
        cut -= 1;
    }
    let mut out = String::with_capacity(cut + 3);
    out.push_str(&text[..cut]);
    out.push('…');
    out
}

/// Estimate the JSON encoding length of a value without allocating. Used
/// to early-reject pathological batches without serializing to a string.
fn json_byte_size(v: &Value) -> usize {
    match v {
        Value::Null => 4,
        Value::Bool(_) => 5,
        Value::Number(n) => n.to_string().len(),
        Value::String(s) => s.len() + 2,
        Value::Array(a) => {
            let mut n = 2; // []
            for item in a {
                n += json_byte_size(item) + 1; // comma
            }
            n
        }
        Value::Object(o) => {
            let mut n = 2; // {}
            for (k, v) in o {
                n += k.len() + 2 + json_byte_size(v) + 1; // "key":v,
            }
            n
        }
    }
}

/// A single rendered cell on a grid.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Cell {
    /// UTF-8 text occupying the cell. For double-width chars the trailing
    /// cell is the empty string.
    pub text: String,
    /// Highlight attribute id (defined by `hl_attr_define`); `0` means default.
    pub hl_id: u64,
}

/// One logical grid (typically one Neovim window). Stored as a flat cell
/// buffer sized `rows * cols`, row-major, so scroll copies are contiguous.
#[derive(Clone, Debug)]
pub struct Grid {
    pub cols: u16,
    pub rows: u16,
    pub cells: Vec<Cell>,
}

/// Position/size information for a window.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WindowPos {
    /// Grid backing this window.
    pub grid: i64,
    /// Neovim window handle.
    pub win: i64,
    /// Top-left row on the parent grid (or screen, for floats).
    pub start_row: i64,
    /// Top-left column on the parent grid (or screen, for floats).
    pub start_col: i64,
    pub width: i64,
    pub height: i64,
    /// Hidden via `win_hide`; grid is retained.
    pub hidden: bool,
    /// Last positioned via `win_float_pos`.
    pub floating: bool,
    pub anchor_grid: i64,
    pub zindex: i64,
}

/// Resolved highlight attributes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Highlight {
    pub fg: Option<i64>,
    pub bg: Option<i64>,
    pub sp: Option<i64>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub reverse: bool,
}

/// A single styled chunk of a cmdline or message.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CmdlineChunk {
    pub text: String,
    pub hl_id: u64,
}

/// One nested cmdline level.
#[derive(Clone, Debug, Default)]
pub struct Cmdline {
    pub content: Vec<CmdlineChunk>,
    pub prompt: String,
    pub firstc: String,
    /// Cursor byte position within the concatenated content.
    pub pos: usize,
    pub indent: u16,
}

/// A single completion item presented in the popupmenu.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PopupItem {
    pub word: String,
    pub kind: String,
    pub menu: String,
    pub info: String,
}

/// Popupmenu state. `visible == false` after `popupmenu_hide`.
#[derive(Clone, Debug, Default)]
pub struct Popupmenu {
    pub items: Vec<PopupItem>,
    pub selected: i64,
    pub row: i64,
    pub col: i64,
    /// Anchor grid; `-1` when anchored to the external cmdline.
    pub grid: i64,
    pub visible: bool,
}

/// A message emitted by `msg_show`. Chunks retain their hl_id; the renderer
/// concatenates `text` and breaks on `\n`.
#[derive(Clone, Debug, Default)]
pub struct Message {
    pub kind: String,
    pub chunks: Vec<MessageChunk>,
    pub id: i64,
    pub replace_last: bool,
}

/// Alias kept for cmdline/msg_render readability.
pub type MessageChunk = CmdlineChunk;

/// Last cursor position declared by `grid_cursor_goto`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Cursor {
    pub grid: i64,
    pub row: i64,
    pub col: i64,
}

/// Pure state of a Neovim UI connection.
///
/// `apply_redraw` advances the model with the contents of one `"redraw"`
/// notification and reports whether the batch's `flush` event was observed at
/// the end. Renderers should consume the model after a true return value.
#[derive(Default, Debug)]
pub struct NeovimModel {
    /// Live grids keyed by grid id. Grid `1` is always the global grid.
    pub grids: HashMap<i64, Grid>,
    /// Running sum of `cells.len()` across all live grids. Used to reject
    /// new grid allocations once `MAX_TOTAL_GRID_CELLS` is reached so the
    /// renderer can never be asked to hold more than the cap.
    pub grid_cell_total: usize,
    /// Windows keyed by Neovim window handle.
    pub windows: HashMap<i64, WindowPos>,
    /// Reverse map grid → window for cursor lookups.
    pub window_for_grid: HashMap<i64, i64>,
    /// Highlight table keyed by hl_id. `0` is the default.
    pub highlights: HashMap<u64, Highlight>,
    pub default_fg: Option<i64>,
    pub default_bg: Option<i64>,
    pub default_sp: Option<i64>,
    /// Last cursor seen via `grid_cursor_goto`.
    pub cursor: Option<Cursor>,
    /// Mode name from `mode_change` (`"normal"`, `"insert"`, …).
    pub mode: String,
    /// Mode index into the array from `mode_info_set`.
    pub mode_idx: i64,
    /// Nested cmdlines keyed by level (1..).
    pub cmdlines: HashMap<i64, Cmdline>,
    /// Whether the cmdline area is visible (any level present).
    pub cmdline_visible: bool,
    pub popupmenu: Option<Popupmenu>,
    /// Visible messages, oldest first; bounded by `MAX_MESSAGES`.
    pub messages: Vec<Message>,
    pub showmode: Vec<MessageChunk>,
    pub showcmd: Vec<MessageChunk>,
    pub ruler: Vec<MessageChunk>,
    pub title: String,
    pub icon: String,
    /// True once the most recent `apply_redraw` saw a terminal `flush`.
    pub flushed: bool,
}

impl NeovimModel {
    /// Construct an empty model.
    pub fn new() -> Self {
        Self::default()
    }

    /// Drop all grids, windows, highlights and state. Keeps the public
    /// allocations in place so a freshly attached Neovim does not see stale
    /// data on the first `grid_resize`.
    pub fn reset(&mut self) {
        self.grids.clear();
        self.grid_cell_total = 0;
        self.windows.clear();
        self.window_for_grid.clear();
        self.highlights.clear();
        self.default_fg = None;
        self.default_bg = None;
        self.default_sp = None;
        self.cursor = None;
        self.mode.clear();
        self.mode_idx = 0;
        self.cmdlines.clear();
        self.cmdline_visible = false;
        self.popupmenu = None;
        self.messages.clear();
        self.showmode.clear();
        self.showcmd.clear();
        self.ruler.clear();
        self.title.clear();
        self.icon.clear();
        self.flushed = false;
    }

    /// Borrow a grid by id.
    pub fn grid(&self, id: i64) -> Option<&Grid> {
        self.grids.get(&id)
    }

    /// Mutable borrow of a grid by id.
    pub fn grid_mut(&mut self, id: i64) -> Option<&mut Grid> {
        self.grids.get_mut(&id)
    }

    /// Apply one redraw batch. `events` is the array passed as the single
    /// argument to the `redraw` RPC notification (a flat array of event
    /// tuples). Returns `true` if the batch terminated with a `flush` event
    /// (i.e. the model is in a consistent state and ready to render).
    pub fn apply_redraw(&mut self, events: &Value) -> bool {
        self.flushed = false;
        let arr = match events.as_array() {
            Some(a) => a,
            None => return false,
        };
        // Cheap total-size guard; reject pathological batches early by walking
        // the JSON tree once. `events.to_string()` allocates a fresh buffer
        // which we avoid here for very large batches.
        if json_byte_size(events) > MAX_EVENT_BYTES {
            return self.flushed;
        }
        let mut saw_flush = false;
        for ev in arr {
            let tup = match ev.as_array() {
                Some(t) => t,
                None => continue,
            };
            if tup.is_empty() {
                continue;
            }
            let name = match tup[0].as_str() {
                Some(s) => s,
                None => continue,
            };
            // Real Neovim UI protocol: after the event name, the payload is a
            // sequence of argument arrays, one per logical invocation. The
            // same event may carry several (e.g. `grid_resize` for every grid
            // in one round, `grid_line` for every scrolled-in row). Walk each
            // trailing element and dispatch once per argument array. The
            // sub-handlers already accept the per-invocation argument slice.
            for sig in &tup[1..] {
                let sig_arr = match sig.as_array() {
                    Some(a) => a,
                    None => continue,
                };
                match name {
                    // ---- Grid sizing/lifecycle ---------------------------
                    "grid_resize" => self.on_grid_resize(sig_arr),
                    "grid_clear" => self.on_grid_clear(sig_arr),
                    "grid_destroy" => self.on_grid_destroy(sig_arr),
                    "grid_line" => self.on_grid_line(sig_arr),
                    "grid_scroll" => self.on_grid_scroll(sig_arr),
                    "grid_cursor_goto" => self.on_grid_cursor_goto(sig_arr),

                    // ---- Multigrid window positioning -------------------
                    "win_pos" => self.on_win_pos(sig_arr),
                    "win_float_pos" => self.on_win_float_pos(sig_arr),
                    "win_external_pos" => self.on_win_external_pos(sig_arr),
                    "win_hide" => self.on_win_hide(sig_arr),
                    "win_close" => self.on_win_close(sig_arr),

                    // ---- Highlights and defaults -----------------------
                    "default_colors_set" => self.on_default_colors_set(sig_arr),
                    "hl_attr_define" => self.on_hl_attr_define(sig_arr),
                    "hl_group_set" => self.on_hl_group_set(sig_arr),

                    // ---- Mode / global ---------------------------------
                    "mode_change" => self.on_mode_change(sig_arr),
                    "set_title" => self.on_set_title(sig_arr),
                    "set_icon" => self.on_set_icon(sig_arr),
                    "busy_start" | "busy_stop" | "mouse_on" | "mouse_off" | "bell"
                    | "visual_bell" | "suspend" | "update_menu" => { /* renderer-side */ }

                    // ---- Cmdline (external) ----------------------------
                    "cmdline_show" => self.on_cmdline_show(sig_arr),
                    "cmdline_pos" => self.on_cmdline_pos(sig_arr),
                    "cmdline_hide" => self.on_cmdline_hide(sig_arr),
                    "cmdline_special_char" => self.on_cmdline_special_char(sig_arr),
                    "cmdline_block_show" => self.on_cmdline_block_show(sig_arr),
                    "cmdline_block_append" => self.on_cmdline_block_append(sig_arr),
                    "cmdline_block_hide" => self.on_cmdline_block_hide(sig_arr),

                    // ---- Popupmenu -------------------------------------
                    "popupmenu_show" => self.on_popupmenu_show(sig_arr),
                    "popupmenu_select" => self.on_popupmenu_select(sig_arr),
                    "popupmenu_hide" => self.on_popupmenu_hide(sig_arr),

                    // ---- Messages ---------------------------------------
                    "msg_show" => self.on_msg_show(sig_arr),
                    "msg_clear" => self.on_msg_clear(sig_arr),
                    "msg_showmode" => self.on_showmode(sig_arr),
                    "msg_showcmd" => self.on_showcmd(sig_arr),
                    "msg_ruler" => self.on_ruler(sig_arr),
                    "msg_history_show" => { /* renderer-side history */ }
                    "msg_set_pos" => { /* multigrid message grid positioning */ }

                    // ---- Terminal ---------------------------------------
                    "flush" => saw_flush = true,

                    // ---- Unknown / future -------------------------------
                    _ => {}
                }
            }
        }
        self.flushed = saw_flush;
        saw_flush
    }

    // ---------------------------------------------------------------------
    // Internal helpers
    // ---------------------------------------------------------------------

    fn clamp_dim(v: i64, hi: usize) -> u16 {
        let v = v.max(0) as usize;
        v.min(hi).max(1) as u16
    }
    fn ensure_grid(&mut self, id: i64, cols: u16, rows: u16) {
        if self.grids.contains_key(&id) {
            return;
        }
        if self.grids.len() >= MAX_GRIDS {
            return;
        }
        let needed = (cols as usize) * (rows as usize);
        // Cap aggregate allocation across the connection so a hostile /
        // pathological Neovim that keeps resizing and floating new windows
        // cannot push the renderer past the budget. Cells we already pay
        // for stay allocated; we only refuse to grow further.
        if self
            .grid_cell_total
            .checked_add(needed)
            .map_or(false, |sum| sum <= MAX_TOTAL_GRID_CELLS)
        {
            let cells = vec![Cell::default(); needed];
            self.grid_cell_total += needed;
            self.grids.insert(id, Grid { cols, rows, cells });
        } else {
            // Track the grid as a tombstone (no cells) so subsequent
            // grid_line / grid_scroll events can detect "no buffer" and
            // skip rather than allocate lazily.
            self.grids.insert(
                id,
                Grid {
                    cols,
                    rows,
                    cells: Vec::new(),
                },
            );
        }
    }

    fn resize_grid_in_place(&mut self, id: i64, cols: u16, rows: u16) {
        // Preserve as much existing content as the new shape permits: copy
        // the minimum overlapping rectangle, fill the rest with default cells.
        let old = match self.grids.remove(&id) {
            Some(g) => g,
            None => return,
        };
        let old_cols = old.cols as usize;
        let old_total = old.cells.len();
        let new_cols = cols as usize;
        let new_rows = rows as usize;
        let needed = new_cols * new_rows;
        let copy_cols = old_cols.min(new_cols);
        let copy_rows = (old.rows as usize).min(new_rows);
        // A grid is either exactly rectangular or a tombstone. Keeping the
        // old length on shrink made the renderer reject otherwise valid grids.
        let room =
            MAX_TOTAL_GRID_CELLS.saturating_sub(self.grid_cell_total.saturating_sub(old_total));
        let mut cells = vec![Cell::default(); if needed <= room { needed } else { 0 }];
        if !cells.is_empty() && old_total == old_cols * old.rows as usize {
            for r in 0..copy_rows {
                let src = &old.cells[r * old_cols..r * old_cols + copy_cols];
                let dst = &mut cells[r * new_cols..r * new_cols + copy_cols];
                dst.clone_from_slice(src);
            }
        }
        // Update running total: subtract old cells, add new cells.
        self.grid_cell_total = self
            .grid_cell_total
            .saturating_sub(old_total)
            .saturating_add(cells.len());
        self.grids.insert(id, Grid { cols, rows, cells });
    }

    fn on_grid_resize(&mut self, args: &[Value]) {
        let (Some(grid), Some(w), Some(h)) = (
            args.first().and_then(Value::as_i64),
            args.get(1).and_then(Value::as_i64),
            args.get(2).and_then(Value::as_i64),
        ) else {
            return;
        };
        let cols = Self::clamp_dim(w, MAX_GRID_COLS);
        let rows = Self::clamp_dim(h, MAX_GRID_ROWS);
        if self.grids.contains_key(&grid) {
            self.resize_grid_in_place(grid, cols, rows);
        } else if self.grids.len() < MAX_GRIDS {
            self.ensure_grid(grid, cols, rows);
        }
    }

    fn on_grid_clear(&mut self, args: &[Value]) {
        let Some(grid) = args.first().and_then(Value::as_i64) else {
            return;
        };
        if let Some(g) = self.grids.get_mut(&grid) {
            for c in &mut g.cells {
                c.text.clear();
                c.hl_id = 0;
            }
        }
    }

    fn on_grid_destroy(&mut self, args: &[Value]) {
        let Some(grid) = args.first().and_then(Value::as_i64) else {
            return;
        };
        if let Some(g) = self.grids.remove(&grid) {
            self.grid_cell_total = self.grid_cell_total.saturating_sub(g.cells.len());
        }
        // Drop the window↔grid association, but keep the window record
        // (Neovim may re-attach a new grid via `win_float_pos`).
        self.window_for_grid.remove(&grid);
    }

    fn on_grid_line(&mut self, args: &[Value]) {
        // Per Neovim's UI protocol the `grid_line` event is bundled so a
        // single notification can carry every row that just scrolled in.
        // The dispatcher in `apply_redraw` already walks `tup[1..]` and
        // hands one row tuple per invocation — so `args` here is exactly
        // one row tuple of shape `[grid, row, col_start, cells, wrap]`.
        if args.len() < 4 {
            return;
        }
        let (Some(grid_id), Some(row), Some(col_start)) =
            (args[0].as_i64(), args[1].as_i64(), args[2].as_i64())
        else {
            return;
        };
        let cells_arr = match args[3].as_array() {
            Some(a) => a,
            None => return,
        };
        if !self.grids.contains_key(&grid_id) {
            return;
        }
        let cols = self.grids[&grid_id].cols;
        let rows = self.grids[&grid_id].rows;
        // A grid kept as a tombstone (no cells) due to the aggregate
        // cap cannot store anything; skip the row write silently.
        if self.grids[&grid_id].cells.is_empty() {
            return;
        }
        if row < 0 || row >= i64::from(rows) || col_start < 0 || col_start >= i64::from(cols) {
            return;
        }
        let mut current_hl: u64 = 0;
        let mut col = col_start as usize;
        let mut count = 0usize;
        for entry in cells_arr {
            if count >= MAX_CELLS_PER_LINE {
                break;
            }
            count += 1;
            let entry = match entry.as_array() {
                Some(e) => e,
                None => continue,
            };
            if entry.is_empty() {
                continue;
            }
            let text = entry[0].as_str().unwrap_or("").to_owned();
            let hl = if let Some(h) = entry.get(1).and_then(Value::as_u64) {
                if h <= MAX_HL_ID {
                    current_hl = h;
                }
                h
            } else {
                current_hl
            };
            let repeat = entry.get(2).and_then(Value::as_i64).unwrap_or(1).max(1) as usize;
            if hl > MAX_HL_ID {
                continue;
            }
            // Advance past any out-of-bounds columns by clamping the
            // first write position; subsequent repeats are clipped too.
            for step in 0..repeat {
                let target_col = col.saturating_add(step);
                if target_col >= cols as usize {
                    break;
                }
                let idx = (row as usize) * (cols as usize) + target_col;
                let cell = &mut self.grids.get_mut(&grid_id).unwrap().cells[idx];
                cell.text.clear();
                cell.text.push_str(&text);
                cell.hl_id = hl.min(MAX_HL_ID);
            }
            // A double-width char's empty trailing cell is represented
            // here by `text == ""`; the renderer treats `text.is_empty()`
            // after a wide char as a half-cell. We simply record it.
            col = col.saturating_add(repeat);
        }
    }

    fn on_grid_scroll(&mut self, args: &[Value]) {
        // Per Neovim's UI protocol the `grid_scroll` event is bundled so a
        // single notification can carry several contiguous scrolls. The
        // dispatcher in `apply_redraw` already walks `tup[1..]` and hands
        // one scroll tuple per invocation — so `args` here is exactly one
        // scroll tuple of shape `[grid, top, bot, left, right, rows, cols]`.
        if args.len() < 7 {
            return;
        }
        let (Some(grid_id), Some(top), Some(bot), Some(left), Some(right), Some(rows), Some(cols)) = (
            args[0].as_i64(),
            args[1].as_i64(),
            args[2].as_i64(),
            args[3].as_i64(),
            args[4].as_i64(),
            args[5].as_i64(),
            args[6].as_i64(),
        ) else {
            return;
        };
        let grid = match self.grids.get(&grid_id) {
            Some(g) => g.clone(),
            None => return,
        };
        let mut new_cells = grid.cells.clone();
        Self::scroll_region(
            &mut new_cells,
            grid.cols,
            grid.rows,
            top,
            bot,
            left,
            right,
            rows,
            cols,
        );
        if let Some(g) = self.grids.get_mut(&grid_id) {
            g.cells = new_cells;
        }
    }

    /// Apply a `grid_scroll` rectangle copy in place. Ranges are half-open
    /// (top/bot/right exclusive), consistent with `api-ui-events.txt`.
    fn scroll_region(
        cells: &mut [Cell],
        cols: u16,
        rows: u16,
        top: i64,
        bot: i64,
        left: i64,
        right: i64,
        scroll_rows: i64,
        scroll_cols: i64,
    ) {
        let cols = cols as usize;
        let rows = rows as usize;
        let top = top.max(0) as usize;
        let bot = bot.max(0) as usize;
        let left = left.max(0) as usize;
        let right = right.max(0) as usize;
        if top >= bot || left >= right || top >= rows || bot > rows || left >= cols || right > cols
        {
            return;
        }
        let sr = scroll_rows;
        let sc = scroll_cols;
        if sr == 0 && sc == 0 {
            return;
        }
        let height = bot - top;
        let width = right - left;
        // For each cell in the dst rectangle, take from src (dst - sr).
        // `cols == 0` is reserved for the future; only `rows` is in use.
        if sr != 0 {
            let n = sr.unsigned_abs() as usize;
            if n >= height {
                // Whole region is overwritten — leave it for the
                // immediately-following grid_line events that fill the
                // newly exposed rows.
                return;
            }
            // `copy_within` handles overlapping regions in either direction
            // because the destination range is always strictly above (sr>0)
            // or strictly below (sr<0) the source range.
            if sr > 0 {
                // Move up: dst rows [top, bot-n) <- src rows [top+n, bot).
                // dst row r receives src row r+n. Read and write row sets are
                // disjoint, so a forward clone copy is safe in place.
                for r in top..(bot - n) {
                    let src_row_start = (r + n) * cols + left;
                    let dst_row_start = r * cols + left;
                    for i in 0..width {
                        cells[dst_row_start + i] = cells[src_row_start + i].clone();
                    }
                }
            } else {
                // Move down: dst rows [top+n, bot) <- src rows [top, bot-n).
                // dst row r receives src row r-n. Reading a source that was not
                // yet overwritten requires descending destination order.
                for r in ((top + n)..bot).rev() {
                    let src_row_start = (r - n) * cols + left;
                    let dst_row_start = r * cols + left;
                    for i in 0..width {
                        cells[dst_row_start + i] = cells[src_row_start + i].clone();
                    }
                }
            }
        }
    }

    fn on_grid_cursor_goto(&mut self, args: &[Value]) {
        let (Some(grid), Some(row), Some(col)) = (
            args.first().and_then(Value::as_i64),
            args.get(1).and_then(Value::as_i64),
            args.get(2).and_then(Value::as_i64),
        ) else {
            return;
        };
        self.cursor = Some(Cursor { grid, row, col });
    }

    fn on_win_pos(&mut self, args: &[Value]) {
        if args.len() < 6 {
            return;
        }
        let (grid, win, sr, sc, w, h) = (
            args[0].as_i64().unwrap_or(0),
            args[1].as_i64().unwrap_or(0),
            args[2].as_i64().unwrap_or(0),
            args[3].as_i64().unwrap_or(0),
            args[4].as_i64().unwrap_or(0),
            args[5].as_i64().unwrap_or(0),
        );
        let entry = self.windows.entry(win).or_default();
        entry.grid = grid;
        entry.win = win;
        entry.start_row = sr;
        entry.start_col = sc;
        entry.width = w;
        entry.height = h;
        entry.hidden = false;
        entry.floating = false;
        self.window_for_grid.insert(grid, win);
    }

    fn on_win_float_pos(&mut self, args: &[Value]) {
        // Args: grid, win, anchor, anchor_grid, anchor_row, anchor_col,
        // mouse_enabled, zindex, compindex, screen_row, screen_col.

        if args.len() < 2 {
            return;
        }
        let grid = args[0].as_i64().unwrap_or(0);
        let win = args[1].as_i64().unwrap_or(0);
        // `screen_row` / `screen_col` are the resolved position when Nvim
        // handled placement; if the UI is told to place manually via
        // `anchor` + `anchor_col`, the values may be unused by Nvim and the
        // renderer should consult `anchor_*` instead. We retain whichever
        // is present so the renderer can choose.
        let screen_row = args.get(9).and_then(Value::as_i64).unwrap_or(0);
        let screen_col = args.get(10).and_then(Value::as_i64).unwrap_or(0);
        // Width/height are not part of win_float_pos; they come from the
        // grid's most recent grid_resize. The renderer reads them from
        // the matching grid record.
        let entry = self.windows.entry(win).or_default();
        entry.grid = grid;
        entry.win = win;
        entry.start_row = screen_row;
        entry.start_col = screen_col;
        entry.hidden = false;
        entry.floating = true;
        entry.anchor_grid = args.get(3).and_then(Value::as_i64).unwrap_or(1);
        entry.zindex = args.get(7).and_then(Value::as_i64).unwrap_or(50);
        self.window_for_grid.insert(grid, win);
    }

    fn on_win_external_pos(&mut self, args: &[Value]) {
        if args.len() < 2 {
            return;
        }
        let grid = args[0].as_i64().unwrap_or(0);
        let win = args[1].as_i64().unwrap_or(0);
        let entry = self.windows.entry(win).or_default();
        entry.grid = grid;
        entry.win = win;
        entry.hidden = false;
        self.window_for_grid.insert(grid, win);
    }

    fn on_win_hide(&mut self, args: &[Value]) {
        let Some(grid) = args.first().and_then(Value::as_i64) else {
            return;
        };
        if let Some(win) = self.window_for_grid.get(&grid).copied() {
            if let Some(w) = self.windows.get_mut(&win) {
                w.hidden = true;
            }
        }
    }

    fn on_win_close(&mut self, args: &[Value]) {
        let Some(grid) = args.first().and_then(Value::as_i64) else {
            return;
        };
        if let Some(win) = self.window_for_grid.remove(&grid) {
            self.windows.remove(&win);
        }
    }

    fn on_default_colors_set(&mut self, args: &[Value]) {
        // `-1` is the sentinel for "unset" when `ext_termcolors` is enabled.

        if let Some(fg) = args.first().and_then(Value::as_i64) {
            self.default_fg = if fg >= 0 { Some(fg) } else { None };
        }
        if let Some(bg) = args.get(1).and_then(Value::as_i64) {
            self.default_bg = if bg >= 0 { Some(bg) } else { None };
        }
        if let Some(sp) = args.get(2).and_then(Value::as_i64) {
            self.default_sp = if sp >= 0 { Some(sp) } else { None };
        }
    }

    fn on_hl_attr_define(&mut self, args: &[Value]) {
        if args.len() < 2 {
            return;
        }
        let Some(id) = args[0].as_u64() else { return };
        if id > MAX_HL_ID {
            return;
        }
        let attr = match args[1].as_object() {
            Some(o) => o,
            None => return,
        };
        let hl = Highlight {
            fg: attr.get("fg").and_then(Value::as_i64).filter(|v| *v >= 0),
            bg: attr.get("bg").and_then(Value::as_i64).filter(|v| *v >= 0),
            sp: attr.get("sp").and_then(Value::as_i64).filter(|v| *v >= 0),
            bold: attr.get("bold").and_then(Value::as_bool).unwrap_or(false),
            italic: attr.get("italic").and_then(Value::as_bool).unwrap_or(false),
            underline: attr
                .get("underline")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            reverse: attr
                .get("reverse")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        };
        self.highlights.insert(id, hl);
    }

    fn on_hl_group_set(&mut self, _args: &[Value]) {
        // Maps group names → hl_ids; useful for the renderer (e.g. Pmenu).
        // Stored highlights suffice; no model state required here.
    }

    fn on_mode_change(&mut self, args: &[Value]) {
        if let Some(m) = args.first().and_then(Value::as_str) {
            self.mode = m.to_owned();
        }
        if let Some(i) = args.get(1).and_then(Value::as_i64) {
            self.mode_idx = i;
        }
    }

    fn on_set_title(&mut self, args: &[Value]) {
        if let Some(s) = args.first().and_then(Value::as_str) {
            self.title = s.to_owned();
        }
    }

    fn on_set_icon(&mut self, args: &[Value]) {
        if let Some(s) = args.first().and_then(Value::as_str) {
            self.icon = s.to_owned();
        }
    }

    // ----- Cmdline --------------------------------------------------------

    fn on_cmdline_show(&mut self, args: &[Value]) {
        if args.len() < 7 {
            return;
        }
        let level = match args.get(5).and_then(Value::as_i64) {
            Some(l) if l >= 1 && (l as usize) <= MAX_CMDLINE_LEVELS => l,
            _ => return,
        };
        let mut chunks: Vec<CmdlineChunk> = Vec::new();
        if let Some(arr) = args.first().and_then(Value::as_array) {
            for entry in arr.iter().take(MAX_CMDLINE_CHUNKS) {
                let cu = match entry.as_array() {
                    Some(cu) => cu,
                    None => continue,
                };
                if cu.len() < 3 {
                    continue;
                }
                let text = cap_chunk_text(cu[1].as_str().unwrap_or(""));
                let hl = cu[2].as_u64().unwrap_or(0).min(MAX_HL_ID);
                chunks.push(CmdlineChunk { text, hl_id: hl });
            }
        }
        let pos = args.get(1).and_then(Value::as_i64).unwrap_or(0).max(0) as usize;
        let firstc = args.get(2).and_then(Value::as_str).unwrap_or("").to_owned();
        let prompt = args.get(3).and_then(Value::as_str).unwrap_or("").to_owned();
        let indent = args.get(4).and_then(Value::as_i64).unwrap_or(0).max(0) as u16;
        // Cap nested levels to bound memory under malicious input.
        if self.cmdlines.len() >= MAX_CMDLINE_LEVELS && !self.cmdlines.contains_key(&level) {
            return;
        }
        self.cmdlines.insert(
            level,
            Cmdline {
                content: chunks,
                prompt,
                firstc,
                pos,
                indent,
            },
        );
        self.cmdline_visible = true;
    }

    fn on_cmdline_pos(&mut self, args: &[Value]) {
        let (Some(pos), Some(level)) = (
            args.first().and_then(Value::as_i64),
            args.get(1).and_then(Value::as_i64),
        ) else {
            return;
        };
        if let Some(c) = self.cmdlines.get_mut(&level) {
            c.pos = pos.max(0) as usize;
        }
    }

    fn on_cmdline_hide(&mut self, args: &[Value]) {
        let level = args.first().and_then(Value::as_i64).unwrap_or(0);
        self.cmdlines.remove(&level);
        if self.cmdlines.is_empty() {
            self.cmdline_visible = false;
        }
    }

    fn on_cmdline_special_char(&mut self, _args: &[Value]) {
        // Pending-state visual hint, renderer-side; nothing to retain here.
    }

    fn on_cmdline_block_show(&mut self, _args: &[Value]) {
        // Block content for function/ex-command definitions; renderer-side.
    }

    fn on_cmdline_block_append(&mut self, _args: &[Value]) {}

    fn on_cmdline_block_hide(&mut self, _args: &[Value]) {}

    // ----- Popupmenu ------------------------------------------------------

    fn on_popupmenu_show(&mut self, args: &[Value]) {
        if args.len() < 5 {
            return;
        }
        let items_arr = match args.first().and_then(Value::as_array) {
            Some(a) => a,
            None => return,
        };
        let mut items: Vec<PopupItem> = Vec::new();
        for entry in items_arr.iter().take(MAX_POPUP_ITEMS) {
            let it = match entry.as_array() {
                Some(it) => it,
                None => continue,
            };
            items.push(PopupItem {
                word: it.first().and_then(Value::as_str).unwrap_or("").to_owned(),
                kind: it.get(1).and_then(Value::as_str).unwrap_or("").to_owned(),
                menu: it.get(2).and_then(Value::as_str).unwrap_or("").to_owned(),
                info: it.get(3).and_then(Value::as_str).unwrap_or("").to_owned(),
            });
        }
        let selected = args.get(1).and_then(Value::as_i64).unwrap_or(-1);
        let row = args.get(2).and_then(Value::as_i64).unwrap_or(0);
        let col = args.get(3).and_then(Value::as_i64).unwrap_or(0);
        let grid = args.get(4).and_then(Value::as_i64).unwrap_or(-1);
        self.popupmenu = Some(Popupmenu {
            items,
            selected,
            row,
            col,
            grid,
            visible: true,
        });
    }

    fn on_popupmenu_select(&mut self, args: &[Value]) {
        let selected = args.first().and_then(Value::as_i64).unwrap_or(-1);
        if let Some(p) = self.popupmenu.as_mut() {
            p.selected = selected;
        }
    }

    fn on_popupmenu_hide(&mut self, _args: &[Value]) {
        if let Some(p) = self.popupmenu.as_mut() {
            p.visible = false;
        }
    }

    // ----- Messages -------------------------------------------------------

    fn on_msg_show(&mut self, args: &[Value]) {
        if args.len() < 3 {
            return;
        }
        let kind = args
            .first()
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let content = match args.get(1).and_then(Value::as_array) {
            Some(a) => a,
            None => return,
        };
        let replace_last = args.get(2).and_then(Value::as_bool).unwrap_or(false);
        let mut chunks: Vec<MessageChunk> = Vec::new();
        for entry in content.iter().take(MAX_CMDLINE_CHUNKS) {
            let cu = match entry.as_array() {
                Some(cu) => cu,
                None => continue,
            };
            if cu.len() < 3 {
                continue;
            }
            chunks.push(MessageChunk {
                text: cap_chunk_text(cu[1].as_str().unwrap_or("")),
                hl_id: cu[2].as_u64().unwrap_or(0).min(MAX_HL_ID),
            });
        }
        let id = args.get(5).and_then(Value::as_i64).unwrap_or(0);
        if replace_last {
            let pos = self
                .messages
                .iter()
                .position(|m| m.id == id)
                .or_else(|| self.messages.len().checked_sub(1));
            if let Some(idx) = pos {
                if let Some(last) = self.messages.get_mut(idx) {
                    last.chunks = chunks;
                    last.kind = kind;
                    return;
                }
            }
        }
        if self.messages.len() >= MAX_MESSAGES {
            self.messages.remove(0);
        }
        self.messages.push(Message {
            kind,
            chunks,
            id,
            replace_last,
        });
    }

    fn on_msg_clear(&mut self, _args: &[Value]) {
        self.messages.clear();
        self.showmode.clear();
        self.showcmd.clear();
        self.ruler.clear();
    }

    fn on_showmode(&mut self, args: &[Value]) {
        self.showmode = read_chunks(args.first());
    }

    fn on_showcmd(&mut self, args: &[Value]) {
        self.showcmd = read_chunks(args.first());
    }

    fn on_ruler(&mut self, args: &[Value]) {
        self.ruler = read_chunks(args.first());
    }
}

fn read_chunks(v: Option<&Value>) -> Vec<MessageChunk> {
    let mut out = Vec::new();
    let arr = match v.and_then(Value::as_array) {
        Some(a) => a,
        None => return out,
    };
    for entry in arr.iter().take(MAX_CMDLINE_CHUNKS) {
        let t = match entry.as_array() {
            Some(t) => t,
            None => continue,
        };
        if t.len() < 3 {
            continue;
        }
        out.push(MessageChunk {
            text: cap_chunk_text(t[1].as_str().unwrap_or("")),
            hl_id: t[2].as_u64().unwrap_or(0).min(MAX_HL_ID),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn batch(events: Vec<Value>) -> Value {
        Value::Array(events)
    }

    /// Build one redraw event. `args` is a list of argument arrays, one per
    /// logical invocation — matching the real Neovim UI protocol which sends
    /// repeated argument arrays after the event name (e.g. one `grid_line`
    /// event carries every row that scrolled into view, one `grid_resize`
    /// event carries every resized grid).
    fn ev(name: &str, args: Vec<Value>) -> Value {
        let mut a = Vec::with_capacity(args.len() + 1);
        a.push(Value::String(name.to_owned()));
        for sig in args {
            a.push(sig);
        }
        Value::Array(a)
    }

    #[::core::prelude::v1::test]
    fn grid_resize_creates_and_reuses_with_preserved_overlap() {
        let mut m = NeovimModel::new();
        m.apply_redraw(&batch(vec![ev("grid_resize", vec![json!([1, 3, 2])])]));
        let g = m.grid(1).unwrap();
        assert_eq!(g.cols, 3);
        assert_eq!(g.rows, 2);
        assert_eq!(g.cells.len(), 6);

        m.apply_redraw(&batch(vec![
            ev(
                "grid_line",
                vec![json!([1, 0, 0, [["a", 7], ["b"], ["c"]], false])],
            ),
            ev("flush", vec![]),
        ]));
        m.apply_redraw(&batch(vec![ev("grid_resize", vec![json!([1, 4, 2])])]));
        let g = m.grid(1).unwrap();
        assert_eq!(g.cols, 4);
        assert_eq!(g.rows, 2);
        // First three cells of row 0 should be preserved.
        assert_eq!(g.cells[0].text, "a");
        assert_eq!(g.cells[1].text, "b");
        assert_eq!(g.cells[2].text, "c");
        assert_eq!(g.cells[3].text, "");
        assert_eq!(m.highlights.get(&7).map(|h| h.bold), None);
        m.apply_redraw(&batch(vec![ev("grid_resize", vec![json!([1, 2, 1])])]));
        let g = m.grid(1).unwrap();
        assert_eq!(g.cells.len(), 2);
        assert_eq!(g.cells[0].text, "a");
        assert_eq!(g.cells[1].text, "b");
        assert_eq!(m.grid_cell_total, 2);
    }

    #[test]
    fn resizing_a_budget_tombstone_restores_a_rectangular_grid() {
        let mut m = NeovimModel::new();
        m.grids.insert(
            2,
            Grid {
                cols: 20,
                rows: 10,
                cells: Vec::new(),
            },
        );
        m.resize_grid_in_place(2, 4, 3);
        assert_eq!(m.grid(2).unwrap().cells.len(), 12);
        assert_eq!(m.grid_cell_total, 12);
    }

    #[::core::prelude::v1::test]
    fn grid_line_repeats_and_continuation_hl_are_handled() {
        let mut m = NeovimModel::new();
        m.apply_redraw(&batch(vec![ev("grid_resize", vec![json!([1, 5, 1])])]));
        // First cell sets hl=7 explicitly; subsequent cells with no hl reuse
        // it via "current_hl". The 4-element run repeats "x" five times.
        m.apply_redraw(&batch(vec![ev(
            "grid_line",
            vec![json!([1, 0, 0, [["a", 7], ["x", null, 5]], false])],
        )]));
        let g = m.grid(1).unwrap();
        assert_eq!(g.cells[0].text, "a");
        assert_eq!(g.cells[0].hl_id, 7);
        for i in 1..g.cols as usize {
            assert_eq!(g.cells[i].text, "x");
            assert_eq!(g.cells[i].hl_id, 7);
        }
        assert!(g.cells.len() >= 5);
    }

    #[::core::prelude::v1::test]
    fn grid_scroll_up_and_down_in_place() {
        let mut m = NeovimModel::new();
        m.apply_redraw(&batch(vec![ev("grid_resize", vec![json!([1, 2, 4])])]));
        // Row contents: 0=A, 1=B, 2=C, 3=D.
        m.apply_redraw(&batch(vec![
            ev("grid_line", vec![json!([1, 0, 0, [["A"], [" "]], false])]),
            ev("grid_line", vec![json!([1, 1, 0, [["B"], [" "]], false])]),
            ev("grid_line", vec![json!([1, 2, 0, [["C"], [" "]], false])]),
            ev("grid_line", vec![json!([1, 3, 0, [["D"], [" "]], false])]),
        ]));
        // Scroll up by 1 within [0,4,0,2): new rows [0,3) <- old rows [1,4).
        m.apply_redraw(&batch(vec![ev(
            "grid_scroll",
            vec![json!([1, 0, 4, 0, 2, 1, 0])],
        )]));
        let g = m.grid(1).unwrap();
        // After move-up: row 0 = B, row 1 = C, row 2 = D, row 3 stale (D).
        assert_eq!(g.cells[0].text, "B");
        assert_eq!(g.cells[2].text, "C");
        assert_eq!(g.cells[4].text, "D");
        // Scroll down by 1 within [0,4,0,2): new rows [1,4) <- old rows [0,3).
        m.apply_redraw(&batch(vec![ev(
            "grid_scroll",
            vec![json!([1, 0, 4, 0, 2, -1, 0])],
        )]));
        let g = m.grid(1).unwrap();
        // After move-down: row 0 stale (B), row 1 = B, row 2 = C, row 3 = D.
        assert_eq!(g.cells[2].text, "B");
        assert_eq!(g.cells[4].text, "C");
        assert_eq!(g.cells[6].text, "D");
    }

    #[::core::prelude::v1::test]
    fn multigrid_lifecycle_position_hide_close_destroy() {
        let mut m = NeovimModel::new();
        m.apply_redraw(&batch(vec![
            ev("grid_resize", vec![json!([1, 80, 24])]),
            ev("grid_resize", vec![json!([2, 20, 5])]),
            ev("win_pos", vec![json!([2, 1000, 0, 0, 20, 5])]),
            ev("flush", vec![]),
        ]));
        let w = m.windows.get(&1000).expect("window tracked");
        assert_eq!(w.grid, 2);
        assert_eq!(w.width, 20);
        assert!(!w.hidden);
        assert!(!w.floating);
        assert_eq!(m.window_for_grid.get(&2).copied(), Some(1000));

        // Hide.
        m.apply_redraw(&batch(vec![ev("win_hide", vec![json!([2])])]));
        assert!(m.windows.get(&1000).unwrap().hidden);

        // Float: re-positions and marks floating. Real protocol delivers one
        // big array per invocation; the dispatcher walks trailing argument
        // arrays and forwards each to the handler.
        m.apply_redraw(&batch(vec![ev(
            "win_float_pos",
            vec![json!([
                2,    // grid
                1000, // win
                "NW", // anchor
                1,    // anchor_grid
                1,    // anchor_row
                1,    // anchor_col
                true, // mouse_enabled
                50,   // zindex
                50,   // compindex
                10,   // screen_row
                20,   // screen_col
            ])],
        )]));
        let w = m.windows.get(&1000).unwrap();
        assert!(w.floating);
        assert_eq!(w.start_row, 10);
        assert_eq!(w.start_col, 20);

        // Close: drops the window record.
        m.apply_redraw(&batch(vec![ev("win_close", vec![json!([2])])]));
        assert!(!m.windows.contains_key(&1000));
        assert!(!m.window_for_grid.contains_key(&2));
        // grid_destroy removes the grid too.
        m.apply_redraw(&batch(vec![ev("grid_destroy", vec![json!([2])])]));
        assert!(m.grid(2).is_none());
    }

    #[::core::prelude::v1::test]
    fn cmdline_nested_levels_and_pos_update() {
        let mut m = NeovimModel::new();
        m.apply_redraw(&batch(vec![
            ev(
                "cmdline_show",
                vec![json!([
                    [[{}, "cold", 0], [{}, "est", 0]],
                    3,
                    ":",
                    "",
                    0,
                    1,
                    0,
                ])],
            ),
            ev(
                "cmdline_show",
                vec![json!([[[{}, "recur", 0]], 1, "=", "", 0, 2, 0,])],
            ),
            ev("flush", vec![]),
        ]));
        assert_eq!(m.cmdlines.len(), 2);
        let outer = m.cmdlines.get(&1).unwrap();
        assert_eq!(outer.firstc, ":");
        assert_eq!(outer.pos, 3);
        assert_eq!(outer.content.len(), 2);
        assert_eq!(outer.content[0].text, "cold");
        let inner = m.cmdlines.get(&2).unwrap();
        assert_eq!(inner.firstc, "=");
        assert_eq!(inner.pos, 1);
        assert!(m.cmdline_visible);

        m.apply_redraw(&batch(vec![ev("cmdline_pos", vec![json!([5, 1])])]));
        assert_eq!(m.cmdlines.get(&1).unwrap().pos, 5);

        // Hide the inner level; outer must remain visible.
        m.apply_redraw(&batch(vec![ev("cmdline_hide", vec![json!([2, false])])]));
        assert_eq!(m.cmdlines.len(), 1);
        assert!(m.cmdline_visible);

        m.apply_redraw(&batch(vec![ev("cmdline_hide", vec![json!([1, false])])]));
        assert!(!m.cmdline_visible);
    }

    #[::core::prelude::v1::test]
    fn highlights_and_default_colors_round_trip() {
        let mut m = NeovimModel::new();
        m.apply_redraw(&batch(vec![
            ev("default_colors_set", vec![json!([-1, -1, -1, 0, 0])]),
            ev(
                "hl_attr_define",
                vec![json!([7, {"fg": 0xffffff, "bg": 0, "bold": true, "italic": false}])],
            ),
            ev(
                "hl_attr_define",
                vec![json!([9, {"fg": -1, "bg": -1, "underline": true, "reverse": true}])],
            ),
            ev("flush", vec![]),
        ]));
        // `-1` is filtered out (sentinel for unset).
        assert_eq!(m.default_fg, None);
        assert_eq!(m.default_bg, None);
        let h7 = m.highlights.get(&7).unwrap();
        assert_eq!(h7.fg, Some(0xffffff));
        assert_eq!(h7.bg, Some(0));
        assert!(h7.bold);
        assert!(!h7.italic);
        let h9 = m.highlights.get(&9).unwrap();
        assert!(h9.underline);
        assert!(h9.reverse);
        assert!(h9.fg.is_none());
    }

    #[::core::prelude::v1::test]
    fn flush_is_only_set_after_terminal_event() {
        let mut m = NeovimModel::new();
        assert!(!m.apply_redraw(&batch(vec![ev("set_title", vec![json!(["hi"])])])));
        assert!(!m.flushed);
        assert!(m.apply_redraw(&batch(vec![ev("flush", vec![json!([])])])));
        assert!(m.flushed);
    }

    #[::core::prelude::v1::test]
    fn popupmenu_show_select_hide() {
        let mut m = NeovimModel::new();
        m.apply_redraw(&batch(vec![ev(
            "popupmenu_show",
            vec![json!([
                [["foo", "f", "File", ""], ["bar", "b", "Bar", "info"]],
                0,
                3,
                2,
                1,
            ])],
        )]));
        let p = m.popupmenu.as_ref().unwrap();
        assert!(p.visible);
        assert_eq!(p.items.len(), 2);
        assert_eq!(p.items[0].word, "foo");
        assert_eq!(p.selected, 0);
        m.apply_redraw(&batch(vec![ev("popupmenu_select", vec![json!([1])])]));
        assert_eq!(m.popupmenu.as_ref().unwrap().selected, 1);
        m.apply_redraw(&batch(vec![ev("popupmenu_hide", vec![json!([])])]));
        assert!(!m.popupmenu.as_ref().unwrap().visible);
    }

    #[::core::prelude::v1::test]
    fn messages_replace_and_clear_and_bound() {
        let mut m = NeovimModel::new();
        for i in 0..(MAX_MESSAGES + 5) {
            m.apply_redraw(&batch(vec![ev(
                "msg_show",
                vec![json!([
                    "echo",
                    [[{}, format!("m{i}"), 0]],
                    false,
                    true,
                    false,
                    i as i64,
                ])],
            )]));
        }
        assert!(m.messages.len() <= MAX_MESSAGES);
        assert_eq!(
            m.messages.last().unwrap().chunks[0].text,
            format!("m{}", MAX_MESSAGES + 4)
        );
        m.apply_redraw(&batch(vec![ev("msg_clear", vec![json!([])])]));
        assert!(m.messages.is_empty());
    }

    #[::core::prelude::v1::test]
    fn mode_change_cursor_and_title() {
        let mut m = NeovimModel::new();
        m.apply_redraw(&batch(vec![
            ev("mode_change", vec![json!(["insert", 1])]),
            ev("grid_resize", vec![json!([1, 2, 2])]),
            ev("grid_cursor_goto", vec![json!([1, 1, 0])]),
            ev("set_title", vec![json!(["scratch"])]),
            ev("set_icon", vec![json!(["ic"])]),
            ev("flush", vec![]),
        ]));
        assert_eq!(m.mode, "insert");
        assert_eq!(m.mode_idx, 1);
        assert_eq!(
            m.cursor,
            Some(Cursor {
                grid: 1,
                row: 1,
                col: 0
            })
        );
        assert_eq!(m.title, "scratch");
        assert_eq!(m.icon, "ic");
    }

    #[::core::prelude::v1::test]
    fn hostile_events_do_not_panic() {
        let mut m = NeovimModel::new();
        // Garbage types, missing fields, oversized repeats, and impossible
        // dims must be tolerated.
        m.apply_redraw(&batch(vec![
            ev("grid_resize", vec![json!([1, -5, 99999])]),
            ev("grid_line", vec![json!([1, 65536, 0, [["x", 0]], false])]),
            ev(
                "grid_line",
                vec![json!([1, -1, -1, [["x", 9999999, 99999]], false])],
            ),
            ev("grid_scroll", vec![json!([1, -1, 99999, -1, 99999, 5, 0])]),
            ev("win_pos", vec![json!([1, -1])]),
            ev(
                "hl_attr_define",
                vec![json!([u64::MAX, {"fg": "not-a-number"}])],
            ),
            // Real protocol passes a single string here; the dispatcher must
            // reject non-array trailing values silently per-typed
            ev("cmdline_show", vec![json!("garbage")]),
        ]));
        // The grid exists at clamped (1, 1) and is at least 1 cell.
        assert!(m.grid(1).is_some());
        assert!(m.cmdlines.is_empty());
        assert!(m.windows.is_empty());
        assert!(m.highlights.is_empty());
    }

    #[::core::prelude::v1::test]
    fn msg_replace_last_updates_existing_by_id() {
        let mut m = NeovimModel::new();
        m.apply_redraw(&batch(vec![ev(
            "msg_show",
            vec![json!(["echo", [[{}, "first", 0]], false, true, false, 42,])],
        )]));
        m.apply_redraw(&batch(vec![ev(
            "msg_show",
            vec![json!(["echo", [[{}, "second", 0]], true, true, false, 42,])],
        )]));
        assert_eq!(m.messages.len(), 1);
        assert_eq!(m.messages[0].chunks[0].text, "second");
    }

    #[::core::prelude::v1::test]
    fn reset_clears_all_state() {
        let mut m = NeovimModel::new();
        m.apply_redraw(&batch(vec![
            ev("grid_resize", vec![json!([1, 3, 3])]),
            ev("set_title", vec![json!(["t"])]),
            ev("hl_attr_define", vec![json!([1, {"bold": true}])]),
            ev("flush", vec![]),
        ]));
        m.reset();
        assert!(m.grids.is_empty());
        assert!(m.windows.is_empty());
        assert!(m.highlights.is_empty());
        assert_eq!(m.cursor, None);
        assert!(m.title.is_empty());
        assert!(!m.flushed);
    }

    /// Real protocol: a single `grid_line` event carries every row that
    /// scrolled in. The dispatcher must iterate the trailing argument arrays
    /// so every row reaches `on_grid_line` in one call.
    #[::core::prelude::v1::test]
    fn grid_line_multi_row_packet_decodes_all_rows() {
        let mut m = NeovimModel::new();
        m.apply_redraw(&batch(vec![ev("grid_resize", vec![json!([1, 4, 3])])]));
        m.apply_redraw(&batch(vec![ev(
            "grid_line",
            vec![
                json!([1, 0, 0, [["A"], ["B"], [" "], ["C"]], false]),
                json!([1, 1, 0, [["D"], ["E"], [" "], ["F"]], false]),
                json!([1, 2, 0, [["G"], ["H"], [" "], ["I"]], false]),
            ],
        )]));
        let g = m.grid(1).unwrap();
        assert_eq!(g.cells[0].text, "A");
        assert_eq!(g.cells[1].text, "B");
        assert_eq!(g.cells[4].text, "D");
        assert_eq!(g.cells[5].text, "E");
        assert_eq!(g.cells[8].text, "G");
        assert_eq!(g.cells[9].text, "H");
    }
}
