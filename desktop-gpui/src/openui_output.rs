//! Bounded, presentation-only OpenUI message blocks. Model text is data for the
//! fixed renderer; it is never evaluated as JavaScript or a backend command.
use crate::{chat::ChatPane, design, openui_state};
use futures::channel::oneshot;
use gpui_kit::{
    component::{button::Button, text::TextView},
    prelude::*,
    *,
};
use nsh_platform_gpui::{
    App as NshApp, AppOptions, AppView, Bundle, ComponentContext, NativeComponent,
};
use serde_json::{Map, Value, json};
use std::{
    cell::RefCell,
    collections::{BTreeMap, HashMap, HashSet},
    path::{Path, PathBuf},
    rc::{Rc, Weak},
    sync::Arc,
    time::{Duration, Instant},
};

const LIBRARY: &str = "monitter-openui-v1/monitter-native-v1/lang-core-1.0.1";
const MAX_MESSAGE: usize = 128 * 1024;
const MAX_SOURCE: usize = 64 * 1024;
const MAX_BLOCKS: usize = 4;
const MAX_ENGINES: usize = 8;
const MAX_STATE: usize = 16 * 1024;
const MAX_DRAFT: usize = 8 * 1024;
const BRIDGE: &str = "monitter.openui.bridge";
const RENDERER: &str = include_str!(concat!(env!("OUT_DIR"), "/openui-renderer.js"));
const MAX_MEDIA_FILES: usize = 8;
const MAX_MEDIA_BYTES: u64 = 2 * 1024 * 1024;

fn media_directory() -> Option<PathBuf> {
    let home = PathBuf::from(std::env::var_os("HOME")?);
    #[cfg(target_os = "macos")]
    return Some(home.join("Library/Application Support/Monitter/openui-public-media"));
    #[cfg(not(target_os = "macos"))]
    return Some(
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".local/share"))
            .join("monitter/openui-public-media"),
    );
}

fn media_name(candidate: &str) -> Option<&str> {
    let (hash, extension) = candidate.split_once('.')?;
    if hash.len() != 64
        || !hash
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return None;
    }
    if !matches!(extension, "jpg" | "png" | "webp") {
        return None;
    }
    Some(candidate)
}

/// Parse immutable media identities without touching the filesystem. Text and
/// palette changes that retain these identities reuse the resolved media map.
fn media_names(source: &str) -> Vec<String> {
    let mut names = Vec::new();
    for (start, _) in source.match_indices("media:") {
        let rest = &source[start + "media:".len()..];
        let end = rest
            .find(|character: char| {
                !character.is_ascii_lowercase() && !character.is_ascii_digit() && character != '.'
            })
            .unwrap_or(rest.len());
        let name = &rest[..end];
        if media_name(name).is_some() && !names.iter().any(|known| known == name) {
            names.push(name.to_owned());
        }
    }
    names
}

#[derive(Clone)]
struct MediaDimensions {
    length: u64,
    modified: Option<std::time::SystemTime>,
    created: Option<std::time::SystemTime>,
    dimensions: (u32, u32),
}
/// Called only on a worker. A bounded header cache also avoids decoding the
/// same approved immutable preview independently for every mounted block.
fn media_dimensions(path: &Path, metadata: &std::fs::Metadata) -> Option<(u32, u32)> {
    static CACHE: std::sync::OnceLock<std::sync::Mutex<HashMap<PathBuf, MediaDimensions>>> =
        std::sync::OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    let modified = metadata.modified().ok();
    let created = metadata.created().ok();
    if let Some(entry) = cache.lock().ok()?.get(path).cloned() {
        if entry.length == metadata.len() && entry.modified == modified && entry.created == created
        {
            return Some(entry.dimensions);
        }
    }
    let dimensions = image::image_dimensions(path).ok()?;
    if let Ok(mut cache) = cache.lock() {
        if cache.len() >= 256 {
            cache.clear();
        }
        cache.insert(
            path.to_owned(),
            MediaDimensions {
                length: metadata.len(),
                modified,
                created,
                dimensions,
            },
        );
    }
    Some(dimensions)
}

// Only host-validated, public preview files are exposed to the NSH Image
// primitive. Model text never supplies a filesystem path or network URL.
fn approved_media(source: &str, directory: &Path) -> Map<String, Value> {
    let mut approved = Map::new();
    let Ok(root) = directory.canonicalize() else {
        return approved;
    };
    let mut seen = HashSet::new();
    for (start, _) in source.match_indices("media:") {
        let rest = &source[start + "media:".len()..];
        let end = rest
            .find(|character: char| {
                !character.is_ascii_lowercase() && !character.is_ascii_digit() && character != '.'
            })
            .unwrap_or(rest.len());
        let name = &rest[..end];
        if media_name(name).is_none() || !seen.insert(name) {
            continue;
        }
        if approved.len() >= MAX_MEDIA_FILES {
            break;
        }
        let path = root.join(name);
        let Ok(metadata) = path.symlink_metadata() else {
            continue;
        };
        if !metadata.is_file()
            || metadata.file_type().is_symlink()
            || metadata.len() > MAX_MEDIA_BYTES
        {
            continue;
        }
        let Ok(canonical) = path.canonicalize() else {
            continue;
        };
        if canonical.parent() != Some(root.as_path()) {
            continue;
        }
        let Some((width, height)) = media_dimensions(&canonical, &metadata) else {
            continue;
        };
        if width == 0
            || height == 0
            || width > 4096
            || height > 4096
            || u64::from(width) * u64::from(height) > 8_000_000
        {
            continue;
        }
        approved.insert(
            format!("media:{name}"),
            Value::String(canonical.to_string_lossy().into_owned()),
        );
    }
    approved
}

