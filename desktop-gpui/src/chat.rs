//! One retained native chat entity per real backend task. No route singleton.
use super::*;
use crate::design::{radius, space, ui_scaled_px as px};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use gpui_kit::assets::IconName;
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable, WindowExt as _};
use std::{sync::Arc, time::Duration};
#[path = "chat_attachments.rs"]
mod attachments;
#[path = "message_attachments.rs"]
mod message_attachments;
#[path = "system_divider.rs"]
mod system_divider;
use attachments::{AttachmentScope, ChatAttachments};
#[path = "run_activity.rs"]
pub(crate) mod run_activity;
use super::transcript::TimelineRow;
#[path = "chat_transcript_view.rs"]
mod transcript_view;
use transcript_view::{TranscriptPreparationKey, TranscriptViewport};
#[path = "chat_slash.rs"]
mod chat_slash;
#[path = "composer_format.rs"]
mod composer_format;
#[path = "task_activity.rs"]
pub(crate) mod task_activity;
#[path = "connection_recovery.rs"]
pub(crate) mod connection_recovery;

#[cfg(test)]
#[path = "chat_column_fixture_tests.rs"]
mod chat_column_fixture_tests;

#[cfg(any(test, feature = "performance-profiling"))]
static TRANSCRIPT_PREPARATIONS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
#[cfg(any(test, feature = "performance-profiling"))]
static TRANSCRIPT_PREPARED_ROWS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Numeric diagnostics only; no draft, message, task or provider data.
#[cfg(any(test, feature = "performance-profiling"))]
pub(crate) fn transcript_preparation_counts() -> (u64, u64) {
    use std::sync::atomic::Ordering::Relaxed;
    (TRANSCRIPT_PREPARATIONS.load(Relaxed), TRANSCRIPT_PREPARED_ROWS.load(Relaxed))
}

#[cfg(feature = "performance-profiling")]
pub(crate) fn transcript_preparation_reason_counts() -> Vec<(&'static str, u64)> {
    transcript_view::preparation_reason_counts()
}

fn mix_rgb(base: u32, color: u32, amount: f32) -> u32 {
    let channel = |shift: u32| {
        let base = ((base >> shift) & 0xffu32) as f32;
        let color = ((color >> shift) & 0xffu32) as f32;
        (base * (1.0 - amount) + color * amount).round() as u32
    };
    (channel(16) << 16) | (channel(8) << 8) | channel(0)
}

/// Keep all chat-pane content on the same centered design-width column.
/// The header and the transcript viewport remain pane-wide; only their
/// content rows use this shared alignment boundary.
fn transcript_column(content: impl IntoElement) -> gpui_kit::AnyElement {
    div()
        .w_full()
        .min_w_0()
        .flex()
        .justify_center()
        .child(
            div()
                .w_full()
                .max_w(px(design::CHAT_MAX_WIDTH))
                .min_w_0()
                .child(content),
        )
        .into_any_element()
}

fn split_operator_message(text: &str) -> (Option<String>, String) {
    let content = text
        .strip_prefix("[Two human operators are collaborating")
        .and_then(|rest| {
            rest.find('\n')
                .filter(|end| rest[..*end].ends_with(']'))
                .map(|end| &rest[end + 1..])
        })
        .unwrap_or(text);
    let Some(rest) = content.strip_prefix("@(") else {
        return (None, content.to_owned());
    };
    let Some(end) = rest.find("):") else {
        return (None, content.to_owned());
    };
    let name = rest[..end].trim();
    if name.is_empty() || name.chars().count() > 48 || name.contains(['(', ')', '\n']) {
        return (None, content.to_owned());
    }
    let tail = rest[end + 2..]
        .strip_prefix(' ')
        .unwrap_or(&rest[end + 2..]);
    (Some(name.to_owned()), tail.to_owned())
}

/// Resolve reply content when clicked, after any transcript reset or stream update.
fn composer_enter_sends(send_with_enter: bool, secondary: bool, shift: bool) -> bool {
    !shift && (send_with_enter || secondary)
}

/// Only plain Enter can submit a typed slash command. Keep ordinary and
/// modified key handling independent of the full composer value.
fn typed_submit_for_key(
    key: &str,
    shift: bool,
    alt: bool,
    control: bool,
    platform: bool,
    input: impl FnOnce() -> gpui::SharedString,
) -> bool {
    if key != "enter" || shift || alt || control || platform {
        return false;
    }
    let input = input();
    chat_slash::typed_submit_before_menu(&input)
}

fn current_reply_content(
    messages: &[Rc<Value>],
    message_id: &str,
    fallback_author: &str,
) -> Option<(String, String)> {
    let message = messages
        .iter()
        .find(|message| field(message, "id") == message_id)?;
    let (operator, text) = match field(message, "role").as_str() {
        "user" => split_operator_message(&field(message, "text")),
        "assistant" => (None, field(message, "text")),
        _ => return None,
    };
    if text.trim().is_empty() {
        return None;
    }
    Some((operator.unwrap_or_else(|| fallback_author.to_owned()), text))
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct MessageStreamPresentation {
    pub(crate) markdown_source: String,
    pub(crate) status_label: Option<&'static str>,
}

fn escape_markdown_plain_text(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        if character == '\n' {
            // A Markdown hard break retains the literal streamed line boundary.
            escaped.push('\\');
        } else if character.is_ascii_punctuation() {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped
}

pub(crate) fn message_stream_presentation(message: &Value, visible_text: &str) -> MessageStreamPresentation {
    let status = field(message, "streamStatus");
    let streaming = field(message, "role") == "assistant" && status == "streaming";
    MessageStreamPresentation {
        markdown_source: if streaming {
            // TextView is selectable but Markdown-backed. Escape punctuation so a
            // live partial answer stays literal until the backend marks it complete.
            escape_markdown_plain_text(visible_text)
        } else {
            visible_text.to_owned()
        },
        status_label: match status.as_str() {
            "streaming" => Some("Receiving…"),
            "interrupted" => Some("Partial reply · interrupted"),
            _ => None,
        },
    }
}

#[cfg(test)]
mod message_stream_tests {
    use super::*;
    use serde_json::json;

    #[::core::prelude::v1::test]
    fn composer_enter_matches_tauri_send_preference_and_shift() {
        for send in [false, true] {
            assert_eq!(composer_enter_sends(send, false, false), send);
            assert!(composer_enter_sends(send, true, false));
            assert!(!composer_enter_sends(send, false, true));
            assert!(!composer_enter_sends(send, true, true));
        }
    }

    #[::core::prelude::v1::test]
    fn typed_submit_reads_draft_only_for_unmodified_enter() {
        use std::cell::Cell;

        let read = Cell::new(false);
        let input = || {
            read.set(true);
            gpui::SharedString::from("/terminal")
        };
        assert!(!typed_submit_for_key("a", false, false, false, false, input));
        assert!(!read.get());

        for modifiers in [
            (true, false, false, false),
            (false, true, false, false),
            (false, false, true, false),
            (false, false, false, true),
        ] {
            let read = Cell::new(false);
            assert!(!typed_submit_for_key(
                "enter",
                modifiers.0,
                modifiers.1,
                modifiers.2,
                modifiers.3,
                || {
                    read.set(true);
                    gpui::SharedString::from("/terminal")
                },
            ));
            assert!(!read.get());
        }

        let read = Cell::new(false);
        assert!(typed_submit_for_key(
            "enter",
            false,
            false,
            false,
            false,
            || {
                read.set(true);
                gpui::SharedString::from("/terminal")
            },
        ));
        assert!(read.get());
    }

    #[::core::prelude::v1::test]
    fn chat_activity_visibility_matches_tauri_without_dropping_diagnostics() {
        let settings = json!({"showToolActivity":false,"showReasoningSummaries":false});
        assert!(!visible_chat_activity(
            &json!({"kind":"computer"}),
            &json!({})
        ));
        assert!(!visible_chat_activity(&json!({"kind":"tool"}), &settings));
        assert!(!visible_chat_activity(
            &json!({"kind":"reasoning"}),
            &settings
        ));
        assert!(visible_chat_activity(&json!({"kind":"tool"}), &json!({})));
        assert!(visible_chat_activity(
            &json!({"kind":"reasoning"}),
            &json!({})
        ));
        for kind in ["usage", "error", "goal"] {
            assert!(visible_chat_activity(&json!({"kind":kind}), &settings));
        }
    }

    #[::core::prelude::v1::test]
    fn reply_uses_current_text_and_rejects_deleted_or_non_message_rows() {
        let messages = vec![
            json!({"id":"a","role":"assistant","text":"Latest streamed answer"}),
            json!({"id":"u","role":"user","text":"@(Alex): Current question"}),
            json!({"id":"s","role":"system","text":"Internal record"}),
            json!({"id":"empty","role":"assistant","text":"  "}),
        ].into_iter().map(Rc::new).collect::<Vec<_>>();
        assert_eq!(
            current_reply_content(&messages, "a", "Agent"),
            Some(("Agent".into(), "Latest streamed answer".into()))
        );
        assert_eq!(
            current_reply_content(&messages, "u", "You"),
            Some(("Alex".into(), "Current question".into()))
        );
        assert!(current_reply_content(&messages, "deleted", "Agent").is_none());
        assert!(current_reply_content(&[], "a", "Agent").is_none());
        assert!(current_reply_content(&messages, "s", "Agent").is_none());
        assert!(current_reply_content(&messages, "empty", "Agent").is_none());
    }

    #[::core::prelude::v1::test]
    fn streaming_assistant_text_is_literal_and_keeps_unicode_and_newlines() {
        let presentation = message_stream_presentation(
            &json!({"role":"assistant","streamStatus":"streaming"}),
            "# Hello *there*\n世界 🙂 `code`",
        );
        assert_eq!(
            presentation.markdown_source,
            "\\# Hello \\*there\\*\\\n世界 🙂 \\`code\\`"
        );
        assert_eq!(presentation.status_label, Some("Receiving…"));
    }

    #[::core::prelude::v1::test]
    fn complete_and_interrupted_messages_keep_markdown_source() {
        let complete = message_stream_presentation(
            &json!({"role":"assistant","streamStatus":"complete"}),
            "**Finished**",
        );
        assert_eq!(complete.markdown_source, "**Finished**");
        assert_eq!(complete.status_label, None);

        let interrupted = message_stream_presentation(
            &json!({"role":"assistant","streamStatus":"interrupted"}),
            "**Partial**",
        );
        assert_eq!(interrupted.markdown_source, "**Partial**");
        assert_eq!(
            interrupted.status_label,
            Some("Partial reply · interrupted")
        );
    }

    #[::core::prelude::v1::test]
    fn only_assistant_streaming_bodies_are_escaped() {
        let user = message_stream_presentation(
            &json!({"role":"user","streamStatus":"streaming"}),
            "*literal*",
        );
        assert_eq!(user.markdown_source, "*literal*");
        assert_eq!(user.status_label, Some("Receiving…"));
    }
}

fn chat_date_label(timestamp_ms: i64, now_ms: i64) -> String {
    let Some(timestamp) = crate::timestamps::local_datetime(timestamp_ms) else {
        return String::new();
    };
    let Some(now) = crate::timestamps::local_datetime(now_ms) else {
        return timestamp.format("%d/%m/%Y").to_string();
    };
    let distance = now
        .date_naive()
        .signed_duration_since(timestamp.date_naive())
        .num_days();
    match distance {
        0 => "Today".into(),
        1 => "Yesterday".into(),
        2 => "2 days ago".into(),
        3..=6 => format!("Last {}", timestamp.format("%A")),
        _ => timestamp.format("%d/%m/%Y").to_string(),
    }
}

fn chat_time_label(timestamp_ms: i64, now_ms: i64) -> String {
    crate::timestamps::local_datetime(timestamp_ms)
        .map(|timestamp| {
            format!(
                "{} - {}",
                timestamp.format("%-I:%M %P"),
                chat_date_label(timestamp_ms, now_ms)
            )
        })
        .unwrap_or_default()
}

fn operator_colour(name: &str) -> u32 {
    let mut hash = 2_166_136_261u32;
    for character in name.trim().to_lowercase().chars() {
        hash = (hash ^ character as u32).wrapping_mul(16_777_619);
    }
    let hue = hash % 360;
    let c: f32 = (1.0_f32 - (2.0_f32 * 0.52_f32 - 1.0_f32).abs()) * 0.55_f32;
    let x: f32 = c * (1.0_f32 - ((hue as f32 / 60.0_f32) % 2.0_f32 - 1.0_f32).abs());
    let (r, g, b) = match hue / 60 {
        0 => (c, x, 0.),
        1 => (x, c, 0.),
        2 => (0., c, x),
        3 => (0., x, c),
        4 => (x, 0., c),
        _ => (c, 0., x),
    };
    let m: f32 = 0.52_f32 - c / 2.0_f32;
    let chan = |value: f32| ((value + m) * 255.0).round() as u32;
    (chan(r) << 16) | (chan(g) << 8) | chan(b)
}

fn visible_chat_activity(event: &Value, settings: &Value) -> bool {
    match event["kind"].as_str().unwrap_or_default() {
        "computer" => false,
        "tool" => settings["showToolActivity"].as_bool() != Some(false),
        "reasoning" => settings["showReasoningSummaries"].as_bool() != Some(false),
        _ => true,
    }
}

thread_local! {
    static AGENT_AVATAR_CACHE: std::cell::RefCell<(
        std::collections::HashMap<(u64, usize), Option<Arc<Image>>>,
        std::collections::VecDeque<(u64, usize)>,
    )> = std::cell::RefCell::new((std::collections::HashMap::new(), std::collections::VecDeque::new()));
}

pub(crate) fn agent_avatar(agent: &Value) -> Option<Arc<Image>> {
    let data = agent["avatar"].as_str()?;
    if data.len() > 3 * 1024 * 1024 {
        return None;
    }
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    data.hash(&mut hasher);
    let key = (hasher.finish(), data.len());
    if let Some(cached) = AGENT_AVATAR_CACHE.with(|cache| cache.borrow().0.get(&key).cloned()) {
        return cached;
    }
    let image = (|| {
        let (prefix, encoded) = data.split_once(',')?;
        let format = if prefix.eq_ignore_ascii_case("data:image/png;base64") {
            ImageFormat::Png
        } else if prefix.eq_ignore_ascii_case("data:image/jpeg;base64") {
            ImageFormat::Jpeg
        } else if prefix.eq_ignore_ascii_case("data:image/webp;base64") {
            ImageFormat::Webp
        } else {
            return None;
        };
        let bytes = STANDARD.decode(encoded).ok()?;
        let valid = match format {
            ImageFormat::Png => bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
            ImageFormat::Jpeg => bytes.starts_with(&[0xff, 0xd8, 0xff]),
            ImageFormat::Webp => {
                bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP"
            }
            _ => false,
        };
        valid.then(|| Arc::new(Image::from_bytes(format, bytes)))
    })();
    cache_avatar(key, image.clone());
    image
}

fn cache_avatar(key: (u64, usize), image: Option<Arc<Image>>) {
    const MAX_CACHED_AVATARS: usize = 32;
    AGENT_AVATAR_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        cache.0.insert(key, image);
        cache.1.retain(|existing| *existing != key);
        cache.1.push_back(key);
        while cache.1.len() > MAX_CACHED_AVATARS {
            if let Some(oldest) = cache.1.pop_front() {
                cache.0.remove(&oldest);
            }
        }
    });
}

/// Inter-block rhythm follows the same density ratio as composer spacing.
/// Normal doubles the former fixed 10-design-pixel transcript separator.
fn transcript_block_gap(settings: &Value) -> f32 {
    match field(settings, "interfaceDensity").as_str() {
        "tight" => space::INSET,
        "spacious" => space::SECTION * 2.,
        _ => space::SECTION,
    }
}

fn date_separator(label: &str, palette: design::Palette) -> gpui_kit::AnyElement {
    div()
        .w_full()
        .my_1()
        .flex()
        .items_center()
        .gap_2()
        .child(div().h(px(1.)).flex_1().bg(rgb(palette.line)))
        .child(
            div()
                .text_size(px(9.))
                .font_family(design::FONT_MONO)
                .text_color(rgb(palette.muted))
                .child(label.to_uppercase()),
        )
        .child(div().h(px(1.)).flex_1().bg(rgb(palette.line)))
        .into_any_element()
}

fn approval_history_label(
    approval: &transcript::ApprovalRow,
    is_input: bool,
    decision: &str,
) -> String {
    let summary = approval.summary.trim();
    let normalized_summary: String = summary
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect();
    let structured = serde_json::from_str::<Value>(summary).is_ok()
        || (summary.starts_with('{') && summary.ends_with('}'))
        || [
            "agentthreadid",
            "receiverthreadids",
            "connectorpermissionmetadata",
            "permissionmetadata",
        ]
        .iter()
        .any(|field| normalized_summary.contains(field));
    let subject = if structured {
        String::new()
    } else {
        let candidate = if summary.is_empty() {
            approval.tool.trim()
        } else {
            summary
        };
        candidate
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .chars()
            .take(100)
            .collect::<String>()
            .trim_end()
            .trim_end_matches(['?', '。'])
            .to_owned()
    };
    let subject = subject.strip_prefix("allow ").unwrap_or(&subject).trim();
    let label = match approval.status.as_str() {
        "approved" if is_input => "Submitted input",
        "approved" if approval.rule_id.is_some() => "Approved by saved rule",
        "approved" if decision == "approve_session" => "Approved for this session",
        "approved" if decision == "approve_always" => "Always approved",
        "approved" => "Approved",
        "denied" => "Denied",
        "expired" => "Approval expired",
        _ => "Approval unavailable",
    };
    if subject.is_empty() {
        label.to_owned()
    } else {
        format!("{label} for {subject}")
    }
}

const FINAL_ANSWER_GREEN: u32 = 0x4f9d69;

#[derive(Clone)]
pub enum ChatEvent {
    Refresh,
    Archived(String),
    DraftChanged {
        task_id: String,
        text: String,
    },
    AttachmentDraftChanged {
        task_id: String,
        state: monitter_lib::experimental_desktop::NativeAttachmentDraft,
    },
    HistoryPage {
        task_id: String,
        page: Result<TaskMessagesPage, String>,
    },
    SnapshotUpdated(Value),
    Settings(String),
    Git(String),
    OpenTask(String),
    DelegateTask(String),
    Terminal {
        task_id: String,
        command: Option<String>,
    },
    Focused(String),
    SecondarySidebarChanged {
        task_id: String,
        state: NativeChatPanePreferences,
    },
}
struct DeferredChatPublication {
    task_id: String,
    snapshot: Rc<Value>,
    revision: Option<String>,
    records: std::sync::Arc<Vec<Value>>,
    child_transcripts: monitter_gpui_desktop::native_publication::PreparedChildTranscripts,
    context_unchanged: bool,
    history: Option<Vec<Value>>,
    reset: bool,
    boundary: Option<Option<String>>,
}
#[cfg(all(test, feature = "performance-profiling"))]
#[path = "hidden_publication_tests.rs"]
mod hidden_publication_tests;

