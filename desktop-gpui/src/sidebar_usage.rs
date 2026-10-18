//! Compact lower-sidebar provider-usage ring + clock widget for the native
//! desktop.
//!
//! This module is the GPUI port of the current Svelte sidebar-statistics
//! block. It mirrors two production components from `monitter`:
//!
//! - [`UsageRings.svelte`](../../src/lib/components/UsageRings.svelte) —
//!   rendered in `iconOnly` / `expanded` / `showHeader={false}` mode so the
//!   rings live at the right of the workspace bar; their shared quota cache
//!   also feeds the retained sidebar resource footer.
//! - [`SidebarClock.svelte`](../../src/lib/components/SidebarClock.svelte) —
//!   rendered with `expanded={true}` / `panel={true}` so the analog face and
//!   the CPU / RAM sparklines stay visible without a toggle header.
//!
//! The full route-surface usage view lives in `usage.rs`. This widget
//! reproduces only the read-only compact projection the lower sidebar shows,
//! without the task context lists, provider balances and process tables.
//!
//! ## Integration contract (frozen — parent wires the module)
//!
//! The companion change in `main.rs` is responsible for declaring
//! `mod sidebar_usage;` and instantiating one `Entity<SidebarUsageWidget>`,
//! then forwarding three signals:
//!
//! - `set_snapshot(&self.snapshot, self.revision.as_deref(), cx)` — called
//!   whenever the global desktop revision changes. **The parent passes its
//!   authoritative revision; the widget never reads `snapshot["revision"]`
//!   as a stand-in** because the parent owns that field separately. The
//!   snapshot payload (settings, theme, accent, `usageOverview`) is
//!   always ingested so the next render reflects the latest state, even
//!   when the revision is `None`. Revision changes alone never launch
//!   backend requests; quota refresh is cache-expiry or explicit-action
//!   driven, while metrics uses its own polling tick. A backward-compat
//!   `set_snapshot_from_value(&snapshot, cx)` is also exposed for callers
//!   that still want to read `snapshot["revision"]`.
//! - The retained entity owns a ~1 s clock tick and calls `tick_metrics(cx)`
//!   every other tick. Clock ticks also check quota-cache expiry.
//! - `tick_metrics(cx)` schedules a fresh
//!   `get_process_metrics` read through the same named-std-thread +
//!   futures-oneshot worker used for the quota refresh; deduplicated by
//!   the in-flight `metrics_busy` flag so a slow worker cannot pile up
//!   redundant calls. The numbers are **backend-process-tree** metrics
//!   (`cpuTimeMs`, `residentMemoryBytes`): the backend server and its child
//!   processes; the separate GPUI GUI process is excluded.
//! - `build_widget(Some(palette), window, cx) -> AnyElement` (also
//!   reachable via the `Render::render(...)` trait impl, which passes
//!   `None` and falls back to `design::palette(...)`) — produces the
//!   lower sidebar statistics row. The parent drops the returned
//!   element into the sidebar-footer slot above the brand actions.
//!   **Pass the composed native palette** (the value
//!   `Desktop::apply_native_theme` returns) so source fidelity holds
//!   against the rest of the shell.
//!
//! ## Threading safety
//!
//! - Every backend call goes through [`start_quota_worker`] or
//!   [`start_metrics_worker`], both of which spawn a named `std::thread`
//!   with an explicit 8 MiB stack (`monitter-sidebar-usage` /
//!   `monitter-sidebar-metrics`) and return a `futures::channel::oneshot`
//!   receiver. The actual `service.invoke(...)` call is executed on that
//!   thread, **never** on the GPUI GCD executor thread.
//! - The result is awaited from a `cx.background_executor()` future and
//!   delivered back via `cx.spawn`, so the public fields are only updated
//!   inside `Entity::update` closures on the GCD thread.
//! - Render-time state projection ([`project_sidebar_usage_map`]) is a
//!   pure function over the cached snapshot. It does no I/O.
//! - The `busy` flags prevent re-entrant requests when a worker is already
//!   in flight; the parent may call `refresh_quota` again at any time and
//!   it will short-circuit while a worker is running.
//!
//! ## Source-fidelity gaps
//!
//! - **Ring geometry and provider artwork**: outer `r=15.5`, inner `r=11.5`,
//!   `viewBox=0 0 40 40`, and `pathLength=100` match the Svelte template.
//!   The center uses the same bundled provider SVG artwork exposed by
//!   `branding::provider_for_ink`; rings are cached GPUI SVG images.
//! - **Clock hands**: identical `viewBox=0 0 60 60` and rotate-around-centre
//!   semantics as the Svelte clock. Per-second motion is driven by the
//!   parent's `tick_clock`, never by setTimeout inside GPUI.
//! - **Sparklines**: identical `viewBox=0 0 58 18` and
//!   `preserveAspectRatio="none"` semantics. Polyline points are computed
//!   from the same history window (30 samples) as the Svelte clock.
//! - **State enums**: status values are an exact mirror of the
//!   `UsageRingStatus` union (`ready | loading | stale | error |
//!   unavailable`). The compact renderer treats `unavailable` providers
//!   as `—` rather than the Svelte full-row collapse, because the GPUI
//!   footer is short and folding rows mid-widget would clip the brand
//!   actions; this is documented as a deliberate divergence in the
//!   `render_rings_row` method.
//! - **Inline detail popover**: the Svelte `ProviderUsageModal` opens as a
//!   dialog. The native widget renders a retained inline detail block
//!   (`render_detail`) under the rings row when a provider is clicked.
//!   The data shown is the same `windows` / `balances` / `accountLabel`
//!   set the modal exposes; the layout is the lower-sidebar geometry
//!   rather than a centred dialog because no GPUI modal primitive is
//!   available without extending the parent.
//! - **No `Browser` wrapper**: rendering goes through the native GPUI
//!   image stack. There is no `<img>` to an HTTP URL, no `WebView`, and no
//!   scripted host. The widget holds no `serde_json::Value` strings in
//!   the DOM beyond what the existing `usage.rs` already feeds to the
//!   native view.
//!
//! ## Settings that gate the widget
//!
//! - `self.snapshot["settings"]["accent"]` — accent override applied to
//!   the ring track and the active ring stroke when the projection is in
//!   `loading` or `unavailable` state.
//! - `self.snapshot["settings"]["theme"]` — `"light"` or `"dark"`, picked
//!   up by [`design::palette`] to drive every token in this widget.
//! - `self.snapshot["usageOverview"]` — the latest cached overview
//!   payload, fed straight into [`project_sidebar_usage_map`]. When it is
//!   absent the widget renders the explicit `"unavailable"` placeholder
//!   rather than fabricating a row.
//! - `self.snapshot["revision"]` — debounce key for [`set_snapshot`].
//!
//! # Tests
//!
//! Pure mapping functions are unit-tested against the real backend payload
//! shape (see [`tests`]). No integration tests are required: the worker
//! pattern is identical to `usage::start_usage_overview_worker`, which is
//! exercised by the existing route-surface pane; behaviour not exercised
//! by the unit tests is exercised by the parent integration in main.rs.
#![forbid(unsafe_code)]

use gpui_kit::component::tooltip::ManagedTooltipExt as _;
use crate::design::ui_scaled_px as px;
use gpui_kit::prelude::FluentBuilder as _;

use chrono;
use futures::channel::oneshot;
use gpui_kit::{
    Image, ImageFormat,
    assets::__private as icons_assets,
    component::{
        ActiveTheme as _, Icon, Sizable, WindowExt as _,
        button::{Button, ButtonVariants},
        tooltip::Tooltip,
    },
    *,
};
use monitter_lib::experimental_desktop::{DesktopService, NativeTileTheme};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, VecDeque},
    rc::Rc,
    sync::{Arc, LazyLock, Mutex},
    time::Duration,
};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const WORKER_STACK: usize = 8 * 1024 * 1024;

/// Mirrors the Svelte `radius = 15.5`.
const RING_OUTER_RADIUS: f32 = 15.5;
/// Mirrors the Svelte `r = "11.5"` weekly ring.
const RING_INNER_RADIUS: f32 = 11.5;
const RING_VIEWBOX: f32 = 40.0;

const SPARK_HEIGHT: f32 = 18.0;
/// Matches the `historyLimit = 30` in `SidebarClock.svelte`.
const SPARK_HISTORY: usize = 30;
const METRICS_HISTORY: usize = 30;
const QUOTA_REFRESH_INTERVAL_MS: i64 = 65_000;
const RING_IMAGE_CACHE_LIMIT: usize = 512;
const PROVIDER_RING_COUNT: usize = 5;
const RING_MAX_DIAMETER: f32 = 64.0;
const CODEX_ACCOUNT_ARROWS_WIDTH: f32 = 44.0;

#[derive(Clone, Copy, Debug, PartialEq)]
struct RingRowLayout {
    diameter: f32,
    gap: f32,
    edge_padding: f32,
}

/// Keep five provider rings on one line. Rings grow until the 64 design-pixel
/// cap; additional width is distributed across the four inter-ring gaps so
/// the strip stays centered with fixed edge breathing room.
fn provider_ring_layout(sidebar_width: f32, has_codex_account_arrows: bool) -> RingRowLayout {
    let width = if sidebar_width.is_finite() {
        sidebar_width.max(1.0)
    } else {
        1.0
    };
    let edge_padding = crate::design::space::TIGHT;
    let base_gap = crate::design::space::TIGHT;
    let arrows = if has_codex_account_arrows {
        CODEX_ACCOUNT_ARROWS_WIDTH
    } else {
        0.0
    };
    let gaps_width = base_gap * (PROVIDER_RING_COUNT - 1) as f32;
    let diameter = ((width - edge_padding * 2.0 - arrows - gaps_width)
        / PROVIDER_RING_COUNT as f32)
        .clamp(1.0, RING_MAX_DIAMETER);
    let gap = if diameter >= RING_MAX_DIAMETER {
        ((width
            - edge_padding * 2.0
            - arrows
            - RING_MAX_DIAMETER * PROVIDER_RING_COUNT as f32)
            / (PROVIDER_RING_COUNT - 1) as f32)
            .max(base_gap)
    } else {
        base_gap
    };
    RingRowLayout {
        diameter,
        gap,
        edge_padding,
    }
}

// ---------------------------------------------------------------------------
// Domain types
// ---------------------------------------------------------------------------

/// Provider identifiers the lower sidebar can surface. Mirrors the
/// `UsageRingProvider` union in `UsageRings.svelte` and the typed key the
/// Svelte projection keeps stable across hot reloads.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum SidebarUsageProvider {
    #[default]
    Codex,
    Claude,
    Gemini,
    MiniMax,
    OpencodeGo,
}