/// NSH's storage API is not controlled by the empty capability grant. Its
/// default file backend must therefore be replaced explicitly. Each mounted
/// view gets a fresh backend; only sanitized form values use the host store.
struct EphemeralStorage;
impl nsh_engine::StorageBackend for EphemeralStorage {
    fn load(&self) -> Result<BTreeMap<String, String>, String> {
        Ok(BTreeMap::new())
    }
    fn write(&self, _: nsh_engine::StorageChange) {}
}

#[derive(Clone, Debug, PartialEq)]
enum Part {
    Markdown(String),
    Block {
        ordinal: usize,
        source: String,
        original: String,
        closed: bool,
    },
}

/// Respect ordinary fenced code blocks: an `openui` example inside a different
/// fence is still code. Only an exact, unindented/three-space info string opts in.
fn parts(text: &str) -> Result<Vec<Part>, &'static str> {
    if text.len() > MAX_MESSAGE {
        return Err("OpenUI message exceeds the supported size.");
    }
    let mut result = Vec::new();
    let mut markdown = String::new();
    let mut fence: Option<(char, usize)> = None;
    let lines: Vec<_> = text.split_inclusive('\n').collect();
    let mut index = 0;
    let mut ordinal = 0;
    while index < lines.len() {
        let line = lines[index];
        let parsed = fence_line(line);
        if let Some((kind, count)) = fence {
            if parsed
                .is_some_and(|(ch, n, tail)| ch == kind && n >= count && tail.trim().is_empty())
            {
                fence = None;
            }
            markdown.push_str(line);
            index += 1;
            continue;
        }
        let Some((kind, count, info)) = parsed else {
            markdown.push_str(line);
            index += 1;
            continue;
        };
        if info.trim() != "openui" {
            fence = Some((kind, count));
            markdown.push_str(line);
            index += 1;
            continue;
        }
        if ordinal == MAX_BLOCKS {
            return Err("A message supports at most four OpenUI blocks.");
        }
        if !markdown.is_empty() {
            result.push(Part::Markdown(std::mem::take(&mut markdown)));
        }
        let mut source = String::new();
        let mut original = line.to_owned();
        let mut closed = false;
        index += 1;
        while index < lines.len() {
            let line = lines[index];
            original.push_str(line);
            index += 1;
            if fence_line(line)
                .is_some_and(|(ch, n, tail)| ch == kind && n >= count && tail.trim().is_empty())
            {
                closed = true;
                break;
            }
            source.push_str(line);
            if source.len() > MAX_SOURCE {
                return Err("OpenUI block exceeds the supported size.");
            }
        }
        result.push(Part::Block {
            ordinal,
            source,
            original,
            closed,
        });
        ordinal += 1;
    }
    if !markdown.is_empty() {
        result.push(Part::Markdown(markdown));
    }
    Ok(result)
}
fn fence_line(line: &str) -> Option<(char, usize, &str)> {
    let spaces = line.bytes().take_while(|b| *b == b' ').count();
    if spaces > 3 {
        return None;
    }
    let line = &line[spaces..];
    let kind = line.chars().next()?;
    if !matches!(kind, '`' | '~') {
        return None;
    }
    let count = line.chars().take_while(|ch| *ch == kind).count();
    (count >= 3).then(|| (kind, count, &line[count..]))
}
pub(crate) fn has_output(message: &Value) -> bool {
    message["role"] == "assistant"
        && message["text"].as_str().is_some_and(|text| {
            text.contains("openui")
                && parts(text)
                    .is_ok_and(|parts| parts.iter().any(|part| matches!(part, Part::Block { .. })))
        })
}

fn identity(task: &str, message: &str, ordinal: usize) -> Option<String> {
    let valid = |value: &str| {
        !value.is_empty()
            && value.len() <= 128
            && value
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || "-_:".contains(ch))
    };
    (valid(task) && valid(message) && ordinal < MAX_BLOCKS)
        .then(|| json!([LIBRARY, task, message, ordinal]).to_string())
}
fn secret_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    [
        "password",
        "passwd",
        "secret",
        "token",
        "credential",
        "authorization",
        "cookie",
        "apikey",
        "api_key",
        "privatekey",
        "private_key",
    ]
    .iter()
    .any(|part| key.contains(part))
}
/// Only ordinary bounded form values survive engine eviction/restart. Password
/// controls are unsupported by the renderer; suspicious names are excluded here
/// again, including nested and reactive (`$name`) form fields.
pub(crate) fn sanitize_state(value: &Value) -> Result<Value, &'static str> {
    fn walk(value: &Value, depth: usize) -> Result<Value, &'static str> {
        if depth > 6 {
            return Err("Form state is too deeply nested.");
        }
        match value {
            Value::Object(values) if values.len() <= 64 => {
                let mut safe = Map::new();
                for (key, value) in values {
                    if key.len() > 64 || key.chars().any(char::is_control) {
                        return Err("Unsupported form field name.");
                    }
                    if !secret_key(key) {
                        safe.insert(key.clone(), walk(value, depth + 1)?);
                    }
                }
                Ok(Value::Object(safe))
            }
            Value::Array(values) if values.len() <= 32 => values
                .iter()
                .map(|v| walk(v, depth + 1))
                .collect::<Result<Vec<_>, _>>()
                .map(Value::Array),
            Value::String(value) if value.len() <= 2048 && !value.contains('\0') => {
                Ok(Value::String(value.clone()))
            }
            Value::Null | Value::Bool(_) | Value::Number(_) => Ok(value.clone()),
            _ => Err("Unsupported or oversized form state."),
        }
    }
    if !value.is_object() {
        return Err("Form state must be an object.");
    }
    let value = walk(value, 0)?;
    if serde_json::to_vec(&value)
        .map_err(|_| "Invalid form state.")?
        .len()
        > MAX_STATE
    {
        return Err("Form state exceeds the supported size.");
    }
    Ok(value)
}
#[derive(Debug, PartialEq)]
struct DraftAction {
    text: String,
    id: Option<String>,
}
fn draft_action(value: &Value) -> Result<DraftAction, &'static str> {
    let object = value.as_object().ok_or("Unsupported OpenUI action.")?;
    if object
        .keys()
        .any(|key| !matches!(key.as_str(), "type" | "text" | "id"))
        || object.get("type").and_then(Value::as_str) != Some("draft")
    {
        return Err("OpenUI may only prepare a draft for review.");
    }
    let text = value["text"]
        .as_str()
        .filter(|text| {
            !text.trim().is_empty()
                && text.len() <= MAX_DRAFT
                && !text
                    .chars()
                    .any(|ch| ch.is_control() && !matches!(ch, '\n' | '\r' | '\t'))
        })
        .ok_or("Unsupported or oversized OpenUI draft.")?;
    let id = match object.get("id") {
        None => None,
        Some(Value::String(id)) if !id.is_empty() && id.len() <= 64 => Some(id.clone()),
        _ => return Err("Unsupported OpenUI action identity."),
    };
    Ok(DraftAction {
        text: text.to_owned(),
        id,
    })
}