pub struct ChatPane {
    dock_group: Option<WeakEntity<gpui_kit::component::dock::TabGroup>>,
    #[cfg(feature = "performance-profiling")]
    performance_fixture: bool,
    pub(super) service: Option<DesktopService>,
    pub(super) snapshot: Rc<Value>,
    composed_palette: Option<design::Palette>,
    pub(super) revision: Option<String>,
    pub(super) selected: Option<String>,
    pub(super) composer: Entity<TextareaState>,
    composer_git: Entity<crate::composer_git::ComposerGitStrip>,
    _composer_git_change: Subscription,
    composer_footer: Entity<crate::composer_footer::ComposerFooterPane>,
    composer_context_bar: Entity<crate::composer_footer::ComposerContextBar>,
    _composer_footer_change: Subscription,
    voice: Entity<crate::chat_voice::VoiceComposer>,
    _voice_change: Subscription,
    _details_change: Subscription,
    slash: Entity<crate::slash_palette::SlashPalette>,
    _slash_change: Subscription,
    slash_notice: Option<String>,
    slash_effect: Option<String>,
    project_picker: Entity<crate::task_project_picker::TaskProjectPicker>,
    _project_picker_change: Subscription,
    pub(super) error: Option<String>,
    pub(super) busy: bool,
    starting_turn: bool,
    activity_rows: HashMap<String, Entity<run_activity::RunActivityRow>>,
    activity_projection_source: Option<(Rc<Vec<transcript::ConversationItem>>, bool, bool)>,
    grouped_rows: HashMap<String, Entity<run_activity::RunActivityRow>>,
    message_attachment_rows: HashMap<String, Entity<message_attachments::MessageAttachments>>,
    message_action_focus: HashMap<String, FocusHandle>,
    streaming_bodies: HashMap<String, Entity<crate::streaming_message::StreamingMessage>>,
    streaming_bodies_initialized: bool,
    transcript_context_unchanged: bool,
    deferred_publication: Option<DeferredChatPublication>,
    activity_refresh_pending: bool,
    #[cfg(test)]
    activity_context_refreshes: usize,
    pub(crate) decoration_bounds: crate::activity_sparkles::ChatDecorationBounds,
    task_actions: Option<Entity<crate::chat_task_actions::ChatTaskActionsPane>>,
    task_actions_subscription: Option<Subscription>,
    fork_dialog: Option<Entity<crate::fork_dialog::ForkDialogPane>>,
    fork_subscription: Option<Subscription>,
    pending_fork_open: Option<String>,
    pub(super) refreshing: bool,
    pub(super) clear_sent: Option<(String, String)>,
    pub(super) transcripts: HashMap<String, TranscriptState>,
    pub(super) dock_visible: bool,
    pub(super) answers: HashMap<(String, String), Entity<InputState>>,
    pub(super) form_choices: HashMap<(String, String), Value>,
    pub(super) queue_edit: Option<String>,
    pub(super) queue_editor: Entity<TextareaState>,
    typography: Option<String>,
    transcript_block_spacing: Option<f32>,
    composer_scroll_distance: Pixels,
    bottom_jump: Option<crate::composer_scroll::BottomJump>,
    bottom_button_exit: Option<crate::composer_scroll::BottomButtonExit>,
    chat_family: SharedString,
    focus: FocusHandle,
    _queue_change: Subscription,
    _composer_change: Subscription,
    draft_touched: bool,
    attachments: Entity<ChatAttachments>,
    _attachment_change: Subscription,
    activity: Entity<task_activity::TaskActivityPane>,
    computer_bubble: Entity<task_activity::ComputerUseBubble>,
    pub(crate) working_bubble: Entity<task_activity::working_bubble::WorkingBubble>,
    _working_bubble_change: Subscription,
    _activity_change: Subscription,
    companion: Entity<crate::collaboration::SubagentPane>,
    companion_visible: bool,
    companion_saved_visible: bool,
    companion_programmatic_resizes: usize,
    pane_width_design: f32,
    pane_height_design: f32,
    compact_detail: bool,
    detail_motion: crate::companion_sidebar::RailMotion,
    detail_animation_frame: bool,
    detail_transcript_cache: Option<Entity<StableTranscriptList>>,
    transcript_viewport: Option<Entity<TranscriptViewport>>,
    transcript_preparation_key: Option<TranscriptPreparationKey>,
    response_disclosure_generation: u64,
    detail_tab: crate::companion_sidebar::DetailTab,
    detail_tabs_motion: Rc<std::cell::RefCell<crate::sidebar_tabs_motion::SidebarTabsMotion>>,
    detail_focus: [FocusHandle; 4],
    details: Entity<crate::pane_details::PaneDetails>,
    approvals: Entity<crate::pane_approvals::PaneApprovals>,
    approval_popup_id: Option<String>,
    pub(super) interaction_request_id: Option<String>,
    pub(super) mcp_approval_supported: bool,
    approval_popup_open: bool,
    approval_focus: FocusHandle,
    pub(super) approval_details_expanded: bool,
    response_details_expanded: HashSet<String>,
    companion_context: (Option<String>, String),
    companion_width: f32,
    companion_scale: u16,
    companion_resize: Entity<ResizableState>,
    _companion_resize_change: Subscription,
    _companion_change: Subscription,
}
impl EventEmitter<ChatEvent> for ChatPane {}
impl Focusable for ChatPane {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl ChatPane {
    pub fn new(
        task_id: String,
        service: DesktopService,
        snapshot: Rc<Value>,
        revision: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        // A GUI-only update can reconnect to an older resident backend.
        // Probe capabilities without interrupting any resident agent.
        let capability_service = service.clone();
        let (capability_sender, capability_receiver) = futures::channel::oneshot::channel();
        let _ = std::thread::Builder::new().name("monitter-approval-capabilities".into())
            .stack_size(8 * 1024 * 1024).spawn(move || {
                let supported = capability_service.invoke("read_mcp_approval_support", json!({}))
                    .is_ok_and(|value| value["nativeScopes"] == true);
                let _ = capability_sender.send(supported);
            });
        cx.spawn(async move |pane, cx| {
            let supported = capability_receiver.await.unwrap_or(false);
            let _ = pane.update(cx, |pane, cx| { pane.mcp_approval_supported = supported; cx.notify(); });
        }).detach();
        let composer = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(1, 12)
                .placeholder("Message this agent…")
        });
        let slash = cx.new(|cx| {
            let mut palette = crate::slash_palette::SlashPalette::new(service.clone());
            palette.set_context(
                &snapshot,
                revision.as_deref().unwrap_or(""),
                task_id.clone(),
                cx,
            );
            palette
        });
        let slash_change =
            cx.subscribe_in(&slash, window, |pane, _, event, window, cx| match event {
                crate::slash_palette::SlashPaletteEvent::Selected { item, input } => {
                    pane.handle_slash(item.clone(), input.clone(), false, window, cx);
                }
                crate::slash_palette::SlashPaletteEvent::Submit { item, input } => {
                    pane.handle_slash(item.clone(), input.clone(), true, window, cx);
                }
            });
        let project_picker = cx.new(|_| {
            crate::task_project_picker::TaskProjectPicker::new(
                service.clone(),
                settings::palette(&snapshot["settings"], window.appearance()),
            )
        });
        let project_picker_change = cx.subscribe_in(
            &project_picker,
            window,
            |pane, _, event, window, cx| match event {
                crate::task_project_picker::TaskProjectPickerEvent::RefreshRequired { task_id } => {
                    pane.refresh(cx);
                    if pane.task_id() == *task_id {
                        window.close_dialog(cx);
                    }
                }
                crate::task_project_picker::TaskProjectPickerEvent::Dismissed { task_id } => {
                    if pane.task_id() == *task_id {
                        window.close_dialog(cx);
                    }
                }
            },
        );
        let voice = cx.new(|cx| crate::chat_voice::VoiceComposer::new(service.clone(), cx));
        voice.update(cx, |voice, cx| voice.set_context(task_id.clone(), String::new(), cx));
        let voice_change =
            cx.subscribe_in(&voice, window, |pane, _, event, window, cx| match event {
                crate::chat_voice::VoiceComposerEvent::StatusChanged => cx.notify(),
                crate::chat_voice::VoiceComposerEvent::TranscriptReady {
                    task_id,
                    draft,
                    text,
                } => {
                    if pane.task_id() != *task_id {
                        return;
                    }
                    let current = pane.composer.read(cx).value().to_string();
                    let composing = pane.composer.update(cx, |input, cx| {
                        EntityInputHandler::marked_text_range(input, window, cx).is_some()
                    });
                    if current != *draft || composing {
                        window.push_notification(
                            gpui_kit::component::notification::Notification::warning(
                                "Voice transcript was not inserted because the draft changed.",
                            ), cx,
                        );
                        cx.notify();
                        return;
                    }
                    let inserted = if draft.is_empty() || draft.ends_with('\n') {
                        format!("{draft}{text}")
                    } else {
                        format!("{draft}\n{text}")
                    };
                    pane.composer
                        .update(cx, |input, cx| input.set_value(inserted.clone(), window, cx));
                    pane.voice.update(cx, |voice, cx| voice.set_context(task_id.clone(), inserted.clone(), cx));
                    pane.slash.update(cx, |slash, cx| slash.set_input(inserted.clone(), cx));
                    cx.emit(ChatEvent::DraftChanged { task_id: task_id.clone(), text: inserted });
                    cx.notify();
                }
            });
        let composer_change = cx.subscribe(&composer, |pane, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                let text = input.read(cx).value().to_string();
                pane.slash
                    .update(cx, |slash, cx| slash.set_input(text.clone(), cx));
                let task_id = pane.task_id();
                pane.voice
                    .update(cx, |voice, cx| voice.set_context(task_id, text.clone(), cx));
                if !text.is_empty() {
                    pane.slash_notice = None;
                }
                pane.bottom_jump = None;
                pane.composer_scroll_distance = gpui_kit::px(0.);
                pane.draft_touched = true;
                cx.emit(ChatEvent::DraftChanged {
                    task_id: pane.task_id(),
                    text,
                });
                cx.notify();
            }
        });
        let queue_editor = cx.new(|cx| TextareaState::new(window, cx));
        let subscription = cx.subscribe(&queue_editor, |_, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        });
        // A newly opened pane must show the already-loaded cache immediately;
        // it may not receive another committed change for an idle conversation.
        let mut transcript = TranscriptState::default();
        transcript.reconcile(
            rows(&snapshot, "messages")
                .into_iter()
                .filter(|message| field(message, "taskId") == task_id)
                .collect(),
            true,
        );
        transcript.reconcile_activities(
            rows(&snapshot, "events")
                .into_iter()
                .filter(|event| {
                    field(event, "taskId") == task_id
                        && visible_chat_activity(event, &snapshot["settings"])
                })
                .collect(),
            true,
        );
        let known_messages = transcript.messages.as_ref().clone();
        let transcripts = HashMap::from([(task_id.clone(), transcript)]);
        let scope = Self::attachment_scope(&snapshot, &task_id);
        let attachments = cx.new(|_| ChatAttachments::new(service.clone(), task_id.clone(), scope));
        let attachment_change = cx.subscribe(
            &attachments,
            |pane, _, _: &attachments::AttachmentEvent, cx| {
                let state = pane.attachments.read(cx).persistence_snapshot();
                cx.emit(ChatEvent::AttachmentDraftChanged {
                    task_id: pane.task_id(),
                    state,
                });
                cx.notify();
            },
        );
        let decoration_bounds = crate::activity_sparkles::ChatDecorationBounds::default();
        let working_palette = design::palette(&snapshot["settings"], window.appearance());
        let working_bubble = cx.new(|_| task_activity::working_bubble::WorkingBubble::new(
            decoration_bounds.working_anchor.clone(), decoration_bounds.chat_column.clone(), working_palette,
        ));
        let working_bubble_change = cx.subscribe(
            &working_bubble,
            |view, _, _: &task_activity::working_bubble::WorkingBubbleEvent, cx| {
                if let Some(id) = view.selected.clone() {
                    view.command("cancel_task", json!({"taskId": id}), cx);
                }
            },
        );
        let activity = cx.new(|cx| task_activity::TaskActivityPane::new(service.clone(), cx));
        activity.update(cx, |view, cx| view.set_context(&snapshot, &task_id, cx));
        let computer_bubble = cx.new(|cx| task_activity::ComputerUseBubble::new(activity.clone(), cx));
        let activity_change = cx.subscribe(
            &activity,
            |view, _, _: &task_activity::ActivityEvent, cx| view.refresh(cx),
        );
        let composer_git = cx.new(|cx| {
            crate::composer_git::ComposerGitStrip::new(
                service.clone(),
                &snapshot,
                revision.as_deref(),
                &task_id,
                window,
                cx,
            )
        });
        let composer_git_change = cx.subscribe(
            &composer_git,
            |_, _, event: &crate::composer_git::ComposerGitEvent, cx| match event {
                crate::composer_git::ComposerGitEvent::OpenGit { task_id } => {
                    cx.emit(ChatEvent::Git(task_id.clone()))
                }
            },
        );
        let composer_footer = cx.new(|cx| {
            crate::composer_footer::ComposerFooterPane::new(
                service.clone(),
                &snapshot,
                &task_id,
                window,
                cx,
            )
        });
        let composer_footer_change = cx.subscribe(
            &composer_footer,
            |_, _, event: &crate::composer_footer::ComposerFooterEvent, cx| match event {
                crate::composer_footer::ComposerFooterEvent::SnapshotUpdated {
                    snapshot, ..
                } => cx.emit(ChatEvent::SnapshotUpdated(snapshot.clone())),
            },
        );
        composer_footer.update(cx, |footer, cx| {
            footer.set_context_messages(known_messages, cx)
        });
        let composer_context_bar = cx.new(|cx| {
            crate::composer_footer::ComposerContextBar::new(
                composer_footer.clone(), working_palette, cx,
            )
        });
        let companion = cx.new(|cx| {
            crate::collaboration::SubagentPane::new(service.clone(), snapshot.clone(), window, cx)
        });
        companion.update(cx, |pane, cx| {
            pane.set_compact_header(true, cx);
            pane.set_context(snapshot.clone(), &task_id, cx);
        });
        // The child list and its visor occupy separate parts of this pane.
        // Child selection and asynchronous transcript reads invalidate both.
        let companion_change = cx.observe(&companion, |_, _, cx| cx.notify());
        let details = cx.new(|cx| {
            crate::pane_details::PaneDetails::new(
                service.clone(),
                &snapshot,
                revision.as_deref(),
                &task_id,
                window,
                cx,
            )
        });
        let details_change = cx.subscribe(
            &details,
            |_, _, event: &crate::pane_details::PaneDetailsEvent, cx| match event {
                crate::pane_details::PaneDetailsEvent::OpenTask(id) => {
                    cx.emit(ChatEvent::OpenTask(id.clone()))
                }
                crate::pane_details::PaneDetailsEvent::DelegateTask(id) => {
                    cx.emit(ChatEvent::DelegateTask(id.clone()))
                }
            },
        );
        let approvals = cx.new(|cx| {
            crate::pane_approvals::PaneApprovals::new(
                service.clone(),
                &snapshot,
                task_id.clone(),
                window,
                cx,
            )
        });
        approvals.update(cx, |pane, cx| {
            pane.set_context(&snapshot, revision.as_deref(), task_id.clone(), cx)
        });
        let companion_resize = cx.new(|_| ResizableState::default());
        let companion_resize_change = cx.subscribe(&companion_resize, |pane, state, event, cx| {
            if matches!(event, ResizablePanelEvent::Resized) {
                if pane.companion_programmatic_resizes > 0 {
                    pane.companion_programmatic_resizes -= 1;
                    return;
                }
                if let Some(width) = state.read(cx).sizes().last() {
                    let width = *width / design::ui_px(1., pane.companion_scale);
                    if pane.pane_width_design >= COMPACT_DETAIL_BREAKPOINT
                        && width.is_finite()
                        && (260.0..=wide_detail_max_width(pane.pane_width_design)).contains(&width)
                    {
                        if (pane.companion_width - width).abs() >= 0.5 {
                            pane.companion_width = width;
                            pane.emit_secondary_sidebar_change(cx);
                            cx.notify();
                        }
                    }
                }
            }
        });
        let companion_context = (revision.clone(), task_id.clone());
        let weak_pane = cx.weak_entity();
        cx.spawn(async move |_, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_secs(65))
                    .await;
                let alive = weak_pane
                    .update(cx, |pane, cx| {
                        pane.composer_footer
                            .update(cx, |footer, cx| footer.tick_usage(cx));
                    })
                    .is_ok();
                if !alive {
                    break;
                }
            }
        })
        .detach();
        Self {
            dock_group: None,
            #[cfg(feature = "performance-profiling")]
            performance_fixture: false,
            decoration_bounds,
            service: Some(service),
            snapshot,
            composed_palette: None,
            revision,
            selected: Some(task_id),
            composer,
            composer_git,
            _composer_git_change: composer_git_change,
            composer_footer,
            composer_context_bar,
            _composer_footer_change: composer_footer_change,
            voice,
            _voice_change: voice_change,
            _details_change: details_change,
            slash,
            _slash_change: slash_change,
            slash_notice: None,
            slash_effect: None,
            project_picker,
            _project_picker_change: project_picker_change,
            error: None,
            busy: false,
            starting_turn: false,
            activity_rows: HashMap::new(),
            activity_projection_source: None,
            grouped_rows: HashMap::new(),
            message_attachment_rows: HashMap::new(),
            message_action_focus: HashMap::new(),
            streaming_bodies: HashMap::new(),
            streaming_bodies_initialized: false,
            transcript_context_unchanged: false,
            deferred_publication: None,
            activity_refresh_pending: false,
            #[cfg(test)]
            activity_context_refreshes: 0,
            task_actions: None,
            task_actions_subscription: None,
            fork_dialog: None,
            fork_subscription: None,
            pending_fork_open: None,
            refreshing: false,
            clear_sent: None,
            transcripts,
            dock_visible: true,
            answers: HashMap::new(),
            form_choices: HashMap::new(),
            queue_edit: None,
            queue_editor,
            typography: None,
            transcript_block_spacing: None,
            composer_scroll_distance: gpui_kit::px(0.),
            bottom_jump: None,
            bottom_button_exit: None,
            chat_family: ".SystemUIFont".into(),
            focus: cx.focus_handle(),
            _queue_change: subscription,
            _composer_change: composer_change,
            draft_touched: false,
            attachments,
            _attachment_change: attachment_change,
            activity,
            computer_bubble,
            working_bubble,
            _working_bubble_change: working_bubble_change,
            _activity_change: activity_change,
            companion,
            companion_visible: true,
            companion_saved_visible: true,
            companion_programmatic_resizes: 0,
            pane_width_design: 0.0,
            pane_height_design: 0.0,
            compact_detail: false,
            detail_motion: Default::default(),
            detail_animation_frame: false,
            detail_transcript_cache: None,
            transcript_viewport: None,
            transcript_preparation_key: None,
            response_disclosure_generation: 0,
            detail_tab: crate::companion_sidebar::DetailTab::Details,
            detail_tabs_motion: Default::default(),
            detail_focus: std::array::from_fn(|_| cx.focus_handle()),
            details,
            approvals,
            approval_popup_id: None,
            interaction_request_id: None,
            mcp_approval_supported: false,
            approval_popup_open: false,
            approval_focus: cx.focus_handle(),
            approval_details_expanded: false,
            response_details_expanded: HashSet::new(),
            companion_context,
            companion_width: reference_detail_width(),
            companion_scale: design::interface_scale(),
            companion_resize,
            _companion_resize_change: companion_resize_change,
            _companion_change: companion_change,
        }
    }
    #[cfg(feature = "performance-profiling")]
    pub(crate) fn performance_fixture(
        task_id: String, snapshot: Rc<Value>, window: &mut Window, cx: &mut Context<Self>,
    ) -> Self {
        let mut pane = Self::new(task_id.clone(),
            DesktopService::disconnected_for_ui_fixture(), snapshot, Some("synthetic-1".into()), window, cx);
        pane.performance_fixture = true;
        // The detached child facade can never perform IPC, while the parent
        // does not expose send, microphone or filesystem actions at all.
        pane.service = None;
        pane.companion_visible = false;
        if let Some(state) = pane.transcripts.get_mut(&task_id) {
            state.boundary = transcript::HistoryBoundary::Complete;
        }
        pane
    }
    fn is_performance_fixture(&self) -> bool {
        #[cfg(feature = "performance-profiling")]
        { self.performance_fixture }
        #[cfg(not(feature = "performance-profiling"))]
        { false }
    }
    #[cfg(feature = "performance-profiling")]
    pub(crate) fn advance_performance_stream(&mut self, text: &str, cx: &mut Context<Self>) {
        if !self.performance_fixture { return; }
        if let Some(body) = self.streaming_bodies.get("fixture-stream") {
            let message = json!({"id":"fixture-stream", "role":"assistant", "streamStatus":"streaming", "text":text});
            body.update(cx, |body, cx| { body.set_message(Rc::new(message), cx); });
        }
    }
    /// Keep native history requests current without invalidating an unchanged
    /// task context when another task publishes a backend revision.
    pub(super) fn advance_revision(&mut self, snapshot: Rc<Value>, revision: Option<String>, cx: &mut Context<Self>) {
        if let Some(pending) = &mut self.deferred_publication {
            let retired = std::mem::replace(&mut pending.snapshot, snapshot.clone());
            crate::retire_snapshot(retired, cx);
            pending.revision = revision.clone();
        }
        if let Some(key) = &mut self.transcript_preparation_key {
            key.advance_unaffected_snapshot(&self.snapshot, &snapshot);
        }
        self.companion.update(cx, |pane, _| pane.advance_unaffected_snapshot(snapshot.clone()));
        self.snapshot = snapshot;
        self.slash.update(cx, |slash, _| slash.advance_revision(revision.as_deref().unwrap_or("")));
        self.revision = revision;
        if self.error.as_deref().is_some_and(connection_recovery::transient) {
            self.error = None;
            cx.notify();
        }
    }
    pub fn show_approval_prompt(&mut self, approval_id: String, window: &mut Window, cx: &mut Context<Self>) {
        self.approval_popup_id = Some(approval_id);
        self.approval_details_expanded = false;
        self.approval_popup_open = true;
        window.focus(&self.approval_focus, cx);
        cx.notify();
    }
    fn pending_tool_approval(&self) -> Option<Value> {
        rows(&self.snapshot, "approvalRequests").into_iter()
            .filter(|r| field(r, "taskId") == self.task_id() && field(r, "status") == "pending" && r["input"].is_null())
            .min_by_key(|r| (self.approval_popup_id.as_ref().is_none_or(|id| *id != field(r,"id")), r["createdAt"].as_i64().unwrap_or_default()))
    }
    fn attachment_scope(snapshot: &Value, task_id: &str) -> AttachmentScope {
        let task = snapshot["tasks"]
            .as_array()
            .and_then(|tasks| tasks.iter().find(|task| field(task, "id") == task_id));
        AttachmentScope {
            host_id: task.map(|task| field(task, "hostId")).unwrap_or_default(),
            cwd: task.map(|task| field(task, "cwd")).unwrap_or_default(),
        }
    }
    /// Reads only the explicit unsent composer, never transcripts/attachments.
    pub fn draft(&self, cx: &App) -> String {
        let value = self.composer.read(cx).value().to_string();
        if self
            .clear_sent
            .as_ref()
            .is_some_and(|(_, sent)| *sent == value)
        {
            String::new()
        } else {
            value
        }
    }
    pub fn draft_was_edited(&self) -> bool {
        self.draft_touched
    }
    pub fn attachment_draft(
        &self,
        cx: &App,
    ) -> monitter_lib::experimental_desktop::NativeAttachmentDraft {
        self.attachments.read(cx).persistence_snapshot()
    }
    pub fn attachment_draft_was_edited(&self, cx: &App) -> bool {
        self.attachments.read(cx).touched()
    }
    pub fn restore_attachment_draft(
        &mut self,
        saved_task: &str,
        state: &monitter_lib::experimental_desktop::NativeAttachmentDraft,
        cx: &mut Context<Self>,
    ) -> bool {
        let current_task = self.task_id();
        let restored = self.attachments.update(cx, |attachments, cx| {
            attachments.restore_persistence_snapshot(&current_task, saved_task, state, cx)
        });
        if restored {
            cx.notify();
        }
        restored
    }
    /// Never replaces locally edited text and never triggers send/create.
    pub fn restore_draft(
        &mut self,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.draft_touched || !self.composer.read(cx).value().is_empty() {
            return false;
        }
        self.composer
            .update(cx, |input, cx| input.set_value(text.to_owned(), window, cx));
        let task_id = self.task_id();
        self.voice.update(cx, |voice, cx| voice.set_context(task_id, text.to_owned(), cx));
        self.slash.update(cx, |slash, cx| slash.set_input(text.to_owned(), cx));
        cx.notify();
        true
    }
    /// Resolve text at activation time: retained streaming row closures may
    /// have been prepared before the latest canonical body arrived.
    fn copy_message_text(&self, task_id: &str, message_id: &str, cx: &mut App) {
        if self.task_id() != task_id { return; }
        let Some(message) = self.transcripts.get(task_id).and_then(|state| {
            state.messages.iter().find(|message| message["id"].as_str() == Some(message_id))
        }) else { return; };
        let text = field(message, "text");
        let text = if message["role"] == "user" { split_operator_message(&text).1 } else { text };
        cx.write_to_clipboard(ClipboardItem::new_string(text));
    }
    pub fn task_id(&self) -> String {
        self.selected.clone().unwrap_or_default()
    }
    pub(crate) fn set_dock_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        if visible && !self.dock_visible {
            self.apply_deferred_publication(cx);
            // Install sources while the reveal effect is still hidden. Normal
            // reconciliation (including completion) updates rows before render,
            // so existing body entities may otherwise still hold old text.
            if let Some(state) = self.transcripts.get(&self.task_id()) {
                for message in state.messages.iter() {
                    if let Some(body) = self.streaming_bodies.get(&field(message, "id")) {
                        body.update(cx, |body, cx| body.set_message(message.clone(), cx));
                    }
                }
            }
            // Bodies first created by the reveal render must also start from
            // their full current source instead of animating the hidden backlog.
            self.streaming_bodies_initialized = false;
        }
        self.dock_visible = visible;
        for body in self.streaming_bodies.values() {
            body.update(cx, |body, cx| body.set_visible(visible, cx));
        }
    }
    /// Keep one latest prepared update while hidden. Backend collection and
    /// the shared canonical snapshot continue; no hidden list/body work runs.
    pub(super) fn publish_prepared(
        &mut self, snapshot: Rc<Value>, revision: Option<String>,
        records: Arc<Vec<Value>>,
        child_transcripts: monitter_gpui_desktop::native_publication::PreparedChildTranscripts,
        context_unchanged: bool, history: Option<Vec<Value>>, reset: bool,
        boundary: Option<Option<String>>, cx: &mut Context<Self>,
    ) {
        let update = DeferredChatPublication {
            task_id: self.task_id(),
            snapshot, revision, records, child_transcripts, context_unchanged,
            history, reset, boundary,
        };
        if self.dock_visible {
            self.install_publication(update, cx);
            return;
        }
        // Rebind source owners without refreshing hidden children. A metadata
        // change must leave preparation invalidated through later body updates.
        if reset || !context_unchanged {
            self.transcript_preparation_key = None;
        } else if let Some(key) = &mut self.transcript_preparation_key {
            key.advance_unaffected_snapshot(&self.snapshot, &update.snapshot);
        }
        self.companion.update(cx, |pane, _| pane.advance_unaffected_snapshot(update.snapshot.clone()));
        let retired = std::mem::replace(&mut self.snapshot, update.snapshot.clone());
        crate::retire_snapshot(retired, cx);
        self.revision = update.revision.clone();
        self.slash.update(cx, |slash, _| slash.advance_revision(self.revision.as_deref().unwrap_or("")));
        self.activity_refresh_pending = false;
        self.transcript_context_unchanged = false;
        if self.deferred_publication.as_ref().is_some_and(|pending| pending.task_id != update.task_id) {
            if let Some(retired) = self.deferred_publication.take() {
                crate::retire_snapshot(retired.snapshot, cx);
                cx.background_executor().spawn(async move { drop(retired.history) }).detach();
            }
        }
        if let Some(pending) = &mut self.deferred_publication {
            let retired = std::mem::replace(&mut pending.snapshot, update.snapshot);
            crate::retire_snapshot(retired, cx);
            pending.revision = update.revision;
            pending.records = update.records;
            pending.child_transcripts = update.child_transcripts;
            pending.context_unchanged &= update.context_unchanged;
            pending.reset |= update.reset;
            if update.reset {
                pending.boundary = None;
                if let Some(retired) = pending.history.take() {
                    cx.background_executor().spawn(async move { drop(retired) }).detach();
                }
            }
            if update.boundary.is_some() { pending.boundary = update.boundary; }
            if let Some(history) = update.history {
                if let Some(retired) = pending.history.replace(history) {
                    cx.background_executor().spawn(async move { drop(retired) }).detach();
                }
            }
        } else {
            self.deferred_publication = Some(update);
            // Invalidate the retained view once so its first revealed draw
            // executes the drain rather than replaying an older cached scene.
            // Subsequent hidden chunks only replace the pending payload.
            cx.notify();
        }
    }
    pub(super) fn apply_deferred_publication(&mut self, cx: &mut Context<Self>) {
        if let Some(update) = self.deferred_publication.take() {
            self.install_publication(update, cx);
        }
    }
    fn install_publication(&mut self, update: DeferredChatPublication, cx: &mut Context<Self>) {
        let DeferredChatPublication { task_id: source_task, snapshot, revision, records, child_transcripts,
            context_unchanged, history, reset, boundary } = update;
        let task_id = self.task_id();
        if source_task != task_id {
            crate::retire_snapshot(snapshot, cx);
            cx.background_executor().spawn(async move { drop(history) }).detach();
            return;
        }
        // Production resets always include prepared history. Keep the helper
        // safe for a sync-only reset by deriving this task's authoritative rows.
        let history = history.or_else(|| reset.then(|| {
            snapshot["messages"].as_array().into_iter().flatten()
                .filter(|row| row["taskId"].as_str() == Some(task_id.as_str())).cloned().collect()
        }));
        // A later reset clears accepted page boundaries. A later removal also
        // prevents an old cursor being restored after authoritative history.
        let boundary = boundary.filter(|before| before.as_ref().is_none_or(|id| {
            snapshot["messages"].as_array().is_some_and(|rows| rows.iter().any(|row| {
                row["id"].as_str() == Some(id) && row["taskId"].as_str() == Some(task_id.as_str())
            }))
        }));
        self.sync_prepared(snapshot, revision, records, child_transcripts, context_unchanged, cx);
        if let Some(history) = history { self.reconcile_history(history, reset, cx); }
        if let Some(boundary) = boundary { self.restore_history_boundary(boundary, cx); }
    }
    pub fn sync_prepared(
        &mut self, snapshot: Rc<Value>, revision: Option<String>,
        companion_records: std::sync::Arc<Vec<Value>>,
        companion_transcripts: monitter_gpui_desktop::native_publication::PreparedChildTranscripts, transcript_context_unchanged: bool, cx: &mut Context<Self>,
    ) {
        let previous = self.snapshot.clone();
        self.sync_with_companion(snapshot, revision, Some((companion_records, companion_transcripts)), transcript_context_unchanged, cx);
        self.transcript_context_unchanged = transcript_context_unchanged;
        if transcript_context_unchanged {
            if let Some(key) = &mut self.transcript_preparation_key {
                key.advance_unaffected_snapshot(&previous, &self.snapshot);
            }
        }
    }
    pub fn sync(&mut self, snapshot: Rc<Value>, revision: Option<String>, cx: &mut Context<Self>) {
        self.transcript_context_unchanged = false;
        self.sync_with_companion(snapshot, revision, None, false, cx);
    }
    fn refresh_activity_context(&mut self, cx: &mut Context<Self>) {
        self.activity_refresh_pending = false;
        #[cfg(test)]
        { self.activity_context_refreshes += 1; }
        self.activity.update(cx, |view, cx| {
            view.set_context(&self.snapshot, &self.task_id(), cx)
        });
    }
    fn defer_activity_refresh(&mut self, cx: &mut Context<Self>) {
        if self.activity_refresh_pending { return; }
        self.activity_refresh_pending = true;
        let owner = cx.weak_entity();
        // The publication installs history synchronously after context. Its
        // exact body-only proof may cancel this work; a sync-only caller must
        // still refresh activity from the latest authoritative snapshot.
        cx.defer(move |cx| {
            let _ = owner.update(cx, |pane, cx| {
                if pane.activity_refresh_pending { pane.refresh_activity_context(cx); }
            });
        });
    }
    fn sync_with_companion(
        &mut self, snapshot: Rc<Value>, revision: Option<String>,
        companion_records: Option<(std::sync::Arc<Vec<Value>>, monitter_gpui_desktop::native_publication::PreparedChildTranscripts)>, transcript_context_unchanged: bool, cx: &mut Context<Self>,
    ) {
        if self.error.as_deref().is_some_and(connection_recovery::transient) {
            self.error = None;
        }
        self.slash.update(cx, |slash, cx| {
            slash.set_context(
                &snapshot,
                revision.as_deref().unwrap_or(""),
                self.task_id(),
                cx,
            )
        });
        let context = (revision.clone(), self.task_id());
        if self.companion_context != context || companion_records.is_some() {
            self.companion.update(cx, |pane, cx| {
                if let Some((items, transcripts)) = companion_records {
                    pane.set_prepared_context(snapshot.clone(), &context.1, items, transcripts, cx);
                } else {
                    pane.set_context(snapshot.clone(), &context.1, cx);
                }
            });
            self.details.update(cx, |pane, cx| {
                pane.set_context(&snapshot, revision.as_deref(), &context.1, cx)
            });
            self.approvals.update(cx, |pane, cx| {
                pane.set_context(&snapshot, revision.as_deref(), context.1.clone(), cx)
            });
            self.companion_context = context;
        }
        self.composer_git.update(cx, |strip, cx| {
            strip.set_context(
                &snapshot,
                revision.as_deref(),
                self.selected.as_deref().unwrap_or_default(),
                cx,
            )
        });
        self.composer_footer.update(cx, |footer, cx| {
            footer.set_snapshot(&snapshot, self.selected.as_deref().unwrap_or_default(), cx)
        });
        let scope = Self::attachment_scope(&snapshot, &self.task_id());
        self.attachments
            .update(cx, |attachments, cx| attachments.set_scope(scope, cx));

        let pending: HashSet<_> = rows(&snapshot, "approvalRequests")
            .into_iter()
            .filter(|request| field(request, "status") == "pending")
            .map(|request| field(&request, "id"))
            .collect();
        self.answers.retain(|(id, _), _| pending.contains(id));
        self.form_choices.retain(|(id, _), _| pending.contains(id));
        self.snapshot = snapshot;
        self.revision = revision;
        if transcript_context_unchanged { self.defer_activity_refresh(cx); }
        else { self.refresh_activity_context(cx); }
        // Open only after the canonical revisioned snapshot contains the fork.
        // A mutation response cannot replace a newer snapshot journal entry.
        if let Some(id) = self.pending_fork_open.as_ref()
            && rows(&self.snapshot, "tasks")
                .iter()
                .any(|task| field(task, "id") == *id)
        {
            let id = self.pending_fork_open.take().unwrap();
            cx.emit(ChatEvent::OpenTask(id));
        }
        if self.project_picker.read(cx).is_open() {
            let task_id = self.task_id();
            let palette = self.composed_palette.unwrap_or_else(|| {
                design::palette(&self.snapshot["settings"], WindowAppearance::Dark)
            });
            self.project_picker.update(cx, |picker, cx| {
                picker.set_context(&self.snapshot, task_id, palette, cx)
            });
        }
        self.refreshing = false;
        if !transcript_context_unchanged && let Some(transcript) = self.transcripts.get(&self.task_id()) {
            let messages = transcript.messages.as_ref().clone();
            self.composer_footer
                .update(cx, |footer, cx| footer.set_context_messages(messages, cx));
        }
        cx.notify();
    }
    pub(super) fn can_trim_hidden_history(&self) -> bool {
        !self.dock_visible && self.transcripts.get(&self.task_id()).is_some_and(TranscriptState::can_trim_inactive)
    }
    pub(super) fn mark_hidden_page_eviction(&mut self) {
        let task_id = self.task_id();
        if let Some(state) = self.transcripts.get_mut(&task_id) {
            state.boundary = transcript::HistoryBoundary::Unknown;
            state.history_top_reached = false;
        }
        self.detail_transcript_cache = None;
        self.transcript_viewport = None;
        self.transcript_preparation_key = None;
        self.companion_context = (None, String::new());
        self.activity_projection_source = None;
    }
    pub(super) fn trim_hidden_history(&mut self, keep: usize) {
        let task_id = self.task_id();
        if self.transcripts.get_mut(&task_id).is_some_and(|state| state.trim_inactive_tail(keep)) {
            // Retained render closures and disclosure/image entities must not
            // keep the evicted payloads alive while the pane stays hidden.
            self.detail_transcript_cache = None;
        self.transcript_viewport = None;
        self.transcript_preparation_key = None;
            self.companion_context = (None, String::new());
            self.activity_projection_source = None;
            self.prune_hidden_entities();
        }
    }
    fn prune_hidden_entities(&mut self) {
        let task_id = self.task_id();
        let state = &self.transcripts[&task_id];
        let event_ids = transcript::visible_projection_event_ids(&state.projection);
        let group_ids: HashSet<_> = state.projection.iter().map(transcript::ConversationItem::stable_key).collect();
        let message_ids: HashSet<_> = state.messages.iter().map(|message| field(message, "id")).collect();
        self.activity_rows.retain(|id, _| event_ids.contains(id));
        self.grouped_rows.retain(|id, _| group_ids.contains(id));
        self.message_attachment_rows.retain(|id, _| message_ids.contains(id));
        self.message_action_focus.retain(|id, _| message_ids.contains(id));
        self.streaming_bodies.retain(|id, _| message_ids.contains(id));
    }
    /// Authoritative cache integration: pass all known task messages, including
    /// loaded older pages. Never use this with a bounded compact snapshot tail.
    pub fn reconcile_history(&mut self, messages: Vec<Value>, reset: bool, cx: &mut Context<Self>) {
        let task_id = self.task_id();
        let context_unchanged = std::mem::take(&mut self.transcript_context_unchanged);
        if context_unchanged && !reset {
            let state = self.transcripts.entry(task_id.clone()).or_default();
            let previous_rows = state.rows.clone();
            if let Some((changed, indices)) = state.reconcile_stream_bodies(
                &messages, |id| self.streaming_bodies.contains_key(id),
            ) {
                self.activity_refresh_pending = false;
                if let Some(key) = &mut self.transcript_preparation_key {
                    key.advance_stream_rows(&previous_rows, &state.rows);
                }
                for message in changed {
                    self.streaming_bodies[&field(&message, "id")].update(cx, |body, cx| body.set_message(message, cx));
                }
                let remeasured = !indices.is_empty();
                for index in indices { state.list.remeasure_items(index..index + 1); }
                if remeasured { self.invalidate_transcript_list(cx); }
                // The publication already owns canonical payloads. Dropping its
                // duplicate full-history transport Vec is background work.
                cx.background_executor().spawn(async move { drop(messages); }).detach();
                cx.notify();
                return;
            }
        }
        // Activity follows canonical current-turn facts even when the reader
        // holds an older transcript off-tail. Only a successful body proof can
        // omit this refresh; completion/new users/metadata/order all drain it.
        if self.activity_refresh_pending { self.refresh_activity_context(cx); }
        let events = rows(&self.snapshot, "events")
            .into_iter()
            .filter(|event| {
                field(event, "taskId") == task_id
                    && visible_chat_activity(event, &self.snapshot["settings"])
            })
            .collect();
        let state = self.transcripts.entry(task_id).or_default();
        state.reconcile(messages, reset);
        state.reconcile_activities(events, reset);
        let known_messages = state.messages.as_ref().clone();
        self.composer_footer.update(cx, |footer, cx| {
            footer.set_context_messages(known_messages, cx)
        });
        if reset {
            if self.streaming_bodies.is_empty() { self.streaming_bodies_initialized = false; }
            self.activity_rows.clear();
            self.grouped_rows.clear();
            self.activity_projection_source = None;
        }
        if self.can_trim_hidden_history() {
            self.prune_hidden_entities();
        }
        cx.notify();
    }
    /// Restore only the root's revision/order-validated page boundary after
    /// a coalesced reset. This never merges payloads or changes the viewport.
    pub(super) fn restore_history_boundary(&mut self, next_before: Option<String>, cx: &mut Context<Self>) {
        let state = self.transcripts.entry(self.task_id()).or_default();
        let unchanged = match (&state.boundary, next_before.as_deref()) {
            (transcript::HistoryBoundary::Complete, None) => true,
            (transcript::HistoryBoundary::More(old), Some(next)) => old == next,
            _ => false,
        };
        if unchanged { return; }
        state.boundary = next_before.map_or(transcript::HistoryBoundary::Complete, transcript::HistoryBoundary::More);
        cx.notify();
    }
    pub(super) fn accept_history_page(
        &mut self,
        result: Result<Value, String>,
        cx: &mut Context<Self>,
    ) {
        let state = self.transcripts.entry(self.task_id()).or_default();
        state.loading = false;
        match result {
            Ok(page) => state.merge_page(&page),
            Err(error) => state.error = Some(error),
        }
        let known_messages = state.messages.as_ref().clone();
        self.composer_footer.update(cx, |footer, cx| {
            footer.set_context_messages(known_messages, cx)
        });
        cx.notify();
    }
    fn apply_chat_typography(&mut self, cx: &App) -> (SharedString, f32, FontWeight, f32) {
        let preferences = &self.snapshot["settings"];
        let size = preferences["chatFontSize"]
            .as_f64()
            .unwrap_or(design::CHAT_FONT_SIZE as f64)
            .clamp(8.0, 32.0) as f32;
        let weight = preferences["chatFontWeight"]
            .as_f64()
            .unwrap_or(400.0)
            .clamp(300.0, 700.0) as f32;
        let height = preferences["chatLineHeight"]
            .as_f64()
            .unwrap_or(1.65)
            .clamp(1.0, 2.5) as f32;
        let requested = field(preferences, "chatFont");
        let signature = format!(
            "{requested}:{size}:{weight}:{height}:{}",
            design::interface_scale()
        );
        if self.typography.as_ref() != Some(&signature) {
            let families = cx.text_system().all_font_names();
            self.chat_family = if !requested.is_empty() && families.contains(&requested) {
                requested.into()
            } else if families.iter().any(|family| family == "IBM Plex Sans") {
                "IBM Plex Sans".into()
            } else {
                ".SystemUIFont".into()
            };
            for state in self.transcripts.values() {
                state.list.remeasure();
            }
            self.typography = Some(signature);
        }
        (
            self.chat_family.clone(),
            size,
            FontWeight(weight),
            size * height,
        )
    }
    fn request_settings(&mut self, cx: &mut Context<Self>) {
        cx.emit(ChatEvent::Settings(self.task_id()));
    }
    fn open_message_fork(
        &mut self,
        source_task_id: String,
        through_message_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.task_id() != source_task_id || self.fork_dialog.is_some() {
            return;
        }
        // Recheck the live message, rather than relying on a captured row's status.
        let message = self.transcripts.get(&source_task_id).and_then(|state| {
            state
                .messages
                .iter()
                .find(|message| field(message, "id") == through_message_id)
        });
        if message.is_none_or(|message| {
            !["user", "assistant"].contains(&field(message, "role").as_str())
                || field(message, "streamStatus") == "streaming"
        }) {
            return;
        }
        let Some(service) = self.service.clone() else {
            return;
        };
        let snapshot = self.snapshot.clone();
        let palette = self
            .composed_palette
            .unwrap_or_else(|| design::palette(&snapshot["settings"], window.appearance()));
        let dialog = cx.new(|cx| {
            crate::fork_dialog::ForkDialogPane::new(
                service,
                snapshot,
                source_task_id,
                through_message_id,
                palette,
                window,
                cx,
            )
        });
        self.fork_subscription =
            Some(
                cx.subscribe_in(&dialog, window, |pane, _, event, window, cx| {
                    match event {
                        crate::fork_dialog::ForkDialogEvent::Created(task) => {
                            let id = field(task, "id");
                            if id.is_empty() {
                                pane.error =
                                    Some("The fork was created without a returned chat ID.".into());
                            } else {
                                pane.pending_fork_open = Some(id);
                                pane.refresh(cx);
                            }
                        }
                        crate::fork_dialog::ForkDialogEvent::Cancelled => {}
                    }
                    window.close_dialog(cx);
                    pane.fork_dialog = None;
                    pane.fork_subscription = None;
                    cx.notify();
                }),
            );
        self.fork_dialog = Some(dialog.clone());
        window.open_dialog(cx, move |modal, _, _| {
            let cancel = dialog.clone();
            modal
                .title("Fork chat")
                .width(px(480.))
                .overlay_closable(false)
                .close_button(false)
                .keyboard(true)
                .on_cancel(move |_, _, cx| {
                    cancel.update(cx, |pane, cx| pane.request_cancel(cx));
                    false
                })
                .child(dialog.clone())
        });
    }

    fn open_task_action(&mut self, archive: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.task_actions.as_ref().is_some_and(|pane| pane.read(cx).is_busy()) { return; }
        let id = self.task_id();
        let Some(task) = rows(&self.snapshot, "tasks").into_iter().find(|task| field(task, "id") == id) else { return; };
        let Some(service) = self.service.clone() else { return; };
        let title = field(&task, "title");
        let palette = self.composed_palette.unwrap_or_else(|| design::palette(&self.snapshot["settings"], window.appearance()));
        let action = cx.new(|cx| crate::chat_task_actions::ChatTaskActionsPane::new(service, id, &title, palette, window, cx));
        self.task_actions_subscription = Some(cx.subscribe_in(&action, window, move |pane, _, event, window, cx| {
            use crate::chat_task_actions::ChatTaskActionsEvent;
            match event {
                ChatTaskActionsEvent::Failed { task_id, error } => {
                    if pane.task_id() == *task_id { pane.error = Some(error.clone()); }
                    cx.notify();
                    return;
                }
                ChatTaskActionsEvent::Changed { task_id, archived } => {
                    if *archived { cx.emit(ChatEvent::Archived(task_id.clone())); }
                    else { cx.emit(ChatEvent::Refresh); }
                }
                ChatTaskActionsEvent::Cancelled => {}
            }
            if !archive { window.close_dialog(cx); }
            pane.task_actions = None;
            pane.task_actions_subscription = None;
            cx.notify();
        }));
        self.task_actions = Some(action.clone());
        if archive { action.update(cx, |pane, cx| pane.archive(cx)); }
        else {
            window.open_dialog(cx, move |dialog, _, _| {
                let cancel = action.clone();
                dialog.title("Rename chat").width(px(460.)).overlay_closable(false).close_button(false)
                    .on_cancel(move |_, _, cx| { cancel.update(cx, |pane, cx| pane.request_cancel(cx)); false })
                    .child(action.clone())
            });
            if let Some(action) = &self.task_actions {
                action.update(cx, |pane, cx| pane.focus_title(window, cx));
            }
        }
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        cx.emit(ChatEvent::Refresh);
    }
    pub(super) fn command(&mut self, command: &'static str, args: Value, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let Some(service) = self.service.clone() else {
            return;
        };
        let resolved_request = matches!(command, "resolve_input" | "resolve_approval" | "resolve_mcp_approval")
            .then(|| field(&args, "approvalId"));
        self.busy = true;
        self.composer_footer
            .update(cx, |footer, cx| footer.set_busy(true, cx));
        self.starting_turn = command == "send_message";
        self.error = None;
        // Compare acknowledgement with the original draft, including // escapes.
        let sent = (command == "send_message").then(|| {
            (
                field(&args, "taskId"),
                self.composer.read(cx).value().to_string(),
            )
        });
        let sent_attachments = (command == "send_message").then(|| {
            args["attachmentIds"]
                .as_array()
                .map(|ids| {
                    ids.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        });
        // Keep only IDs needed to distinguish new acknowledgements. Retaining
        // another full snapshot/transcript for every command is unnecessary.
        let known_messages = self.transcripts.get(&field(&args, "taskId"))
            .map(|transcript| transcript.messages.iter().map(|message| json!({"id":message["id"]})).collect::<Vec<_>>())
            .unwrap_or_default();
        let before = json!({
            "messages": known_messages,
            "queuedMessages": self.snapshot["queuedMessages"].as_array().into_iter().flatten()
                .map(|message| json!({"id":message["id"]})).collect::<Vec<_>>(),
            "recoveryStartedAt": chrono::Utc::now().timestamp_millis(),
        });
        let task = cx
            .background_executor()
            .spawn(async move {
                let result = service.invoke(command, args.clone());
                let Err(error) = &result else { return result; };
                if !connection_recovery::transient(error) { return result; }
                // Read only: a lost response may follow a fully executed mutation.
                let mut connected = false;
                for delay in [0, 150, 450] {
                    if delay != 0 { std::thread::sleep(Duration::from_millis(delay)); }
                    if let Ok(mut value) = service.invoke("get_ui_snapshot", json!({})) {
                        connected = true;
                        if command == "send_message" {
                            if let Ok(page) = service.read_task_messages(&field(&args, "taskId"), None, Some(100)) {
                                value["snapshot"]["messages"] = json!(page.messages);
                            }
                        }
                        if connection_recovery::confirmed(command, &args, &before, &value["snapshot"]) {
                            return Ok(value);
                        }
                    }
                }
                Err(if connected {
                    "Reconnected, but could not confirm the change. Check the chat before trying again. Your draft is kept."
                } else {
                    "Could not reconnect. Your draft is kept; check the chat after reconnecting before trying again."
                }.into())
            });
        cx.spawn(async move |pane, cx| {
            let result = task.await;
            let _ = pane.update(cx, |pane, cx| {
                pane.busy = false;
                pane.composer_footer
                    .update(cx, |footer, cx| footer.set_busy(false, cx));
                pane.starting_turn = false;
                match result {
                    Ok(_) => {
                        if let Some(id) = resolved_request {
                            pane.answers.retain(|(request_id, _), _| request_id != &id);
                            pane.form_choices
                                .retain(|(request_id, _), _| request_id != &id);
                        }
                        if command == "edit_queued_message" {
                            pane.queue_edit = None;
                        }
                        if let Some(ids) = sent_attachments {
                            pane.attachments
                                .update(cx, |attachments, cx| attachments.acknowledge(&ids, cx));
                        }
                        pane.clear_sent = sent;
                        pane.refresh(cx);
                    }
                    Err(error) => {
                        pane.error = Some(error);
                        pane.refresh(cx);
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    /// ListState carries geometry, not entity invalidation. Programmatic
    /// scroll/remeasure must invalidate its retained native render owner.
    fn invalidate_transcript_list(&self, cx: &mut Context<Self>) {
        if let Some(list) = &self.detail_transcript_cache {
            list.update(cx, |_, cx| cx.notify());
        }
    }
    fn finish_bottom_jump(&mut self, cx: &mut Context<Self>) {
        self.bottom_jump = None;
        self.composer_scroll_distance = gpui_kit::px(0.);
        if let Some(state) = self.transcripts.get_mut(&self.task_id()) {
            state.release_live();
            state.list.set_follow_mode(FollowMode::Tail);
            state.list.scroll_to_end();
        }
        self.invalidate_transcript_list(cx);
        cx.notify();
    }
    fn back_to_bottom(&mut self, cx: &mut Context<Self>) {
        if !crate::native_motion::motion_allowed() {
            self.bottom_button_exit = None;
            self.finish_bottom_jump(cx);
            return;
        }
        if let Some(state) = self.transcripts.get_mut(&self.task_id()) {
            state.release_live();
            state.list.set_follow_mode(FollowMode::Normal);
        }
        let now = std::time::Instant::now();
        self.bottom_button_exit = Some(crate::composer_scroll::BottomButtonExit::new(now));
        self.bottom_jump = Some(crate::composer_scroll::BottomJump::new(now));
        cx.notify();
    }
    fn advance_bottom_jump(&mut self, cx: &mut Context<Self>) {
        let Some(jump) = &mut self.bottom_jump else { return; };
        let (ratio, done) = jump.step(std::time::Instant::now());
        if done { self.finish_bottom_jump(cx); return; }
        let height = self.decoration_bounds.working_anchor.get()
            .map(|bounds| bounds.size.height).unwrap_or(px(180.));
        let hidden = crate::composer_scroll::composer_slide(self.composer_scroll_distance, height, false);
        if let Some(state) = self.transcripts.get_mut(&self.task_id()) {
            let distance = crate::composer_scroll::remaining_tail_distance(
                state.list.max_offset_for_scrollbar().y,
                state.list.scroll_px_offset_for_scrollbar().y, hidden);
            let remaining = distance * ratio;
            state.list.scroll_by(distance - remaining);
            self.composer_scroll_distance = remaining;
        }
        self.invalidate_transcript_list(cx);
        cx.notify();
    }
    /// Programmatic editor writes emit no native change event. At the explicit
    /// send boundary, use the actual draft rather than a previous slash command.
    fn submit_current_slash_input(&mut self, cx: &mut Context<Self>) -> bool {
        let text = self.composer.read(cx).value().to_string();
        self.slash.update(cx, |slash, cx| {
            slash.sync_input(text, cx);
            slash.submit_typed_input(cx)
        })
    }
    fn send(&mut self, cx: &mut Context<Self>) {
        if self.is_performance_fixture() { return; }
        if self.busy {
            return;
        }
        self.voice.update(cx, |voice, cx| voice.cancel(cx));
        if self.submit_current_slash_input(cx) {
            return;
        }
        let id = self.task_id();
        let text = self.composer.read(cx).value().to_string();
        let attachment_ids = self.attachments.read(cx).ids();
        if self.attachments.read(cx).pending()
            || (text.trim().is_empty() && attachment_ids.is_empty())
        {
            return;
        }
        self.bottom_jump = None;
        self.composer_scroll_distance = gpui_kit::px(0.);
        if let Some(state) = self.transcripts.get_mut(&id) {
            state.list.set_follow_mode(FollowMode::Tail);
            state.release_live();
            state.list.scroll_to_end();
        }
        self.invalidate_transcript_list(cx);
        self.command(
            "send_message",
            json!({"taskId":id,"text":chat_slash::literal_prompt(&text),"attachmentIds":attachment_ids}),
            cx,
        );
    }
}
impl ChatPane {
    fn update_pane_width(
        &mut self,
        width_design: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !width_design.is_finite() || width_design <= 0.0 {
            return;
        }
        let compact = width_design < COMPACT_DETAIL_BREAKPOINT;
        let changed =
            (self.pane_width_design - width_design).abs() >= 1.0 || self.compact_detail != compact;
        if !changed {
            return;
        }
        if should_hide_detail_on_compact_entry(self.compact_detail, compact, self.companion_visible)
        {
            self.companion_visible = false;
        }
        let restore_wide_width = !compact && (self.pane_width_design <= 0.0 || self.compact_detail);
        self.pane_width_design = width_design;
        self.compact_detail = compact;
        if restore_wide_width && self.companion_resize.read(cx).sizes().len() == 2 {
            let width = design::ui_px(
                self.companion_width
                    .clamp(260.0, wide_detail_max_width(width_design)),
                self.companion_scale,
            );
            self.companion_programmatic_resizes += 1;
            self.companion_resize
                .update(cx, |state, cx| state.resize_panel(1, width, window, cx));
        }
        cx.notify();
    }

    pub(super) fn secondary_sidebar_state(&self) -> NativeChatPanePreferences {
        use crate::companion_sidebar::DetailTab;
        NativeChatPanePreferences {
            visible: self.companion_saved_visible,
            width: self.companion_width,
            tab: match self.detail_tab {
                DetailTab::Details => NativeDetailTab::Details,
                DetailTab::Timeline => NativeDetailTab::Timeline,
                DetailTab::Approvals => NativeDetailTab::Approvals,
                DetailTab::Subagents => NativeDetailTab::Subagents,
            },
        }
    }
    fn emit_secondary_sidebar_change(&self, cx: &mut Context<Self>) {
        cx.emit(ChatEvent::SecondarySidebarChanged {
            task_id: self.task_id(),
            state: self.secondary_sidebar_state(),
        });
    }
    pub(super) fn restore_secondary_sidebar(
        &mut self,
        saved: &NativeChatPanePreferences,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use crate::companion_sidebar::DetailTab;
        self.companion_saved_visible = saved.visible;
        self.companion_visible = saved.visible && !self.compact_detail;
        self.companion_width = saved.width;
        self.detail_tab = match saved.tab {
            NativeDetailTab::Details => DetailTab::Details,
            NativeDetailTab::Timeline => DetailTab::Timeline,
            NativeDetailTab::Approvals => DetailTab::Approvals,
            NativeDetailTab::Subagents => DetailTab::Subagents,
        };
        if !self.compact_detail
            && self.pane_width_design > 0.0
            && self.companion_resize.read(cx).sizes().len() == 2
        {
            let width = design::ui_px(
                saved
                    .width
                    .min(wide_detail_max_width(self.pane_width_design)),
                self.companion_scale,
            );
            self.companion_programmatic_resizes += 1;
            self.companion_resize
                .update(cx, |state, cx| state.resize_panel(1, width, window, cx));
        }
        cx.notify();
    }

    pub(super) fn toggle_companion(&mut self, cx: &mut Context<Self>) {
        self.companion_visible = !self.companion_visible;
        self.companion_saved_visible = self.companion_visible;
        self.emit_secondary_sidebar_change(cx);
        cx.notify();
    }

    pub(super) fn show_companion(&mut self, cx: &mut Context<Self>) {
        if !self.companion_visible {
            self.companion_visible = true;
            self.companion_saved_visible = true;
            self.emit_secondary_sidebar_change(cx);
            cx.notify();
        }
    }

    pub(super) fn set_composed_palette(
        &mut self,
        palette: design::Palette,
        cx: &mut Context<Self>,
    ) {
        if self.composed_palette != Some(palette) {
            self.composed_palette = Some(palette);
            cx.notify();
        }
    }

    pub(crate) fn deactivate_voice(&mut self, cx: &mut Context<Self>) {
        self.voice.update(cx, |voice, cx| voice.cancel(cx));
    }
}

type StableRowRenderer = Rc<std::cell::RefCell<dyn FnMut(usize, &mut Window, &mut App) -> AnyElement>>;
struct StableTranscriptList {
    state: ListState,
    rows: StableRowRenderer,
}
fn retain_transcript_list(
    retained: &mut Option<Entity<StableTranscriptList>>,
    state: ListState,
    rows: StableRowRenderer,
    cx: &mut App,
) {
    if let Some(cached) = retained.as_ref() {
        cached.update(cx, |list, cx| {
            list.state = state;
            list.rows = rows;
            cx.notify();
        });
    } else {
        *retained = Some(cx.new(|_| StableTranscriptList { state, rows }));
    }
}
#[cfg(all(test, feature = "visual-export"))]
#[path = "chat_tree_click_tests.rs"]
mod tree_click_tests;
impl Render for StableTranscriptList {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let rows = self.rows.clone();
        list(self.state.clone(), move |index, window, cx| rows.borrow_mut()(index, window, cx)).size_full()
    }
}

impl Render for ChatPane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Painting is the earliest authoritative reveal observation; the dock
        // visibility callback follows this draw. Install latest content now so
        // even the first revealed frame cannot display an old hidden snapshot.
        if !self.dock_visible { self.apply_deferred_publication(cx); }
        let button_flight = self.bottom_button_exit.as_ref().map(|exit| exit.sample(std::time::Instant::now()));
        if button_flight.is_some_and(|(_, finished)| finished) { self.bottom_button_exit = None; }
        let button_flight = button_flight.filter(|(_, finished)| !finished).map_or(0., |(progress, _)| progress);
        if self.bottom_jump.is_some() || self.bottom_button_exit.is_some() {
            let owner = cx.weak_entity();
            window.on_next_frame(move |_, cx| {
                let _ = owner.update(cx, |pane, cx| {
                    pane.advance_bottom_jump(cx);
                    if pane.bottom_button_exit.is_some() { cx.notify(); }
                });
            });
        }
        self.detail_animation_frame = false;
        let palette = self
            .composed_palette
            .unwrap_or_else(|| settings::palette(&self.snapshot["settings"], window.appearance()));
        let transcript_gap = transcript_block_gap(&self.snapshot["settings"]);
        if self.transcript_block_spacing != Some(transcript_gap) {
            // Separators belong to measured virtual rows. Density changes must
            // invalidate cached heights even when transcript content is equal.
            for state in self.transcripts.values() {
                state.list.remeasure();
            }
            self.transcript_block_spacing = Some(transcript_gap);
        }
        let interface_font_ratio = self.snapshot["settings"]["interfaceFontSize"]
            .as_f64()
            .unwrap_or(14.)
            .clamp(8., 32.) as f32
            / 14.;
        // Use the validated interface face, independently of the chat body font.
        let interface_family = cx.theme().font_family.clone();
        self.attachments.update(cx, |attachments, cx| {
            attachments.set_appearance(palette, interface_font_ratio, cx);
        });
        self.composer_git.update(cx, |strip, cx| {
            strip.set_composed_palette(palette, cx);
        });
        self.slash.update(cx, |slash, cx| {
            slash.set_composed_palette(Some(palette), cx)
        });
        let voice_engine = field(&self.snapshot["settings"], "voiceTranscriptionEngine");
        self.voice.update(cx, |voice, cx| {
            voice.set_engine(&voice_engine, cx);
        });
        let voice_status = self.voice.read(cx).status();
        let voice_controls = self
            .voice
            .update(cx, |voice, cx| voice.render_controls(palette, window, cx));
        if let Some(effect) = self.slash_effect.take() {
            if effect == "openModel" {
                self.composer_footer.update(cx, |footer, cx| {
                    footer.open_model_picker(window, cx, Some(palette))
                });
            } else if effect == "refreshGoal" {
                self.activity
                    .update(cx, |activity, cx| activity.refresh(cx));
            }
        }
        let (chat_family, chat_size, chat_weight, chat_line_height) =
            self.apply_chat_typography(cx);
        let composer_chat_family = chat_family.clone();
        if let Some((task_id, sent)) = self.clear_sent.take() {
            if self.composer.read(cx).value().as_ref() == sent {
                self.composer
                    .update(cx, |input, cx| input.set_value("", window, cx));
                let voice_task = self.task_id();
                self.voice.update(cx, |voice, cx| voice.set_context(voice_task, String::new(), cx));
                self.slash.update(cx, |slash, cx| slash.set_input(String::new(), cx));
                // Programmatic set_value does not emit InputEvent::Change.
                // Persist the acknowledged clear explicitly, while keeping
                // a newer draft typed during the send untouched.
                cx.emit(ChatEvent::DraftChanged {
                    task_id,
                    text: String::new(),
                });
            }
        }
        let agents = rows(&self.snapshot, "agents");
        let tasks = rows(&self.snapshot, "tasks");
        let selected = tasks
            .iter()
            .find(|task| Some(field(task, "id")) == self.selected);
        let header = self.render_chat_header(selected, &agents, palette, window, cx);
        let mut content = div()
            .relative()
            .flex_1()
            .h_full()
            .flex()
            .flex_col()
            .min_w_0()
            .min_h_0()
            .overflow_hidden()
            .child({
                let geometry = self.decoration_bounds.chat_column.clone();
                canvas(move |bounds, _, _| geometry.set(Some(bounds)), |_, _, _, _| ())
                    .absolute().inset_0()
            });
        if let Some(error) = self.error.as_deref().and_then(connection_recovery::visible_error) {
            content = content.child(transcript_column(
                div()
                    .p_3()
                    .bg(design::raised_surface_color(0x462535))
                    .child(format!("Error: {error}")),
            ));
        }
        if self.service.is_none() && self.error.is_none() && !self.is_performance_fixture() {
            content = content.child(transcript_column(
                div().child("Opening isolated experimental backend…"),
            ));
        }
        // Composer and decoration invalidations can render this ancestor. They
        // must not traverse, prune, regroup or replace the transcript renderer.
        let preparation_key = TranscriptPreparationKey::capture(
            self, palette, interface_family.clone(), window.appearance(),
        );
        if self.transcript_preparation_key.as_ref() != Some(&preparation_key)
            || self.transcript_viewport.is_none()
        {
            #[cfg(feature = "performance-profiling")]
            preparation_key.record_invalidation(
                self.transcript_preparation_key.as_ref(), self.transcript_viewport.is_none(),
            );
            #[cfg(any(test, feature = "performance-profiling"))]
            TRANSCRIPT_PREPARATIONS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let mut history_error = None;
        if let Some(task) = selected {
            let task_id = field(task, "id");
            let state = self.transcripts.entry(field(task, "id")).or_default();
            let approvals: Vec<_> = rows(&self.snapshot, "approvalRequests")
                .into_iter()
                .filter(|request| {
                    field(request, "taskId") == task_id
                        && (request["resolvedAt"].is_number()
                            || matches!(field(request, "status").as_str(), "approved" | "denied"))
                })
                .map(|request| transcript::ApprovalRow {
                    id: field(&request, "id"),
                    task_id: field(&request, "taskId"),
                    status: field(&request, "status"),
                    created_at: request["createdAt"].as_i64().unwrap_or(0),
                    resolved_at: request["resolvedAt"].as_i64(),
                    rule_id: request["ruleId"].as_str().map(str::to_owned),
                    summary: field(&request, "summary"),
                    tool: field(&request, "tool"),
                })
                .collect();
            state.set_compress_tools(
                self.snapshot["settings"]["compressToolCalls"]
                    .as_bool()
                    .unwrap_or(false),
            );
            state.sync_projection(&approvals);
            let timeline_rows = state.projection.clone();
            #[cfg(any(test, feature = "performance-profiling"))]
            TRANSCRIPT_PREPARED_ROWS.fetch_add(timeline_rows.len() as u64, std::sync::atomic::Ordering::Relaxed);
            history_error = state.error.as_deref()
                .and_then(connection_recovery::visible_error).map(str::to_owned);
            let history_top_reached = state.history_top_reached
                && state.list.logical_scroll_top().item_ix == 0
                && state.list.is_scrolled_to_end() == Some(false);
            let more_history = !state.complete();
            let history_loading = state.loading;
            let refreshing = self.refreshing;
            let user_name = field(&self.snapshot["settings"], "userName");
            let list_state = state.list.clone();
            let task_id = field(task, "id");
            let scroll_pane = cx.entity().downgrade();
            let scroll_task_id = task_id.clone();
            // Native user-scroll events replace the permanently visible pager row.
            // Defer before touching the list: its callback holds a RefCell borrow.
            list_state.set_scroll_handler(move |event, _, cx| {
                if event.count == 0 {
                    return;
                }
                let near_top = event.is_scrolled && event.visible_range.start <= 2;
                let at_bottom = !event.is_scrolled;
                let reached_start = event.is_scrolled && event.visible_range.start == 0;
                let pane = scroll_pane.clone();
                let task_id = scroll_task_id.clone();
                cx.defer(move |cx| {
                    let _ = pane.update(cx, |pane, cx| {
                        if pane.selected.as_deref() != Some(task_id.as_str()) {
                            return;
                        }
                        let Some(state) = pane.transcripts.get_mut(&task_id) else {
                            return;
                        };
                        pane.bottom_jump = None;
                        // Viewport growth must not reattach tail-follow on its
                        // own. Only a user scroll reaching bottom or the jump
                        // button may resume it.
                        if !at_bottom { state.list.set_follow_mode(FollowMode::Normal); }
                        let height = pane.decoration_bounds.working_anchor.get()
                            .map(|bounds| bounds.size.height).unwrap_or(px(180.));
                        let hidden = crate::composer_scroll::composer_slide(
                            pane.composer_scroll_distance, height, !crate::native_motion::motion_allowed());
                        let distance = if at_bottom { gpui_kit::px(0.) } else {
                            crate::composer_scroll::remaining_tail_distance(
                                state.list.max_offset_for_scrollbar().y,
                                state.list.scroll_px_offset_for_scrollbar().y, hidden)
                        };
                        if pane.composer_scroll_distance != distance {
                            pane.composer_scroll_distance = distance;
                            cx.notify();
                        }
                        let at_start = reached_start
                            && state.list.logical_scroll_top().item_ix == 0
                            && state.list.is_scrolled_to_end() == Some(false);
                        if state.history_top_reached != at_start {
                            state.history_top_reached = at_start;
                            state.list.remeasure_items(0..1);
                            cx.notify();
                        }
                        if at_bottom && state.list.is_scrolled_to_end() != Some(false) {
                            if !state.list.is_following_tail() || state.has_pending_live() {
                                state.release_live();
                                state.list.set_follow_mode(FollowMode::Tail);
                                state.list.scroll_to_end();
                                cx.notify();
                            }
                        } else if near_top
                            && state.list.logical_scroll_top().item_ix <= 2
                            && state.list.is_scrolled_to_end() == Some(false)
                            && !state.loading
                            && !state.complete()
                            && state.error.is_none()
                        {
                            pane.load_earlier(cx);
                        }
                    });
                });
            });
            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
                .unwrap_or_default();
            let mut previous_day = None;
            let date_breaks: HashMap<usize, String> = timeline_rows
                .iter()
                .enumerate()
                .filter_map(|(index, item)| {
                    let at = item.at();
                    let day = crate::timestamps::local_datetime(at)?.date_naive();
                    if previous_day == Some(day) {
                        None
                    } else {
                        previous_day = Some(day);
                        Some((index, chat_date_label(at, now_ms)))
                    }
                })
                .collect();
            let task_snapshot = task.clone();
            let agents_snapshot = agents.clone();
            let visible_agent_ids: HashSet<String> = timeline_rows
                .iter()
                .filter_map(|item| match item {
                    transcript::ConversationItem::Message(message)
                        if field(message, "role") == "assistant" =>
                    {
                        let sender = field(message, "senderAgentId");
                        Some(if sender.is_empty() {
                            field(&task_snapshot, "agentId")
                        } else {
                            sender
                        })
                    }
                    _ => None,
                })
                .collect();
            let agent_images: HashMap<String, Arc<Image>> = agents_snapshot
                .iter()
                .filter(|agent| visible_agent_ids.contains(&field(agent, "id")))
                .filter_map(|agent| agent_avatar(agent).map(|image| (field(agent, "id"), image)))
                .collect();
            let sharing_primary = field(&self.snapshot["sharing"]["primary"], "name");
            let input_request_ids: HashSet<String> = rows(&self.snapshot, "approvalRequests")
                .into_iter()
                .filter(|request| {
                    field(request, "taskId") == task_id
                        && request.get("input").is_some_and(|input| !input.is_null())
                })
                .map(|request| field(&request, "id"))
                .collect();
            let approval_decisions: HashMap<String, String> =
                rows(&self.snapshot, "approvalRequests")
                    .into_iter()
                    .filter(|request| field(request, "taskId") == task_id)
                    .map(|request| (field(&request, "id"), field(&request, "decision")))
                    .collect();
            let visible_ids = transcript::visible_projection_event_ids(&timeline_rows);
            self.activity_rows.retain(|id, _| visible_ids.contains(id));
            let visible_groups: HashSet<String> = timeline_rows
                .iter()
                .filter_map(|item| match item {
                    transcript::ConversationItem::ReasoningGroup(_) => Some(item.stable_key()),
                    transcript::ConversationItem::ToolGroup(_) => Some(item.stable_key()),
                    transcript::ConversationItem::ProcessGroup { .. } => Some(item.stable_key()),
                    _ => None,
                })
                .collect();
            self.grouped_rows
                .retain(|id, _| visible_groups.contains(id));
            let running = field(task, "status") == "running";
            let compressed = self.snapshot["settings"]["compressToolCalls"].as_bool().unwrap_or(false);
            let activity_changed = self.activity_projection_source.as_ref().is_none_or(|(source, was_running, was_compressed)| {
                !Rc::ptr_eq(source, &timeline_rows) || *was_running != running || *was_compressed != compressed
            });
            if activity_changed {
                self.activity_projection_source = Some((timeline_rows.clone(), running, compressed));
            }
            // Decorative clocks and composer input must not deep-clone every
            // retained process tree. Reconcile only committed projection changes.
            if let Some(service) = self.service.clone().filter(|_| activity_changed) {
                for (index, item) in timeline_rows.iter().enumerate() {
                    let events: Vec<Rc<Value>> = match item {
                        transcript::ConversationItem::Activity(event) => vec![event.clone()],
                        transcript::ConversationItem::ReasoningGroup(events)
                        | transcript::ConversationItem::ToolGroup(events) => events.clone(),
                        transcript::ConversationItem::ProcessGroup { values, .. } => values.clone(),
                        _ => vec![],
                    };
                    for event in &events {
                        let id = field(event, "id");
                        if let Some(row) = self.activity_rows.get(&id) {
                            row.update(cx, |row, cx| {
                                row.set_list_item(list_state.clone(), index);
                                row.set_event(&task_id, event, cx);
                            });
                        } else {
                            let row = cx.new(|cx| {
                                let mut row = run_activity::RunActivityRow::new(
                                    service.clone(),
                                    task_id.clone(),
                                    event,
                                    window,
                                    cx,
                                );
                                row.set_list_item(list_state.clone(), index);
                                row
                            });
                            self.activity_rows.insert(id, row);
                        }
                    }
                    let key = item.stable_key();
                    let group = match item {
                        transcript::ConversationItem::ReasoningGroup(events) => {
                            Some((events, run_activity::GroupMode::Reasoning))
                        }
                        transcript::ConversationItem::ToolGroup(events) => {
                            Some((events, run_activity::GroupMode::Tool))
                        }
                        transcript::ConversationItem::ProcessGroup { values, .. } => {
                            Some((values, run_activity::GroupMode::Process))
                        }
                        _ => None,
                    };
                    if let Some((events, mode)) = group {
                        let running = field(task, "status") == "running";
                        let entity = if let Some(entity) = self.grouped_rows.get(&key) {
                            entity.clone()
                        } else {
                            let entity = cx.new(|_| match mode {
                                run_activity::GroupMode::Reasoning => {
                                    run_activity::RunActivityRow::for_reasoning(
                                        service.clone(),
                                        task_id.clone(),
                                        events.iter().map(|event| (**event).clone()).collect(),
                                        running,
                                    )
                                }
                                run_activity::GroupMode::Tool => {
                                    run_activity::RunActivityRow::for_tool(
                                        service.clone(),
                                        task_id.clone(),
                                        events.iter().map(|event| (**event).clone()).collect(),
                                        running,
                                        self.snapshot["settings"]["compressToolCalls"]
                                            .as_bool()
                                            .unwrap_or(false),
                                    )
                                }
                                _ => run_activity::RunActivityRow::for_process(
                                    service.clone(),
                                    task_id.clone(),
                                    events.iter().map(|event| (**event).clone()).collect(),
                                    vec![],
                                    running,
                                ),
                            });
                            self.grouped_rows.insert(key.clone(), entity.clone());
                            entity
                        };
                        let children: Vec<_> = events
                            .iter()
                            .filter_map(|event| {
                                let id = field(event, "id");
                                self.activity_rows.get(&id).cloned().map(|row| (id, row))
                            })
                            .collect();
                        entity.update(cx, |row, cx| {
                            row.set_list_item(list_state.clone(), index);
                            row.set_group(&task_id, events.iter().map(|event| (**event).clone()).collect(), running, self.snapshot["settings"]["compressToolCalls"].as_bool().unwrap_or(false), cx);
                            row.set_detail_rows(children, cx);
                            if let transcript::ConversationItem::ProcessGroup { groups, approvals, .. } = item {
                                let branches = groups.iter().map(|group| match group {
                                    transcript::NestedGroup::Reasoning(events) => run_activity::ProcessBranch {
                                        mode: run_activity::GroupMode::Reasoning,
                                        events: events.iter().map(|event| (**event).clone()).collect(),
                                    },
                                    transcript::NestedGroup::Tool(events) => run_activity::ProcessBranch {
                                        mode: run_activity::GroupMode::Tool,
                                        events: events.iter().map(|event| (**event).clone()).collect(),
                                    },
                                }).collect();
                                let approvals = approvals.iter().map(|approval| json!({
                                    "id": approval.id,
                                    "status": approval.status,
                                    "ruleId": approval.rule_id,
                                    "createdAt": approval.at(),
                                    "displayLabel": approval_history_label(
                                        approval,
                                        input_request_ids.contains(&approval.id),
                                        approval_decisions.get(&approval.id).map(String::as_str).unwrap_or_default(),
                                    ),
                                })).collect();
                                row.set_process_tree(branches, approvals, cx);
                            } else if let transcript::ConversationItem::ToolGroup(events) = item {
                                row.set_process_tree(vec![run_activity::ProcessBranch {
                                    mode: run_activity::GroupMode::Tool,
                                    events: events.iter().map(|event| (**event).clone()).collect(),
                                }], vec![], cx);
                            }
                        });
                    }
                }
            }
            let message_ids: HashSet<String> = timeline_rows
                .iter()
                .filter_map(|item| match item {
                    transcript::ConversationItem::Message(message)
                        if ["user", "assistant"].contains(&field(message, "role").as_str()) =>
                    {
                        Some(field(message, "id"))
                    }
                    _ => None,
                })
                .collect();
            self.response_details_expanded.retain(|id| message_ids.contains(id));
            self.message_action_focus
                .retain(|id, _| message_ids.contains(id));
            for id in &message_ids {
                self.message_action_focus
                    .entry(id.clone())
                    .or_insert_with(|| cx.focus_handle());
            }
            let attachment_ids: HashSet<String> = timeline_rows.iter().filter_map(|item| {
                let transcript::ConversationItem::Message(message) = item else { return None; };
                message["attachments"].as_array().filter(|items| !items.is_empty())
                    .map(|_| field(message, "id"))
            }).collect();
            self.message_attachment_rows.retain(|id, _| attachment_ids.contains(id));
            if let Some(service) = self.service.clone() {
                for (index, item) in timeline_rows.iter().enumerate() {
                    let transcript::ConversationItem::Message(message) = item else { continue; };
                    let Some(attachments) = message["attachments"].as_array().filter(|items| !items.is_empty()) else { continue; };
                    let entity = self.message_attachment_rows.entry(field(message, "id"))
                        .or_insert_with(|| cx.new(|cx| message_attachments::MessageAttachments::new(service.clone(), attachments, cx)));
                    entity.update(cx, |row, cx| {
                        row.set_attachments(attachments, cx);
                        row.set_appearance(palette, cx);
                        row.set_list_item(list_state.clone(), index);
                    });
                }
            }
            let message_attachment_rows = self.message_attachment_rows.clone();
            // Each live body owns its reveal timer. Character ticks invalidate
            // only its virtual row, never the full pane or backend snapshot.
            self.streaming_bodies.retain(|id, _| message_ids.contains(id));
            for (index, item) in timeline_rows.iter().enumerate() {
                let transcript::ConversationItem::Message(message) = item else { continue; };
                let id = field(message, "id");
                if field(message, "role") != "assistant" ||
                    (field(message, "streamStatus") != "streaming" && !self.streaming_bodies.contains_key(&id)) { continue; }
                let animate = self.streaming_bodies_initialized && self.dock_visible;
                let body = self.streaming_bodies.entry(id).or_insert_with(|| cx.new(|cx| {
                    crate::streaming_message::StreamingMessage::new(message.clone(), animate, cx)
                }));
                body.update(cx, |body, cx| {
                    body.set_list_item(list_state.clone(), index);
                    body.set_visible(self.dock_visible, cx);
                    body.set_message(message.clone(), cx);
                    body.set_palette(palette);
                });
            }
            self.streaming_bodies_initialized = true;
            let streaming_bodies = self.streaming_bodies.clone();
            let message_action_focus = self.message_action_focus.clone();
            let response_details_expanded = self.response_details_expanded.clone();
            let activities = self.activity_rows.clone();
            let grouped = self.grouped_rows.clone();
            let pane = cx.entity().downgrade();
            let message_settings = self.snapshot["settings"].clone();
            let message_appearance = window.appearance();
            let transcript_width = self.pane_width_design
                - if self.companion_visible && !self.compact_detail {
                    self.companion_width
                } else {
                    0.
                }
                - space::SECTION * 2.;
            let bubble_width = if transcript_width <= 420. { 1. } else { 0.9 };
            let interface_family = interface_family.clone();
            let row_renderer: StableRowRenderer = Rc::new(std::cell::RefCell::new(move |index: usize, _: &mut Window, cx: &mut App| {
                    let rendered = (|| {
                    // ListState can have a newer row count than this retained render closure.
                    let Some(timeline_row) = timeline_rows.get(index) else {
                        return div().into_any_element();
                    };
                    let date_break = date_breaks.get(&index).cloned();
                    let message = match timeline_row {
                        transcript::ConversationItem::Activity(event) => {
                            let row = activities
                                .get(&field(event, "id"))
                                .map(|row| div().w_full().child(row.clone()).into_any_element())
                                .unwrap_or_else(|| {
                                    div().child(field(event, "title")).into_any_element()
                                });
                            // List measures each top-level item as a root; keep
                            // its trailing separator in the measured height so
                            // the next virtualized entry starts after the gap.
                            let mut wrapped = div().w_full().pb(px(transcript_gap));
                            if let Some(label) = date_break.as_deref() {
                                wrapped = wrapped.child(date_separator(label, palette));
                            }
                            return transcript_column(wrapped.child(row));
                        }
                        transcript::ConversationItem::ReasoningGroup(_)
                        | transcript::ConversationItem::ToolGroup(_)
                        | transcript::ConversationItem::ProcessGroup { .. } => {
                            let row = grouped
                                .get(&timeline_row.stable_key())
                                .map(|row| div().w_full().child(row.clone()).into_any_element())
                                .unwrap_or_else(|| div().into_any_element());
                            let mut wrapped = div().w_full().pb(px(transcript_gap));
                            if let Some(label) = date_break.as_deref() {
                                wrapped = wrapped.child(date_separator(label, palette));
                            }
                            return transcript_column(wrapped.child(row));
                        }
                        transcript::ConversationItem::Approval(approval) => {
                            let mut row = div()
                                .w_full()
                                .px_3()
                                .py_2()
                                .rounded_md()
                                .bg(design::raised_surface_color(palette.panel))
                                .border_1()
                                .border_color(rgb(palette.line))
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .text_size(px(11.))
                                        .text_color(rgb(palette.ink))
                                        .child(approval_history_label(
                                            approval,
                                            input_request_ids.contains(&approval.id),
                                            approval_decisions
                                                .get(&approval.id)
                                                .map(String::as_str)
                                                .unwrap_or_default(),
                                        )),
                                );
                            if let Some(timestamp) =
                                crate::timestamps::local_datetime(approval.at())
                            {
                                row = row.child(
                                    div()
                                        .text_size(px(10.))
                                        .text_color(rgb(palette.muted))
                                        .child(timestamp.format("%-I:%M %P").to_string()),
                                );
                            }
                            let mut wrapped = div().w_full().pb(px(transcript_gap));
                            if let Some(label) = date_break.as_deref() {
                                wrapped = wrapped.child(date_separator(label, palette));
                            }
                            return transcript_column(wrapped.child(row));
                        }
                        transcript::ConversationItem::Message(message) => message,
                    };
                    if let Some(divider) = system_divider::render(message, &message_settings, palette) {
                        let mut wrapper = div().w_full().pb(px(transcript_gap));
                        if let Some(label) = date_break.as_deref() {
                            wrapper = wrapper.child(date_separator(label, palette));
                        }
                        return transcript_column(wrapper.child(divider));
                    }
                    let user = field(message, "role") == "user";
                    let system = field(message, "role") == "system";
                    let final_answer = field(message, "phase") == "final_answer";
                    let message_agent_id = field(message, "senderAgentId");
                    let message_agent = if !message_agent_id.is_empty() {
                        agents_snapshot
                            .iter()
                            .find(|agent| field(agent, "id") == message_agent_id)
                    } else if field(message, "role") == "assistant" {
                        agents_snapshot
                            .iter()
                            .find(|agent| field(agent, "id") == field(&task_snapshot, "agentId"))
                    } else {
                        None
                    };
                    let (operator_name, visible_text) = if user {
                        split_operator_message(&field(message, "text"))
                    } else {
                        (None, field(message, "text"))
                    };
                    let stream_presentation = message_stream_presentation(message, &visible_text);
                    let streaming_body = streaming_bodies.get(&field(message, "id")).cloned();
                    let participant_name = operator_name.as_deref().or_else(||
                        (user && !sharing_primary.is_empty()).then_some(sharing_primary.as_str()));
                    let participant_colour = participant_name.map(operator_colour);
                    let label = if user {
                        operator_name
                            .clone()
                            .filter(|name| !name.is_empty())
                            .or_else(|| {
                                (!sharing_primary.is_empty()).then(|| sharing_primary.clone())
                            })
                            .unwrap_or_else(|| {
                                if user_name.is_empty() {
                                    "You".into()
                                } else {
                                    user_name.clone()
                                }
                            })
                    } else if system {
                        "System".into()
                    } else if let Some(agent) = message_agent {
                        field(agent, "name")
                    } else if !message_agent_id.is_empty() {
                        "Agent".into()
                    } else {
                        let archived = field(&task_snapshot, "archivedAgentName");
                        if archived.is_empty() {
                            "Agent".into()
                        } else {
                            archived
                        }
                    };
                    let timestamp = message["createdAt"].as_i64().unwrap_or(0);
                    let mut metadata = div().w_full().min_w_0().flex().items_center().gap_2().mb_1()
                        .font_family(interface_family.clone())
                        .text_size(px(11. * interface_font_ratio))
                        .line_height(px(14.3 * interface_font_ratio))
                        .font_weight(FontWeight::SEMIBOLD);
                    if user {
                        if participant_name.is_some() {
                            metadata = metadata.child(
                                div()
                                    .w(px(21.))
                                    .h(px(21.))
                                    .rounded_md()
                                    .bg(rgb(mix_rgb(palette.panel, participant_colour.unwrap_or(palette.accent), 0.25)))
                                    .text_color(rgb(palette.ink))
                                    .text_size(px(10.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(
                                        label
                                            .chars()
                                            .next()
                                            .unwrap_or('?')
                                            .to_uppercase()
                                            .to_string(),
                                    ),
                            );
                        }
                    } else if let Some(agent) = message_agent {
                        let avatar = agent_images.get(&field(agent, "id")).cloned();
                        if let Some(avatar) = avatar {
                            metadata =
                                metadata.child(img(avatar).w(px(21.)).h(px(21.)).rounded_md());
                        } else if field(agent, "provider").eq_ignore_ascii_case("mona")
                            || field(agent, "model").eq_ignore_ascii_case("mona")
                        {
                            metadata = metadata.child(
                                div()
                                    .w(px(21.))
                                    .h(px(21.))
                                    .rounded_md()
                                    .bg(rgb(palette.accent))
                                    .text_color(rgb(0xffffff))
                                    .text_size(px(11.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child("M"),
                            );
                        } else if let Some(mark) = branding::provider_for_ink(
                            &field(agent, "provider"),
                            &field(agent, "model"),
                            palette.ink,
                        ) {
                            metadata = metadata.child(img(mark).w(px(21.)).h(px(21.)).rounded_md());
                        } else {
                            let provider = field(agent, "provider");
                            let mark = provider.chars().take(2).collect::<String>().to_uppercase();
                            metadata = metadata.child(
                                div()
                                    .w(px(21.))
                                    .h(px(21.))
                                    .rounded_md()
                                    .bg(rgb(design::parse_color(
                                        &field(agent, "color"),
                                        palette.accent,
                                    )))
                                    .text_color(rgb(0xffffff))
                                    .text_size(px(8.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(mark),
                            );
                        }
                    }
                    metadata = metadata.child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_size(px(11. * interface_font_ratio))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(palette.ink))
                            .child(label.clone()),
                    );
                    let message_id = field(message, "id");
                    let group: SharedString = format!("message-actions-{message_id}").into();
                    if message_action_focus.contains_key(&message_id) {
                        let copy_message = message_id.clone();
                        let copy_task = field(&task_snapshot, "id");
                        let copy_pane = pane.clone();
                        let reply_message = message_id.clone();
                        let reply_author = label.clone();
                        let reply_task = field(&task_snapshot, "id");
                        let fork_task = reply_task.clone();
                        let fork_message = message_id.clone();
                        let reply_pane = pane.clone();
                        let fork_pane = pane.clone();
                        let actions = div()
                            .id(SharedString::from(format!("message-action-strip-{message_id}")))
                            .flex().flex_shrink_0().items_center().gap(px(2.))
                            .max_w(px(0.)).opacity(0.).overflow_hidden()
                            .group_hover(group.clone(), |style| style.max_w(px(76.)).opacity(1.))
                            .child(Button::new(SharedString::from(format!("copy-message-{message_id}")))
                                .ghost().compact().w(px(23.)).h(px(23.)).p_0()
                                .icon(Icon::new(IconName::Copy).size(px(13.)))
                                .disabled(visible_text.is_empty())
                                .accessibility_label("Copy message text").tooltip("Copy message text")
                                .on_click(move |_, _, cx| {
                                    let _ = copy_pane.update(cx, |pane, cx| {
                                        pane.copy_message_text(&copy_task, &copy_message, cx);
                                    });
                                }))
                            .child(Button::new(SharedString::from(format!("reply-message-{message_id}")))
                                .ghost().compact().w(px(23.)).h(px(23.)).p_0()
                                .icon(Icon::new(IconName::Reply).size(px(14.)))
                                .disabled(visible_text.is_empty())
                                .accessibility_label("Attach message to reply").tooltip("Attach message to reply")
                                .on_click(move |_, window, cx| {
                                    let _ = reply_pane.update(cx, |pane, cx| {
                                        if pane.task_id() != reply_task { return; }
                                        let Some((author, text)) = pane.transcripts.get(&reply_task)
                                            .and_then(|state| current_reply_content(&state.messages, &reply_message, &reply_author))
                                        else { return; };
                                        pane.attachments.update(cx, |attachments, cx| {
                                            attachments.attach_message_text(&author, &text, cx);
                                        });
                                        pane.composer.read(cx).focus_handle(cx).focus(window, cx);
                                    });
                                }))
                            .child(Button::new(SharedString::from(format!("fork-message-{message_id}")))
                                .ghost().compact().w(px(23.)).h(px(23.)).p_0()
                                .icon(Icon::new(IconName::GitFork).size(px(13.)))
                                .disabled(field(message, "streamStatus") == "streaming")
                                .accessibility_label("Fork chat through this message").tooltip("Fork chat through this message")
                                .on_click(move |_, window, cx| {
                                    let _ = fork_pane.update(cx, |pane, cx| {
                                        pane.open_message_fork(fork_task.clone(), fork_message.clone(), window, cx);
                                    });
                                }));
                        metadata = metadata.child(actions);
                    }
                    if crate::timestamps::local_datetime(timestamp).is_some() {
                        metadata = metadata.child(
                            div().ml_auto().flex_shrink_0().whitespace_nowrap()
                                .font_family(design::FONT_MONO)
                                .font_weight(FontWeight::NORMAL)
                                .text_size(px(10. * interface_font_ratio))
                                .line_height(px(13. * interface_font_ratio))
                                .text_color(rgb(palette.muted))
                                .child(chat_time_label(timestamp, now_ms)),
                        );
                    }
                    let (bubble_metadata, full_width_metadata) = if user || system {
                        (Some(metadata), None)
                    } else {
                        (None, Some(metadata))
                    };
                    let mut bubble = div().w(relative(bubble_width)).min_w_0().max_w_full()
                        .flex_shrink_1().flex().flex_col();
                    let mut body = div()
                        .id(SharedString::from(format!("message-body-{message_id}")))
                        .w_full()
                        .min_w_0()
                        .max_w_full()
                        .flex_shrink_1()
                        .flex()
                        .flex_col()
                        .gap(px(space::TIGHT))
                        .font_family(chat_family.clone())
                        .font_weight(chat_weight)
                        .text_size(px(chat_size))
                        .line_height(px(chat_line_height))
                        .children(bubble_metadata)
                        .child(if let Some(streaming_body) = &streaming_body {
                            streaming_body.clone().into_any_element()
                        } else {
                            gpui_kit::component::text::TextView::markdown(
                                SharedString::from(format!(
                                    "message-markdown-{}",
                                    field(message, "id")
                                )),
                                stream_presentation.markdown_source,
                            )
                            .selectable(true)
                            .scrollable(false)
                            .w_full()
                            .min_w_0()
                            .max_w_full().into_any_element()
                        });
                    if let Some(status_label) = stream_presentation.status_label.filter(|_| streaming_body.is_none()) {
                        body = body.child(
                            div()
                                .text_size(px(11.))
                                .text_color(rgb(palette.muted))
                                .child(status_label),
                        );
                    }
                    if user {
                        let tinted = message_settings["tintUserMessages"].as_bool() == Some(true) || participant_name.is_some();
                        let tint_colour = participant_colour.unwrap_or(palette.accent);
                        let user_border = if tinted {
                            mix_rgb(palette.soft, tint_colour, 0.32)
                        } else {
                            mix_rgb(palette.soft, palette.ink, 0.14)
                        };
                        let user_background = if tinted {
                            mix_rgb(palette.panel, tint_colour, 0.16)
                        } else {
                            palette.soft
                        };
                        body = body.p(px(space::INSET));
                        bubble = bubble
                            .border_1()
                            .border_color(design::surface_color(user_border))
                            .rounded_tl(px(radius::CARD))
                            .rounded_tr(px(radius::CARD))
                            .rounded_bl(px(radius::CARD))
                            .rounded_br(px(3.))
                            .bg(crate::surface_blend::raised_background(
                                crate::surface_blend::Area::Bubbles, user_background,
                            ));
                    } else if final_answer {
                        body = body.p(px(space::INSET));
                        bubble = bubble
                            .border_1()
                            .border_color(design::surface_color(mix_rgb(
                                palette.line,
                                FINAL_ANSWER_GREEN,
                                0.18,
                            )))
                            .rounded_tl(px(radius::CARD))
                            .rounded_tr(px(radius::CARD))
                            .rounded_bl(px(3.))
                            .rounded_br(px(radius::CARD))
                            .bg(crate::surface_blend::raised_background(
                                crate::surface_blend::Area::Bubbles,
                                mix_rgb(palette.panel, FINAL_ANSWER_GREEN, 0.07),
                            ));
                    } else if system {
                        body = body
                            .pl_3()
                            .border_l_2()
                            .border_color(design::surface_color(palette.line))
                            .text_color(rgb(palette.muted));
                    }
                    if let Some(attachments) = message_attachment_rows.get(&message_id) {
                        use message_attachments::AttachmentLayout;
                        let width = transcript_width.min(design::CHAT_MAX_WIDTH).max(120.) * bubble_width;
                        let mut layout = attachments.read(cx).layout();
                        if width < 320. && layout == AttachmentLayout::PortraitLeft {
                            layout = AttachmentLayout::LandscapeBottom;
                        }
                        attachments.update(cx, |attachments, cx| attachments.set_bubble_layout(width, layout, cx));
                        match layout {
                            AttachmentLayout::PortraitLeft => {
                                let (image_width, image_height) = attachments.read(cx).portrait_dimensions(width);
                                bubble = bubble.flex_row().items_stretch().min_h(px(image_height)).overflow_hidden()
                                    .child(div().relative().w(px(image_width)).flex_shrink_0().overflow_hidden()
                                        .child(div().absolute().size_full().child(attachments.clone())))
                                    .child(body.w_auto().flex_1());
                            }
                            AttachmentLayout::LandscapeBottom => {
                                bubble = bubble.overflow_hidden().child(body).child(attachments.clone());
                            }
                            AttachmentLayout::Cards => { bubble = bubble.child(body.child(attachments.clone())); }
                        }
                    } else {
                        bubble = bubble.child(body);
                    }
                    let mut row = div().w_full().min_w_0().flex();
                    if user {
                        row = row.justify_end();
                    }
                    let mut wrapper = div().w_full().group(group).pb(px(transcript_gap));
                    if let Some(label) = date_break.as_deref() {
                        wrapper = wrapper.child(date_separator(label, palette));
                    }
                    // Keep messages inside the same reading-width constraint as
                    // the composer and activity rows. Bubbles fill their share
                    // of that column, aligned to the speaker's side.
                    wrapper = wrapper.children(full_width_metadata).child(row.child(bubble));
                    if field(message, "role") == "assistant" && message["responseMetadata"].as_object().is_some_and(|m| !m.is_empty()) {
                        let toggle_pane = pane.clone();
                        let toggle_id = message_id.clone();
                        wrapper = wrapper.child(crate::response_metadata::render(
                            &message["responseMetadata"], &message_id, &message_settings, message_appearance,
                            response_details_expanded.contains(&message_id),
                            move |_, cx| {
                                let _ = toggle_pane.update(cx, |pane, cx| {
                                    if !pane.response_details_expanded.remove(&toggle_id) {
                                        pane.response_details_expanded.insert(toggle_id.clone());
                                    }
                                    pane.response_disclosure_generation = pane.response_disclosure_generation.wrapping_add(1);
                                    // Disclosure changes the virtual row height, too.
                                    if let Some(state) = pane.transcripts.get(&pane.task_id()) {
                                        if let Some(index) = state.projection.iter().position(|item| {
                                            matches!(item, transcript::ConversationItem::Message(message) if field(message, "id") == toggle_id)
                                        }) {
                                            if index < state.list.item_count() {
                                                state.list.splice(index..index + 1, 1);
                                            }
                                        }
                                    }
                                    cx.notify();
                                });
                            },
                        ));
                    }
                    transcript_column(wrapper)
                    })();
                    if index == 0 && history_top_reached {
                        let reload_pane = pane.clone();
                        let earlier_pane = pane.clone();
                        let controls = div().w_full().mb(px(transcript_gap)).flex().items_center().justify_center().gap(px(space::TIGHT))
                                .when(more_history, |this| this.child(
                                    Button::new("load-earlier-at-history-boundary").compact().text()
                                        .label(if history_loading { "Loading earlier messages…" } else { "Load earlier messages" })
                                        .disabled(history_loading)
                                        .on_click(move |_, _, cx| { let _ = earlier_pane.update(cx, |pane, cx| pane.load_earlier(cx)); })))
                                .child(Button::new("reload-at-history-boundary").compact().text()
                                    .icon(IconName::RefreshCw).label("Reload transcript")
                                    .accessibility_label("Reload transcript at history start")
                                    .tooltip("Reload stored transcript; does not restart the agent")
                                    .disabled(refreshing || history_loading)
                                    .on_click(move |_, _, cx| { let _ = reload_pane.update(cx, |pane, cx| pane.refresh(cx)); }));
                        div().w_full()
                            .child(transcript_column(controls))
                            .child(rendered)
                            .into_any_element()
                    } else { rendered }
                }));
            retain_transcript_list(&mut self.detail_transcript_cache, list_state, row_renderer, cx);
        } else {
            self.detail_transcript_cache = None;
        }
            let list = self.detail_transcript_cache.clone();
            if let Some(viewport) = self.transcript_viewport.as_ref() {
                viewport.update(cx, |viewport, cx| {
                    viewport.set_content(self.task_id(), list, history_error, palette, cx);
                });
            } else {
                let owner = cx.weak_entity();
                self.transcript_viewport = Some(cx.new(|_| {
                    TranscriptViewport::new(owner, self.task_id(), list, history_error, palette)
                }));
            }
            // Projection reconciliation can adjust the list boundary. Capture
            // after preparation so that its own bookkeeping does not force an
            // extra history walk on the next keystroke.
            self.transcript_preparation_key = Some(TranscriptPreparationKey::capture(
                self, palette, cx.theme().font_family.clone(), window.appearance(),
            ));
        }
        self.streaming_bodies_initialized = true;
        let transcript = self.transcript_viewport.as_ref().unwrap().clone()
            .cached(StyleRefinement::default().flex_1().min_h_0());
        let task_running = selected.is_some_and(|task| field(task, "status") == "running");
        let composer_bottom = match field(&self.snapshot["settings"], "interfaceDensity").as_str() {
            "tight" => space::TIGHT,
            "spacious" => space::SECTION,
            _ => space::INSET,
        };
        self.activity.update(cx, |activity, cx| {
            activity.set_appearance(palette, interface_font_ratio, composer_bottom, cx);
        });
        let composer_height = self.decoration_bounds.working_anchor.get()
            .map(|bounds| bounds.size.height).unwrap_or(px(180.));
        let composer_hidden = crate::composer_scroll::composer_slide(
            self.composer_scroll_distance, composer_height, !crate::native_motion::motion_allowed());
        let away_from_bottom = self.transcripts.get(&self.task_id())
            .is_some_and(|state| !state.list.is_following_tail());
        let attachments_for_paste = self.attachments.clone();
        let git_docked = self.composer_git.read(cx).has_visible_strip();
        let slash_menu = self.slash.clone();
        let slash_open = self.slash.read(cx).is_open();
        let rail_width = if self.companion_visible && !self.compact_detail {
            self.companion_resize.read(cx).sizes().last().copied()
                .map(|width| width / design::ui_scaled_px(1.)).unwrap_or(self.companion_width)
        } else { 0. };
        let footer_width = (self.pane_width_design > 0.).then_some(
            (self.pane_width_design - rail_width - composer_bottom * 2.)
                .min(design::CHAT_MAX_WIDTH).max(0.),
        );
        self.composer_footer.update(cx, |footer, cx| footer.set_appearance(palette, footer_width, cx));
        self.composer_context_bar.update(cx, |bar, cx| bar.set_appearance(palette, cx));
        content = content
            // Anchor the alpha ramps to the viewport edges, flush with the
            // pane dock above and composer area below, rather than an inset list.
            .child(crate::transcript_edge_fade::TranscriptEdgeFade::new(
                transcript,
                // Window viewport is already in native logical pixels; scale
                // only the fixed bottom design token, not the percentage ramp.
                window.viewport_size().height * 0.10,
                px(24.),
            ))
            .child(transcript_column(self.render_interactions(window, cx)))
            .when(slash_open, |content| {
                content.child(
                    div()
                        .w_full()
                        .px(px(composer_bottom))
                        .flex_shrink_0()
                        .child(transcript_column(div().w_full().px(px(5.)).child(slash_menu))),
                )
            })
            .when_some(self.slash_notice.clone(), |content, notice| {
                content.child(
                    transcript_column(
                        div()
                            .px_3()
                            .text_size(px(11.))
                            .text_color(rgb(palette.muted))
                            .child(notice),
                    ),
                )
            })
            .when_some(voice_status, |content, status| {
                content.child(
                    transcript_column(
                        div()
                            .px_3()
                            .text_size(px(11.))
                            .text_color(rgb(palette.muted))
                            .child(status),
                    ),
                )
            })
            .child(div().relative().w_full().flex().flex_col().flex_shrink_0()
                .mb(-composer_hidden)
                .child({
                    let geometry = self.decoration_bounds.working_anchor.clone();
                    canvas(move |bounds, _, _| geometry.set(Some(bounds)), |_, _, _, _| ())
                        .absolute().inset_0()
                })
            .child(transcript_column(self.activity.clone()))
            .child(
                div()
                    .w_full()
                    .px(px(composer_bottom))
                    .flex_shrink_0()
                    .child(transcript_column(
                        div().w_full().child(self.composer_git.clone()),
                    )),
            )
            .child(
                div()
                    .w_full()
                    .px(px(composer_bottom))
                    .mb(px(composer_bottom))
                    .flex_shrink_0()
                    .child(transcript_column(
                        div()
                            .w_full()
                            .flex()
                            .flex_col()
                            .flex_shrink_0()
                            .child(
                                div()
                                    .relative()
                                    .w_full()
                                    .child({
                                        let geometry = self.decoration_bounds.composer.clone();
                                        canvas(move |bounds, _, _| geometry.set(Some(bounds)), |_, _, _, _| ())
                                            .absolute().inset_0()
                                    })
                                    .p(px(space::INSET))
                                    .border_1()
                                    .border_color(design::surface_color(palette.line))
                                    .rounded(px(radius::CONTROL))
                                    .when(git_docked, |this| {
                                        this.rounded_tl(px(0.)).rounded_tr(px(0.))
                                    })
                                    .overflow_hidden()
                                    .bg(crate::surface_blend::raised_background(
                                        crate::surface_blend::Area::Composer, palette.card,
                                    ))
                                    .child(self.attachments.clone())
                                    .child(
                                        div()
                                            .flex()
                                            .items_end()
                                            .gap(px(space::TIGHT))
                                            .child(
                                                Button::new("choose-chat-attachments")
                                                    .compact()
                                                    .w(px(design::COMPOSER_CONTROL_SIZE))
                                                    .h(px(design::COMPOSER_CONTROL_SIZE))
                                                    .icon(Icon::new(IconName::Paperclip))
                                                    .accessibility_label("Attach files")
                                                    .tooltip(
                                                        if self.attachments.read(cx).pending() {
                                                            "Attaching files…"
                                                        } else {
                                                            "Attach files"
                                                        },
                                                    )
                                                    .disabled(
                                                        !self.attachments.read(cx).can_choose() || self.is_performance_fixture(),
                                                    )
                                                    .on_click(cx.listener(|pane, _, _, cx| {
                                                        pane.attachments.update(
                                                            cx,
                                                            |attachments, cx| {
                                                                attachments.choose(cx)
                                                            },
                                                        )
                                                    })),
                                            )
                                            .child(
                                                Textarea::new(&self.composer)
                                                    .on_paste(move |item, _, cx| {
                                                        attachments_for_paste.update(cx, |attachments, cx| {
                                                            attachments.paste(item, cx)
                                                        })
                                                    })
                                                    .small()
                                                    .appearance(false)
                                                    .bordered(false)
                                                    .font_family(composer_chat_family)
                                                    .font_weight(chat_weight)
                                                    .text_size(px(chat_size))
                                                    .line_height(px(chat_line_height))
                                                    .min_h(px(design::COMPOSER_CONTROL_SIZE))
                                                    .flex_1()
                                                    .disabled(selected.is_none() || self.busy),
                                            )
                                            .when(selected.is_some() && !self.busy && !self.is_performance_fixture(), |row| {
                                                row.child(voice_controls)
                                            })
                                            .child(
                                                Button::new("send")
                                                    .outline()
                                                    .w(px(design::COMPOSER_CONTROL_SIZE))
                                                    .h(px(design::COMPOSER_CONTROL_SIZE))
                                                    .border_color(rgb(palette.accent))
                                                    .text_color(rgb(palette.accent))
                                                    .compact()
                                                    .icon(Icon::new(if self.busy {
                                                        IconName::LoaderCircle
                                                    } else {
                                                        IconName::CornerDownLeft
                                                    }))
                                                    .accessibility_label(if self.busy {
                                                        "Sending"
                                                    } else {
                                                        "Send"
                                                    })
                                                    .tooltip("Send message")
                                                    .disabled(
                                                        selected.is_none()
                                                            || self.busy
                                                            || self.attachments.read(cx).pending()
                                                            || self.is_performance_fixture(),
                                                    )
                                                    .on_click(
                                                        cx.listener(|view, _, _, cx| view.send(cx)),
                                                    ),
                                            ),
                                    )
                                    .child(
                                        self.composer_context_bar.clone().cached(StyleRefinement::default().absolute().bottom_0().left_0().right_0().h(px(3.))),
                                    ),
                            )
                            .child(transcript_column(div().w_full().flex_shrink_0().child(
                                self.composer_footer.clone().cached(self.composer_footer.read(cx).cached_style()),
                            ))),
                    )),
            ));
        let scale = design::interface_scale();
        if self.companion_scale != scale {
            self.companion_scale = scale;
            if self.companion_resize.read(cx).sizes().len() == 2 {
                let width = design::ui_px(self.companion_width, scale);
                self.companion_programmatic_resizes += 1;
                self.companion_resize.update(cx, |state, cx| {
                    state.resize_panel(1, width, window, cx);
                });
            }
        }
        // Notify this retained chat entity explicitly: a window redraw alone
        // can replay NativePanel's cached chat subtree without advancing layout.
        let (detail_progress, detail_running) = self.detail_motion.sample(
            self.companion_visible && self.pane_width_design > 0.0,
            cx.reduce_motion(),
        );
        if detail_running {
            let owner = cx.weak_entity();
            window.on_next_frame(move |_, cx| {
                let _ = owner.update(cx, |pane, cx| {
                    pane.detail_animation_frame = true;
                    cx.notify();
                });
            });
        }
        let detail_mounted = self.pane_width_design > 0.0
            && (self.companion_visible || detail_progress > 0.0);
        self.working_bubble.update(cx, |bubble, cx| {
            bubble.set_context(
                (task_running || self.starting_turn) && !(self.compact_detail && detail_mounted),
                !task_running && self.starting_turn, self.busy, palette, cx,
            );
        });
        let rail = detail_mounted.then(|| {
            self.companion
                .update(cx, |pane, cx| pane.set_composed_palette(palette, cx));
            use crate::companion_sidebar::DetailTab;
            let body = match self.detail_tab {
                DetailTab::Details => {
                    let git_summary = self
                        .composer_git
                        .update(cx, |strip, cx| strip.render_detail_summary(palette, cx));
                    self.details.update(cx, |pane, cx| {
                        pane.render_details(palette, git_summary, window, cx)
                    })
                }
                DetailTab::Timeline => self
                    .details
                    .update(cx, |pane, cx| pane.render_timeline(palette, window, cx)),
                DetailTab::Approvals => self
                    .approvals
                    .update(cx, |pane, cx| pane.render_body(palette, window, cx)),
                DetailTab::Subagents => self.companion.clone().into_any_element(),
            };
            let owner = cx.weak_entity();
            crate::companion_sidebar::render_detail_frame(
                &self.task_id(),
                self.detail_tab,
                Some(self.companion.read(cx).record_count()),
                body,
                palette,
                &self.snapshot["settings"],
                &self.detail_focus,
                self.detail_tabs_motion.clone(),
                window,
                cx,
                move |tab, _, cx| {
                    let _ = owner.update(cx, |pane, cx| {
                        if pane.detail_tab != tab {
                            pane.detail_tab = tab;
                            pane.emit_secondary_sidebar_change(cx);
                            cx.notify();
                        }
                    });
                },
            )
            .into_any_element()
        });
        if ((away_from_bottom && self.bottom_jump.is_none()) || self.bottom_button_exit.is_some())
            && !(self.compact_detail && detail_mounted) {
            let hover = gpui_kit::component::hover_motion::HoverMotion::new(
                "chat-back-to-bottom-surface", self.bottom_button_exit.is_none(), window, cx,
            );
            let background = hover.background("background",
                crate::surface_blend::floating_control_background(palette.panel),
                crate::surface_blend::floating_control_hover_background(palette.panel), window, cx);
            let highlight: gpui_kit::Hsla = rgb(palette.accent).into();
            let foreground = hover.color("foreground", rgb(palette.ink).into(), highlight, window, cx);
            let border = hover.color("border", rgb(palette.line).into(), highlight, window, cx);
            let glow = hover.color("glow", highlight.opacity(0.), highlight.opacity(0.28), window, cx);
            let jump = div().size(px(36.)).rounded_full()
                .border_1().border_color(border)
                .bg(background).shadow(vec![
                    gpui_kit::BoxShadow::new(px(0.), px(3.), rgba(0x00000033).into()).blur_radius(px(8.)),
                    gpui_kit::BoxShadow::new(px(0.), px(0.), glow).blur_radius(px(12.)).spread_radius(px(1.)),
                ])
                .child(Button::new("chat-back-to-bottom")
                    .custom(gpui_kit::component::button::ButtonCustomVariant::new(cx)
                        .foreground(foreground))
                    .compact().size_full().p_0().rounded_full()
                    .disabled(self.bottom_button_exit.is_some())
                    .icon(Icon::new(IconName::ArrowDown).size(px(18.)))
                    .accessibility_label("Back to bottom").tooltip("Back to bottom")
                    .on_hover(cx.listener(move |_, hovered, _, cx| {
                        hover.on_hover(*hovered, cx);
                        cx.notify();
                    }))
                    .on_click(cx.listener(|pane, _, _, cx| pane.back_to_bottom(cx))))
                .into_any_element();
            content = content.child(task_activity::working_bubble::ComposerAnchor::new(
                self.decoration_bounds.working_anchor.clone(),
                self.decoration_bounds.chat_column.clone(), jump,
                px(if task_running || self.starting_turn { 46. } else { 14. }),
            ).fly_down(button_flight));
        }
        content = content.child(self.computer_bubble.clone());
        let pending_approval = self.pending_tool_approval();
        let next_approval_id = pending_approval.as_ref().map(|r| field(r, "id"));
        if next_approval_id != self.approval_popup_id {
            // Pending requests stay in the approval dock above the composer.
            self.approval_popup_open = false;
            if self.approval_popup_open && self.focus.contains_focused(window,cx) { window.focus(&self.approval_focus,cx); }
            self.approval_details_expanded = false;
            self.approval_popup_id = next_approval_id;
        }
        if let Some(request) = pending_approval.filter(|_| self.approval_popup_open) {
            let approval_id = field(&request, "id");
            let owner = cx.weak_entity();
            let decision = Rc::new(move |decision: &'static str, _: &mut Window, cx: &mut App| {
                let _ = owner.update(cx, |pane, cx| pane.command("resolve_approval", json!({"approvalId": approval_id, "decision": decision}), cx));
            });
            let owner = cx.weak_entity();
            let toggle = Rc::new(move |_: &mut Window, cx: &mut App| {
                let _ = owner.update(cx, |pane, cx| { pane.approval_details_expanded = !pane.approval_details_expanded; cx.notify(); });
            });
            let task = rows(&self.snapshot, "tasks").into_iter().find(|t| field(t,"id") == self.task_id()).unwrap_or(Value::Null);
            let prompt = crate::approval_prompt::render_approval_prompt(&request, &task, &self.snapshot, palette, cx.theme().font_family.clone(), cx.theme().font_size, self.busy, self.approval_details_expanded, self.mcp_approval_supported, decision, toggle);
            content = content.child(div().absolute().inset_0().occlude().flex().items_center().justify_center().p_3()
                .child(div().absolute().inset_0()
                    .bg(Background::from(rgba(0x00000055)).backdrop_blur(20.))
                    .on_mouse_down(MouseButton::Left, cx.listener(|pane, _, _, cx| { pane.approval_popup_open = false; cx.stop_propagation(); cx.notify(); })))
                .child(div().id("pane-approval-modal").role(Role::Dialog).aria_label("Approval needed").font_family(cx.theme().font_family.clone()).text_size(cx.theme().font_size).track_focus(&self.approval_focus).relative().w_full().max_w(gpui_kit::px(560.))
                    .max_h(px((self.pane_height_design - 32.).max(200.))).overflow_y_scroll()
                    .rounded_lg().bg(design::modal_surface_color(palette.panel))
                    .shadow(vec![BoxShadow::new(px(0.), px(18.), rgba(0x00000066).into()).blur_radius(px(48.))])
                    .on_mouse_down(MouseButton::Left, |_,_,cx| cx.stop_propagation())
                    .child(prompt)
                    .children(self.error.as_deref().and_then(connection_recovery::visible_error).map(|error| div().px_4().py_2().text_color(rgb(0xe2645a)).child(error.to_owned())))
                    .child(div().flex().justify_end().p_2().child(Button::new("approval-review-later").text_size(cx.theme().font_size).label("Review later")
                        .on_click(cx.listener(|pane,_,_,cx| { pane.approval_popup_open = false; cx.notify(); }))))));
        }

        let pane_content = if self.compact_detail {
            let mut container = div()
                .relative()
                .flex_1()
                .min_h_0()
                .min_w_0()
                .overflow_hidden()
                .child(content.into_any_element());
            if let Some(rail) = rail {
                let owner = cx.weak_entity();
                container = container
                    .child(
                        div()
                            .id(SharedString::from(format!(
                                "chat-detail-backdrop-{}",
                                self.task_id()
                            )))
                            .role(Role::Button)
                            .aria_label("Dismiss this pane’s secondary sidebar")
                            .opacity(detail_progress)
                            .absolute()
                            .inset_0()
                            .occlude()
                            .bg(gpui_kit::Background::from(rgba(if palette.dark {
                                0x00000045
                            } else {
                                0xffffff45
                            })).backdrop_blur(20.))
                            .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                                let _ = owner.update(cx, |pane, cx| {
                                    pane.companion_visible = false;
                                    pane.companion_saved_visible = false;
                                    pane.emit_secondary_sidebar_change(cx);
                                    cx.notify();
                                });
                                cx.stop_propagation();
                            }),
                    )
                    .child(
                        div()
                            .absolute()
                            .occlude()
                            .top(px(0.))
                            .right(design::ui_px(
                                -(1. - detail_progress) * compact_detail_width(self.pane_width_design, self.companion_width),
                                scale,
                            ))
                            .bottom(px(0.))
                            .w(design::ui_px(
                                compact_detail_width(self.pane_width_design, self.companion_width),
                                scale,
                            ))
                            .max_w_full()
                            .bg(crate::surface_blend::tinted_pane_background(palette.paper, palette.dark, 0.15))
                            .overflow_hidden()
                            .child(rail),
                    );
            }
            container.into_any_element()
        } else if let Some(rail) = rail {
            let resize_appearance = gpui_kit::component::resizable::resize_handle_appearance();
            div()
                .flex_1()
                .min_h_0()
                .min_w_0()
                .child(
                    h_resizable(SharedString::from(format!(
                        "chat-companion-{}",
                        self.task_id()
                    )))
                    .with_state(&self.companion_resize)
                    // The sliding detail frame owns its left divider. Keep the
                    // split's grab band, but only paint its resize feedback
                    // when the sidebar is settled and the handle is engaged.
                    .with_handle_appearance(Rc::new(move |handle, window, cx| {
                        if detail_progress >= 1.
                            && handle.state() != gpui_kit::base::ResizeHandleState::Idle
                        {
                            resize_appearance(handle, window, cx)
                        } else {
                            // None would restore the base handle's fixed line.
                            Some(div().size_full().into_any_element())
                        }
                    }))
                    .child(
                        resizable_panel().size_range(px(0.)..Pixels::MAX).child(
                            div()
                                .size_full()
                                .min_w_0()
                                .min_h_0()
                                .overflow_hidden()
                                .child(content),
                        ),
                    )
                    .child(
                        resizable_panel()
                            .size(design::ui_px(self.companion_width, scale))
                            .size_range(
                                design::ui_px(260.0, scale)
                                    ..design::ui_px(
                                        wide_detail_max_width(self.pane_width_design),
                                        scale,
                                    ),
                            )
                            .flex_none()
                            .child(
                                div().size_full().min_w_0().min_h_0().overflow_hidden()
                                    .child(
                                        div().relative().size_full()
                                            .left(design::ui_px((1. - detail_progress) * self.companion_width, scale))
                                            .child(rail),
                                    ),
                            ),
                    ),
                )
                .into_any_element()
        } else {
            div()
                .flex_1()
                .min_h_0()
                .min_w_0()
                .child(content.into_any_element())
                .into_any_element()
        };
        let measurement_owner = cx.weak_entity();
        let measurement = canvas(
            move |bounds, window, cx| {
                if bounds.size.width > px(0.) {
                    let width_design = bounds.size.width / design::ui_px(1.0, scale);
                    let height_design = bounds.size.height / design::ui_px(1.0, scale);
                    let owner = measurement_owner.clone();
                    window.defer(cx, move |window, cx| {
                        let _ = owner.update(cx, |pane, cx| {
                            if height_design.is_finite()
                                && height_design > 0.0
                                && (pane.pane_height_design - height_design).abs() >= 1.0
                            {
                                pane.pane_height_design = height_design;
                                cx.notify();
                            }
                            pane.update_pane_width(width_design, window, cx);
                        });
                    });
                }
            },
            |_, _, _, _| (),
        )
        .absolute()
        .size_full();
        // The visor is a root layout sibling: it spans the entire pane and
        // reduces the upper workspace, including its decorative field.
        let visor = (self.companion.read(cx).record_count() > 0).then(|| {
            self.companion.update(cx, |pane, cx| {
                pane.render_visor(palette, self.pane_height_design, window, cx)
            })
        });
        let upper_workspace = div()
            .w_full()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .relative()
            .flex()
            .flex_col()
            .overflow_hidden()
            .child({
                let geometry = self.decoration_bounds.pane.clone();
                canvas(move |bounds, _, _| geometry.set(Some(bounds)), |_, _, _, _| ())
                    .absolute().size_full()
            })
            .child(header)
            .child(pane_content);
        div()
            .key_context("MonitterChat")
            .track_focus(&self.focus)
            .capture_action(cx.listener(
                |pane, _: &gpui_kit::component::input::MoveUp, window, cx| {
                    if pane.composer_keyboard_available(window, cx)
                        && pane.slash.update(cx, |slash, cx| {
                            slash.handle_action(crate::slash_palette::PaletteAction::Previous, cx)
                        })
                    {
                        cx.stop_propagation();
                    }
                },
            ))
            .capture_action(cx.listener(
                |pane, _: &gpui_kit::component::input::MoveDown, window, cx| {
                    if pane.composer_keyboard_available(window, cx)
                        && pane.slash.update(cx, |slash, cx| {
                            slash.handle_action(crate::slash_palette::PaletteAction::Next, cx)
                        })
                    {
                        cx.stop_propagation();
                    }
                },
            ))
            .capture_action(cx.listener(
                |pane, action: &gpui_kit::component::input::Enter, window, cx| {
                    if !action.shift
                        && !window.has_active_dialog(cx)
                        && pane.composer_keyboard_available(window, cx)
                    {
                        let text = pane.composer.read(cx).value().to_string();
                        let handled = pane.slash.update(cx, |slash, cx| {
                            let typed_submit = chat_slash::typed_submit_before_menu(&text);
                            slash.sync_input(text, cx);
                            if typed_submit {
                                slash.submit_typed_input(cx)
                            } else {
                                slash
                                    .handle_action(crate::slash_palette::PaletteAction::Confirm, cx)
                            }
                        });
                        if handled {
                            cx.stop_propagation();
                        } else if composer_enter_sends(
                            pane.snapshot["settings"]["sendWithEnter"]
                                .as_bool()
                                .unwrap_or(false),
                            action.secondary,
                            action.shift,
                        ) {
                            pane.send(cx);
                            cx.stop_propagation();
                        }
                    }
                },
            ))
            .capture_action(cx.listener(
                |pane, _: &gpui_kit::component::input::Escape, window, cx| {
                    if pane.composer_keyboard_available(window, cx)
                        && pane.slash.update(cx, |slash, cx| {
                            slash.handle_action(crate::slash_palette::PaletteAction::Dismiss, cx)
                        })
                    {
                        cx.stop_propagation();
                    }
                },
            ))
            .capture_key_down(cx.listener(|pane, event: &KeyDownEvent, window, cx| {
                if pane.approval_popup_open {
                    if event.keystroke.key == "escape" {
                        pane.approval_popup_open = false; window.focus(&pane.focus,cx); cx.stop_propagation(); cx.notify();
                    }
                    return;
                }

                if std::env::var_os("MONITTER_GPUI_TRACE_COMPOSER_KEYS").is_some()
                    && matches!(
                        event.keystroke.key.as_str(),
                        "up" | "down" | "escape" | "enter"
                    )
                {
                    eprintln!(
                        "[composer-key] key={} focused={} paletteOpen={}",
                        event.keystroke.key,
                        pane.composer.read(cx).focus_handle(cx).is_focused(window),
                        pane.slash.read(cx).is_open()
                    );
                }
                if pane.composer.read(cx).focus_handle(cx).is_focused(window)
                    && !pane.composer.update(cx, |input, cx| {
                        EntityInputHandler::marked_text_range(input, window, cx).is_some()
                    })
                {
                    let m = &event.keystroke.modifiers;
                    let typed_submit = typed_submit_for_key(
                        &event.keystroke.key,
                        m.shift,
                        m.alt,
                        m.control,
                        m.platform,
                        || {
                            let text = pane.composer.read(cx).value();
                            pane.slash.update(cx, |slash, cx| slash.sync_input(text.to_string(), cx));
                            text
                        },
                    );
                    let handled = pane.slash.update(cx, |slash, cx| {
                        if typed_submit {
                            slash.submit_typed_input(cx)
                        } else {
                            slash.handle_key(event, cx)
                        }
                    });
                    if handled {
                        cx.stop_propagation();
                    }
                }
            }))
            .on_action(cx.listener(|pane, _: &SendMessage, window, cx| {
                if !window.has_active_dialog(cx) && pane.composer_keyboard_available(window, cx) {
                    pane.send(cx);
                }
            }))
            .on_key_down(cx.listener(|pane, event: &KeyDownEvent, window, cx| {
                let modifiers = &event.keystroke.modifiers;
                if event.keystroke.key == "escape" && pane.approval_popup_open {
                    pane.approval_popup_open = false;
                    cx.stop_propagation(); cx.notify(); return;
                }

                if event.keystroke.key == "escape"
                    && !modifiers.control
                    && !modifiers.platform
                    && !modifiers.alt
                    && !modifiers.shift
                    && pane.compact_detail
                    && pane.companion_visible
                    && !window.has_active_dialog(cx)
                {
                    pane.companion_visible = false;
                    pane.companion_saved_visible = false;
                    pane.emit_secondary_sidebar_change(cx);
                    cx.stop_propagation();
                    cx.notify();
                }
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|pane, _, _, cx| cx.emit(ChatEvent::Focused(pane.task_id()))),
            )
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .bg(crate::surface_blend::tinted_pane_background(palette.paper, palette.dark, 0.15))
            .text_color(rgb(palette.ink))
            .child(measurement)
            .child(upper_workspace)
            .children(visor.map(|visor| div().w_full().flex_shrink_0().child(visor)))
    }
}
// Comparison-only override; normal app launches retain Tauri's default.
// Per-pane saved width persistence remains owned by the UI layout work.
fn reference_detail_width() -> f32 {
    std::env::var("MONITTER_GPUI_REFERENCE_DETAIL_WIDTH")
        .ok()
        .and_then(|value| value.parse::<f32>().ok())
        .filter(|width| width.is_finite() && (260.0..=1600.0).contains(width))
        .unwrap_or(292.0)
}
const COMPACT_DETAIL_BREAKPOINT: f32 = 700.0;
const COMPACT_DETAIL_MIN_WIDTH: f32 = 260.0;
const COMPACT_DETAIL_GUTTER: f32 = 24.0;
const WIDE_DETAIL_GUTTER: f32 = 300.0;

fn should_hide_detail_on_compact_entry(was_compact: bool, is_compact: bool, visible: bool) -> bool {
    !was_compact && is_compact && visible
}

fn compact_detail_width(pane_width: f32, saved_width: f32) -> f32 {
    (pane_width - COMPACT_DETAIL_GUTTER)
        .max(0.0)
        .min(saved_width.max(COMPACT_DETAIL_MIN_WIDTH))
}

fn wide_detail_max_width(pane_width: f32) -> f32 {
    (pane_width - WIDE_DETAIL_GUTTER).clamp(260.0, 1600.0)
}

#[cfg(test)]
mod compact_detail_tests {
    use super::{
        COMPACT_DETAIL_BREAKPOINT, compact_detail_width, should_hide_detail_on_compact_entry,
        wide_detail_max_width,
    };

    #[::core::prelude::v1::test]
    fn pane_local_breakpoint_and_entry_transition_match_tauri_behavior() {
        assert_eq!(COMPACT_DETAIL_BREAKPOINT, 700.0);
        assert!(should_hide_detail_on_compact_entry(false, true, true));
        assert!(!should_hide_detail_on_compact_entry(false, true, false));
        assert!(!should_hide_detail_on_compact_entry(true, true, true));
        assert!(!should_hide_detail_on_compact_entry(true, false, true));
    }

    #[::core::prelude::v1::test]
    fn compact_overlay_uses_saved_width_with_minimum_and_pane_gutter() {
        assert_eq!(compact_detail_width(680.0, 200.0), 260.0);
        assert_eq!(compact_detail_width(680.0, 420.0), 420.0);
        assert_eq!(compact_detail_width(400.0, 500.0), 376.0);
        assert_eq!(compact_detail_width(20.0, 500.0), 0.0);
    }

    #[::core::prelude::v1::test]
    fn wide_detail_cap_reserves_conversation_space() {
        assert_eq!(wide_detail_max_width(1000.0), 700.0);
        assert_eq!(wide_detail_max_width(700.0), 400.0);
        assert_eq!(wide_detail_max_width(400.0), 260.0);
        assert_eq!(wide_detail_max_width(4000.0), 1600.0);
    }
}

impl ChatPane {
    pub(crate) fn set_dock_group(
        &mut self,
        group: Option<WeakEntity<gpui_kit::component::dock::TabGroup>>,
        cx: &mut Context<Self>,
    ) {
        self.dock_group = group;
        cx.notify();
    }
    pub(super) fn render_chat_header(
        &self,
        task: Option<&Value>,
        agents: &[Value],
        palette: design::Palette,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let agent = task.and_then(|task| {
            agents
                .iter()
                .find(|agent| field(agent, "id") == field(task, "agentId"))
        });
        let title = task
            .map(|task| field(task, "title"))
            .unwrap_or_else(|| "Workspace".into());
        let title = if crate::activity_effects::is_naming(&self.task_id(), cx) {
            crate::naming_title::render(&self.task_id(), "header", title.into(), palette.accent, window, cx)
        } else {
            div().child(title).into_any_element()
        };
        let name = agent
            .map(|agent| field(agent, "name"))
            .unwrap_or_else(|| "monitter".into());
        let color = agent
            .map(|agent| design::parse_color(&field(agent, "color"), palette.accent))
            .unwrap_or(palette.accent);
        let provider_icon = task.and_then(|task| {
            branding::provider_for_ink(&field(task, "provider"), &field(task, "model"), palette.ink)
        });
        let tern_tiles = design::tile_theme()
            == monitter_lib::experimental_desktop::NativeTileTheme::Tern;
        let pane_appearance = crate::pane_appearance::PaneAppearance::from_settings(
            &self.snapshot["settings"], palette,
        );
        let active_border_visible = pane_appearance.show_active_border
            && crate::docking::pane_visual_active(&self.focus, window, cx)
            && !crate::preferences_modal::workspace_blocked(cx);
        let density = field(&self.snapshot["settings"], "interfaceDensity");
        let header_padding_y = match (tern_tiles, density.as_str()) {
            (true, "spacious") => 8.,
            (true, _) => 6.,
            (false, "tight") => space::TIGHT,
            (false, "spacious") => space::GROUP,
            _ => space::INSET,
        };
        let avatar_size = match (tern_tiles, density.as_str()) {
            (true, _) => 24.,
            (false, "tight") => 26.,
            (false, "spacious") => 34.,
            _ => 30.,
        };
        let identity_mark = if let Some(image) = provider_icon {
            div()
                .size(px(avatar_size))
                .flex()
                .items_center()
                .justify_center()
                .child(
                    img(image)
                        .w(px(22. * avatar_size / 30.))
                        .h(px(22. * avatar_size / 30.))
                        .flex_shrink_0(),
                )
                .into_any_element()
        } else {
            div()
                .size(px(avatar_size))
                .rounded_md()
                .bg(rgb(color))
                .text_color(rgb(0xffffff))
                .text_size(px(13.))
                .flex()
                .items_center()
                .justify_center()
                .child(name.chars().next().unwrap_or('m').to_string())
                .into_any_element()
        };
        let identity = div()
            .flex_1().min_w_0().h_full().flex().items_center()
            .gap(px(if tern_tiles { 8. } else { space::INSET }))
            .child(identity_mark)
            .child(div().flex_1().min_w_0().flex().flex_col().child(
                div().text_size(px(13.2))
                    .when(tern_tiles, |title| title.relative().top(px(2.)))
                    .font_weight(if tern_tiles { FontWeight::MEDIUM } else { FontWeight::BOLD })
                    .truncate().child(title),
            ));
        let identity = if tern_tiles {
            if let Some(group) = self.dock_group.as_ref().and_then(|group| group.upgrade()) {
                gpui_kit::component::dock::pane_drag_region(identity, &group.read(cx).context(cx), cx)
                    .into_any_element()
            } else { identity.into_any_element() }
        } else { identity.into_any_element() };
        let mut header = div()
            // Keep avatar and row geometry aligned with the native density tokens.
            .h(px(avatar_size + 2. * header_padding_y))
            .flex_shrink_0()
            .relative()
            .overflow_hidden()
            .flex()
            .items_center()
            .gap(px(if tern_tiles { 8. } else { space::INSET }))
            .px(px(if tern_tiles { 13. } else { space::GROUP }))
            .border_b_1()
            .border_color(rgb(palette.line))
            .border_blend_mode(crate::surface_blend::separator_mode())
            .when(tern_tiles, |this| {
                this.border_color(design::tile_border_color(palette))
                    .border_blend_mode(0)
            })
            .when(tern_tiles && design::border_opacity() == 0, |this| this.border_b_0())
            .when(active_border_visible, |this| {
                if tern_tiles && pane_appearance.border_opacity == 0 {
                    this.border_b_0()
                } else {
                    let mut color = rgb(pane_appearance.resolved_active_border_color());
                    if tern_tiles {
                        color.a = pane_appearance.border_opacity as f32 / 100.;
                    }
                    this.border_color(color).border_blend_mode(0)
                }
            })
            .child({
                let geometry = self.decoration_bounds.header.clone();
                canvas(move |bounds, _, _| geometry.set(Some(bounds)), |_, _, _, _| ())
                    .absolute().size_full()
            })
            .children(crate::activity_effects::for_task_in_window(
                &self.task_id(), "header-wash", crate::activity_sparkles::SparkleLayout::HeaderWash,
                true, window, cx,
            ))
            .child(identity);
        if let Some(group) = self.dock_group.clone() {
            use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
            let owner = cx.weak_entity();
            let task_id = self.task_id();
            let can_archive = rows(&self.snapshot, "tasks").into_iter().find(|task| field(task, "id") == task_id).is_some_and(|task| field(&task, "status") != "running");
            header = header.child(
                Button::new("header-pane-menu")
                    .compact()
                    .text()
                    .icon(Icon::new(IconName::Ellipsis))
                    .accessibility_label("Pane menu")
                    .tooltip("Pane menu")
                    .dropdown_menu(move |menu, window, cx| {
                        let Some(group) = group.upgrade() else {
                            return menu;
                        };
                        // Read selection/zoom/close policy at opening time,
                        // even when this header came from the retained cache.
                        let context = group.read(cx).context(cx);
                        let rename_owner = owner.clone();
                        let archive_owner = owner.clone();
                        let rename_id = task_id.clone();
                        let archive_id = task_id.clone();
                        let menu = menu
                            .item(PopupMenuItem::new("Rename chat…").on_click(move |_, window, cx| {
                                let _ = rename_owner.update(cx, |pane, cx| {
                                    if pane.task_id() == rename_id { pane.open_task_action(false, window, cx); }
                                });
                            }))
                            .item(PopupMenuItem::new("Archive chat").disabled(!can_archive).on_click(move |_, window, cx| {
                                let _ = archive_owner.update(cx, |pane, cx| {
                                    if pane.task_id() == archive_id { pane.open_task_action(true, window, cx); }
                                });
                            }));
                        gpui_kit::component::dock::pane_menu(menu, &context, window, cx)
                    })
                    .anchor(Anchor::TopRight),
            );
        }
        header = header.child(
                Button::new("toggle-pane-secondary-sidebar")
                    .compact()
                    .text()
                    .icon(
                        Icon::default()
                            .data(gpui_kit::assets::__private::PanelRight.1)
                            .with_size(px(16.)),
                    )
                    .accessibility_label(if self.companion_visible {
                        "Hide this pane's secondary sidebar"
                    } else {
                        "Show this pane's secondary sidebar"
                    })
                    .on_click(cx.listener(|pane, _, _, cx| pane.toggle_companion(cx))),
            );
        if tern_tiles {
            if let Some(group) = self.dock_group.as_ref().and_then(|group| group.upgrade()) {
                header = header.child(gpui_kit::component::dock::pane_controls(&group.read(cx).context(cx), cx));
            }
        }
        header.into_any_element()
    }
}