impl SidebarUsageProvider {
    pub const ALL: [Self; 5] = [
        Self::Codex,
        Self::Claude,
        Self::Gemini,
        Self::MiniMax,
        Self::OpencodeGo,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Codex => "Codex",
            Self::Claude => "Claude",
            Self::Gemini => "Gemini",
            Self::MiniMax => "MiniMax",
            Self::OpencodeGo => "OpenCode Go",
        }
    }

    /// The single-letter / two-letter mark rendered inside the icon-only
    /// ring when no numeric value is available. Mirrors the Svelte
    /// `provider.mark` table.
    pub fn mark(self) -> &'static str {
        match self {
            Self::Codex => "C",
            Self::Claude => "Cl",
            Self::Gemini => "G",
            Self::MiniMax => "M",
            Self::OpencodeGo => "Go",
        }
    }

    /// Canonical id string used by the typed contract — matches the
    /// `UsageRingProvider` union and the `UsageProvider` strings the
    /// backend publishes inside `SubscriptionUsageSource`.
    pub fn id(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
            Self::Gemini => "gemini",
            Self::MiniMax => "minimax",
            Self::OpencodeGo => "opencode-go",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "codex" => Some(Self::Codex),
            "claude" => Some(Self::Claude),
            "gemini" => Some(Self::Gemini),
            "minimax" | "mmx" => Some(Self::MiniMax),
            "opencode-go" => Some(Self::OpencodeGo),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum SidebarUsageStatus {
    Ready,
    Loading,
    Stale,
    Error,
    #[default]
    Unavailable,
}

impl SidebarUsageStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Loading => "loading",
            Self::Stale => "stale",
            Self::Error => "error",
            Self::Unavailable => "unavailable",
        }
    }

    fn from_str(value: &str) -> Self {
        match value {
            "ready" => Self::Ready,
            "loading" => Self::Loading,
            "stale" => Self::Stale,
            "error" => Self::Error,
            _ => Self::Unavailable,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SidebarUsageWindow {
    pub label: String,
    pub used_percent: Option<f64>,
    pub unlimited: bool,
    pub resets_at: Option<i64>,
    pub reset_label: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SidebarUsageAccount {
    pub key: String,
    pub label: String,
    pub status: SidebarUsageStatus,
    pub active: Option<SidebarUsageWindow>,
    pub weekly: Option<SidebarUsageWindow>,
    pub message: Option<String>,
    pub updated_at: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct SidebarUsageRing {
    pub provider: SidebarUsageProvider,
    pub status: SidebarUsageStatus,
    pub active: Option<SidebarUsageWindow>,
    pub weekly: Option<SidebarUsageWindow>,
    pub message: Option<String>,
    pub updated_at: Option<i64>,
    pub account_label: Option<String>,
    pub accounts: Vec<SidebarUsageAccount>,
}

impl SidebarUsageRing {
    /// True when the ring has a numeric value the icon-only ring can show.
    /// Mirrors `showValue` in the Svelte component so the percent text is
    /// never fabricated when the provider omitted it.
    pub fn shows_value(&self) -> bool {
        if self.status != SidebarUsageStatus::Ready && self.status != SidebarUsageStatus::Stale {
            return false;
        }
        match &self.active {
            Some(window) => window.unlimited || window.used_percent.is_some(),
            None => false,
        }
    }
}

pub type SidebarUsageMap = HashMap<SidebarUsageProvider, SidebarUsageRing>;

// ---------------------------------------------------------------------------
// Backend workers — never invoked on the GPUI GCD thread.
// ---------------------------------------------------------------------------

fn start_quota_worker(
    service: DesktopService,
    policy: &'static str,
) -> Result<oneshot::Receiver<Result<Value, String>>, String> {
    let (sender, receiver) = oneshot::channel();
    let worker = std::thread::Builder::new()
        .name("monitter-sidebar-usage".into())
        .stack_size(WORKER_STACK)
        .spawn(move || {
            let _ = sender.send(service.invoke("get_usage_overview", json!({"policy": policy})));
        })
        .map_err(|error| format!("Could not start sidebar usage worker: {error}"))?;
    drop(worker);
    Ok(receiver)
}

fn start_metrics_worker(
    service: DesktopService,
) -> Result<oneshot::Receiver<Result<Value, String>>, String> {
    let (sender, receiver) = oneshot::channel();
    let worker = std::thread::Builder::new()
        .name("monitter-sidebar-metrics".into())
        .stack_size(WORKER_STACK)
        .spawn(move || {
            let _ = sender.send(service.invoke("get_process_metrics", json!({})).and_then(crate::gui_process_metrics::include_gui));
        })
        .map_err(|error| format!("Could not start sidebar metrics worker: {error}"))?;
    drop(worker);
    Ok(receiver)
}

// ---------------------------------------------------------------------------
// Pure projection — exercised by the unit tests below.
// ---------------------------------------------------------------------------

fn json_get<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    value.get(key)
}
fn json_text(value: &Value, key: &str) -> String {
    json_get(value, key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}
fn json_number(value: &Value, key: &str) -> Option<f64> {
    json_get(value, key)
        .and_then(Value::as_f64)
        .filter(|n| n.is_finite())
}
fn json_int(value: &Value, key: &str) -> Option<i64> {
    json_get(value, key).and_then(Value::as_i64)
}
fn json_rows(value: &Value, key: &str) -> Vec<Value> {
    json_get(value, key)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}
fn window_label_lower(window: &Value) -> String {
    format!(
        "{} {}",
        json_text(window, "key"),
        json_text(window, "label")
    )
    .to_lowercase()
}
fn is_weekly_label(label: &str) -> bool {
    label.contains("week") || label.contains("seven.day") || label.contains("secondary")
}
fn is_primary_label(label: &str) -> bool {
    label.contains("5.hour")
        || label.contains("five.hour")
        || label.contains("current.interval")
        || label.contains("primary")
}

/// Mirrors `window(source, weekly)` in `usage-ring-data.ts` exactly:
/// the weekly window is selected by `week|seven.day|secondary` keys; the
/// primary window is selected by `5.hour|five.hour|current.interval|primary`,
/// falling back to the first window that is not weekly when no primary
/// key matches.
fn select_window<'a>(windows: &'a [Value], weekly: bool) -> Option<&'a Value> {
    let mut fallback = None;
    for window in windows {
        let label = window_label_lower(window);
        if weekly {
            if is_weekly_label(&label) {
                return Some(window);
            }
        } else {
            if is_primary_label(&label) {
                return Some(window);
            }
            if !is_weekly_label(&label) && fallback.is_none() {
                fallback = Some(window);
            }
        }
    }
    fallback
}

fn window_to_sidebar(window: &Value, weekly: bool) -> SidebarUsageWindow {
    let raw_label = json_text(window, "label");
    let label = if weekly {
        "Week".into()
    } else {
        raw_label.clone()
    };
    SidebarUsageWindow {
        label,
        used_percent: json_number(window, "usedPercent"),
        unlimited: raw_label.to_lowercase().contains("unlimited"),
        resets_at: json_int(window, "resetsAt"),
        reset_label: None,
    }
}

fn status_for(
    source: Option<&Value>,
    loading: bool,
    request_error: &str,
    now: i64,
) -> SidebarUsageStatus {
    let Some(source) = source else {
        return if loading {
            SidebarUsageStatus::Loading
        } else if !request_error.is_empty() {
            SidebarUsageStatus::Error
        } else {
            SidebarUsageStatus::Unavailable
        };
    };
    let state = json_text(source, "state");
    let stale_after = json_int(source, "staleAfter");
    let is_stale = stale_after.map(|at| at < now).unwrap_or(false);
    match state.as_str() {
        "available" => {
            if is_stale {
                SidebarUsageStatus::Stale
            } else {
                SidebarUsageStatus::Ready
            }
        }
        "error" => SidebarUsageStatus::Error,
        _ => SidebarUsageStatus::Unavailable,
    }
}

fn ring_from_source(
    source: &Value,
    provider: SidebarUsageProvider,
    loading: bool,
    request_error: &str,
    now: i64,
) -> SidebarUsageRing {
    let status = status_for(Some(source), loading, request_error, now);
    let windows = json_rows(source, "windows");
    let active_window = select_window(&windows, false);
    let weekly_window = select_window(&windows, true);
    let active = active_window
        .map(|w| window_to_sidebar(w, false))
        .or_else(|| weekly_window.map(|w| window_to_sidebar(w, true)));
    let weekly = if active_window.is_some() {
        weekly_window.map(|w| window_to_sidebar(w, true))
    } else {
        // Weekly-only plans use that allowance as the primary ring, rather
        // than duplicating it as a second window.
        None
    };
    let raw_message = json_text(source, "error");
    let message = if !raw_message.is_empty() {
        Some(raw_message)
    } else if provider == SidebarUsageProvider::Claude
        && json_text(source, "state") == "unsupported"
    {
        Some("Claude does not expose an on-demand quota read.".into())
    } else {
        None
    };
    let account_label = {
        let raw = json_text(source, "accountLabel");
        if raw.is_empty() { None } else { Some(raw) }
    };
    SidebarUsageRing {
        provider,
        status,
        active,
        weekly,
        message,
        updated_at: json_int(source, "fetchedAt"),
        account_label,
        accounts: Vec::new(),
    }
}

fn provider_sources<'a>(overview: &'a Value, provider: SidebarUsageProvider) -> Vec<&'a Value> {
    let Some(subscriptions) = json_get(overview, "subscriptions").and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut out: Vec<&'a Value> = Vec::new();
    for source in subscriptions {
        let id = json_text(source, "provider");
        let source_name = json_text(source, "source");
        let matches = match provider {
            SidebarUsageProvider::Codex => id == "codex",
            SidebarUsageProvider::Claude => id == "claude",
            SidebarUsageProvider::Gemini => id == "gemini",
            SidebarUsageProvider::MiniMax => id == "minimax" || source_name.contains("mmx"),
            SidebarUsageProvider::OpencodeGo => id == "opencode-go",
        };
        if matches {
            out.push(source);
        }
    }
    out
}

fn account_key(source: &Value) -> String {
    let home = json_text(source, "codexHome");
    if !home.is_empty() {
        return home;
    }
    let provider = json_text(source, "provider");
    let host_id = json_text(source, "hostId");
    let source_name = json_text(source, "source");
    format!("{provider}:{host_id}:{source_name}")
}

fn account_label_for(source: &Value) -> String {
    let label = json_text(source, "accountLabel");
    let home = json_text(source, "codexHome");
    if !label.is_empty() {
        label
    } else if !home.is_empty() {
        home
    } else {
        "Default account".into()
    }
}

fn empty_overview() -> Value {
    let mut map = serde_json::Map::new();
    map.insert("subscriptions".into(), Value::Array(Vec::new()));
    map.insert("providerTotals".into(), Value::Array(Vec::new()));
    map.insert("recentRuns".into(), Value::Array(Vec::new()));
    Value::Object(map)
}

/// Project the backend payload into a per-provider ring map. This is the
/// only function that decides what numeric values the widget renders; every
/// null, missing key and state transition is mirrored from
/// `usage-ring-data.ts`. Loading and error states are projected when the
/// caller passes them through.
pub fn project_sidebar_usage_map(
    overview: Option<&Value>,
    loading: bool,
    request_error: &str,
    now: i64,
) -> SidebarUsageMap {
    let mut out = SidebarUsageMap::with_capacity(SidebarUsageProvider::ALL.len());
    let empty = empty_overview();
    let overview_value = overview.unwrap_or(&empty);
    for provider in SidebarUsageProvider::ALL {
        let sources = provider_sources(overview_value, provider);
        let mut ring = if let Some(primary) = sources.first() {
            ring_from_source(primary, provider, loading, request_error, now)
        } else {
            let status = status_for(None, loading, request_error, now);
            SidebarUsageRing {
                provider,
                status,
                ..Default::default()
            }
        };
        if sources.len() > 1 {
            ring.accounts = sources
                .iter()
                .map(|source| {
                    let r = ring_from_source(source, provider, loading, request_error, now);
                    SidebarUsageAccount {
                        key: account_key(source),
                        label: account_label_for(source),
                        status: r.status,
                        active: r.active,
                        weekly: r.weekly,
                        message: r.message,
                        updated_at: r.updated_at,
                    }
                })
                .collect();
        }
        out.insert(provider, ring);
    }
    out
}

// ---------------------------------------------------------------------------
// Color helpers — same interpolation as `usageColor` in UsageRings.svelte.
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Rgb(u8, u8, u8);

const RGB_GREEN: Rgb = Rgb(63, 157, 106);
const RGB_ORANGE: Rgb = Rgb(224, 133, 51);
const RGB_RED: Rgb = Rgb(205, 76, 76);

fn blend(from: Rgb, to: Rgb, amount: f32) -> Rgb {
    let progress = amount.clamp(0.0, 1.0);
    Rgb(
        (from.0 as f32 + (to.0 as f32 - from.0 as f32) * progress).round() as u8,
        (from.1 as f32 + (to.1 as f32 - from.1 as f32) * progress).round() as u8,
        (from.2 as f32 + (to.2 as f32 - from.2 as f32) * progress).round() as u8,
    )
}

fn usage_color(window: Option<&SidebarUsageWindow>) -> Option<Rgb> {
    let window = window?;
    if window.unlimited {
        return None; // Caller falls back to the accent token.
    }
    let used = match window.used_percent {
        Some(value) if value.is_finite() => value.clamp(0.0, 100.0),
        _ => return None,
    };
    let remaining = 100.0 - used;
    let rgb = if remaining >= 40.0 {
        RGB_GREEN
    } else if remaining >= 10.0 {
        blend(RGB_ORANGE, RGB_GREEN, ((remaining - 10.0) / 30.0) as f32)
    } else {
        blend(RGB_RED, RGB_ORANGE, (remaining / 10.0) as f32)
    };
    Some(rgb)
}

// ---------------------------------------------------------------------------
// SVG generation. Cached as Image so render can stay cheap.
// ---------------------------------------------------------------------------

fn svg_image(svg: String) -> Arc<Image> {
    Arc::new(Image::from_bytes(ImageFormat::Svg, svg.into_bytes()))
}

/// Cache keyed by `percent` (0..=100) so the active and weekly rings reuse
/// the same compiled texture across renders.
static RING_TRACK: LazyLock<Arc<Image>> = LazyLock::new(|| {
    let svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="160" height="160" viewBox="0 0 {vb} {vb}"><circle cx="20" cy="20" r="{r}" fill="none" stroke="#555" stroke-opacity="0.35" stroke-width="3.5"/></svg>"##,
        vb = RING_VIEWBOX,
        r = RING_OUTER_RADIUS
    );
    svg_image(svg)
});
static RING_WEEKLY_TRACK: LazyLock<Arc<Image>> = LazyLock::new(|| {
    let svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="160" height="160" viewBox="0 0 {vb} {vb}"><circle cx="20" cy="20" r="{r}" fill="none" stroke="#555" stroke-opacity="0.35" stroke-width="2.5"/></svg>"##,
        vb = RING_VIEWBOX,
        r = RING_INNER_RADIUS
    );
    svg_image(svg)
});
type RingImageKey = (i32, u8, u8, u8, bool);

static RING_OUTER_CACHE: LazyLock<Mutex<HashMap<RingImageKey, Arc<Image>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static RING_WEEKLY_CACHE: LazyLock<Mutex<HashMap<RingImageKey, Arc<Image>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn ring_image_key(percent: f64, color: Rgb, dim: bool) -> RingImageKey {
    (percent_to_key(percent), color.0, color.1, color.2, dim)
}

fn cache_ring_image(
    cache: &LazyLock<Mutex<HashMap<RingImageKey, Arc<Image>>>>,
    key: RingImageKey,
    image: Arc<Image>,
) {
    if let Ok(mut cache) = cache.lock() {
        cache.entry(key).or_insert(image);
        while cache.len() > RING_IMAGE_CACHE_LIMIT {
            if let Some(oldest) = cache.keys().next().copied() {
                cache.remove(&oldest);
            } else {
                break;
            }
        }
    }
}

fn percent_to_key(percent: f64) -> i32 {
    percent.clamp(0.0, 100.0).round() as i32
}

fn ring_active_svg(percent: f64, color: Rgb, dim: bool) -> Arc<Image> {
    let key = ring_image_key(percent, color, dim);
    if let Ok(cache) = RING_OUTER_CACHE.lock() {
        if let Some(image) = cache.get(&key) {
            return image.clone();
        }
    }
    let offset = 100.0 - key.0 as f64;
    let stroke = format!("#{:02x}{:02x}{:02x}", color.0, color.1, color.2);
    let opacity = if dim { 0.55_f32 } else { 1.0_f32 };
    let svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="160" height="160" viewBox="0 0 {vb} {vb}"><circle cx="20" cy="20" r="{r}" fill="none" stroke="{stroke}" stroke-opacity="{opacity}" stroke-width="3.5" pathLength="100" stroke-dasharray="100" stroke-dashoffset="{offset}" transform="rotate(-90 20 20)"/></svg>"##,
        vb = RING_VIEWBOX,
        r = RING_OUTER_RADIUS,
        stroke = stroke,
        opacity = opacity,
        offset = offset
    );
    let image = svg_image(svg);
    cache_ring_image(&RING_OUTER_CACHE, key, image.clone());
    image
}

fn ring_weekly_svg(percent: f64, color: Rgb, dim: bool) -> Arc<Image> {
    let key = ring_image_key(percent, color, dim);
    if let Ok(cache) = RING_WEEKLY_CACHE.lock() {
        if let Some(image) = cache.get(&key) {
            return image.clone();
        }
    }
    let offset = 100.0 - key.0 as f64;
    let stroke = format!("#{:02x}{:02x}{:02x}", color.0, color.1, color.2);
    let opacity = if dim { 0.55_f32 } else { 1.0_f32 };
    let svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="160" height="160" viewBox="0 0 {vb} {vb}"><circle cx="20" cy="20" r="{r}" fill="none" stroke="{stroke}" stroke-opacity="{opacity}" stroke-width="2.5" pathLength="100" stroke-dasharray="100" stroke-dashoffset="{offset}" transform="rotate(-90 20 20)"/></svg>"##,
        vb = RING_VIEWBOX,
        r = RING_INNER_RADIUS,
        stroke = stroke,
        opacity = opacity,
        offset = offset
    );
    let image = svg_image(svg);
    cache_ring_image(&RING_WEEKLY_CACHE, key, image.clone());
    image
}