#[derive(Default)]
pub(crate) struct MessageOutput {
    blocks: HashMap<String, Entity<OutputBlock>>,
}
impl MessageOutput {
    pub(crate) fn set_visible(&mut self, visible: bool, cx: &mut App) {
        if !visible {
            for block in self.blocks.values() {
                block.update(cx, |block, cx| block.stop(cx));
            }
        }
    }
    pub(crate) fn render(
        &mut self,
        message: &Value,
        owner: Option<WeakEntity<ChatPane>>,
        palette: Option<design::Palette>,
        list_item: Option<(ListState, usize)>,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<AnyElement> {
        if message["role"] != "assistant"
            || !message["text"]
                .as_str()
                .is_some_and(|text| text.contains("openui"))
        {
            if !self.blocks.is_empty() {
                self.set_visible(false, cx);
                self.blocks.clear();
            }
            return None;
        }
        let parsed = match message["text"].as_str().and_then(|text| parts(text).ok()) {
            Some(parsed) => parsed,
            None => {
                self.set_visible(false, cx);
                self.blocks.clear();
                return None;
            }
        };
        if !parsed.iter().any(|part| matches!(part, Part::Block { .. })) {
            self.set_visible(false, cx);
            self.blocks.clear();
            return None;
        }
        let task = message["taskId"].as_str()?;
        let message_id = message["id"].as_str()?;
        let mut keep = HashSet::new();
        let mut content = div().w_full().min_w_0().flex().flex_col().gap_2();
        for (index, part) in parsed.into_iter().enumerate() {
            match part {
                Part::Markdown(text) => {
                    content = content.child(markdown(
                        format!("openui-text-{message_id}-{index}"),
                        text,
                        owner.clone(),
                    ))
                }
                Part::Block {
                    ordinal,
                    source,
                    original,
                    closed,
                } => {
                    let key = identity(task, message_id, ordinal)?;
                    keep.insert(key.clone());
                    let block = self.blocks.entry(key.clone()).or_insert_with(|| {
                        cx.new(|cx| {
                            OutputBlock::new(
                                key,
                                task.to_owned(),
                                message_id.to_owned(),
                                ordinal,
                                cx,
                            )
                        })
                    });
                    block.update(cx, |block, cx| {
                        block.set_document(
                            source,
                            original,
                            closed && message["streamStatus"] != "streaming",
                            owner.clone(),
                            palette,
                            list_item.clone(),
                            cx,
                        );
                    });
                    content = content.child(block.clone());
                }
            }
        }
        self.blocks.retain(|key, block| {
            if keep.contains(key) {
                true
            } else {
                block.update(cx, |block, cx| block.stop(cx));
                false
            }
        });
        let _ = window;
        Some(content.into_any_element())
    }
}
fn markdown(id: String, text: String, owner: Option<WeakEntity<ChatPane>>) -> AnyElement {
    TextView::markdown(SharedString::from(id), text)
        .on_link_click(move |url, _, _, cx| {
            crate::chat::open_markdown_link(owner.clone(), url.as_str(), cx)
        })
        .selectable(true)
        .scrollable(false)
        .w_full()
        .min_w_0()
        .max_w_full()
        .into_any_element()
}

struct Lease {
    view: Option<Entity<AppView>>,
    last_painted: Instant,
    owner: WeakEntity<OutputBlock>,
}
thread_local! { static ENGINES: RefCell<Vec<Weak<RefCell<Lease>>>> = const { RefCell::new(Vec::new()) }; }
fn reserve(lease: &Rc<RefCell<Lease>>, cx: &mut App) -> bool {
    ENGINES.with(|engines| {
        let mut engines = engines.borrow_mut();
        engines.retain(|entry| entry.strong_count() > 0);
        let active: Vec<_> = engines
            .iter()
            .filter_map(Weak::upgrade)
            .filter(|entry| entry.borrow().view.is_some())
            .collect();
        if active.len() >= MAX_ENGINES {
            let Some(oldest) = active
                .into_iter()
                .filter(|entry| entry.borrow().last_painted.elapsed() > Duration::from_secs(2))
                .min_by_key(|entry| entry.borrow().last_painted)
            else {
                return false;
            };
            let owner = oldest.borrow().owner.clone();
            let _ = owner.update(cx, |block, cx| block.stop(cx));
        }
        if !engines
            .iter()
            .any(|entry| entry.ptr_eq(&Rc::downgrade(lease)))
        {
            engines.push(Rc::downgrade(lease));
        }
        true
    })
}

struct OutputBlock {
    key: String,
    task: String,
    message: String,
    ordinal: usize,
    source: String,
    original: String,
    complete: bool,
    owner: Option<WeakEntity<ChatPane>>,
    palette: Option<design::Palette>,
    list_item: Option<(ListState, usize)>,
    lease: Rc<RefCell<Lease>>,
    bridge: Option<ComponentContext>,
    status: String,
    state: Value,
    hydrated: bool,
    loading: Option<Task<()>>,
    height: f32,
    notice: Option<String>,
    render_notice: Option<String>,
    pending: Option<DraftAction>,
    last_action: Option<String>,
    revision: u64,
    error_task: Option<Task<()>>,
    generation: u64,
    media_names: Vec<String>,
    media: Map<String, Value>,
    media_generation: u64,
    media_task: Option<Task<()>>,
    media_retry_after: Option<Instant>,
}
impl OutputBlock {
    fn new(
        key: String,
        task: String,
        message: String,
        ordinal: usize,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            key,
            task,
            message,
            ordinal,
            source: String::new(),
            original: String::new(),
            complete: false,
            owner: None,
            palette: None,
            list_item: None,
            lease: Rc::new(RefCell::new(Lease {
                view: None,
                last_painted: Instant::now(),
                owner: cx.weak_entity(),
            })),
            bridge: None,
            status: "loading".into(),
            state: json!({}),
            hydrated: false,
            loading: None,
            height: 320.,
            notice: None,
            render_notice: None,
            pending: None,
            last_action: None,
            revision: 0,
            error_task: None,
            generation: 0,
            media_names: Vec::new(),
            media: Map::new(),
            media_generation: 0,
            media_task: None,
            media_retry_after: None,
        }
    }
    fn set_document(
        &mut self,
        source: String,
        original: String,
        complete: bool,
        owner: Option<WeakEntity<ChatPane>>,
        palette: Option<design::Palette>,
        list_item: Option<(ListState, usize)>,
        cx: &mut Context<Self>,
    ) {
        self.owner = owner;
        self.list_item = list_item;
        let changed = self.source != source || self.complete != complete || self.palette != palette;
        self.original = original;
        if changed {
            self.source = source;
            self.refresh_media(cx);
            self.complete = complete;
            self.palette = palette;
            self.pending = None;
            self.revision = self.revision.wrapping_add(1);
            self.push_update();
            self.invalidate(cx);
            cx.notify();
        }
    }
    fn invalidate(&self, cx: &mut Context<Self>) {
        // Documents are reconciled while the virtual list renders a row. Its
        // ListState is borrowed for layout then, so even reading item_count
        // synchronously would panic. Recheck the current row after layout.
        let owner = cx.weak_entity();
        cx.defer(move |cx| {
            let _ = owner.update(cx, |block, cx| {
                if let Some((list, index)) = &block.list_item {
                    if *index < list.item_count() {
                        list.remeasure_items(*index..*index + 1);
                        cx.notify();
                    }
                }
            });
        });
    }
    fn refresh_media(&mut self, cx: &mut Context<Self>) {
        let names = media_names(&self.source);
        let same_names = self.media_names == names;
        if same_names
            && (self.media_task.is_some()
                || self.media.len() >= names.len().min(MAX_MEDIA_FILES)
                || self
                    .media_retry_after
                    .is_some_and(|until| Instant::now() < until))
        {
            return;
        }
        self.media_names = names;
        self.media_generation = self.media_generation.wrapping_add(1);
        let generation = self.media_generation;
        self.media_task = None;
        if !same_names {
            self.media.clear();
        }
        if self.media_names.is_empty() {
            return;
        }
        // Missing previews may appear after a streamed document. Retry only
        // on a later document update or remount, never with a polling task.
        self.media_retry_after = Some(Instant::now() + Duration::from_secs(1));
        let source = self.source.clone();
        let worker = cx.background_executor().spawn(async move {
            media_directory()
                .map(|directory| approved_media(&source, &directory))
                .unwrap_or_default()
        });
        self.media_task = Some(cx.spawn(async move |owner, cx| {
            let media = worker.await;
            let _ = owner.update(cx, |block, cx| {
                if block.media_generation != generation {
                    return;
                }
                block.media = media;
                block.media_task = None;
                block.push_update();
                block.invalidate(cx);
                cx.notify();
            });
        }));
    }
    fn push_update(&self) {
        if let Some(bridge) = &self.bridge {
            let theme = self.palette.map(|p| json!({"dark":p.dark,"accent":format!("#{:06x}",p.accent),
                "paper":format!("#{:06x}",p.paper),"panel":format!("#{:06x}",p.panel),
                "ink":format!("#{:06x}",p.ink),"muted":format!("#{:06x}",p.muted),"line":format!("#{:06x}",p.line),
                "fontFamily":design::FONT_SANS,"monoFontFamily":design::FONT_MONO,"fontSize":13}));
            bridge.emit(
                "Update",
                json!({"source":self.source,"complete":self.complete,"state":self.state,
                "theme":theme,"media":self.media,
                "messageId":self.message,"blockId":self.ordinal,"revision":self.revision}),
            );
        }
    }
    fn hydrate(&mut self, cx: &mut Context<Self>) -> bool {
        if self.hydrated {
            return true;
        }
        match openui_state::read(&self.key) {
            openui_state::Read::Loading => {
                if self.loading.is_none() {
                    self.loading = Some(cx.spawn(async move |owner, cx| {
                        cx.background_executor()
                            .timer(Duration::from_millis(30))
                            .await;
                        let _ = owner.update(cx, |block, cx| {
                            block.loading = None;
                            cx.notify();
                        });
                    }));
                }
                false
            }
            openui_state::Read::Ready(saved) => {
                self.state = saved
                    .and_then(|value| sanitize_state(&value).ok())
                    .unwrap_or_else(|| json!({}));
                self.hydrated = true;
                true
            }
            openui_state::Read::Failed(_) => {
                self.notice =
                    Some("Saved form values are unavailable; changes remain local for now.".into());
                self.hydrated = true;
                true
            }
        }
    }
    fn stop(&mut self, cx: &mut Context<Self>) {
        self.generation = self.generation.wrapping_add(1);
        self.bridge = None;
        self.pending = None;
        self.status = "loading".into();
        self.loading = None;
        self.error_task = None;
        self.last_action = None;
        if let Some(view) = self.lease.borrow_mut().view.take() {
            view.read(cx).stop();
        }
        cx.notify();
    }
    fn mount(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.lease.borrow().view.is_some() || !self.hydrate(cx) {
            return;
        }
        self.refresh_media(cx);
        if !reserve(&self.lease, cx) {
            self.render_notice =
                Some("Interactive output limit reached; source is available below.".into());
            return;
        }
        self.bridge = None;
        self.status = "loading".into();
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        let factory_owner = cx.weak_entity();
        let (send_error, receive_error) = oneshot::channel::<()>();
        let send_error = RefCell::new(Some(send_error));
        self.error_task = Some(cx.spawn(async move |owner, cx| {
            if receive_error.await.is_ok() {
                let _ = owner.update(cx, |block, cx| {
                    if block.generation != generation {
                        return;
                    }
                    block.status = "error".into();
                    block.pending = None;
                    block.render_notice =
                        Some("Interactive output could not start; source is shown below.".into());
                    block.invalidate(cx);
                    cx.notify();
                });
            }
        }));
        let view = cx.new(|cx| {
            let view = AppView::new(
                AppOptions {
                    app_id: format!("com.monitter.openui.{}", uuid::Uuid::new_v4()),
                    // No NSH storage reaches disk. Empty grants separately deny
                    // network/filesystem/process capabilities.
                    storage: Some(Arc::new(EphemeralStorage)),
                    data_dir: Some(
                        std::env::temp_dir()
                            .join(format!("monitter-openui-{}", std::process::id())),
                    ),
                    limits: nsh_platform_gpui::Limits {
                        memory_bytes: 32 * 1024 * 1024,
                        load_budget: Duration::from_secs(3),
                        call_budget: Duration::from_millis(100),
                        max_nodes: 1000,
                        max_op_values: 16_384,
                        max_canvas_ops: 8192,
                        ..Default::default()
                    },
                    ..Default::default()
                },
                window,
                cx,
            );
            // Embedded output must reveal the transcript beneath it, including
            // from the first frame before the renderer receives its palette.
            let mut theme = nsh_base::Theme::default();
            theme.set_color("background", nsh_core::Color(0).into());
            view.set_theme(Some(theme), cx);
            view.register_component(
                BRIDGE,
                Rc::new(move |context| {
                    Box::new(Bridge {
                        owner: factory_owner.clone(),
                        context,
                        generation,
                    })
                }),
            );
            // Never display/log JS stack or generated form values in diagnostics.
            view.on_error(move |_error| {
                #[cfg(test)]
                if std::env::var_os("MONITTER_OPENUI_MOUNTED_TEST_CHILD").is_some() {
                    eprintln!(
                        "OpenUI mounted engine error category: {}",
                        if _error.contains("budget") || _error.contains("interrupted") {
                            "time-budget"
                        } else if _error.contains("memory") {
                            "memory"
                        } else {
                            "runtime"
                        }
                    );
                }
                if let Some(send) = send_error.borrow_mut().take() {
                    let _ = send.send(());
                }
            });
            let mut bundle = Bundle::source(RENDERER.to_owned());
            bundle.policy.declared = Some(nsh_core::access::Capabilities::default());
            bundle.policy.granted = Some(nsh_core::access::Capabilities::default());
            view.load(NshApp::Js(bundle));
            view
        });
        self.lease.borrow_mut().view = Some(view);
    }
    fn property(
        &mut self,
        generation: u64,
        context: &ComponentContext,
        name: &str,
        value: Option<&Value>,
        cx: &mut Context<Self>,
    ) {
        if self.generation != generation {
            return;
        }
        if matches!((name, value), ("status", Some(Value::String(status))) if status == "ready") {
            self.bridge = Some(context.clone());
            self.push_update();
            return;
        }
        #[cfg(test)]
        if name == "status" && std::env::var_os("MONITTER_OPENUI_MOUNTED_TEST_CHILD").is_some() {
            eprintln!("OpenUI mounted status: {:?}", value.and_then(Value::as_str));
        }
        self.apply_property(name, value, cx);
    }
    fn apply_property(&mut self, name: &str, value: Option<&Value>, cx: &mut Context<Self>) {
        match (name, value) {
            ("status", Some(Value::String(status)))
                if matches!(
                    status.as_str(),
                    "rendered" | "streaming" | "incomplete" | "error"
                ) =>
            {
                if status != "rendered" {
                    self.pending = None;
                } else if self.render_notice.take().is_some() {
                    cx.notify();
                }
                if self.status != *status {
                    self.status = status.clone();
                    self.invalidate(cx);
                    cx.notify();
                }
            }
            ("height", Some(value)) => {
                if let Some(height) = value
                    .as_f64()
                    .filter(|height| height.is_finite() && *height >= 0.)
                {
                    let height = (height as f32).clamp(80., 100_000.);
                    if (self.height - height).abs() >= 1. {
                        self.height = height;
                        self.invalidate(cx);
                        cx.notify();
                    }
                }
            }
            ("state", Some(Value::Null)) => {}
            ("state", Some(value)) => match sanitize_state(value) {
                Ok(state) if self.state != state => {
                    self.state = state.clone();
                    // A prepared action describes the previous form snapshot.
                    // Require another explicit Prepare after a user edits it.
                    if self.pending.take().is_some() {
                        cx.notify();
                    }
                    if openui_state::save(self.key.clone(), state).is_err() {
                        self.notice = Some("Form values could not be saved.".into());
                        cx.notify();
                    }
                }
                Err(_) => {
                    self.notice = Some("Unsupported form values were not saved.".into());
                    cx.notify();
                }
                _ => {}
            },
            ("action", Some(value))
                if !value.is_null() && self.complete && self.status == "rendered" =>
            {
                match draft_action(value) {
                    Ok(action) if action.id.is_none() || action.id != self.last_action => {
                        self.last_action = action.id.clone();
                        self.pending = Some(action);
                        cx.notify();
                    }
                    Err(_) => {
                        self.notice = Some(
                            "Unsupported action refused. OpenUI can only prepare a draft.".into(),
                        );
                        cx.notify();
                    }
                    _ => {}
                }
            }
            ("action", Some(value)) if !value.is_null() => {
                self.pending = None;
                self.notice = Some(
                    "Wait for the complete interactive output before preparing a response.".into(),
                );
                cx.notify();
            }
            ("action", Some(Value::Null)) => {
                if self.pending.take().is_some() {
                    cx.notify();
                }
            }
            ("error", Some(Value::String(error))) if !error.is_empty() => {
                self.render_notice = Some(
                    "This OpenUI block is unsupported or incomplete; source is shown below.".into(),
                );
                self.status = "error".into();
                self.pending = None;
                self.invalidate(cx);
                cx.notify();
            }
            _ => {}
        }
    }
}
struct Bridge {
    owner: WeakEntity<OutputBlock>,
    context: ComponentContext,
    generation: u64,
}
impl NativeComponent for Bridge {
    fn set_prop(&mut self, name: &str, value: Option<&Value>, _: &mut Window, cx: &mut App) {
        let _ = self.owner.update(cx, |block, cx| {
            block.property(self.generation, &self.context, name, value, cx)
        });
    }
}
impl Render for OutputBlock {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.mount(window, cx);
        let active = matches!(self.status.as_str(), "rendered" | "streaming")
            && self.lease.borrow().view.is_some();
        let mut result = div().w_full().min_w_0().flex().flex_col().gap_2().child(
            div()
                .text_size(px(11.))
                .child("Interactive output · actions prepare a draft; review it and press Send."),
        );
        if let Some(view) = self.lease.borrow().view.clone() {
            let lease = self.lease.clone();
            // This observer follows the cached paint range during Fast replay.
            // Visible rows remain recent; old offscreen engines are evictable.
            result = result.child(
                div()
                    .w_full()
                    .relative()
                    .h(px(if active { self.height } else { 0. }))
                    .overflow_hidden()
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .left_0()
                            .right_0()
                            .h(px(self.height))
                            .child(view),
                    )
                    .child(
                        canvas(
                            |_, _, _| (),
                            move |_, _, window, _| {
                                let lease = lease.clone();
                                window.observe_paint(move || {
                                    lease.borrow_mut().last_painted = Instant::now()
                                });
                            },
                        )
                        .absolute()
                        .inset_0(),
                    ),
            );
        }
        if !active {
            result = result.child(markdown(
                format!("openui-source-{}-{}", self.message, self.ordinal),
                self.original.clone(),
                self.owner.clone(),
            ));
        }
        if let Some(notice) = &self.notice {
            result = result.child(div().text_size(px(11.)).child(notice.clone()));
        }
        if let Some(notice) = &self.render_notice {
            result = result.child(div().text_size(px(11.)).child(notice.clone()));
        }
        if self.pending.is_some() {
            result = result.child(Button::new("openui-review-draft").label("Review response in composer")
                .on_click(cx.listener(|block, _, window, cx| {
                    let Some(action) = block.pending.take() else { return; };
                    if !block.complete || block.status != "rendered" { cx.notify(); return; }
                    let result = block.owner.as_ref().and_then(|owner| owner.update(cx, |pane, cx| {
                        pane.prepare_openui_draft(&block.task, &action.text, window, cx)
                    }).ok());
                    block.notice = Some(if result == Some(true) { "Response added to your draft. Review it, then press Send." }
                        else { "The original conversation is no longer available; no draft was changed." }.into());
                    cx.notify();
                })));
        }
        result
    }
}