/// Paint changing history synchronously with the clock's frame. A fresh Image
/// ID on every refresh can cause asynchronous decode/cache churn and blank frames.
fn native_sparkline<I>(values: I, ceiling: f64, accent: u32) -> AnyElement
where
    I: IntoIterator<Item = f64>,
{
    let values: Vec<f64> = values.into_iter().collect();
    canvas(
        move |bounds, _, _| bounds,
        move |bounds, _, window, _| {
            let width = f32::from(bounds.size.width);
            let height = f32::from(bounds.size.height);
            let scale = height / SPARK_HEIGHT;
            let inset = scale;
            let p = |x: f32, y: f32| {
                point(
                    gpui_kit::px(f32::from(bounds.origin.x) + x),
                    gpui_kit::px(f32::from(bounds.origin.y) + y),
                )
            };
            let mut baseline = PathBuilder::stroke(gpui_kit::px(scale));
            baseline.move_to(p(inset, height - inset));
            baseline.line_to(p(width - inset, height - inset));
            if let Ok(path) = baseline.build() {
                window.paint_path(path, rgba(0x7f7f7f59));
            }
            if values.len() < 2 {
                return;
            }
            let ceiling = ceiling.max(1.0);
            let divisor = (values.len() - 1) as f32;
            let mut trace = PathBuilder::stroke(gpui_kit::px(1.6 * scale));
            for (index, value) in values.iter().enumerate() {
                let normalized = if value.is_finite() {
                    value.clamp(0.0, ceiling) / ceiling
                } else {
                    0.0
                };
                let x = inset + index as f32 / divisor * (width - 2.0 * inset);
                let y = inset + (1.0 - normalized as f32) * (height - 2.0 * inset);
                if index == 0 {
                    trace.move_to(p(x, y));
                } else {
                    trace.line_to(p(x, y));
                }
            }
            if let Ok(path) = trace.build() {
                window.paint_path(path, rgb(accent));
            }
        },
    )
    .w_full()
    .h(px(14.))
    .into_any_element()
}

// ---------------------------------------------------------------------------
// Misc helpers
// ---------------------------------------------------------------------------

fn chrono_now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or_default()
}

fn push_bounded(history: &mut VecDeque<f64>, value: f64, limit: usize) {
    if history.len() >= limit {
        history.pop_front();
    }
    history.push_back(value);
}

fn percent_text(percent: Option<f64>, unlimited: bool) -> String {
    if unlimited {
        return "∞".into();
    }
    match percent {
        Some(value) if value.is_finite() => format!("{}%", value.clamp(0.0, 100.0).round() as i64),
        _ => "—".into(),
    }
}

fn compact_percent_text(percent: Option<f64>, unlimited: bool) -> String {
    if unlimited {
        return "∞".into();
    }
    match percent {
        Some(value) if value.is_finite() => {
            let clamped = value.clamp(0.0, 100.0);
            format!("{}%", clamped.round() as i64)
        }
        _ => "—".into(),
    }
}

fn format_memory(mib: f64) -> String {
    if mib >= 1024.0 {
        format!("{:.1} GB", mib / 1024.0)
    } else {
        format!("{} MB", mib.round() as i64)
    }
}

fn compact_memory(mib: f64) -> String {
    if mib >= 1024.0 {
        format!("{:.1}G", mib / 1024.0)
    } else {
        format!("{}M", mib.round() as i64)
    }
}

fn format_cpu(percent: Option<f64>, compact: bool) -> String {
    let Some(value) = percent else {
        return "···".into();
    };
    if compact {
        format!("{}%", value.clamp(0.0, 100.0).round() as i64)
    } else if value >= 100.0 {
        format!("{}%", value.round() as i64)
    } else {
        format!("{:.1}%", value)
    }
}

fn metric_level_color(level: &str) -> u32 {
    match level {
        "low" => 0x45ad78,
        "medium" => 0xd4ad2f,
        "high" => 0xe48632,
        "critical" => 0xdf5656,
        _ => DEFAULT_MUTED_TOKEN,
    }
}

fn level_for_cpu(value: Option<f64>) -> &'static str {
    let Some(value) = value else {
        return "unknown";
    };
    if value < 35.0 {
        "low"
    } else if value < 60.0 {
        "medium"
    } else if value < 85.0 {
        "high"
    } else {
        "critical"
    }
}

fn level_for_memory(value: Option<f64>) -> &'static str {
    let Some(value) = value else {
        return "unknown";
    };
    if value < 512.0 {
        "low"
    } else if value < 1024.0 {
        "medium"
    } else if value < 2048.0 {
        "high"
    } else {
        "critical"
    }
}

fn reset_summary(window: Option<&SidebarUsageWindow>, now: i64) -> String {
    let Some(window) = window else {
        return "reset unavailable".into();
    };
    let Some(at) = window.resets_at else {
        if let Some(label) = &window.reset_label {
            return format!("resets {}", strip_reset_prefix(label));
        }
        return "reset unavailable".into();
    };
    let diff_ms = at - now;
    let minutes = (diff_ms / 60_000) as i64;
    if diff_ms >= 60_000 {
        format!("resets in {}", compact_duration(minutes.max(1)))
    } else if diff_ms > 0 {
        "resets in less than a minute".into()
    } else if diff_ms > -60_000 {
        "resets now".into()
    } else {
        format!("reset overdue by {}", compact_duration((-minutes).max(1)))
    }
}

/// Mirrors the Svelte `resetLabel.replace(/^resets?(?:\s+in)?\s*/i, '')`
/// without pulling in the `regex` crate. Keeps the leading word + `in`
/// strip when present, otherwise returns the label verbatim.
fn strip_reset_prefix(label: &str) -> String {
    let trimmed = label.trim_start();
    let lowered = trimmed.to_ascii_lowercase();
    if !lowered.starts_with("reset") && !lowered.starts_with("resets") {
        return trimmed.to_owned();
    }
    let after_reset = if lowered.starts_with("resets") {
        &trimmed[6..]
    } else {
        &trimmed[5..]
    };
    let after_in = after_reset
        .trim_start()
        .strip_prefix("in")
        .map(str::trim_start)
        .unwrap_or(after_reset.trim_start());
    after_in.to_owned()
}

fn compact_duration(minutes: i64) -> String {
    let days = minutes / 1_440;
    let hours = (minutes % 1_440) / 60;
    let remaining = minutes % 60;
    [
        if days > 0 {
            format!("{days}d")
        } else {
            String::new()
        },
        if hours > 0 {
            format!("{hours}h")
        } else {
            String::new()
        },
        if remaining > 0 {
            format!("{remaining}m")
        } else {
            String::new()
        },
    ]
    .into_iter()
    .filter(|segment| !segment.is_empty())
    .collect::<Vec<_>>()
    .join(" ")
}

fn state_label(ring: &SidebarUsageRing) -> String {
    match ring.status {
        SidebarUsageStatus::Ready => {
            if ring.shows_value() {
                "Available".into()
            } else {
                "No allowance data".into()
            }
        }
        SidebarUsageStatus::Loading => "Loading".into(),
        SidebarUsageStatus::Stale => "Stale".into(),
        SidebarUsageStatus::Error => "Could not load".into(),
        SidebarUsageStatus::Unavailable => "Unavailable".into(),
    }
}

// ---------------------------------------------------------------------------
// Widget
// ---------------------------------------------------------------------------

/// Retained compact lower-sidebar widget. Created by the parent
/// (`Desktop::new`) and dropped into the sidebar-footer slot by
/// `render_sidebar`.
pub(crate) enum ResourceUsageEvent { Open }
impl EventEmitter<ResourceUsageEvent> for SidebarUsageWidget {}

pub struct SidebarUsageWidget {
    service: DesktopService,
    snapshot: Rc<Value>,
    revision: Option<String>,
    composed_palette: Option<crate::design::Palette>,
    tile_theme: NativeTileTheme,
    border_opacity: u16,
    sidebar_width: f32,
    map: SidebarUsageMap,
    usage_busy: bool,
    usage_error: Option<String>,
    last_quota_refresh_ms: Option<i64>,
    last_overview_generated_at: Option<i64>,
    quota_overview: Option<Value>,
    account_selection: HashMap<SidebarUsageProvider, String>,
    now_ms: i64,
    metrics: Option<Value>,
    previous_metrics: Option<Value>,
    metrics_history: VecDeque<Value>,
    cpu_percent: Option<f64>,
    cpu_history: VecDeque<f64>,
    memory_history: VecDeque<f64>,
    metrics_busy: bool,
    metrics_error: Option<String>,
    usage_request_generation: u64,
    metrics_request_generation: u64,
    clock_pulse: u64,
    metrics_pulse: u64,
}

impl SidebarUsageWidget {
    /// Create the widget, kicking off the first quota + metrics refresh on
    /// a named `std::thread`. Mirror of `usage::UsagePane::new`.
    pub fn new(
        service: DesktopService,
        snapshot: Rc<Value>,
        revision: Option<&str>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let now = chrono_now_ms();
        let mut widget = Self {
            service,
            snapshot,
            revision: revision.map(str::to_owned),
            composed_palette: None,
            tile_theme: crate::design::tile_theme(),
            border_opacity: crate::design::border_opacity(),
            sidebar_width: 252.,
            map: SidebarUsageMap::new(),
            usage_busy: false,
            usage_error: None,
            last_quota_refresh_ms: None,
            last_overview_generated_at: None,
            quota_overview: None,
            account_selection: HashMap::new(),
            now_ms: now,
            metrics: None,
            previous_metrics: None,
            metrics_history: VecDeque::with_capacity(METRICS_HISTORY),
            cpu_percent: None,
            cpu_history: VecDeque::with_capacity(SPARK_HISTORY + 2),
            memory_history: VecDeque::with_capacity(SPARK_HISTORY + 2),
            metrics_busy: false,
            metrics_error: None,
            usage_request_generation: 0,
            metrics_request_generation: 0,
            clock_pulse: 0,
            metrics_pulse: 0,
        };
        widget.refresh_quota(cx);
        widget.refresh_metrics(cx);
        cx.spawn(async move |widget, cx| {
            let mut seconds = 0_u8;
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                seconds = seconds.wrapping_add(1);
                let alive = widget
                    .update(cx, |widget, cx| {
                        widget.tick_clock(cx);
                        if seconds % 2 == 0 {
                            widget.tick_metrics(cx);
                        }
                    })
                    .is_ok();
                if !alive {
                    break;
                }
            }
        })
        .detach();
        widget
    }

    /// Retain the fully composed desktop palette so this widget can be
    /// embedded as its own GPUI entity instead of flattening its view into
    /// the parent's render tree. Palette, width and native tile theme are
    /// retained together so a live tile-theme change invalidates this entity
    /// even when its palette and width have not changed.
    pub fn set_composed_palette(
        &mut self,
        palette: crate::design::Palette,
        sidebar_width: f32,
        cx: &mut Context<Self>,
    ) {
        let sidebar_width = if sidebar_width.is_finite() { sidebar_width.max(1.) } else { self.sidebar_width };
        let tile_theme = crate::design::tile_theme();
        let border_opacity = crate::design::border_opacity();
        if self.composed_palette != Some(palette)
            || (self.sidebar_width - sidebar_width).abs() > 0.01
            || self.tile_theme != tile_theme
            || self.border_opacity != border_opacity
        {
            self.composed_palette = Some(palette);
            self.sidebar_width = sidebar_width;
            self.tile_theme = tile_theme;
            self.border_opacity = border_opacity;
            cx.notify();
        }
    }

    /// Refresh the cached snapshot. The parent passes its authoritative
    /// revision (`Desktop::self.revision`); the snapshot's own `revision`
    /// key is **not** used as a fallback because the parent owns that
    /// field separately. The snapshot payload (settings, theme, accent,
    /// `usageOverview`) is always ingested so the next render reflects the
    /// latest state, even when the revision is missing. Quota refreshes are
    /// governed by the backend cache lifetime and explicit configuration
    /// refreshes, not by unrelated global revisions.
    pub fn set_snapshot(
        &mut self,
        snapshot: &Value,
        revision: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        self.set_snapshot_shared(Rc::new(snapshot.clone()), revision, cx);
    }

    /// Shared-snapshot form used by the parent desktop. Its published value
    /// is already reference-counted, so the retained widget need not clone
    /// large transcript-bearing state merely to track settings and usage.
    pub fn set_snapshot_shared(
        &mut self,
        snapshot: Rc<Value>,
        revision: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        // Always ingest the latest payload — settings / theme / accent /
        // `usageOverview` must be visible to the very next render.
        self.snapshot = snapshot;
        let next_revision = revision.map(str::to_owned);
        if next_revision.is_none() {
            // Missing revision: ingest state, do not refetch. Render-only
            // refresh is fine; backend stays idle until the parent emits a
            // real revision.
            cx.notify();
            return;
        }
        if self.revision == next_revision {
            cx.notify();
            return;
        }
        self.revision = next_revision;
        cx.notify();
    }

    /// Backward-compatible convenience for callers that still want to
    /// drive the widget from `snapshot["revision"]`. Prefer the
    /// revision-explicit overload — the parent owns the authoritative
    /// revision field and should pass it directly.
    pub fn set_snapshot_from_value(&mut self, snapshot: &Value, cx: &mut Context<Self>) {
        let revision = snapshot.get("revision").and_then(Value::as_str);
        self.set_snapshot(snapshot, revision, cx);
    }

    /// Refresh the clock and reset countdown; also check quota-cache expiry.
    pub fn tick_clock(&mut self, cx: &mut Context<Self>) {
        self.now_ms = chrono_now_ms();
        self.clock_pulse = self.clock_pulse.wrapping_add(1);
        if self
            .last_quota_refresh_ms
            .is_some_and(|last| self.now_ms.saturating_sub(last) >= QUOTA_REFRESH_INTERVAL_MS)
        {
            self.refresh_quota(cx);
        }
        cx.notify();
    }

    /// Issue a metrics read through the named-thread worker, deduplicated by
    /// the `metrics_busy` flag.
    pub fn tick_metrics(&mut self, cx: &mut Context<Self>) {
        self.metrics_pulse = self.metrics_pulse.wrapping_add(1);
        self.refresh_metrics(cx);
    }

    /// Explicit cache-aware quota refresh. The widget also calls this after
    /// the backend cache interval expires. Never called from `render`.
    pub fn refresh_quota(&mut self, cx: &mut Context<Self>) {
        if self.usage_busy {
            return;
        }
        self.usage_request_generation = self.usage_request_generation.wrapping_add(1);
        let request_generation = self.usage_request_generation;
        self.usage_busy = true;
        self.usage_error = None;
        let service = self.service.clone();
        let worker = cx.background_executor().spawn(async move {
            match start_quota_worker(service, "if-stale") {
                Ok(receiver) => receiver.await.unwrap_or_else(|_| {
                    Err("Sidebar usage worker exited without returning a result.".into())
                }),
                Err(error) => Err(error),
            }
        });
        cx.spawn(async move |widget, cx| {
            let result = worker.await;
            let _ = widget.update(cx, |widget, cx| {
                if widget.usage_request_generation != request_generation {
                    return;
                }
                widget.usage_busy = false;
                widget.last_quota_refresh_ms = Some(chrono_now_ms());
                match result {
                    Ok(value) => {
                        widget.last_overview_generated_at =
                            value.get("generatedAt").and_then(Value::as_i64);
                        widget.map =
                            project_sidebar_usage_map(Some(&value), false, "", chrono_now_ms());
                        widget.quota_overview = Some(value);
                    }
                    Err(error) => {
                        widget.usage_error = Some(error);
                        widget.map = project_sidebar_usage_map(
                            None,
                            false,
                            widget.usage_error.as_deref().unwrap_or(""),
                            chrono_now_ms(),
                        );
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn refresh_metrics(&mut self, cx: &mut Context<Self>) {
        if self.metrics_busy {
            return;
        }
        self.metrics_request_generation = self.metrics_request_generation.wrapping_add(1);
        let request_generation = self.metrics_request_generation;
        self.metrics_busy = true;
        let service = self.service.clone();
        let worker = cx.background_executor().spawn(async move {
            match start_metrics_worker(service) {
                Ok(receiver) => receiver.await.unwrap_or_else(|_| {
                    Err("Sidebar metrics worker exited without returning a result.".into())
                }),
                Err(error) => Err(error),
            }
        });
        cx.spawn(async move |widget, cx| {
            let result = worker.await;
            let _ = widget.update(cx, |widget, cx| {
                if widget.metrics_request_generation != request_generation {
                    return;
                }
                widget.metrics_busy = false;
                match result {
                    Ok(value) => widget.ingest_metrics_sample(value),
                    Err(error) => widget.metrics_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn ingest_metrics_sample(&mut self, sample: Value) {
        let now_ms = chrono_now_ms();
        let sampled_at = sample
            .get("sampledAt")
            .and_then(Value::as_i64)
            .unwrap_or(now_ms);
        let cpu_time_ms = sample
            .get("cpuTimeMs")
            .and_then(Value::as_f64)
            .unwrap_or_default();
        let resident_memory_bytes = sample
            .get("residentMemoryBytes")
            .and_then(Value::as_f64)
            .unwrap_or_default();
        if let Some(prev) = &self.previous_metrics {
            let prev_at = prev
                .get("sampledAt")
                .and_then(Value::as_i64)
                .unwrap_or(now_ms);
            let prev_cpu = prev
                .get("cpuTimeMs")
                .and_then(Value::as_f64)
                .unwrap_or_default();
            let elapsed = (sampled_at - prev_at).max(0) as f64;
            let cpu_elapsed = (cpu_time_ms - prev_cpu).max(0.0);
            if elapsed > 0.0 {
                let percent = (cpu_elapsed / elapsed) * 100.0;
                let clamped = percent.max(0.0);
                self.cpu_percent = Some(clamped);
                push_bounded(&mut self.cpu_history, clamped, SPARK_HISTORY);
            }
        }
        self.previous_metrics = Some(sample.clone());
        self.metrics = Some(sample.clone());
        self.metrics_history.push_back(sample);
        while self.metrics_history.len() > METRICS_HISTORY {
            self.metrics_history.pop_front();
        }
        let memory_mib = resident_memory_bytes / (1024.0 * 1024.0);
        push_bounded(&mut self.memory_history, memory_mib, SPARK_HISTORY);
        self.metrics_error = None;
    }

    /// Re-issue a quota refresh with `policy=refresh`, ignoring cache. The
    /// parent can call this after a provider account/configuration change or
    /// from a user refresh affordance. Never invoked from render.
    pub fn refresh_quota_force(&mut self, cx: &mut Context<Self>) {
        if self.usage_busy {
            return;
        }
        self.usage_request_generation = self.usage_request_generation.wrapping_add(1);
        let request_generation = self.usage_request_generation;
        self.usage_busy = true;
        self.usage_error = None;
        let service = self.service.clone();
        let worker = cx.background_executor().spawn(async move {
            match start_quota_worker(service, "refresh") {
                Ok(receiver) => receiver.await.unwrap_or_else(|_| {
                    Err("Sidebar usage worker exited without returning a result.".into())
                }),
                Err(error) => Err(error),
            }
        });
        cx.spawn(async move |widget, cx| {
            let result = worker.await;
            let _ = widget.update(cx, |widget, cx| {
                if widget.usage_request_generation != request_generation {
                    return;
                }
                widget.usage_busy = false;
                widget.last_quota_refresh_ms = Some(chrono_now_ms());
                match result {
                    Ok(value) => {
                        widget.last_overview_generated_at =
                            value.get("generatedAt").and_then(Value::as_i64);
                        widget.map =
                            project_sidebar_usage_map(Some(&value), false, "", chrono_now_ms());
                        widget.quota_overview = Some(value);
                    }
                    Err(error) => {
                        widget.usage_error = Some(error);
                        widget.map = project_sidebar_usage_map(
                            None,
                            false,
                            widget.usage_error.as_deref().unwrap_or(""),
                            chrono_now_ms(),
                        );
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn overview_payload(&self) -> Option<&Value> {
        let published = self
            .snapshot
            .get("usageOverview")
            .filter(|value| value.is_object());
        match (published, self.quota_overview.as_ref()) {
            (Some(published), Some(fetched)) => {
                if published["generatedAt"].as_i64().unwrap_or(0)
                    >= fetched["generatedAt"].as_i64().unwrap_or(0)
                {
                    Some(published)
                } else {
                    Some(fetched)
                }
            }
            (published, fetched) => fetched.or(published),
        }
    }

    /// A collapsed Tern sidebar keeps all providers and resource readings
    /// visible, but stacks them instead of squeezing the expanded layout.
    fn compact_tern(&self) -> bool {
        crate::design::tile_theme() == NativeTileTheme::Tern
            && self.sidebar_width * crate::design::interface_scale() as f32 / 100. <= 100.
    }

    /// Public render entry. Produces the lower sidebar statistics row
    /// containing the rings strip, the clock face, the metrics summary and
    /// (when expanded) the retained per-provider detail panel.
    ///
    /// `palette` is the **fully resolved** palette the parent already
    /// composed — the same value `Desktop::apply_native_theme` returns
    /// (stored light/dark preset, custom accent, contrast, surface tint,
    /// `native_theme_preset` override). Passing the resolved palette
    /// keeps source fidelity with the rest of the native shell, which
    /// uses the composed palette, not the raw `design::palette(...)`
    /// preset. When the parent cannot supply one, it may pass `None`
    /// and the widget falls back to `design::palette(...)` so the
    /// widget remains usable in isolation.
    pub fn build_widget(
        &mut self,
        palette: Option<crate::design::Palette>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = palette.unwrap_or_else(|| {
            crate::design::palette(&self.snapshot["settings"], window.appearance())
        });
        let merged = project_sidebar_usage_map(
            self.overview_payload(),
            self.usage_busy,
            self.usage_error.as_deref().unwrap_or(""),
            self.now_ms,
        );
        self.map = merged.clone();
        // Use one diameter for the entire footer, including the clock's
        // exact four-pixel size increment, so resize/account changes agree.
        let account_controls = merged.get(&SidebarUsageProvider::Codex)
            .is_some_and(|ring| ring.accounts.len() > 1);
        let ring_layout = if self.compact_tern() {
            let scale = crate::design::interface_scale() as f32 / 100.;
            RingRowLayout { diameter: 24. / scale, gap: 4. / scale, edge_padding: 4. / scale }
        } else {
            provider_ring_layout(self.sidebar_width, account_controls)
        };

        let mut root = div()
            .id("sidebar-usage-stats")
            .w_full()
            .flex()
            .flex_col()
            .gap_0();
        root = if crate::design::tile_theme() == NativeTileTheme::Tern {
            root
                .min_w_0()
                .flex_shrink_0()
        } else {
            root.border_t_1().border_color(rgb(palette.line))
        };

        // Provider rings live in the workspace bar; resource readings stay here.
        root = root.child(self.render_clock_row(ring_layout.diameter + 4., palette, window, cx));
        if let Some(error) = &self.usage_error {
            root = root.child(
                div()
                    .text_size(px(10.))
                    .text_color(rgb(0xf87171))
                    .child(format!("Allowance refresh failed: {error}")),
            );
        }
        if let Some(error) = &self.metrics_error {
            root = root.child(
                div()
                    .text_size(px(10.))
                    .text_color(rgb(0xf87171))
                    .child(format!("Metrics refresh failed: {error}")),
            );
        }
        // Quiet unused field warnings for fields that exist for future hooks
        // (`last_quota_refresh_ms`, `clock_pulse`, `metrics_pulse`).
        let _ = (
            self.last_quota_refresh_ms,
            self.clock_pulse,
            self.metrics_pulse,
        );
        root.into_any_element()
    }

    fn build_workspace_rings(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let palette = self.composed_palette.unwrap_or_else(|| {
            crate::design::palette(&self.snapshot["settings"], window.appearance())
        });
        let merged = project_sidebar_usage_map(self.overview_payload(), self.usage_busy,
            self.usage_error.as_deref().unwrap_or(""), self.now_ms);
        let account_controls = merged.get(&SidebarUsageProvider::Codex)
            .is_some_and(|ring| ring.accounts.len() > 1);
        let layout = RingRowLayout { diameter: 28., gap: 5., edge_padding: 0. };
        let width = SidebarUsageProvider::ALL.len() as f32 * layout.diameter
            + (SidebarUsageProvider::ALL.len() - 1) as f32 * layout.gap
            + if account_controls { CODEX_ACCOUNT_ARROWS_WIDTH } else { 0. };
        // This retained header view already observes the sidebar's one-second
        // tick, so the clock shares its time and palette without another timer.
        div().id("workspace-usage-rings").flex_shrink_0()
            .flex().items_center().gap(px(10.))
            .child(div().w(px(width)).flex_shrink_0()
                .child(self.render_rings_row(&merged, layout, palette, true, cx)))
            .child(div().id("workspace-analog-clock")
                .debug_selector(|| "workspace-analog-clock".into())
                .flex_shrink_0()
                .child(crate::native_clock::clock_face(self.now_ms.max(0) as u64, palette, 28.)))
            .into_any_element()
    }

    fn render_rings_row(
        &mut self,
        map: &SidebarUsageMap,
        layout: RingRowLayout,
        palette: crate::design::Palette,
        in_header: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        // Five providers share the available width, including the existing
        // Codex account arrows. SVG artwork supplies additional clear space
        // outside its strokes; token-sized gaps and edge padding keep it airy.
        let mut strip = div()
            .id("sidebar-usage-rings")
            .w_full()
            .py(px(if in_header { 0. } else { crate::design::space::INSET }))
            .flex()
            .flex_row()
            .gap(px(layout.gap))
            .items_center()
            .justify_center()
            .px(px(layout.edge_padding));
        if !in_header && self.compact_tern() {
            strip = strip.flex_col().py(gpui_kit::px(4.));
        }
        for provider in SidebarUsageProvider::ALL {
            let ring = map.get(&provider).cloned().unwrap_or_default();
            strip = strip.child(self.render_ring(provider, &ring, layout.diameter, palette, !in_header && self.compact_tern(), cx));
        }
        strip.into_any_element()
    }

    fn render_ring(
        &mut self,
        provider: SidebarUsageProvider,
        ring: &SidebarUsageRing,
        ring_size: f32,
        palette: crate::design::Palette,
        compact: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let active = ring.active.as_ref();
        let weekly = ring.weekly.as_ref();
        let fallback = Rgb(
            ((palette.ink >> 16) & 0xff) as u8,
            ((palette.ink >> 8) & 0xff) as u8,
            (palette.ink & 0xff) as u8,
        );
        let active_color = usage_color(active).unwrap_or(fallback);
        let weekly_color = usage_color(weekly).unwrap_or(fallback);
        let active_dim = matches!(
            ring.status,
            SidebarUsageStatus::Loading
                | SidebarUsageStatus::Unavailable
                | SidebarUsageStatus::Error
        );
        let active_percent = active.and_then(|w| w.used_percent).unwrap_or(0.0);
        let weekly_percent = weekly.and_then(|w| w.used_percent).unwrap_or(0.0);
        let active_image = ring_active_svg(active_percent, active_color, active_dim);
        let weekly_image = ring_weekly_svg(weekly_percent, weekly_color, active_dim);

        let selection_key = self
            .account_selection
            .get(&provider)
            .cloned()
            .or_else(|| ring.accounts.first().map(|a| a.key.clone()));
        let selected_account = ring
            .accounts
            .iter()
            .find(|account| Some(&account.key) == selection_key.as_ref())
            .cloned();

        // Match the production icon-only view: provider artwork is centered
        // inside the usage strokes rather than replacing it with a letter.
        let provider_icon = crate::branding::provider_for_ink(provider.id(), "", palette.ink);

        let wrapper_id = format!("sidebar-usage-ring-{}", provider.id());
        let click_id = format!("sidebar-usage-ring-click-{}", provider.id());

        let mut ring_container = div()
            .id(SharedString::from(format!(
                "sidebar-usage-ring-container-{}",
                provider.id()
            )))
            .flex()
            .flex_shrink_0()
            .flex_row()
            .items_center()
            .justify_center();
        if provider == SidebarUsageProvider::Codex && ring.accounts.len() > 1 {
            let previous_provider = provider;
            ring_container = ring_container.child(
                Button::new("sidebar-usage-account-previous")
                    .ghost()
                    .xsmall()
                    .compact()
                    .w(if compact { gpui_kit::px(16.) } else { px(22.) })
                    .h(if compact { gpui_kit::px(24.) } else { px(30.) })
                    .when(compact, |button| button.min_w_0())
                    .p_0()
                    .icon(
                        Icon::default()
                            .data(icons_assets::ChevronLeft.1)
                            .with_size(if compact { gpui_kit::px(10.) } else { px(13.) }),
                    )
                    .tooltip("Previous Codex account")
                    .accessibility_label("Previous Codex account")
                    .on_click(cx.listener(move |widget, _, _, cx| {
                        widget.cycle_account(previous_provider, -1, cx);
                    })),
            );
        }
        let selected_window = selected_account.clone();
        let dialog_ring = ring.clone();
        let dialog_provider = provider;
        let face_content = div()
            .w(px(ring_size))
            .h(px(ring_size))
            .relative()
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .child(img(RING_TRACK.clone()).size_full()),
            )
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .child(img(active_image).size_full()),
            )
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .child(img(weekly_image).size_full()),
            )
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        provider_icon
                            .map(|icon| img(icon).w(px(ring_size * 28. / 72.)).h(px(ring_size * 28. / 72.)).into_any_element())
                            .unwrap_or_else(|| div().size_0().into_any_element()),
                    ),
            );
        let click_title = format!("{} usage", provider.label());
        let click_label = format!("Show {} usage details", provider.label());
        let ring_face = Button::new(SharedString::from(click_id))
            .ghost()
            .xsmall()
            .compact()
            .p_0()
            .min_w_0()
            .accessibility_label(click_label)
            .w(px(ring_size))
            .h(px(ring_size))
            .on_click(cx.listener(move |_, _, window, cx| {
                let title = click_title.clone();
                let ring = dialog_ring.clone();
                let account = selected_window.clone();
                window.open_dialog(cx, move |dialog, _, _| {
                    dialog
                        .title(title.clone())
                        .width(px(420.))
                        .child(provider_dialog_content(
                            dialog_provider,
                            &ring,
                            account.as_ref(),
                            palette,
                        ))
                });
            }))
            .child(face_content);
        ring_container = ring_container.child(ring_face);
        if provider == SidebarUsageProvider::Codex && ring.accounts.len() > 1 {
            ring_container = ring_container.child(
                Button::new("sidebar-usage-account-next")
                    .ghost()
                    .xsmall()
                    .compact()
                    .w(if compact { gpui_kit::px(16.) } else { px(22.) })
                    .h(if compact { gpui_kit::px(24.) } else { px(30.) })
                    .when(compact, |button| button.min_w_0())
                    .p_0()
                    .icon(
                        Icon::default()
                            .data(icons_assets::ChevronRight.1)
                            .with_size(if compact { gpui_kit::px(10.) } else { px(13.) }),
                    )
                    .tooltip("Next Codex account")
                    .accessibility_label("Next Codex account")
                    .on_click(cx.listener(move |widget, _, _, cx| {
                        widget.cycle_account(provider, 1, cx);
                    })),
            );
        }
        let wrap = div()
            .id(SharedString::from(wrapper_id))
            .flex()
            .flex_col()
            .items_center()
            .gap_0()
            .child(ring_container);

        wrap.into_any_element()
    }

    fn render_clock_row(
        &self,
        clock_size: f32,
        palette: crate::design::Palette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let cpu = self.cpu_percent;
        let memory_mib = self
            .metrics
            .as_ref()
            .and_then(|sample| sample.get("residentMemoryBytes"))
            .and_then(Value::as_f64)
            .map(|bytes| bytes / (1024.0 * 1024.0));
        let memory_text = memory_mib
            .map(format_memory)
            .unwrap_or_else(|| "···".into());
        let compact_memory_text = memory_mib
            .map(compact_memory)
            .unwrap_or_else(|| "···".into());
        let memory_ceiling = memory_history_ceiling(&self.memory_history);
        let cpu_ceiling = 100.0_f64;
        let cpu_level = level_for_cpu(cpu);
        let memory_level = level_for_memory(memory_mib);
        let trace_color = |level| match level {
            "medium" | "high" | "critical" => metric_level_color(level),
            _ => palette.accent,
        };
        let cpu_spark = native_sparkline(
            self.cpu_history.iter().copied(),
            cpu_ceiling,
            trace_color(cpu_level),
        );
        let memory_spark = native_sparkline(
            self.memory_history.iter().copied(),
            memory_ceiling,
            trace_color(memory_level),
        );

        let mut body = div()
            .id("sidebar-usage-clock-row")
            .w_full()
            .border_t_1()
            .border_color(rgb(palette.line))
            .p(px(crate::design::space::INSET))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(crate::design::space::TIGHT));
        if self.compact_tern() {
            body = body.flex_col().p(gpui_kit::px(4.)).gap(gpui_kit::px(4.));
        }
        body = body.child(crate::native_clock::clock_face(
            self.now_ms.max(0) as u64,
            palette,
            clock_size,
        ));
        body = body.child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .when(self.compact_tern(), |metrics| metrics.w_full())
                .gap(px(6.))
                .child(self.render_metric_row(
                    "CPU",
                    cpu_level,
                    cpu,
                    format_cpu(cpu, false),
                    format_cpu(cpu, true),
                    cpu_spark,
                ))
                .child(self.render_metric_row(
                    "RAM",
                    memory_level,
                    memory_mib,
                    memory_text,
                    compact_memory_text,
                    memory_spark,
                )),
        );
        Button::new("sidebar-resource-usage")
            .ghost()
            .rounded(gpui_kit::component::button::ButtonRounded::None)
            .w_full()
            .p_0()
            .compact()
            .h_auto()
            .min_h(px(clock_size))
            .accessibility_label("Open app resource usage")
            .tooltip("GUI, backend and child processes")
            .on_click(cx.listener(|_, _, _, cx| cx.emit(ResourceUsageEvent::Open)))
            .child(body)
            .into_any_element()
    }

    pub(crate) fn resource_revision(&self) -> (i64, Option<String>, Option<crate::design::Palette>) {
        (self.metrics_history.back().and_then(|sample| sample["sampledAt"].as_i64()).unwrap_or_default(), self.metrics_error.clone(), self.composed_palette)
    }

    /// Default-open the backend root and its direct harness groups. The owner
    /// records keys it has seen so manually collapsed groups stay collapsed.
    pub(crate) fn resource_default_expanded_keys(&self) -> Vec<String> {
        let Some(sample) = self.metrics_history.back() else { return Vec::new(); };
        let Some(root_pid) = sample["rootPid"].as_i64() else { return Vec::new(); };
        let Some(processes) = sample["processes"].as_array() else { return Vec::new(); };
        if !processes.iter().any(|process| process_identity(process).0 == root_pid) {
            return Vec::new();
        }
        let mut keys = vec![format!("root:{root_pid}")];
        keys.extend(processes.iter().filter(|process| {
            process["parentPid"].as_i64() == Some(root_pid) && harness_label(process).is_some()
        }).map(|process| {
            let (pid, started) = process_identity(process);
            format!("harness:{pid}:{started}")
        }));
        keys
    }

    pub(crate) fn resource_content(
        &self, palette: crate::design::Palette,
        expanded: &std::collections::HashSet<String>, toggle: ResourceGroupToggle,
    ) -> AnyElement {
        let samples = self.metrics_history.iter().cloned().collect::<Vec<_>>();
        metrics_dialog_content(&samples, self.metrics_error.as_deref(), palette, expanded, toggle)
    }
    pub(crate) fn resource_header_height(&self) -> f32 {
        match self.snapshot["settings"]["interfaceDensity"].as_str().unwrap_or_default() {
            "tight" => 26. + 2. * crate::design::space::TIGHT,
            "spacious" => 34. + 2. * crate::design::space::GROUP,
            _ => 30. + 2. * crate::design::space::INSET,
        }
    }
    pub(crate) fn resource_palette(&self, window: &Window) -> crate::design::Palette {
        self.composed_palette.unwrap_or_else(|| crate::settings::palette(&self.snapshot["settings"], window.appearance()))
    }

    fn render_metric_row(
        &self,
        label: &'static str,
        level: &'static str,
        _value: Option<f64>,
        full_text: String,
        compact_text: String,
        sparkline: AnyElement,
    ) -> AnyElement {
        let token = metric_level_color(level);
        // The shared metrics contract is sampled by the backend server in
        // GPUI, so this row covers the backend and its child processes and
        // excludes the separate GUI process. Keep the visible CPU/RAM labels
        // and geometry aligned with Tauri; expose the native-specific scope
        // through the supported Div tooltip and accessibility label.
        let (tooltip, accessibility_label) = match label {
            "CPU" => (
                "Backend CPU — backend and child processes; GUI excluded",
                "Backend CPU",
            ),
            "RAM" => (
                "Backend RAM — backend and child processes; GUI excluded",
                "Backend RAM — backend and child processes; GUI excluded",
            ),
            _ => ("Backend process metrics", "Backend process metrics"),
        };
        if self.compact_tern() {
            return div()
                .id(format!("sidebar-usage-metric-{label}"))
                .aria_label(accessibility_label)
                .managed_tooltip(move |window, cx| Tooltip::new(tooltip).build(window, cx))
                .w_full()
                .min_w_0()
                .flex()
                .flex_col()
                .child(
                    div()
                        .w_full()
                        .flex()
                        .justify_between()
                        .text_size(gpui_kit::px(8.))
                        .font_family(crate::design::FONT_MONO)
                        .child(div().text_color(rgb(DEFAULT_MUTED_TOKEN)).child(label))
                        .child(div().text_color(rgb(token)).child(compact_text)),
                )
                .child(
                    div()
                        .w_full()
                        .h(gpui_kit::px(14.))
                        .overflow_hidden()
                        .child(sparkline),
                )
                .into_any_element();
        }
        div()
            .id(format!("sidebar-usage-metric-{label}"))
            .aria_label(accessibility_label)
            .managed_tooltip(move |window, cx| Tooltip::new(tooltip).build(window, cx))
            .flex()
            .flex_row()
            .items_center()
            .gap_1()
            .child(
                div()
                    .w(px(24.))
                    .text_size(px(9.))
                    .text_color(rgb(DEFAULT_MUTED_TOKEN))
                    .font_family(crate::design::FONT_MONO)
                    .child(label),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h(px(14.))
                    .overflow_hidden()
                    .child(sparkline),
            )
            .child(
                div()
                    .w(px(38.))
                    .flex_shrink_0()
                    .text_size(px(10.))
                    .text_color(rgb(token))
                    .font_family(crate::design::FONT_MONO)
                    .child(if full_text == compact_text {
                        full_text
                    } else {
                        compact_text
                    }),
            )
            .into_any_element()
    }

    fn cycle_account(
        &mut self,
        provider: SidebarUsageProvider,
        delta: isize,
        cx: &mut Context<Self>,
    ) {
        let Some(ring) = self.map.get(&provider) else {
            return;
        };
        if ring.accounts.len() < 2 {
            return;
        }
        let current_key = self
            .account_selection
            .get(&provider)
            .or_else(|| ring.accounts.first().map(|account| &account.key));
        let current_index = current_key
            .and_then(|key| ring.accounts.iter().position(|account| &account.key == key))
            .unwrap_or(0) as isize;
        let account_count = ring.accounts.len() as isize;
        let next_index = (current_index + delta).rem_euclid(account_count) as usize;
        self.account_selection
            .insert(provider, ring.accounts[next_index].key.clone());
        cx.notify();
    }

    /// Public access for tests and parent queries. Last projection seen by
    /// the renderer; never fabricated.
    pub fn current_map(&self) -> SidebarUsageMap {
        self.map.clone()
    }

    /// Public access for tests and parent queries. Whether the metrics
    /// sample slot is filled.
    pub fn has_metrics(&self) -> bool {
        self.metrics.is_some()
    }
}

fn merge_rings(cached: SidebarUsageRing, next: SidebarUsageRing) -> SidebarUsageRing {
    SidebarUsageRing {
        provider: next.provider,
        status: merge_status(cached.status, next.status),
        active: next.active.or(cached.active),
        weekly: next.weekly.or(cached.weekly),
        message: next.message.or(cached.message),
        updated_at: next.updated_at.or(cached.updated_at),
        account_label: next.account_label.or(cached.account_label),
        accounts: if next.accounts.is_empty() {
            cached.accounts
        } else {
            next.accounts
        },
    }
}

fn provider_dialog_content(
    provider: SidebarUsageProvider,
    ring: &SidebarUsageRing,
    account: Option<&SidebarUsageAccount>,
    palette: crate::design::Palette,
) -> AnyElement {
    // A selected account owns all displayed fields, including absent values.
    // Never combine its status with another account's allowance or timestamp.
    let selected = account.map(|account| SidebarUsageRing {
        provider,
        status: account.status,
        active: account.active.clone(),
        weekly: account.weekly.clone(),
        message: account.message.clone(),
        updated_at: account.updated_at,
        account_label: Some(account.label.clone()),
        accounts: Vec::new(),
    });
    let ring = selected.as_ref().unwrap_or(ring);
    let active = account
        .and_then(|account| account.active.as_ref())
        .or(ring.active.as_ref());
    let weekly = account
        .and_then(|account| account.weekly.as_ref())
        .or(ring.weekly.as_ref());
    let mut body = div()
        .id(format!("sidebar-usage-dialog-{}", provider.id()))
        .flex()
        .flex_col()
        .gap_2()
        .p_3()
        .text_color(rgb(palette.ink));
    body = body.child(
        div()
            .text_size(px(12.))
            .text_color(rgb(palette.muted))
            .child(format!("{} · {}", state_label(ring), provider.label())),
    );
    if let Some(label) = account
        .map(|account| account.label.as_str())
        .or(ring.account_label.as_deref())
    {
        body = body.child(format!("Account: {label}"));
    }
    for window in [active, weekly].into_iter().flatten() {
        let used = if window.unlimited {
            "Unlimited".to_owned()
        } else {
            window
                .used_percent
                .filter(|value| value.is_finite() && (0.0..=100.0).contains(value))
                .map(|value| format!("{value:.0}%"))
                .unwrap_or_else(|| "Not reported".into())
        };
        let remaining = if window.unlimited {
            "Unlimited".to_owned()
        } else {
            window
                .used_percent
                .filter(|value| value.is_finite() && (0.0..=100.0).contains(value))
                .map(|value| format!("{:.0}%", 100.0 - value))
                .unwrap_or_else(|| "Not reported".into())
        };
        let reset = window
            .resets_at
            .map(|stamp| {
                chrono::DateTime::<chrono::Utc>::from_timestamp_millis(stamp)
                    .map(|time| {
                        time.with_timezone(&chrono::Local)
                            .format("%Y-%m-%d %H:%M %Z")
                            .to_string()
                    })
                    .unwrap_or_else(|| "Not reported".into())
            })
            .or_else(|| {
                window
                    .reset_label
                    .as_deref()
                    .as_deref()
                    .map(strip_reset_prefix)
            })
            .unwrap_or_else(|| "Not reported".into());
        body = body.child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .p_2()
                .rounded_md()
                .bg(crate::design::surface_color(palette.soft))
                .child(div().text_size(px(14.)).child(window.label.clone()))
                .child(format!("Used: {used}"))
                .child(format!("Remaining: {remaining}"))
                .child(format!("Resets: {reset}")),
        );
    }
    if active.is_none() && weekly.is_none() {
        body = body.child("No allowance windows reported.");
    }
    if let Some(message) = account
        .and_then(|account| account.message.as_ref())
        .or(ring.message.as_ref())
    {
        if !message.is_empty() {
            body = body.child(message.clone());
        }
    }
    if let Some(updated_at) = account
        .and_then(|account| account.updated_at)
        .or(ring.updated_at)
    {
        let checked = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(updated_at)
            .map(|time| {
                time.with_timezone(&chrono::Local)
                    .format("%Y-%m-%d %H:%M %Z")
                    .to_string()
            })
            .unwrap_or_else(|| "Not reported".into());
        body = body.child(
            div()
                .text_size(px(11.))
                .text_color(rgb(palette.muted))
                .child(format!("Last checked: {checked}")),
        );
    } else {
        body = body.child(
            div()
                .text_size(px(11.))
                .text_color(rgb(palette.muted))
                .child("Last checked: Not reported"),
        );
    }
    body.into_any_element()
}

fn merge_status(cached: SidebarUsageStatus, next: SidebarUsageStatus) -> SidebarUsageStatus {
    // A cached `ready` projection survives a transient `loading` overlay
    // produced by an in-flight refresh so the widget does not flicker.
    if next == SidebarUsageStatus::Loading
        && cached != SidebarUsageStatus::Loading
        && cached != SidebarUsageStatus::Unavailable
    {
        return cached;
    }
    next
}

fn memory_history_ceiling(history: &VecDeque<f64>) -> f64 {
    history.iter().copied().fold(512.0_f64, f64::max).max(512.0) * 1.08
}

fn metric_number(value: &Value, key: &str) -> f64 {
    value.get(key).and_then(Value::as_f64).unwrap_or_default()
}

fn metric_text(value: &Value, key: &str, fallback: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(fallback)
        .to_owned()
}

fn metric_cpu_delta(current: &Value, previous: &Value) -> Option<f64> {
    let elapsed = metric_number(current, "sampledAt") - metric_number(previous, "sampledAt");
    let cpu = metric_number(current, "cpuTimeMs") - metric_number(previous, "cpuTimeMs");
    (elapsed > 0.0 && cpu >= 0.0).then_some(cpu / elapsed * 100.0)
}

fn process_identity(process: &Value) -> (i64, i64) {
    (
        process
            .get("pid")
            .and_then(Value::as_i64)
            .unwrap_or_default(),
        process
            .get("startedAt")
            .and_then(Value::as_i64)
            .unwrap_or_default(),
    )
}

fn harness_label(process: &Value) -> Option<String> {
    let name = metric_text(process, "name", "Process").to_lowercase();
    if name.contains("mona") {
        Some("Mona".into())
    } else if name.contains("codex") {
        Some("Codex".into())
    } else if name.contains("claude") {
        Some("Claude".into())
    } else if name.contains("opencode") || name.contains("open-code") {
        Some("OpenCode".into())
    } else if name.contains("hermes") {
        Some("Hermes".into())
    } else if name.contains("minimax") || name.contains("mmx") {
        Some("MiniMax".into())
    } else {
        None
    }
}

fn process_cpu(
    current_sample: &Value,
    previous_sample: Option<&Value>,
    process: &Value,
) -> Option<f64> {
    let before = previous_sample?;
    let prior = before
        .get("processes")?
        .as_array()?
        .iter()
        .find(|p| process_identity(p) == process_identity(process))?;
    let elapsed = metric_number(current_sample, "sampledAt") - metric_number(before, "sampledAt");
    let cpu = metric_number(process, "cpuTimeMs") - metric_number(prior, "cpuTimeMs");
    (elapsed > 0.0 && cpu >= 0.0).then_some(cpu / elapsed * 100.0)
}

// Immediate native vector paint avoids a fresh asynchronous SVG asset load
// (and its blank frame) whenever a new metrics sample arrives.
fn metric_chart(values: &[f64], ceiling: f64, color: u32, _width: u32, height: u32) -> AnyElement {
    let normalized = values.iter().map(|value| value.max(0.).min(ceiling) / ceiling.max(1.)).collect::<Vec<_>>();
    canvas(|_, _, _| (), move |bounds, _, window, _| {
        let mut baseline = PathBuilder::stroke(px(1.));
        let y = bounds.bottom() - px(1.);
        baseline.move_to(point(bounds.left(), y));
        baseline.line_to(point(bounds.right(), y));
        if let Ok(path) = baseline.build() { window.paint_path(path, rgb(color)); }
        if normalized.len() < 2 { return; }
        let mut line = PathBuilder::stroke(px(2.));
        let width = (bounds.size.width - px(4.)).max(px(0.));
        let height = (bounds.size.height - px(6.)).max(px(0.));
        for (index, value) in normalized.iter().enumerate() {
            let p = point(bounds.left() + px(2.) + width * (index as f32 / (normalized.len() - 1) as f32),
                bounds.top() + px(3.) + height * (1. - *value as f32));
            if index == 0 { line.move_to(p); } else { line.line_to(p); }
        }
        if let Ok(path) = line.build() { window.paint_path(path, rgb(color)); }
    }).w_full().h(px(height as f32)).into_any_element()
}

pub(crate) type ResourceGroupToggle = Rc<dyn Fn(String, &mut Window, &mut App)>;

fn metrics_dialog_content(
    samples: &[Value],
    error: Option<&str>,
    palette: crate::design::Palette,
    expanded: &std::collections::HashSet<String>,
    toggle: ResourceGroupToggle,
) -> AnyElement {
    let latest = samples.last();
    let previous = samples.get(samples.len().saturating_sub(2));
    let cpu = latest
        .zip(previous)
        .and_then(|(now, before)| metric_cpu_delta(now, before));
    let memory = latest.map(|sample| metric_number(sample, "residentMemoryBytes"));
    let live_count = latest
        .and_then(|sample| sample.get("processes").and_then(Value::as_array))
        .map_or(0, Vec::len);
    let elapsed = latest
        .zip(samples.first())
        .map(|(last, first)| {
            (metric_number(last, "sampledAt") - metric_number(first, "sampledAt")).max(0.0) / 1000.0
        })
        .unwrap_or_default();
    let cpu_history = samples
        .windows(2)
        .filter_map(|pair| metric_cpu_delta(&pair[1], &pair[0]))
        .collect::<Vec<_>>();
    let ram_history = samples
        .iter()
        .map(|sample| metric_number(sample, "residentMemoryBytes") / 1_048_576.0)
        .collect::<Vec<_>>();
    let cpu_graph = metric_chart(
        &cpu_history,
        cpu_history.iter().copied().fold(100.0, f64::max) * 1.08,
        palette.accent,
        640,
        112,
    );
    let ram_graph = metric_chart(
        &ram_history,
        ram_history.iter().copied().fold(128.0, f64::max) * 1.08,
        0x6e9f82,
        640,
        112,
    );
    let mut content = div()
        .id("resource-usage-content")
        .w_full()
        .min_w_0()
        .font_family(crate::design::FONT_SANS)
        .text_size(px(12.))
        .flex()
        .flex_col()
        .gap_3()
        .p_2();
    if let Some(error) = error {
        content = content.child(div().text_color(rgb(0xdf5656)).child(error.to_owned()));
    }
    if let Some(latest) = latest {
        let cpu_text = cpu
            .map(|n| format!("{n:.1}%"))
            .unwrap_or_else(|| "···".into());
        let ram_text = memory
            .map(|n| format_memory(n / 1_048_576.0))
            .unwrap_or_else(|| "···".into());
        content = content.child(
            div()
                .w_full().min_w_0().flex().flex_col()
                .gap_2()
                .child(metric_summary_card(
                    "CPU",
                    &cpu_text,
                    "GUI + backend + child processes",
                    palette,
                ))
                .child(metric_summary_card(
                    "RAM",
                    &ram_text,
                    &format!(
                        "{live_count} live process{}",
                        if live_count == 1 { "" } else { "es" }
                    ),
                    palette,
                ))
                .child(metric_summary_card(
                    "WINDOW",
                    &format!("{}s history", elapsed.round() as u64),
                    "Sampled every 2 seconds · live",
                    palette,
                )),
        );
        content = content.child(
            div()
                .w_full().min_w_0().flex().flex_col()
                .gap_3()
                .child(metric_graph_card(
                    "CPU usage",
                    &cpu_text,
                    cpu_graph,
                    palette,
                ))
                .child(metric_graph_card(
                    "RAM usage",
                    &ram_text,
                    ram_graph,
                    palette,
                )),
        );
        content = content.child(render_process_breakdown(
            samples, latest, palette, error, expanded, toggle,
        ));
    } else if error.is_none() {
        content = content.child(
            div()
                .p_6()
                .text_color(rgb(palette.muted))
                .child("Collecting the first process sample…"),
        );
    }
    content.into_any_element()
}

fn metric_summary_card(
    title: &str,
    value: &str,
    detail: &str,
    palette: crate::design::Palette,
) -> AnyElement {
    div()
        .w_full().min_w_0()
        .flex()
        .flex_col()
        .gap_1()
        .p_3()
        .rounded_md()
        .border_1()
        .border_color(rgb(palette.line))
        .bg(crate::design::surface_color(palette.soft))
        .child(
            div()
                .text_size(px(9.))
                .text_color(rgb(palette.muted))
                .child(title.to_owned()),
        )
        .child(
            div()
                .font_family(crate::design::FONT_MONO)
                .text_size(px(19.))
                .child(value.to_owned()),
        )
        .child(
            div()
                .text_size(px(10.))
                .text_color(rgb(palette.muted))
                .child(detail.to_owned()),
        )
        .into_any_element()
}

fn metric_graph_card(
    title: &str,
    value: &str,
    graph: AnyElement,
    palette: crate::design::Palette,
) -> AnyElement {
    div()
        .w_full()
        .min_w_0()
        .flex()
        .flex_col()
        .gap_2()
        .p_3()
        .rounded_md()
        .border_1()
        .border_color(rgb(palette.line))
        .bg(crate::design::surface_color(palette.panel))
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(div().text_color(rgb(palette.muted)).child(title.to_owned()))
                .child(
                    div()
                        .font_family(crate::design::FONT_MONO)
                        .child(value.to_owned()),
                ),
        )
        .child(graph)
        .into_any_element()
}

fn metrics_memory_color(bytes: f64, palette: crate::design::Palette) -> u32 {
    if bytes > 500.0 * 1_048_576.0 {
        if palette.dark { 0xff727b } else { 0xb82f3e }
    } else if bytes > 250.0 * 1_048_576.0 {
        if palette.dark { 0xffb15c } else { 0xa95a0d }
    } else {
        palette.ink
    }
}

fn render_process_breakdown(
    samples: &[Value],
    latest: &Value,
    palette: crate::design::Palette,
    error: Option<&str>,
    expanded: &std::collections::HashSet<String>,
    toggle: ResourceGroupToggle,
) -> AnyElement {
    let processes = latest
        .get("processes")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let root_pid = latest
        .get("rootPid")
        .and_then(Value::as_i64)
        .unwrap_or_default();
    let previous = samples.get(samples.len().saturating_sub(2));
    let mut children: HashMap<i64, Vec<Value>> = HashMap::new();
    for process in &processes {
        children
            .entry(
                process
                    .get("parentPid")
                    .and_then(Value::as_i64)
                    .unwrap_or_default(),
            )
            .or_default()
            .push(process.clone());
    }
    for list in children.values_mut() {
        list.sort_by(|a, b| {
            process_cpu(latest, previous, b)
                .unwrap_or(-1.0)
                .total_cmp(&process_cpu(latest, previous, a).unwrap_or(-1.0))
        });
    }
    let root = processes.iter().find(|p| process_identity(p).0 == root_pid);
    let mut body = div()
        .id("metrics-process-list")
        .w_full()
        .min_w_0()
        .flex()
        .flex_col()
        .gap_0()
        .rounded_md()
        .border_1()
        .border_color(rgb(palette.line));
    body = body.child(
        div().w_full().min_w_0().p_2()
            .bg(crate::design::surface_color(palette.soft))
            .text_size(px(9.)).text_color(rgb(palette.muted))
            .child("PROCESSES · CPU / RAM / RECENT CPU"),
    );
    let mut visited = std::collections::HashSet::new();
    if let Some(root) = root {
        visited.insert(process_identity(root).0);
        let root_key = format!("root:{root_pid}");
        let root_open = expanded.contains(&root_key);
        let root_row = process_metric_row(
            "Backend server",
            root,
            latest,
            previous,
            samples,
            palette,
            true,
            root_open,
            children.get(&root_pid).map_or(false, |v| !v.is_empty()),
        );
        body = body.child(metric_group_toggle(
            root_row, &root_key, root_open, toggle.clone(),
        ));
        let mut agent_roots = if root_open {
            children
                .get(&root_pid)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter(|process| harness_label(process).is_some())
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        agent_roots.sort_by(|a, b| {
            process_cpu(latest, previous, b)
                .unwrap_or(-1.0)
                .total_cmp(&process_cpu(latest, previous, a).unwrap_or(-1.0))
        });
        for agent in agent_roots {
            let label = agent
                .get("chatTitle")
                .and_then(Value::as_str)
                .filter(|s| !s.trim().is_empty())
                .map(str::to_owned)
                .or_else(|| harness_label(&agent))
                .unwrap_or_else(|| "Harness".into());
            let pid = process_identity(&agent).0;
            let group_key = format!("harness:{pid}:{}", process_identity(&agent).1);
            let group_open = expanded.contains(&group_key);
            visited.insert(pid);
            let row = process_metric_row(
                &label,
                &agent,
                latest,
                previous,
                samples,
                palette,
                true,
                group_open,
                children.get(&pid).map_or(false, |v| !v.is_empty()),
            );
            body = body.child(metric_group_toggle(
                row, &group_key, group_open, toggle.clone(),
            ));
            let mut descendants = Vec::new();
            let mut stack = children.get(&pid).cloned().unwrap_or_default();
            let mut depth = 1usize;
            while !stack.is_empty() && depth <= 64 {
                let batch = std::mem::take(&mut stack);
                for process in batch {
                    let child_pid = process_identity(&process).0;
                    if !visited.insert(child_pid) {
                        continue;
                    }
                    descendants.push(process.clone());
                    stack.extend(children.get(&child_pid).cloned().unwrap_or_default());
                }
                depth += 1;
            }
            if group_open {
                for process in descendants {
                    body = body.child(process_metric_row(
                        &metric_text(&process, "name", "Process"),
                        &process,
                        latest,
                        previous,
                        samples,
                        palette,
                        false,
                        false,
                        false,
                    ));
                }
            }
        }
    } else {
        body = body.child(div().p_6().text_color(rgb(palette.muted)).child(
            "Per-process details will appear after the native app is rebuilt and relaunched.",
        ));
    }
    for gui in processes.iter().filter(|p| metric_text(p, "role", "") == "gui") {
        visited.insert(process_identity(gui).0);
        body = body.child(process_metric_row(
            "GUI", gui, latest, previous, samples, palette, false, false, false,
        ));
    }
    let root_open = expanded.contains(&format!("root:{root_pid}"));
    let mut others = if root_open {
        processes
            .into_iter()
            .filter(|p| !visited.contains(&process_identity(p).0))
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    others.sort_by(|a, b| process_identity(a).0.cmp(&process_identity(b).0));
    for process in others {
        body = body.child(process_metric_row(
            &metric_text(&process, "name", "Process"),
            &process,
            latest,
            previous,
            samples,
            palette,
            false,
            false,
            false,
        ));
    }
    body.into_any_element()
}

fn metric_group_toggle(
    row: AnyElement, key: &str, is_open: bool, toggle: ResourceGroupToggle,
) -> AnyElement {
    let key = key.to_owned();
    let label = if is_open { format!("Collapse process group {key}") } else { format!("Expand process group {key}") };
    Button::new(SharedString::from(format!("metrics-toggle-{key}")))
        .text().compact().w_full().h_auto().min_w_0().p_0().justify_start()
        .accessibility_label(label).child(row)
        .on_click(move |_, window, cx| toggle(key.clone(), window, cx))
        .into_any_element()
}

// The backend group owns every backend sample row, including live children
// whose parent disappeared during collection. GUI is a separate scope.
fn process_metric_totals(
    latest: &Value,
    previous: Option<&Value>,
    process: &Value,
    group: bool,
) -> (Option<f64>, f64) {
    let pid = process_identity(process).0;
    let root_pid = latest.get("rootPid").and_then(Value::as_i64).unwrap_or_default();
    let all = latest.get("processes").and_then(Value::as_array);
    if group && pid == root_pid {
        let gui = all.into_iter().flatten().filter(|p| metric_text(p, "role", "") == "gui");
        let gui_memory = gui.clone().map(|p| metric_number(p, "residentMemoryBytes")).sum::<f64>();
        let gui_cpu = gui.filter_map(|p| process_cpu(latest, previous, p)).sum::<f64>();
        return (
            previous.and_then(|before| metric_cpu_delta(latest, before)).map(|cpu| (cpu - gui_cpu).max(0.0)),
            (metric_number(latest, "residentMemoryBytes") - gui_memory).max(0.0),
        );
    }
    let mut cpu = process_cpu(latest, previous, process);
    let mut memory = metric_number(process, "residentMemoryBytes");
    if group {
        let mut pending = vec![pid];
        let mut seen = std::collections::HashSet::from([pid]);
        while let Some(parent) = pending.pop() {
            for child in all.into_iter().flatten().filter(|p| p.get("parentPid").and_then(Value::as_i64) == Some(parent)) {
                let child_pid = process_identity(child).0;
                if !seen.insert(child_pid) { continue; }
                pending.push(child_pid);
                memory += metric_number(child, "residentMemoryBytes");
                if let Some(child_cpu) = process_cpu(latest, previous, child) {
                    cpu = Some(cpu.unwrap_or_default() + child_cpu);
                }
            }
        }
    }
    (cpu, memory)
}

fn process_metric_row(
    label: &str,
    process: &Value,
    latest: &Value,
    previous: Option<&Value>,
    samples: &[Value],
    palette: crate::design::Palette,
    group: bool,
    expanded: bool,
    expandable: bool,
) -> AnyElement {
    let pid = process_identity(process).0;
    let (cpu_total, memory) = process_metric_totals(latest, previous, process, group);
    let cpu = cpu_total
        .map(|n| format!("{n:.1}%"))
        .unwrap_or_else(|| "···".into());
    let command = metric_text(
        process,
        "commandLine",
        &metric_text(process, "name", "Process"),
    );
    let history = samples
        .windows(2)
        .filter_map(|pair| {
            let current = pair[1].get("processes")?.as_array()?.iter()
                .find(|p| process_identity(p) == process_identity(process))?;
            process_metric_totals(&pair[1], Some(&pair[0]), current, group).0
        })
        .collect::<Vec<_>>();
    let graph = metric_chart(
        &history,
        history.iter().copied().fold(100.0, f64::max) * 1.08,
        palette.accent,
        92,
        24,
    );
    let mut row = div()
        .w_full()
        .min_w_0()
        .flex()
        .flex_wrap()
        .items_center()
        .gap_2()
        .p_2()
        .border_b_1()
        .border_color(rgb(palette.line));
    let icon = process_row_icon(
        process,
        process_identity(process).0
            == latest
                .get("rootPid")
                .and_then(Value::as_i64)
                .unwrap_or_default(),
    );
    let group_mark = if group {
        if expanded {
            "⌄"
        } else if expandable {
            "›"
        } else {
            ""
        }
    } else {
        ""
    };
    row = row.child(
        div()
            .flex_1()
            .flex_basis(px(180.))
            .min_w_0()
            .max_w_full()
            .flex()
            .items_center()
            .gap_2()
            .child(
                div()
                    .w(px(18.))
                    .text_color(rgb(palette.muted))
                    .child(group_mark),
            )
            .child(icon)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap_0()
                    .child(div().min_w_0().whitespace_normal().child(if group {
                        format!("{label} + children  ·  PID {pid}")
                    } else {
                        format!("   {label}  ·  PID {pid}")
                    }))
                    .child(
                        div()
                            .truncate()
                            .text_size(px(9.))
                            .text_color(rgb(palette.muted))
                            .child(command),
                    ),
            ),
    );
    row = row.child(
        div().flex_1().flex_basis(px(240.)).min_w_0().max_w_full().flex().flex_wrap().items_center().justify_end().gap_2()
            .child(div().min_w(px(52.)).text_right()
                .font_family(crate::design::FONT_MONO).text_size(px(10.))
                .child(format!("CPU {cpu}")))
            .child(div().min_w(px(68.)).text_right()
                .text_color(rgb(metrics_memory_color(memory, palette)))
                .font_family(crate::design::FONT_MONO).text_size(px(10.))
                .child(format!("RAM {}", format_memory(memory / 1_048_576.0))))
            .child(div().w(px(92.)).max_w_full().min_w_0().child(graph)),
    );
    row.into_any_element()
}

fn process_row_icon(process: &Value, is_root: bool) -> AnyElement {
    let name = metric_text(process, "name", "Process").to_lowercase();
    let asset = if is_root {
        icons_assets::Monitor.1
    } else if [
        "codex", "claude", "opencode", "hermes", "acp", "agent", "minimax", "mmx",
    ]
    .iter()
    .any(|part| name.contains(part))
    {
        icons_assets::Bot.1
    } else if [
        "node", "npm", "npx", "bun", "deno", "python", "ruby", "perl", "java",
    ]
    .iter()
    .any(|part| name.contains(part))
    {
        icons_assets::Code.1
    } else if ["ssh", "tunnel", "relay", "websocket"]
        .iter()
        .any(|part| name.contains(part))
    {
        icons_assets::Network.1
    } else if ["zsh", "bash", "fish", "shell", "terminal"]
        .iter()
        .any(|part| name.contains(part))
    {
        icons_assets::Terminal.1
    } else {
        icons_assets::Wrench.1
    };
    Icon::default()
        .data(asset)
        .with_size(px(14.))
        .into_any_element()
}

/// A retained header view shares the sidebar's quota cache and click handlers.
/// Observing the source keeps account switches and background refreshes visible.
pub struct WorkspaceUsageRings {
    source: Entity<SidebarUsageWidget>,
    _subscription: Subscription,
}
impl WorkspaceUsageRings {
    pub fn new(source: Entity<SidebarUsageWidget>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.observe(&source, |_, _, cx| cx.notify());
        Self { source, _subscription: subscription }
    }
}
impl Render for WorkspaceUsageRings {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.source.update(cx, |source, cx| source.build_workspace_rings(window, cx))
    }
}

impl Render for SidebarUsageWidget {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Prefer the parent-composed tokens (custom accent / contrast / tint /
        // stored preset); the fallback keeps the retained widget usable alone.
        self.build_widget(self.composed_palette, window, cx)
    }
}

// ---------------------------------------------------------------------------
// Constants for the metrics row token. Lives in `design` in the production
// modules; locally aliased so this module stays self-contained.
// ---------------------------------------------------------------------------

const DEFAULT_MUTED_TOKEN: u32 = 0xa4a4a4;

// ---------------------------------------------------------------------------
// Tests — pure mapping functions over real backend payload shapes.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[::core::prelude::v1::test]
    fn backend_group_and_gui_rows_reconcile_with_total_even_when_parent_is_missing() {
        let before = json!({"rootPid":2,"sampledAt":1000,"cpuTimeMs":100,"residentMemoryBytes":600,"processes":[
            {"pid":2,"startedAt":10,"parentPid":1,"cpuTimeMs":20,"residentMemoryBytes":100},
            {"pid":3,"startedAt":10,"parentPid":999,"cpuTimeMs":50,"residentMemoryBytes":200},
            {"pid":7,"startedAt":10,"parentPid":1,"role":"gui","cpuTimeMs":30,"residentMemoryBytes":300}]});
        let mut now = before.clone();
        now["sampledAt"] = json!(2000);
        now["cpuTimeMs"] = json!(160);
        now["processes"][0]["cpuTimeMs"] = json!(30);
        now["processes"][1]["cpuTimeMs"] = json!(70);
        now["processes"][2]["cpuTimeMs"] = json!(60);
        let backend = process_metric_totals(&now, Some(&before), &now["processes"][0], true);
        let gui = process_metric_totals(&now, Some(&before), &now["processes"][2], false);
        assert_eq!(backend, (Some(3.0), 300.0));
        assert_eq!(gui, (Some(3.0), 300.0));
        assert_eq!(backend.0.unwrap() + gui.0.unwrap(), metric_cpu_delta(&now, &before).unwrap());
        assert_eq!(backend.1 + gui.1, metric_number(&now, "residentMemoryBytes"));
    }

    #[::core::prelude::v1::test]
    fn ring_row_stays_single_line_at_narrow_and_standard_sidebar_widths() {
        for (width, account_arrows) in [
            (100.0, true),
            (120.0, true),
            (252.0, false),
            (252.0, true),
        ] {
            let layout = provider_ring_layout(width, account_arrows);
            let occupied = layout.diameter * PROVIDER_RING_COUNT as f32
                + layout.gap * (PROVIDER_RING_COUNT - 1) as f32
                + layout.edge_padding * 2.0
                + if account_arrows { CODEX_ACCOUNT_ARROWS_WIDTH } else { 0.0 };
            assert!(
                layout.diameter >= 1.0 && layout.diameter <= RING_MAX_DIAMETER
            );
            assert_eq!(layout.gap, crate::design::space::TIGHT);
            assert!(
                occupied <= width + 0.01,
                "{occupied} exceeds {width}"
            );
        }
    }

    #[::core::prelude::v1::test]
    fn capped_rings_spread_extra_width_between_slots_and_keep_clock_increment() {
        let standard = provider_ring_layout(400.0, false);
        assert_eq!(standard.diameter, 64.0);
        assert!((standard.gap - 17.5).abs() < 0.01);
        assert_eq!(standard.diameter + 4.0, 68.0);

        let with_codex_arrows = provider_ring_layout(400.0, true);
        assert_eq!(with_codex_arrows.diameter, 64.0);
        assert!((with_codex_arrows.gap - 6.5).abs() < 0.01);
        assert_eq!(with_codex_arrows.edge_padding, crate::design::space::TIGHT);
    }

    fn sample_overview() -> Value {
        let raw = r#"{
            "generatedAt": 1700000000000,
            "capturedSince": 1699999000000,
            "subscriptions": [
                {
                    "provider": "codex", "hostId": "host-a", "source": "codex-cli",
                    "accountLabel": "Alex", "codexHome": "/Users/alex/.codex",
                    "state": "available", "planType": "Plus", "fetchedAt": 1699999900000,
                    "staleAfter": 1700000500000,
                    "windows": [
                        { "key": "five_hour", "label": "5h", "usedPercent": 32.0, "resetsAt": 1700000400000 },
                        { "key": "weekly", "label": "Week", "usedPercent": 78.0, "resetsAt": 1700500000000 }
                    ],
                    "balances": [],
                    "error": null
                },
                {
                    "provider": "claude", "hostId": "host-a", "source": "anthropic-cli",
                    "state": "unsupported", "planType": null, "fetchedAt": 1699999900000,
                    "staleAfter": null,
                    "windows": [], "balances": [], "error": null
                },
                {
                    "provider": "minimax", "hostId": "host-a", "source": "mmx-cli",
                    "state": "available", "planType": null,
                    "fetchedAt": 1699999900000, "staleAfter": 1700000500000,
                    "windows": [
                        { "key": "current_interval", "label": "5h", "usedPercent": 12.0, "resetsAt": 1700000700000 }
                    ],
                    "balances": [], "error": null
                },
                {
                    "provider": "gemini", "hostId": "host-a", "source": "gemini-cli",
                    "state": "error", "planType": null, "fetchedAt": 1699999900000,
                    "staleAfter": null,
                    "windows": [], "balances": [], "error": "OAuth refresh failed"
                },
                {
                    "provider": "opencode-go", "hostId": "host-a", "source": "opencode-cli",
                    "state": "available", "planType": null, "fetchedAt": 1699999900000,
                    "staleAfter": 1700000500000,
                    "windows": [
                        { "key": "primary", "label": "Primary", "usedPercent": 65.0 }
                    ],
                    "balances": [], "error": null
                }
            ],
            "providerTotals": [], "recentRuns": []
        }"#;
        serde_json::from_str(raw).expect("sample overview should parse")
    }

    #[::core::prelude::v1::test]
    fn projection_keeps_every_provider_with_no_fabrication() {
        let overview = sample_overview();
        let map = project_sidebar_usage_map(Some(&overview), false, "", 1_700_000_000_000);
        for provider in SidebarUsageProvider::ALL {
            assert!(map.contains_key(&provider), "missing provider {provider:?}");
        }
        // Claude unsupported → status == unavailable + structured message
        let claude = map.get(&SidebarUsageProvider::Claude).unwrap();
        assert_eq!(claude.status, SidebarUsageStatus::Unavailable);
        assert_eq!(
            claude.message.as_deref(),
            Some("Claude does not expose an on-demand quota read.")
        );
        // Codex shows numeric value because used_percent is reported
        let codex = map.get(&SidebarUsageProvider::Codex).unwrap();
        assert_eq!(codex.status, SidebarUsageStatus::Ready);
        assert!(codex.active.is_some());
        assert!(codex.shows_value());
        // Gemini error → status = Error + message carried through
        let gemini = map.get(&SidebarUsageProvider::Gemini).unwrap();
        assert_eq!(gemini.status, SidebarUsageStatus::Error);
        assert_eq!(gemini.message.as_deref(), Some("OAuth refresh failed"));
        // MiniMax primary window
        let minimax = map.get(&SidebarUsageProvider::MiniMax).unwrap();
        assert_eq!(minimax.status, SidebarUsageStatus::Ready);
        assert!(minimax.active.is_some());
        // Opencode-go primary
        let opencode = map.get(&SidebarUsageProvider::OpencodeGo).unwrap();
        assert!(opencode.shows_value());
    }

    #[::core::prelude::v1::test]
    fn stale_status_transitions_when_stale_after_is_in_the_past() {
        let mut overview = sample_overview();
        overview["subscriptions"][0]["staleAfter"] = json!(1_699_999_000_000_i64);
        let map = project_sidebar_usage_map(Some(&overview), false, "", 1_700_000_000_000);
        let codex = map.get(&SidebarUsageProvider::Codex).unwrap();
        assert_eq!(codex.status, SidebarUsageStatus::Stale);
    }

    #[::core::prelude::v1::test]
    fn empty_overview_projects_explicit_unavailable_per_provider() {
        let map = project_sidebar_usage_map(None, false, "", 1_700_000_000_000);
        for provider in SidebarUsageProvider::ALL {
            let ring = map.get(&provider).unwrap();
            assert_eq!(ring.status, SidebarUsageStatus::Unavailable);
            assert!(ring.message.is_none());
            assert!(ring.active.is_none());
            assert!(ring.weekly.is_none());
        }
    }

    #[::core::prelude::v1::test]
    fn loading_state_overrides_unavailable_only() {
        let map = project_sidebar_usage_map(None, true, "", 1_700_000_000_000);
        for provider in SidebarUsageProvider::ALL {
            assert_eq!(
                map.get(&provider).unwrap().status,
                SidebarUsageStatus::Loading
            );
        }
    }

    #[::core::prelude::v1::test]
    fn request_error_without_loading_projects_error_state() {
        let map = project_sidebar_usage_map(None, false, "boom", 1_700_000_000_000);
        for provider in SidebarUsageProvider::ALL {
            assert_eq!(
                map.get(&provider).unwrap().status,
                SidebarUsageStatus::Error
            );
        }
    }

    #[::core::prelude::v1::test]
    fn shows_value_is_false_when_percent_missing() {
        let mut overview = sample_overview();
        overview["subscriptions"][0]["windows"] = json!([]);
        let map = project_sidebar_usage_map(Some(&overview), false, "", 1_700_000_000_000);
        let codex = map.get(&SidebarUsageProvider::Codex).unwrap();
        assert!(!codex.shows_value());
    }

    #[::core::prelude::v1::test]
    fn unlimited_window_shows_value_with_no_percent() {
        let raw = r#"{
            "generatedAt": 1700000000000,
            "capturedSince": 1699999000000,
            "subscriptions": [
                {
                    "provider": "codex", "hostId": "host-a", "source": "codex-cli",
                    "state": "available", "planType": null, "fetchedAt": 1699999900000,
                    "staleAfter": 1700000500000,
                    "windows": [
                        { "key": "five_hour", "label": "5h (Unlimited)", "usedPercent": null }
                    ],
                    "balances": [], "error": null
                }
            ],
            "providerTotals": [], "recentRuns": []
        }"#;
        let overview: Value = serde_json::from_str(raw).unwrap();
        let map = project_sidebar_usage_map(Some(&overview), false, "", 1_700_000_000_000);
        let codex = map.get(&SidebarUsageProvider::Codex).unwrap();
        assert!(codex.shows_value());
        assert_eq!(
            percent_text(codex.active.as_ref().and_then(|w| w.used_percent), true),
            "∞"
        );
    }

    #[::core::prelude::v1::test]
    fn multi_account_codex_emits_account_keys() {
        let raw = r#"{
            "generatedAt": 1700000000000,
            "capturedSince": 1699999000000,
            "subscriptions": [
                {
                    "provider": "codex", "hostId": "host-a", "source": "codex-cli",
                    "codexHome": "/Users/alex/.codex", "accountLabel": "Alex",
                    "state": "available", "planType": null, "fetchedAt": 1699999900000,
                    "staleAfter": 1700000500000,
                    "windows": [{ "key": "five_hour", "label": "5h", "usedPercent": 25.0 }],
                    "balances": [], "error": null
                },
                {
                    "provider": "codex", "hostId": "host-a", "source": "codex-cli",
                    "codexHome": "/Users/work/.codex", "accountLabel": "Work",
                    "state": "available", "planType": null, "fetchedAt": 1699999900000,
                    "staleAfter": 1700000500000,
                    "windows": [{ "key": "five_hour", "label": "5h", "usedPercent": 55.0 }],
                    "balances": [], "error": null
                }
            ],
            "providerTotals": [], "recentRuns": []
        }"#;
        let overview: Value = serde_json::from_str(raw).unwrap();
        let map = project_sidebar_usage_map(Some(&overview), false, "", 1_700_000_000_000);
        let codex = map.get(&SidebarUsageProvider::Codex).unwrap();
        assert_eq!(codex.accounts.len(), 2);
        assert_eq!(codex.accounts[0].label, "Alex");
        assert_eq!(codex.accounts[1].label, "Work");
        // Primary entry is the first source.
        assert_eq!(codex.account_label.as_deref(), Some("Alex"));
    }

    #[::core::prelude::v1::test]
    fn minimax_source_matches_by_name_when_provider_id_is_mminsimax() {
        // Source-of-truth mismatch where the backend writes the source
        // name with the mmx prefix but the provider id is `mmx`. Mirror
        // the Svelte provider filter fallback.
        let raw = r#"{
            "generatedAt": 1700000000000,
            "capturedSince": 1699999000000,
            "subscriptions": [
                {
                    "provider": "mmx", "hostId": "host-a", "source": "mmx-cli",
                    "state": "available", "planType": null, "fetchedAt": 1699999900000,
                    "staleAfter": 1700000500000,
                    "windows": [{ "key": "primary", "label": "Primary", "usedPercent": 18.0 }],
                    "balances": [], "error": null
                }
            ],
            "providerTotals": [], "recentRuns": []
        }"#;
        let overview: Value = serde_json::from_str(raw).unwrap();
        let map = project_sidebar_usage_map(Some(&overview), false, "", 1_700_000_000_000);
        let minimax = map.get(&SidebarUsageProvider::MiniMax).unwrap();
        assert_eq!(minimax.status, SidebarUsageStatus::Ready);
        assert!(minimax.active.is_some());
    }

    #[::core::prelude::v1::test]
    fn weekly_window_is_only_picked_when_active_present() {
        let raw = r#"{
            "generatedAt": 1700000000000,
            "capturedSince": 1699999000000,
            "subscriptions": [
                {
                    "provider": "codex", "hostId": "host-a", "source": "codex-cli",
                    "state": "available", "planType": null, "fetchedAt": 1699999900000,
                    "staleAfter": 1700000500000,
                    "windows": [{ "key": "weekly", "label": "Week", "usedPercent": 88.0 }],
                    "balances": [], "error": null
                }
            ],
            "providerTotals": [], "recentRuns": []
        }"#;
        let overview: Value = serde_json::from_str(raw).unwrap();
        let map = project_sidebar_usage_map(Some(&overview), false, "", 1_700_000_000_000);
        let codex = map.get(&SidebarUsageProvider::Codex).unwrap();
        // Plans that expose only a weekly window still get a usable ring.
        assert!(codex.active.is_some());
        assert!(codex.weekly.is_none());
        assert!(codex.shows_value());
    }

    #[::core::prelude::v1::test]
    fn merge_rings_preserves_cached_active_window() {
        let cached = SidebarUsageRing {
            provider: SidebarUsageProvider::Codex,
            status: SidebarUsageStatus::Ready,
            active: Some(SidebarUsageWindow {
                label: "5h".into(),
                used_percent: Some(42.0),
                unlimited: false,
                resets_at: Some(1_700_000_500_000),
                reset_label: None,
            }),
            weekly: None,
            message: None,
            updated_at: Some(1_699_999_900_000),
            account_label: Some("Alex".into()),
            accounts: Vec::new(),
        };
        // Render-side projection before the worker has caught up.
        let next = SidebarUsageRing {
            provider: SidebarUsageProvider::Codex,
            status: SidebarUsageStatus::Loading,
            active: None,
            weekly: None,
            message: None,
            updated_at: None,
            account_label: None,
            accounts: Vec::new(),
        };
        let merged = merge_rings(cached.clone(), next);
        assert_eq!(merged.status, SidebarUsageStatus::Ready);
        assert!(merged.active.is_some());
        assert_eq!(merged.account_label.as_deref(), Some("Alex"));
    }

    #[::core::prelude::v1::test]
    fn percent_text_renders_unknown_as_em_dash() {
        assert_eq!(percent_text(None, false), "—");
        assert_eq!(percent_text(Some(f64::NAN), false), "—");
        assert_eq!(percent_text(Some(42.4), false), "42%");
        assert_eq!(percent_text(Some(42.4), true), "∞");
    }

    #[::core::prelude::v1::test]
    fn compact_duration_formats_days_hours_minutes() {
        assert_eq!(compact_duration(0), "");
        assert_eq!(compact_duration(60), "1h");
        assert_eq!(compact_duration(1), "1m");
        assert_eq!(compact_duration(1_440 + 2 * 60 + 3), "1d 2h 3m");
    }

    #[::core::prelude::v1::test]
    fn reset_summary_emits_relative_text() {
        let window = SidebarUsageWindow {
            label: "5h".into(),
            used_percent: Some(20.0),
            unlimited: false,
            resets_at: Some(1_700_000_300_000),
            reset_label: None,
        };
        let now = 1_700_000_000_000;
        let summary = reset_summary(Some(&window), now);
        assert!(
            summary.contains("resets in"),
            "unexpected summary: {summary}"
        );
    }

    #[::core::prelude::v1::test]
    fn reset_summary_handles_overdue_and_unavailable() {
        let window = SidebarUsageWindow {
            label: "5h".into(),
            used_percent: Some(20.0),
            unlimited: false,
            resets_at: Some(1_699_999_000_000),
            reset_label: None,
        };
        let now = 1_700_000_000_000;
        let summary = reset_summary(Some(&window), now);
        assert!(summary.contains("overdue"));

        assert_eq!(reset_summary(None, now), "reset unavailable");
    }

    #[::core::prelude::v1::test]
    fn usage_color_returns_none_for_unlimited_or_absent_window() {
        let unlimited = SidebarUsageWindow {
            label: "5h (Unlimited)".into(),
            used_percent: None,
            unlimited: true,
            resets_at: None,
            reset_label: None,
        };
        assert!(usage_color(Some(&unlimited)).is_none());
        assert!(usage_color(None).is_none());
    }

    #[::core::prelude::v1::test]
    fn usage_color_classifies_remaining_buckets() {
        let high = SidebarUsageWindow {
            label: "5h".into(),
            used_percent: Some(20.0),
            unlimited: false,
            resets_at: None,
            reset_label: None,
        };
        let medium = SidebarUsageWindow {
            label: "5h".into(),
            used_percent: Some(70.0),
            unlimited: false,
            resets_at: None,
            reset_label: None,
        };
        let critical = SidebarUsageWindow {
            label: "5h".into(),
            used_percent: Some(95.0),
            unlimited: false,
            resets_at: None,
            reset_label: None,
        };
        let c_high = usage_color(Some(&high)).unwrap();
        let c_medium = usage_color(Some(&medium)).unwrap();
        let c_critical = usage_color(Some(&critical)).unwrap();
        // Remaining bucket: green > 40%, blended into green from 10-40%,
        // blended into red below 10%.
        assert_eq!(c_high, RGB_GREEN);
        assert_ne!(c_medium, RGB_GREEN);
        assert!(c_critical.0 >= c_medium.0 || c_critical.0 < RGB_RED.0 + 5);
    }

    #[::core::prelude::v1::test]
    fn state_label_distinguishes_ready_without_data() {
        let empty = SidebarUsageRing {
            provider: SidebarUsageProvider::Codex,
            status: SidebarUsageStatus::Ready,
            ..Default::default()
        };
        assert_eq!(state_label(&empty), "No allowance data");
        let loaded = SidebarUsageRing {
            provider: SidebarUsageProvider::Codex,
            status: SidebarUsageStatus::Ready,
            active: Some(SidebarUsageWindow {
                label: "5h".into(),
                used_percent: Some(20.0),
                unlimited: false,
                resets_at: None,
                reset_label: None,
            }),
            ..Default::default()
        };
        assert_eq!(state_label(&loaded), "Available");
    }

    #[::core::prelude::v1::test]
    fn from_id_round_trip_for_every_provider() {
        for provider in SidebarUsageProvider::ALL {
            assert_eq!(SidebarUsageProvider::from_id(provider.id()), Some(provider));
        }
    }

    #[::core::prelude::v1::test]
    fn every_sidebar_ring_uses_a_bundled_provider_icon() {
        for provider in SidebarUsageProvider::ALL {
            assert!(
                crate::branding::provider_for_ink(provider.id(), "", 0xffffff).is_some(),
                "missing bundled icon for {}",
                provider.id()
            );
        }
    }
}