#[cfg(test)]
mod openui_output_tests {
    use super::*;
    use ::core::prelude::v1::test;

    struct EmptyHost;
    impl Render for EmptyHost {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
        }
    }
    #[::core::prelude::v1::test]
    fn fences_preserve_markdown_and_ignore_nested_examples() {
        let text = "Before\n```openui\nroot = Text(\"Hi\")\n```\nAfter\n";
        let parsed = parts(text).unwrap();
        assert_eq!(parsed.len(), 3);
        assert!(
            matches!(&parsed[1], Part::Block { ordinal:0, source, closed:true, .. } if source == "root = Text(\"Hi\")\n")
        );
        assert_eq!(
            parts("````markdown\n```openui\nx\n```\n````\n")
                .unwrap()
                .len(),
            1
        );
        assert!(matches!(
            &parts("```openui\nx").unwrap()[0],
            Part::Block { closed: false, .. }
        ));
        assert!(!has_output(&json!({"role":"user","text":text})));
        assert!(!has_output(
            &json!({"role":"assistant","text":"```openui-js\nx\n```"})
        ));
    }
    #[::core::prelude::v1::test]
    fn limits_and_host_owned_identity() {
        assert!(parts(&format!("```openui\n{}", "x".repeat(MAX_SOURCE + 1))).is_err());
        assert!(parts(&"```openui\nx\n```\n".repeat(MAX_BLOCKS + 1)).is_err());
        assert_eq!(
            identity("task", "message", 0),
            identity("task", "message", 0)
        );
        assert_ne!(
            identity("task", "message", 0),
            identity("other", "message", 0)
        );
        assert_ne!(
            identity("task", "message", 0),
            identity("task", "message", 1)
        );
        assert!(identity("../profile", "message", 0).is_none());
    }
    #[::core::prelude::v1::test]
    fn public_media_is_bounded_to_regular_raster_files_in_its_directory() {
        let root = std::env::temp_dir().join(format!("openui-media-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let name = format!("{}.png", "a".repeat(64));
        let path = root.join(&name);
        image::RgbImage::new(2, 2).save(&path).unwrap();
        let source = format!("root = Image(\"Photo\", \"media:{name}\")");
        let media = approved_media(&source, &root);
        assert_eq!(
            media.get(&format!("media:{name}")).and_then(Value::as_str),
            Some(path.to_str().unwrap())
        );
        assert!(approved_media("root = Image(\"media:../../private.png\")", &root).is_empty());
        assert!(
            approved_media("root = Image(\"https://example.com/photo.jpg\")", &root).is_empty()
        );
        std::fs::remove_file(&path).unwrap();
        std::fs::write(&path, b"not an image").unwrap();
        assert!(approved_media(&source, &root).is_empty());
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            std::fs::remove_file(&path).unwrap();
            let outside =
                std::env::temp_dir().join(format!("openui-outside-{}", uuid::Uuid::new_v4()));
            image::RgbImage::new(2, 2)
                .save(&outside.with_extension("png"))
                .unwrap();
            symlink(outside.with_extension("png"), &path).unwrap();
            assert!(approved_media(&source, &root).is_empty());
            std::fs::remove_file(&path).unwrap();
            std::fs::remove_file(outside.with_extension("png")).unwrap();
        }
        std::fs::remove_dir_all(root).unwrap();
    }
    #[::core::prelude::v1::test]
    fn state_replay_is_bounded_and_excludes_secret_fields() {
        let state = json!({"$choice":"blue","form":{"name":"Alex","password":"never-save","api_key":"never-save","count":2,"enabled":true}});
        let safe = sanitize_state(&state).unwrap();
        assert_eq!(
            safe,
            json!({"$choice":"blue","form":{"name":"Alex","count":2,"enabled":true}})
        );
        assert_eq!(
            sanitize_state(&safe).unwrap(),
            safe,
            "restart hydration must preserve supported state"
        );
        let table = json!({"reply":{"table":[{"id":"one","values":["kept",2]}]}});
        assert_eq!(sanitize_state(&table).unwrap(), table);
        assert!(sanitize_state(&json!({"field":"x".repeat(2049)})).is_err());
        assert!(sanitize_state(&json!({"a":{"b":{"c":{"d":{"e":{"f":{"g":1}}}}}}})).is_err());
    }

    #[::core::prelude::v1::test]
    fn nsh_storage_never_persists_or_hydrates_raw_values() {
        let storage = EphemeralStorage;
        nsh_engine::StorageBackend::write(
            &storage,
            nsh_engine::StorageChange::Set("password".into(), "not-persisted".into()),
        );
        assert!(
            nsh_engine::StorageBackend::load(&storage)
                .unwrap()
                .is_empty()
        );
        assert!(
            nsh_engine::StorageBackend::load(&EphemeralStorage)
                .unwrap()
                .is_empty()
        );
    }
    #[::core::prelude::v1::test]
    fn actions_refuse_commands_and_never_choose_a_task() {
        assert!(draft_action(&json!({"type":"invoke","command":"send_message"})).is_err());
        assert!(draft_action(&json!({"type":"draft","text":"hello","taskId":"other"})).is_err());
        assert!(draft_action(&json!({"type":"draft","text":"x".repeat(MAX_DRAFT+1)})).is_err());
        assert_eq!(
            draft_action(&json!({"type":"draft","text":"colour: blue","id":"1"})).unwrap(),
            DraftAction {
                text: "colour: blue".into(),
                id: Some("1".into())
            }
        );
    }

    #[::gpui::test]
    fn editing_form_invalidates_prepared_response_until_prepared_again(cx: &mut TestAppContext) {
        let block = cx.new(|cx| {
            // An invalid storage key keeps this bridge-state test off disk.
            OutputBlock::new(String::new(), "task".into(), "message".into(), 0, cx)
        });
        block.update(cx, |block, cx| {
            let initial = json!({"form":{"colour":"blue"}});
            let edited = json!({"form":{"colour":"red"}});
            block.state = initial.clone();
            block.complete = true;
            block.status = "rendered".into();

            let prepared =
                json!({"type":"draft","id":"1","text":"Form response\n{\"colour\":\"blue\"}"});
            block.apply_property("action", Some(&prepared), cx);
            assert!(block.pending.as_ref().unwrap().text.contains("blue"));

            block.apply_property("state", Some(&initial), cx);
            assert!(
                block.pending.is_some(),
                "an unchanged state echo keeps the prepared response"
            );
            block.apply_property("state", Some(&edited), cx);
            assert_eq!(block.state, edited);
            assert!(
                block.pending.is_none(),
                "editing must remove the stale review action"
            );

            let prepared =
                json!({"type":"draft","id":"2","text":"Form response\n{\"colour\":\"red\"}"});
            block.apply_property("action", Some(&prepared), cx);
            let pending = block.pending.as_ref().unwrap();
            assert_eq!(pending.id.as_deref(), Some("2"));
            assert!(pending.text.contains("red"));
            assert!(!pending.text.contains("blue"));
        });
    }

    #[::gpui::test]
    fn retained_block_identity_and_state_survive_append_completion_and_hide(
        cx: &mut TestAppContext,
    ) {
        cx.update(|cx| gpui_kit::init(cx));
        let (_, visual) = cx.add_window_view(|_, _| EmptyHost);
        visual.update(|window, cx| {
            let mut output = MessageOutput::default();
            let mut message = json!({"taskId":"task","id":"message","role":"assistant",
                "streamStatus":"streaming","text":"```openui\nroot"});
            let _ = output.render(&message, None, None, None, window, cx);
            let key = identity("task", "message", 0).unwrap();
            let block = output.blocks[&key].clone();
            block.update(cx, |block, _| {
                block.state = sanitize_state(&json!({"form":{"colour":"blue"}})).unwrap();
                block.hydrated = true;
            });
            message["text"] = json!("```openui\nroot = Text(\"Ready\")\n```\n");
            message["streamStatus"] = json!("complete");
            let _ = output.render(&message, None, None, None, window, cx);
            assert_eq!(
                output.blocks[&key], block,
                "stream completion must keep the same native entity"
            );
            output.set_visible(false, cx);
            assert_eq!(
                block.read(cx).state,
                json!({"form":{"colour":"blue"}}),
                "parking an engine retains ordinary form values"
            );
            assert!(block.read(cx).lease.borrow().view.is_none());
            message["text"] = json!("The output block was removed.");
            assert!(
                output
                    .render(&message, None, None, None, window, cx)
                    .is_none()
            );
            assert!(
                output.blocks.is_empty(),
                "removed messages must not keep mount callbacks alive"
            );
        });
    }

    #[::gpui::test]
    fn live_surface_limit_evicts_only_old_mounts_and_keeps_form_state(cx: &mut TestAppContext) {
        cx.update(|cx| gpui_kit::init(cx));
        let (_, visual) = cx.add_window_view(|_, _| EmptyHost);
        visual.update(|window, cx| {
            let mut blocks = Vec::new();
            for index in 0..MAX_ENGINES {
                let block = cx.new(|cx| {
                    OutputBlock::new(
                        format!("test-{index}"),
                        "task".into(),
                        format!("message-{index}"),
                        0,
                        cx,
                    )
                });
                let lease = block.read(cx).lease.clone();
                assert!(reserve(&lease, cx));
                let view = cx.new(|cx| AppView::new(AppOptions::default(), window, cx));
                lease.borrow_mut().view = Some(view); // no JS is loaded by this lifecycle test
                blocks.push(block);
            }
            let extra =
                cx.new(|cx| OutputBlock::new("extra".into(), "task".into(), "extra".into(), 0, cx));
            let extra_lease = extra.read(cx).lease.clone();
            assert!(
                !reserve(&extra_lease, cx),
                "visible mounts must not thrash when the limit is full"
            );
            blocks[0].update(cx, |block, _| {
                block.state = json!({"choice":"saved"});
                block.lease.borrow_mut().last_painted = Instant::now() - Duration::from_secs(3);
            });
            assert!(reserve(&extra_lease, cx));
            assert!(blocks[0].read(cx).lease.borrow().view.is_none());
            assert_eq!(blocks[0].read(cx).state, json!({"choice":"saved"}));
            for block in blocks {
                block.update(cx, |block, cx| block.stop(cx));
            }
        });
    }

    #[cfg(feature = "performance-profiling")]
    #[::gpui::test]
    fn native_draft_action_keeps_typing_and_rejects_other_conversation(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::surface_blend::initialize_for_ui_fixture();
            crate::native_motion::initialize_for_ui_fixture(cx);
        });
        let snapshot = Rc::new(json!({"settings":{},
            "tasks":[{"id":"openui-task","title":"OpenUI","agentId":"fixture-agent","provider":"codex","status":"idle"}],
            "agents":[{"id":"fixture-agent","name":"Fixture","provider":"codex"}],
            "messages":[],"events":[],"approvalRequests":[],"queuedMessages":[],"hosts":[],"channels":[]}));
        let (pane, visual) = cx.add_window_view(|window, cx| {
            ChatPane::performance_fixture("openui-task".into(), snapshot.clone(), window, cx)
        });
        pane.update_in(visual, |pane, window, cx| {
            pane.composer.update(cx, |editor, cx| {
                editor.set_value("My unfinished draft", window, cx)
            });
            assert!(!pane.prepare_openui_draft("other-task", "form result", window, cx));
            assert_eq!(
                pane.composer.read(cx).value().as_str(),
                "My unfinished draft"
            );
            assert!(pane.prepare_openui_draft("openui-task", "colour: blue", window, cx));
            assert_eq!(
                pane.composer.read(cx).value().as_str(),
                "My unfinished draft\n\ncolour: blue"
            );
            assert!(
                pane.snapshot["messages"].as_array().unwrap().is_empty(),
                "preparing a draft must never append/send a message"
            );
            assert!(
                pane.snapshot["queuedMessages"]
                    .as_array()
                    .unwrap()
                    .is_empty(),
                "no backend submission is queued"
            );
        });
    }
}
