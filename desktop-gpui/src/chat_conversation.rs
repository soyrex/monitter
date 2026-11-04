//! Retained native controls for the opt-in local conversation prototype.
//! The audio child has no task authority; all final text uses the private
//! backend's durable admission endpoint. Stopping speech never stops agent work.

use crate::{
    design::{ui_scaled_px as px, Palette},
    native_voice_session::{AudioEvent, NativeVoiceSession},
};
use gpui_kit::{
    component::{
        button::{Button, ButtonVariants},
        Disableable,
    },
    prelude::*,
    *,
};
use monitter_gpui_desktop::voice_conversation_model::{
    AssistantText, ConversationModel, InputDisposition,
};
use monitter_gpui_desktop::voice_delivery_model::VoiceDelivery;
use monitter_lib::{
    experimental_desktop::DesktopService,
    voice_admission::{VoiceReceipt, VoiceUtterance},
    voice_session::VoiceLease,
};
use serde_json::Value;
#[path = "voice_capture_policy.rs"]
mod voice_capture_policy;
use voice_capture_policy::CapturePolicy;

use std::{
    collections::HashSet,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[derive(Default)]
struct VoiceOwner {
    owner: Option<WeakEntity<ChatConversation>>,
    serial: Arc<Mutex<()>>,
    releases: Vec<std::sync::mpsc::Receiver<Result<(), String>>>,
    cleanup_error: Arc<Mutex<Option<String>>>,
}
impl Global for VoiceOwner {}

const POLL: Duration = Duration::from_millis(50);
const LEASE_RENEW: Duration = Duration::from_secs(4);
const LEASE_LOCAL_TIMEOUT: Duration = Duration::from_secs(12);
const START_TIMEOUT: Duration = Duration::from_secs(120);
const PERMISSION_TIMEOUT: Duration = Duration::from_secs(120);
const MUTE_TIMEOUT: Duration = Duration::from_secs(25);

// Returns false for cancellation, including a grant delivered after cancellation.
fn wait_for_permission(
    receiver: &std::sync::mpsc::Receiver<bool>,
    cancelled: &AtomicBool,
    requested_at: Instant,
    timeout: Duration,
) -> Result<bool, String> {
    loop {
        if cancelled.load(Ordering::SeqCst) { return Ok(false); }
        let remaining = timeout.saturating_sub(requested_at.elapsed());
        if remaining.is_zero() {
            return Err("Microphone permission timed out. Start voice again to retry.".into());
        }
        match receiver.recv_timeout(remaining.min(POLL)) {
            Ok(true) => {
                if cancelled.load(Ordering::SeqCst) { return Ok(false); }
                if requested_at.elapsed() >= timeout {
                    return Err("Microphone permission timed out. Start voice again to retry.".into());
                }
                return Ok(true);
            }
            Ok(false) => return Err("Microphone access was denied. Allow it in System Settings to use voice.".into()),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return Err("Microphone permission request stopped.".into()),
        }
    }
}

fn voice_enabled_for_platform(is_macos: bool, setting: Option<&str>) -> bool {
    is_macos && setting != Some("0")
}

fn lease_ack_fresh(requested: Instant, previous: Option<Instant>, now: Instant) -> bool {
    now.saturating_duration_since(requested) < LEASE_LOCAL_TIMEOUT
        && previous.is_none_or(|last| now.saturating_duration_since(last) < LEASE_LOCAL_TIMEOUT)
}

fn active_voice_status(
    starting: bool,
    pending: Option<bool>,
    muted: bool,
    paused: bool,
    speaking: bool,
) -> &'static str {
    if starting {
        "Starting local voice…"
    } else if let Some(muted) = pending {
        if muted {
            "Muting microphone…"
        } else {
            "Preparing microphone…"
        }
    } else if muted && speaking {
        "Speaking · microphone muted"
    } else if muted {
        "Microphone muted"
    } else if paused {
        "Listening…"
    } else if speaking {
        "Speaking · you can interrupt"
    } else {
        "Listening · local voice"
    }
}

fn removes_live_assistant_message(seen: &HashSet<String>, removed: &HashSet<String>) -> bool {
    !seen.is_disjoint(removed)
}

#[cfg(test)]
mod refresh_tests {
    use super::*;

    #[test]
    fn voice_is_enabled_on_macos_unless_explicitly_disabled() {
        assert!(voice_enabled_for_platform(true, None));
        assert!(voice_enabled_for_platform(true, Some("1")));
        assert!(!voice_enabled_for_platform(true, Some("0")));
        assert!(!voice_enabled_for_platform(false, None));
    }

    #[test]
    fn permission_wait_rejects_denial_disconnection_and_cancelled_grant() {
        let cancelled = AtomicBool::new(false);
        let (tx, rx) = std::sync::mpsc::channel();
        tx.send(false).unwrap();
        assert!(wait_for_permission(&rx, &cancelled, Instant::now(), Duration::from_secs(1)).unwrap_err().contains("denied"));
        let (tx, rx) = std::sync::mpsc::channel();
        drop(tx);
        assert!(wait_for_permission(&rx, &cancelled, Instant::now(), Duration::from_secs(1)).unwrap_err().contains("stopped"));
        let (tx, rx) = std::sync::mpsc::channel();
        cancelled.store(true, Ordering::SeqCst);
        tx.send(true).unwrap();
        assert_eq!(wait_for_permission(&rx, &cancelled, Instant::now(), Duration::from_secs(1)), Ok(false));
    }

    #[test]
    fn permission_grant_after_deadline_is_not_accepted() {
        let (tx, rx) = std::sync::mpsc::channel();
        tx.send(true).unwrap();
        let cancelled = AtomicBool::new(false);
        assert!(wait_for_permission(&rx, &cancelled, Instant::now() - Duration::from_secs(2), Duration::from_secs(1)).unwrap_err().contains("timed out"));
    }

    #[test]
    fn explicit_removal_invalidates_only_observed_response() {
        let seen = HashSet::from(["response".to_owned()]);
        assert!(!removes_live_assistant_message(
            &seen,
            &HashSet::from(["other".to_owned()])
        ));
        assert!(removes_live_assistant_message(
            &seen,
            &HashSet::from(["response".to_owned()])
        ));
    }
    #[test]
    fn stale_lease_acknowledgements_cannot_extend_capture() {
        let base = Instant::now();
        assert!(lease_ack_fresh(
            base,
            Some(base),
            base + Duration::from_secs(11)
        ));
        assert!(!lease_ack_fresh(base, None, base + Duration::from_secs(12)));
        assert!(!lease_ack_fresh(
            base + Duration::from_secs(10),
            Some(base),
            base + Duration::from_secs(12)
        ));
    }
    #[test]
    fn muted_speech_is_visible_and_pending_controls_take_precedence() {
        assert_eq!(
            active_voice_status(false, None, true, false, true),
            "Speaking · microphone muted"
        );
        assert_eq!(
            active_voice_status(false, Some(false), true, false, true),
            "Preparing microphone…"
        );
    }
    #[test]
    fn overlapping_refresh_does_not_replay_or_resume_interrupted_speech() {
        let mut policy = ConversationModel::default();
        policy.begin(100, []);
        fn row<'a>(
            id: &'a str,
            text: &'a str,
            complete: bool,
            interrupted: bool,
        ) -> AssistantText<'a> {
            AssistantText {
                id,
                text,
                created_at: 101,
                direct_assistant: true,
                visible_phase: true,
                complete,
                interrupted,
            }
        }
        assert!(policy.observe(row("a", "First. Unfinished", false, false)));
        let first = policy.next_speech().unwrap();
        assert_eq!(first.text, "First.");
        assert!(policy.completed(first.generation));
        assert!(policy.observe(row("a", "First. Unfinished", false, false)));
        assert!(policy.next_speech().is_none());
        assert!(policy.observe(row("a", "First. Unfinished", false, true)));
        assert!(policy.observe(row("a", "First. Unfinished sentence.", true, false)));
        assert!(policy.next_speech().is_none());
    }
}

#[derive(Clone, Debug)]
pub(crate) enum ConversationEvent {
    Starting,
    ShowComposer,
    Changed,
    RefreshRequired,
    UseDraft { task_id: String, text: String },
}

pub(crate) struct ChatConversation {
    service: DesktopService,
    task_id: String,
    client_id: String,
    session_id: String,
    lease: Option<VoiceLease>,
    audio: Option<NativeVoiceSession>,
    policy: ConversationModel,
    observed_ids: Vec<String>,
    live_assistant_ids: HashSet<String>,
    session_started_at: i64,
    starting: bool,
    awaiting_permission: bool,
    running: bool,
    presented: bool,
    composer_visible: bool,
    capture: CapturePolicy,
    working: bool,
    input_level: f32,
    output_level: f32,
    startup_cancel: Option<Arc<AtomicBool>>,
    mute_pending: Option<(bool, Instant)>,
    finishing_input: Option<Instant>,
    run: u64,
    started: Instant,
    helper_started: Option<Instant>,
    elapsed_before: Duration,
    playback_shown: HashSet<u64>,
    last_lease: Instant,
    renewing: bool,
    explicit_control_pending: bool,
    error: Option<String>,
    review_text: Option<String>,
    delivery: Option<String>,
    admissions: VoiceDelivery,
    receipt_polling: bool,
    last_receipt_poll: Instant,
    transcript: Vec<crate::voice_mode_view::VoiceTranscriptLine>,
    stage: Option<Entity<crate::voice_mode_view::VoiceStage>>,
}

impl EventEmitter<ConversationEvent> for ChatConversation {}

impl ChatConversation {
    pub(crate) fn enabled() -> bool {
        voice_enabled_for_platform(cfg!(all(target_os = "macos", target_arch = "aarch64")), std::env::var("MONITTER_LOCAL_VOICE").ok().as_deref())
    }

    pub(crate) fn new(service: DesktopService, task_id: String) -> Self {
        Self {
            service,
            task_id,
            client_id: uuid::Uuid::new_v4().to_string(),
            session_id: String::new(),
            lease: None,
            audio: None,
            policy: ConversationModel::default(),
            observed_ids: vec![],
            live_assistant_ids: HashSet::new(),
            session_started_at: 0,
            starting: false,
            awaiting_permission: false,
            running: false,
            presented: false,
            composer_visible: false,
            capture: CapturePolicy::default(),
            working: false,
            input_level: 0.,
            output_level: 0.,
            startup_cancel: None,
            mute_pending: None,
            finishing_input: None,
            run: 0,
            started: Instant::now(),
            helper_started: None,
            elapsed_before: Duration::ZERO,
            playback_shown: HashSet::new(),
            last_lease: Instant::now(),
            renewing: false,
            explicit_control_pending: false,
            error: None,
            review_text: None,
            delivery: None,
            admissions: VoiceDelivery::default(),
            receipt_polling: false,
            last_receipt_poll: Instant::now(),
            transcript: Vec::new(),
            stage: None,
        }
    }

    pub(crate) fn active(&self) -> bool {
        self.starting || self.running
    }

    pub(crate) fn presented(&self) -> bool {
        self.presented
    }
    pub(crate) fn composer_visible(&self) -> bool {
        self.composer_visible
    }
    pub(crate) fn paused(&self) -> bool {
        self.presented && !self.active() && self.error.is_none()
    }

    pub(crate) fn push_to_talk_enabled(&self) -> bool {
        self.capture.push_mode && self.active()
    }
    pub(crate) fn any_active_elsewhere(cx: &App) -> bool {
        cx.try_global::<VoiceOwner>()
            .is_some_and(|owner| owner.owner.as_ref().and_then(WeakEntity::upgrade).is_some())
    }
    pub(crate) fn set_working(&mut self, working: bool, cx: &mut Context<Self>) {
        if self.working != working {
            self.working = working;
            self.changed(cx);
        }
    }

    pub(crate) fn push_to_talk(&mut self, pressed: bool, cx: &mut Context<Self>) {
        if !self.capture.set_held(pressed) {
            return;
        }
        self.reconcile_capture(cx);
    }

    pub(crate) fn set_context(
        &mut self,
        task_id: &str,
        explicit_control_pending: bool,
        cx: &mut Context<Self>,
    ) {
        if self.task_id != task_id {
            self.end(cx);
            self.task_id = task_id.to_owned();
            self.observed_ids.clear();
            self.review_text = None;
            self.error = None;
        }
        self.explicit_control_pending = explicit_control_pending;
    }

    pub(crate) fn invalidate_removed(&mut self, removed: &HashSet<String>, cx: &mut Context<Self>) {
        if self.active() && removes_live_assistant_message(&self.live_assistant_ids, removed) {
            self.fail(
                "The chat was reloaded. Start voice again to continue.".into(),
                cx,
            );
        }
    }

    pub(crate) fn observe(&mut self, messages: &[Value], _reset: bool, cx: &mut Context<Self>) {
        if !self.active() {
            self.observed_ids = messages
                .iter()
                .filter(|m| m["taskId"].as_str() == Some(self.task_id.as_str()))
                .filter_map(|m| m["id"].as_str().map(str::to_owned))
                .rev()
                .take(512)
                .collect();
            return;
        }
        for row in messages {
            if row["taskId"].as_str() != Some(self.task_id.as_str()) {
                continue;
            }
            let Some(id) = row["id"].as_str() else {
                continue;
            };
            if self.live_assistant_ids.contains(id)
                && (row["role"].as_str() != Some("assistant")
                    || row["senderAgentId"].as_str().is_some_and(|v| !v.is_empty())
                    || row["collaborationId"]
                        .as_str()
                        .is_some_and(|v| !v.is_empty()))
            {
                self.fail(
                    "The chat changed. Start voice again to continue.".into(),
                    cx,
                );
                return;
            }
            let Some(text) = row["text"].as_str() else {
                continue;
            };
            if self.composer_visible
                && row["role"].as_str() == Some("user")
                && !self.observed_ids.iter().any(|old| old == id)
                && !text.trim().is_empty()
            {
                self.transcript
                    .push(crate::voice_mode_view::VoiceTranscriptLine {
                        text: text.to_owned(),
                        is_user: true,
                        partial: false,
                    });
                if self.transcript.len() > 8 {
                    self.transcript.remove(0);
                }
            }
            let before = self.policy.current().map(|s| s.generation);
            let valid = self.policy.observe(AssistantText {
                id,
                text,
                created_at: row["createdAt"].as_i64().unwrap_or(0),
                direct_assistant: row["role"].as_str() == Some("assistant")
                    && row["senderAgentId"].as_str().is_none_or(str::is_empty)
                    && row["collaborationId"].as_str().is_none_or(str::is_empty),
                // Codex agentMessage is user-visible; reasoning arrives as a
                // distinct item type. Phase-unknown rows are read only after
                // completion, never provisionally streamed as commentary.
                visible_phase: matches!(row["phase"].as_str(), Some("commentary" | "final_answer"))
                    || (row["phase"].is_null() && row["streamStatus"].as_str() == Some("complete")),
                complete: row["streamStatus"].as_str() == Some("complete"),
                interrupted: row["streamStatus"].as_str() == Some("interrupted"),
            });
            if let Some(generation) = before.filter(|_| self.policy.current().is_none()) {
                self.audio_command("cancelSpeech", Some(generation), None, None, cx);
            }
            if !valid {
                self.fail(
                    "This voice session reached its response limit. Start voice again to continue."
                        .into(),
                    cx,
                );
                return;
            }
            if row["createdAt"].as_i64().unwrap_or(0) >= self.session_started_at
                && !self.observed_ids.iter().any(|old| old == id)
                && row["role"].as_str() == Some("assistant")
                && row["streamStatus"].as_str() != Some("interrupted")
                && row["senderAgentId"].as_str().is_none_or(str::is_empty)
                && row["collaborationId"].as_str().is_none_or(str::is_empty)
                && (matches!(row["phase"].as_str(), Some("commentary" | "final_answer"))
                    || (row["phase"].is_null() && row["streamStatus"].as_str() == Some("complete")))
            {
                self.live_assistant_ids.insert(id.to_owned());
            }
        }
        for id in messages
            .iter()
            .filter(|m| m["taskId"].as_str() == Some(self.task_id.as_str()))
            .filter_map(|m| m["id"].as_str())
        {
            if !self.observed_ids.iter().any(|old| old == id) {
                self.observed_ids.push(id.to_owned());
            }
        }
        if self.observed_ids.len() > 512 {
            self.observed_ids.drain(..self.observed_ids.len() - 512);
        }
        self.speak_next(cx);
    }

    pub(crate) fn start(&mut self, cx: &mut Context<Self>) {
        if !Self::enabled() || self.active() || self.task_id.is_empty() {
            return;
        }
        // Request from the frontmost GPUI process, on the main thread, only
        // in response to this explicit user action. No lease or helper exists yet.
        let permission = match crate::native_voice_capture::request_microphone_permission() {
            Ok(receiver) => receiver,
            Err(error) => {
                self.error = Some(error);
                self.presented = true;
                self.changed(cx);
                return;
            }
        };
        let permission_requested_at = Instant::now();
        if cx.try_global::<VoiceOwner>().is_none() {
            cx.set_global(VoiceOwner::default());
        }
        let previous = cx
            .global::<VoiceOwner>()
            .owner
            .as_ref()
            .and_then(WeakEntity::upgrade);
        if let Some(owner) = previous.filter(|owner| owner.entity_id() != cx.entity().entity_id()) {
            owner.update(cx, |voice, cx| voice.pause_for_handoff(cx));
        }
        let previous_releases = std::mem::take(&mut cx.global_mut::<VoiceOwner>().releases);
        let serial = cx.global::<VoiceOwner>().serial.clone();
        let cleanup_error = cx.global::<VoiceOwner>().cleanup_error.clone();
        cx.global_mut::<VoiceOwner>().owner = Some(cx.weak_entity());
        self.error = None;
        self.presented = true;
        self.delivery = None;
        self.starting = true;
        self.awaiting_permission = true;
        self.running = false;
        self.started = Instant::now();
        self.run = self.run.wrapping_add(1);
        self.session_id = uuid::Uuid::new_v4().to_string();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        self.session_started_at = now;
        self.live_assistant_ids.clear();
        self.policy.begin(now, self.observed_ids.iter().cloned());
        if self.capture.desired_muted() {
            self.policy.set_muted(true);
        }
        cx.emit(ConversationEvent::Starting);
        self.changed(cx);
        let run = self.run;
        let task_id = self.task_id.clone();
        let client_id = self.client_id.clone();
        let session_id = self.session_id.clone();
        let service = self.service.clone();
        let cancelled = Arc::new(AtomicBool::new(false));
        self.startup_cancel = Some(cancelled.clone());
        let (tx, rx) = futures::channel::oneshot::channel();
        let (ack_tx, ack_rx) = std::sync::mpsc::channel();
        let (permission_tx, permission_rx) = futures::channel::oneshot::channel();
        if let Err(error) = std::thread::Builder::new()
            .name("voice-session-start".into())
            .spawn(move || {
                for release in previous_releases {
                    if let Err(error) = release
                        .recv()
                        .unwrap_or_else(|_| Err("Voice lease release did not finish.".into()))
                    {
                        let _ = tx.send(Err(format!(
                            "Cannot release the previous voice session: {error}"
                        )));
                        return;
                    }
                }
                match wait_for_permission(&permission, &cancelled, permission_requested_at, PERMISSION_TIMEOUT) {
                    Ok(true) => {},
                    Ok(false) => return,
                    Err(error) => { let _ = tx.send(Err(error)); return; }
                }
                let _ = permission_tx.send(());
                let _guard = serial.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(error) = cleanup_error
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .clone()
                {
                    let _ = tx.send(Err(format!(
                        "Cannot release the previous voice session: {error}"
                    )));
                    return;
                }
                if cancelled.load(Ordering::SeqCst) {
                    return;
                }
                let requested_at = Instant::now();
                let result = service
                    .begin_voice_session(&task_id, &client_id)
                    .and_then(|lease| {
                        match NativeVoiceSession::spawn(session_id, lease.epoch.clone()) {
                            Ok(audio) => Ok((lease, audio, requested_at)),
                            Err(error) => {
                                if let Err(error) = service.end_voice_session(&lease) {
                                    *cleanup_error.lock().unwrap_or_else(|e| e.into_inner()) =
                                        Some(error);
                                }
                                Err(error)
                            }
                        }
                    });
                if cancelled.load(Ordering::SeqCst) {
                    if let Ok((lease, _audio, _)) = result {
                        if let Err(error) = service.end_voice_session(&lease) {
                            *cleanup_error.lock().unwrap_or_else(|e| e.into_inner()) = Some(error);
                        }
                    }
                    return;
                }
                let acquired = result.as_ref().ok().map(|(lease, _, _)| lease.clone());
                if tx.send(result).is_err() || !ack_rx.recv().unwrap_or(false) {
                    if let Some(lease) = acquired {
                        if let Err(error) = service.end_voice_session(&lease) {
                            *cleanup_error.lock().unwrap_or_else(|e| e.into_inner()) = Some(error);
                        }
                    }
                }
            })
        {
            self.fail(format!("Cannot start voice: {error}"), cx);
            return;
        }
        let permission_weak = cx.weak_entity();
        cx.spawn(async move |_, cx| {
            if permission_rx.await.is_ok() {
                let _ = permission_weak.update(cx, |voice, cx| {
                    if voice.run == run && voice.starting {
                        voice.awaiting_permission = false;
                        voice.changed(cx);
                    }
                });
            }
        }).detach();
        let weak = cx.weak_entity();
        cx.spawn(async move |_, cx| {
            let result = rx
                .await
                .unwrap_or_else(|_| Err("Voice startup stopped.".into()));
            let mut owned = Some(result);
            let _ = weak.update(cx, |voice, cx| {
                if voice.run != run || !voice.starting {
                    return;
                }
                if owned.as_ref().is_some_and(|result| {
                    result.as_ref().is_ok_and(|(_, _, requested_at)| {
                        !lease_ack_fresh(*requested_at, None, Instant::now())
                    })
                }) {
                    voice.fail(
                        "The chat connection was lost. Voice has stopped.".into(),
                        cx,
                    );
                    return;
                }
                match owned.take().unwrap() {
                    Ok((lease, audio, requested_at)) => {
                        // This result and the permission-status notification are
                        // independent UI tasks. The accepted session itself also
                        // establishes permission, regardless of callback order.
                        voice.awaiting_permission = false;
                        voice.lease = Some(lease);
                        voice.audio = Some(audio);
                        voice.helper_started = Some(Instant::now());
                        voice.last_lease = requested_at;
                        voice.audio_command("load", None, None, None, cx);
                        voice.poll_loop(run, cx);
                    }
                    Err(error) => voice.fail(error, cx),
                }
            });
            let _ = ack_tx.send(owned.is_none());
        })
        .detach();
    }

    fn pause_for_handoff(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Option<std::sync::mpsc::Receiver<Result<(), String>>> {
        if !self.active() {
            return None;
        }
        self.stop_audio(cx);
        None
    }

    fn stop_audio(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Option<std::sync::mpsc::Receiver<Result<(), String>>> {
        self.run = self.run.wrapping_add(1);
        if let Some(cancel) = self.startup_cancel.take() {
            cancel.store(true, Ordering::SeqCst);
        }
        self.audio.take();
        let release = self.lease.take().map(|lease| {
            let service = self.service.clone();
            let serial = cx.global::<VoiceOwner>().serial.clone();
            let cleanup_error = cx.global::<VoiceOwner>().cleanup_error.clone();
            let (tx, rx) = std::sync::mpsc::channel();
            let _ = std::thread::Builder::new()
                .name("voice-session-end".into())
                .spawn(move || {
                    let _guard = serial.lock().unwrap_or_else(|e| e.into_inner());
                    let result = service.end_voice_session(&lease);
                    if let Err(error) = &result {
                        *cleanup_error.lock().unwrap_or_else(|e| e.into_inner()) =
                            Some(error.clone());
                    }
                    let _ = tx.send(result);
                });
            rx
        });
        let pending = self.admissions.review_text();
        if !pending.is_empty() {
            self.review_text = Some(pending);
        }
        if self.active() {
            self.elapsed_before += self.started.elapsed();
        }
        self.policy.end();
        self.live_assistant_ids.clear();
        self.playback_shown.clear();
        self.starting = false;
        self.awaiting_permission = false;
        self.helper_started = None;
        self.running = false;
        self.capture.set_held(false);
        self.input_level = 0.;
        self.output_level = 0.;
        self.mute_pending = None;
        self.finishing_input = None;
        self.renewing = false;
        self.admissions = VoiceDelivery::default();
        self.receipt_polling = false;
        if let Some(release) = release {
            cx.global_mut::<VoiceOwner>().releases.push(release);
        }
        let owns = cx
            .try_global::<VoiceOwner>()
            .and_then(|owner| owner.owner.as_ref())
            .and_then(WeakEntity::upgrade)
            .is_some_and(|owner| owner.entity_id() == cx.entity().entity_id());
        if owns {
            cx.global_mut::<VoiceOwner>().owner = None;
        }
        self.changed(cx);
        None
    }

    pub(crate) fn pause(&mut self, cx: &mut Context<Self>) {
        self.stop_audio(cx);
    }

    pub(crate) fn end(&mut self, cx: &mut Context<Self>) {
        self.stop_audio(cx);
        self.presented = false;
        self.composer_visible = false;
        self.elapsed_before = Duration::ZERO;
        self.changed(cx);
    }

    fn fail(&mut self, error: String, cx: &mut Context<Self>) {
        self.error = Some(error.chars().take(240).collect());
        self.stop_audio(cx);
    }

    fn changed(&self, cx: &mut Context<Self>) {
        cx.emit(ConversationEvent::Changed);
        cx.notify();
    }

    fn audio_command(
        &mut self,
        kind: &str,
        generation: Option<u64>,
        text: Option<&str>,
        muted: Option<bool>,
        cx: &mut Context<Self>,
    ) -> bool {
        let result = self
            .audio
            .as_ref()
            .ok_or_else(|| "Local audio is unavailable.".to_owned())
            .and_then(|audio| audio.command(kind, generation, text, muted));
        if let Err(error) = result {
            self.fail(error, cx);
            false
        } else {
            true
        }
    }

    fn poll_loop(&self, run: u64, cx: &mut Context<Self>) {
        let weak = cx.weak_entity();
        let diagnostics = std::env::var("MONITTER_VOICE_DIAGNOSTICS").as_deref() == Ok("1");
        cx.spawn(async move |_, cx| {
          if diagnostics { eprintln!("voice-controller poll-start"); }
          let mut last_diagnostic = Instant::now();
          loop {
            cx.background_executor().timer(POLL).await;
            let keep = weak
                .update(cx, |voice, cx| {
                    if voice.run != run || !voice.active() {
                        return false;
                    }
                    voice.tick(cx);
                    if diagnostics && last_diagnostic.elapsed() >= Duration::from_secs(5) {
                        eprintln!("voice-controller poll starting={} running={}", voice.starting, voice.running);
                        last_diagnostic = Instant::now();
                    }
                    voice.active()
                })
                .unwrap_or(false);
            if !keep {
                if diagnostics { eprintln!("voice-controller poll-stop"); }
                break;
            }
          }
        })
        .detach();
    }

    fn tick(&mut self, cx: &mut Context<Self>) {
        if self
            .finishing_input
            .is_some_and(|since| since.elapsed() > MUTE_TIMEOUT)
        {
            self.fail(
                "Finishing speech took too long. Check the chat before retrying.".into(),
                cx,
            );
            return;
        }
        if self
            .mute_pending
            .is_some_and(|(_, since)| since.elapsed() > MUTE_TIMEOUT)
        {
            self.fail(
                "The microphone did not respond. Voice has stopped.".into(),
                cx,
            );
            return;
        }
        if self.starting && self.helper_started.is_some_and(|started| started.elapsed() > START_TIMEOUT) {
            self.fail("Local voice took too long to start.".into(), cx);
            return;
        }
        if self.last_lease.elapsed() >= LEASE_LOCAL_TIMEOUT {
            self.fail(
                "The chat connection was lost. Voice has stopped.".into(),
                cx,
            );
            return;
        }
        if !self.renewing && self.last_lease.elapsed() > LEASE_RENEW {
            self.renew(cx);
        }
        if self.admissions.waiting_id().is_some()
            && !self.receipt_polling
            && self.last_receipt_poll.elapsed() > Duration::from_millis(500)
        {
            self.poll_receipt(cx);
        }
        for _ in 0..32 {
            let event = self.audio.as_ref().map(NativeVoiceSession::poll);
            match event {
                Some(Ok(Some(event))) => self.audio_event(event, cx),
                Some(Err(error)) => {
                    self.fail(error, cx);
                    return;
                }
                _ => break,
            }
            if !self.active() {
                return;
            }
        }
        if let Some(generation) = self
            .policy
            .resume_after_false_cue(self.started.elapsed().as_millis() as u64)
        {
            self.audio_command("resumeSpeech", Some(generation), None, None, cx);
            self.changed(cx);
        }
        self.speak_next(cx);
    }

    fn renew(&mut self, cx: &mut Context<Self>) {
        let Some(lease) = self.lease.clone() else {
            return;
        };
        self.renewing = true;
        let service = self.service.clone();
        let run = self.run;
        let (tx, rx) = futures::channel::oneshot::channel();
        if std::thread::Builder::new()
            .name("voice-lease-renew".into())
            .spawn(move || {
                let requested_at = Instant::now();
                let _ = tx.send((service.renew_voice_session(&lease), requested_at));
            })
            .is_err()
        {
            self.fail("Cannot maintain the voice connection.".into(), cx);
            return;
        }
        let weak = cx.weak_entity();
        cx.spawn(async move |_, cx| {
            let result = rx.await;
            let _ = weak.update(cx, |voice, cx| {
                if voice.run != run {
                    return;
                }
                voice.renewing = false;
                match result {
                    Ok((Ok(()), requested_at))
                        if lease_ack_fresh(
                            requested_at,
                            Some(voice.last_lease),
                            Instant::now(),
                        ) =>
                    {
                        voice.last_lease = requested_at
                    }
                    Ok((Ok(()), _)) => voice.fail(
                        "The chat connection was lost. Voice has stopped.".into(),
                        cx,
                    ),
                    Ok((Err(error), _)) => voice.fail(error, cx),
                    Err(_) => voice.fail("Voice connection stopped.".into(), cx),
                }
            });
        })
        .detach();
    }

    fn audio_event(&mut self, event: AudioEvent, cx: &mut Context<Self>) {
        match event.kind.as_str() {
            "ready" => match event.status.as_deref() {
                Some("loaded") if self.starting => {
                    self.audio_command("start", None, None, None, cx);
                }
                Some("started") if self.starting => {
                    if event.voice_processing != Some(true) {
                        self.fail("Voice could not enable echo processing.".into(), cx);
                        return;
                    }
                    self.starting = false;
                    self.running = true;
                    if self.capture.desired_muted() {
                        self.mute_pending = Some((true, Instant::now()));
                        self.audio_command("mute", None, None, Some(true), cx);
                    }
                    self.changed(cx);
                }
                Some("inputFinished") if self.finishing_input.is_some() => {
                    self.finishing_input = None;
                    self.policy.set_muted(true);
                    self.reconcile_capture(cx);
                    self.changed(cx);
                }
                Some(status @ ("muted" | "unmuted")) => {
                    let muted = status == "muted";
                    if self
                        .mute_pending
                        .is_some_and(|(expected, _)| expected == muted)
                    {
                        self.policy.set_muted(muted);
                        self.mute_pending = None;
                        self.reconcile_capture(cx);
                        self.changed(cx);
                    }
                }
                _ => {}
            },
            "audioLevel" => {
                if event.source.as_deref() == Some("input") {
                    self.input_level = event.level.unwrap_or(0.).clamp(0., 1.) as f32;
                    self.changed(cx);
                }
            }
            "inputActivity" => {
                if let Some(active) = event.active {
                    self.policy
                        .input_activity(active, self.started.elapsed().as_millis() as u64);
                }
            }
            "audioProgress" => {
                let Some(generation) = event.generation else {
                    return;
                };
                if self.policy.current().map(|segment| segment.generation) != Some(generation) {
                    return;
                }
                if let Some(level) = event.level {
                    self.output_level = level.clamp(0., 1.) as f32;
                }
                if matches!(
                    event.status.as_deref(),
                    Some("completed" | "cancelled" | "paused")
                ) {
                    self.output_level = 0.;
                }
                if let Some(generation) = event.generation {
                    if event.status.as_deref() == Some("progress")
                        && event.samples_played.unwrap_or(0) > 0
                        && !self.playback_shown.contains(&generation)
                    {
                        if let Some(segment) = self
                            .policy
                            .current()
                            .filter(|segment| segment.generation == generation)
                        {
                            self.transcript
                                .push(crate::voice_mode_view::VoiceTranscriptLine {
                                    text: segment.text.clone(),
                                    is_user: false,
                                    partial: false,
                                });
                            if self.transcript.len() > 8 {
                                self.transcript.remove(0);
                            }
                            self.playback_shown.insert(generation);
                        }
                    }
                    match event.status.as_deref() {
                        Some("paused") => {
                            self.policy
                                .pause(generation, self.started.elapsed().as_millis() as u64);
                        }
                        Some("completed") | Some("cancelled") => {
                            self.policy.completed(generation);
                        }
                        _ => {}
                    }
                    self.changed(cx);
                }
            }
            "transcript" if self.running => {
                let (Some(id), Some(revision), Some(text), Some(final_text)) = (
                    event.utterance_id,
                    event.revision,
                    event.text,
                    event.is_final,
                ) else {
                    return;
                };
                let id = match uuid::Uuid::parse_str(&id) {
                    Ok(id) => id.to_string(),
                    Err(_) => {
                        self.fail(
                            "Local recognition returned an invalid utterance.".into(),
                            cx,
                        );
                        return;
                    }
                };
                self.transcript.retain(|line| !line.partial);
                if !text.trim().is_empty() {
                    self.transcript
                        .push(crate::voice_mode_view::VoiceTranscriptLine {
                            text: text.clone(),
                            is_user: true,
                            partial: !final_text,
                        });
                    if self.transcript.len() > 8 {
                        self.transcript.remove(0);
                    }
                }
                match self.policy.transcript(
                    &id,
                    revision,
                    &text,
                    final_text,
                    self.explicit_control_pending,
                ) {
                    InputDisposition::Submit => {
                        self.stop_speaking(cx);
                        self.policy.await_admission();
                        self.admit(id, text.trim().to_owned(), cx);
                    }
                    InputDisposition::StopSpeaking => self.stop_speaking(cx),
                    InputDisposition::Backchannel => {
                        if let Some(generation) = self.policy.resume() {
                            self.audio_command("resumeSpeech", Some(generation), None, None, cx);
                        }
                    }
                    InputDisposition::Review => {
                        self.review_text = Some(text);
                        self.fail("Review this speech in the composer. Use the existing controls for questions and approvals.".into(), cx);
                    }
                    _ => {}
                }
                self.changed(cx);
            }
            "error" => self.fail(
                event
                    .message
                    .unwrap_or_else(|| "Local voice failed.".into()),
                cx,
            ),
            _ => {}
        }
    }

    fn admit(&mut self, id: String, text: String, cx: &mut Context<Self>) {
        if !self.active() {
            return;
        }
        if !self.admissions.enqueue(id, text.clone()) {
            let pending = self.admissions.review_text();
            self.review_text = Some(if pending.is_empty() {
                text
            } else {
                format!("{pending}\n\n{text}")
            });
            self.fail(
                "Voice is waiting for the chat. Check the chat before resending these messages."
                    .into(),
                cx,
            );
            return;
        }
        self.submit_next(cx);
    }

    fn submit_next(&mut self, cx: &mut Context<Self>) {
        let Some(lease) = self.lease.clone() else {
            return;
        };
        let Some((id, text)) = self.admissions.next_submission() else {
            return;
        };
        self.policy.await_admission();
        self.delivery = Some("Sending to chat…".into());
        let service = self.service.clone();
        let run = self.run;
        let (tx, rx) = futures::channel::oneshot::channel();
        if std::thread::Builder::new()
            .name("voice-utterance-admission".into())
            .spawn(move || {
                let input = VoiceUtterance {
                    lease: lease.clone(),
                    utterance_id: id.clone(),
                    text,
                    is_final: true,
                };
                let result = match service.admit_voice_utterance(input) {
                    Ok(receipt) => Ok(receipt),
                    Err(error) => {
                        match service.read_voice_receipt(&lease.task_id, &lease.client_id, &id) {
                            Ok(Some(receipt)) => Ok(receipt),
                            _ => Err(error),
                        }
                    }
                };
                // Recovery reads only. Never send a second mutation after failure.
                let _ = tx.send(result);
            })
            .is_err()
        {
            self.review_text = Some(self.admissions.review_text());
            self.fail("Cannot send recognized speech to the chat.".into(), cx);
            return;
        }
        let weak = cx.weak_entity();
        cx.spawn(async move |_, cx| {
            let result = rx.await.unwrap_or_else(|_| Err("Speech delivery is not confirmed.".into()));
            let _ = weak.update(cx, |voice, cx| {
                if voice.run != run { return; }
                match result {
                    Ok(receipt) => {
                        voice.admissions.accepted(&receipt.utterance_id);
                        voice.apply_receipt(receipt, cx);
                        voice.submit_next(cx);
                        cx.emit(ConversationEvent::RefreshRequired);
                    }
                    Err(error) => {
                        voice.review_text = Some(voice.admissions.review_text());
                        voice.fail(format!("Delivery is not confirmed. Check the chat before resending. {error}"), cx);
                    }
                }
                voice.changed(cx);
            });
        }).detach();
    }

    fn apply_receipt(&mut self, receipt: VoiceReceipt, cx: &mut Context<Self>) {
        if !self.admissions.contains(&receipt.utterance_id) {
            return;
        }
        match receipt.delivery_status.as_str() {
            "delivered" => {
                let Some(created_at) = receipt.message_created_at else {
                    self.fail("Speech delivery could not be verified.".into(), cx);
                    return;
                };
                self.admissions.delivered(&receipt.utterance_id, created_at);
                self.delivery = Some("Sent to chat".into());
                if self
                    .admissions
                    .take_ready_boundary()
                    .is_some_and(|boundary| !self.policy.admission_delivered(boundary))
                {
                    self.fail(
                        "The spoken response exceeded this session’s limit.".into(),
                        cx,
                    );
                }
            }
            "queued" => self.delivery = Some("Queued · waiting for the agent".into()),
            "steering" => self.delivery = Some("Sending interjection…".into()),
            _ => self.fail(
                "This spoken message was interrupted or edited. Check the chat before continuing."
                    .into(),
                cx,
            ),
        }
        self.changed(cx);
    }

    fn poll_receipt(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.admissions.waiting_id().map(str::to_owned) else {
            return;
        };
        self.receipt_polling = true;
        self.last_receipt_poll = Instant::now();
        let service = self.service.clone();
        let task_id = self.task_id.clone();
        let client_id = self.client_id.clone();
        let run = self.run;
        let (tx, rx) = futures::channel::oneshot::channel();
        if std::thread::Builder::new()
            .name("voice-receipt-status".into())
            .spawn(move || {
                let _ = tx.send(service.read_voice_receipt(&task_id, &client_id, &id));
            })
            .is_err()
        {
            self.fail("Cannot check speech delivery.".into(), cx);
            return;
        }
        let weak = cx.weak_entity();
        cx.spawn(async move |_, cx| {
            let result = rx
                .await
                .unwrap_or_else(|_| Err("Speech delivery check stopped.".into()));
            let _ = weak.update(cx, |voice, cx| {
                if voice.run != run {
                    return;
                }
                voice.receipt_polling = false;
                match result {
                    Ok(Some(receipt)) => voice.apply_receipt(receipt, cx),
                    Ok(None) => voice.fail(
                        "Speech delivery is no longer available. Check the chat.".into(),
                        cx,
                    ),
                    Err(error) => voice.fail(error, cx),
                }
            });
        })
        .detach();
    }

    fn speak_next(&mut self, cx: &mut Context<Self>) {
        if !self.running {
            return;
        }
        if let Some(segment) = self.policy.next_speech() {
            self.audio_command(
                "speak",
                Some(segment.generation),
                Some(&segment.text),
                None,
                cx,
            );
            self.changed(cx);
        }
    }

    fn stop_speaking(&mut self, cx: &mut Context<Self>) {
        let generation = self.policy.current().map(|segment| segment.generation);
        self.policy.cancel_speech();
        if let Some(generation) = generation.filter(|_| self.audio.is_some()) {
            self.audio_command("cancelSpeech", Some(generation), None, None, cx);
        }
        self.changed(cx);
    }

    fn toggle_mute(&mut self, cx: &mut Context<Self>) {
        self.capture.toggle_explicit_mute();
        self.reconcile_capture(cx);
    }

    fn reconcile_capture(&mut self, cx: &mut Context<Self>) {
        if !self.running || self.mute_pending.is_some() || self.finishing_input.is_some() {
            return;
        }
        let desired = self.capture.desired_muted();
        if desired && self.capture.push_mode && !self.capture.explicit_muted && !self.policy.muted()
        {
            self.finishing_input = Some(Instant::now());
            self.audio_command("finishInput", None, None, None, cx);
            self.changed(cx);
        } else {
            self.set_capture_muted(desired, cx);
        }
    }

    fn set_capture_muted(&mut self, muted: bool, cx: &mut Context<Self>) {
        if !self.running || self.mute_pending.is_some() || self.policy.muted() == muted {
            return;
        }
        // Close admission immediately on mute; reopen it only after the helper
        // confirms that recognition is ready again. A slow reset stays muted.
        self.policy.set_muted(true);
        self.mute_pending = Some((muted, Instant::now()));
        self.audio_command("mute", None, None, Some(muted), cx);
        self.changed(cx);
    }

    pub(crate) fn render_voice_pane(
        &mut self,
        agent_name: &str,
        accent: u32,
        palette: Palette,
        activity: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        use crate::voice_mode_view::{VoiceStage, VoiceStageProps, VoiceStageState};
        let state = if !self.active() {
            VoiceStageState::Paused
        } else if self.policy.speaking() {
            VoiceStageState::Speaking
        } else if self.starting
            || self.working
            || self
                .delivery
                .as_deref()
                .is_some_and(|v| v.contains("waiting") || v.contains("Sending"))
        {
            VoiceStageState::Thinking
        } else {
            VoiceStageState::Listening
        };
        let status = self.error.clone().unwrap_or_else(|| {
            if !self.active() {
                "Voice paused".into()
            } else if self.awaiting_permission {
                "Waiting for microphone permission…".into()
            } else if self.finishing_input.is_some() {
                "Finishing your sentence…".into()
            } else if self.capture.push_mode && !self.capture.explicit_muted
                && self.capture.desired_muted() && !self.starting && !self.working
                && !self.policy.speaking() && self.mute_pending.is_none() {
                "Hold Space to talk".into()
            } else if state == VoiceStageState::Thinking
                && !self.starting
                && self.mute_pending.is_none()
            {
                if self.policy.muted() {
                    "Thinking · microphone muted".into()
                } else {
                    "Thinking".into()
                }
            } else {
                active_voice_status(
                    self.starting,
                    self.mute_pending.map(|(muted, _)| muted),
                    self.policy.muted(),
                    self.policy.paused(),
                    self.policy.speaking(),
                )
                .into()
            }
        });
        let props = VoiceStageProps {
                state,
                status,
                agent_name: agent_name.into(),
                accent,
                elapsed_seconds: (self.elapsed_before
                    + if self.active() {
                        self.started.elapsed()
                    } else {
                        Duration::ZERO
                    })
                .as_secs(),
                input_level: self.input_level,
                output_level: self.output_level,
                lines: self.transcript.clone(),
                activity: if self.working {
                    activity.or_else(|| self.delivery.clone())
                } else {
                    None
                },
            };
        let stage = if let Some(stage) = &self.stage {
            stage.update(cx, |stage, cx| stage.set(props, palette, cx));
            stage.clone()
        } else {
            let stage = cx.new(|_| VoiceStage::new(props, palette));
            self.stage = Some(stage.clone());
            stage
        };
        let muted = self.capture.explicit_muted;
        use gpui_kit::assets::IconName;
        let actions = div()
            .flex()
            .items_center()
            .justify_center()
            .flex_shrink_0()
            .pt(px(16.))
            .text_size(px(12.))
            .text_color(rgb(palette.muted))
            .gap_4()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_1()
                    .child(
                        Button::new("voice-toggle-mute-round")
                            .compact()
                            .w(px(44.))
                            .h(px(44.))
                            .rounded_full()
                            .bg(rgb(if muted { 0xcc3d46 } else { palette.soft }))
                            .icon(if muted {
                                IconName::MicOff
                            } else {
                                IconName::Mic
                            })
                            .accessibility_label(if muted {
                                "Unmute microphone"
                            } else {
                                "Mute microphone"
                            })
                            .disabled(!self.running)
                            .on_click(cx.listener(|voice, _, _, cx| voice.toggle_mute(cx))),
                    )
                    .child(if muted { "Unmute" } else { "Mute" }),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_1()
                    .child(
                        Button::new("voice-end-round")
                            .compact()
                            .w(px(56.))
                            .h(px(56.))
                            .rounded_full()
                            .bg(rgb(palette.soft))
                            .icon(IconName::X)
                            .accessibility_label("End voice conversation")
                            .on_click(cx.listener(|voice, _, _, cx| voice.end(cx))),
                    )
                    .child("End"),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_1()
                    .child(
                        Button::new("voice-type-round")
                            .compact()
                            .w(px(44.))
                            .h(px(44.))
                            .rounded_full()
                            .bg(rgb(palette.soft))
                            .icon(IconName::Keyboard)
                            .accessibility_label("Type instead")
                            .on_click(cx.listener(|voice, _, _, cx| {
                                voice.composer_visible = true;
                                cx.emit(ConversationEvent::ShowComposer);
                                voice.changed(cx);
                            })),
                    )
                    .child("Type instead"),
            );
        let mut root = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(stage)
            .child(actions);
        if !self.active() {
            root = root.child(
                Button::new("voice-resume")
                    .outline()
                    .label("Resume")
                    .on_click(cx.listener(|voice, _, _, cx| voice.start(cx))),
            );
        }
        root = root.child(div().flex().justify_center().py(px(12.)).child(
            Button::new("voice-input-mode")
                .ghost()
                .compact()
                .text_size(px(12.))
                .label(if self.capture.push_mode {
                    "Push to talk · hold Space"
                } else {
                    "Open mic"
                })
                .accessibility_label(if self.capture.push_mode { "Push-to-talk mode; switch to open microphone" } else { "Open microphone mode; switch to push-to-talk" })
                .on_click(cx.listener(|voice, _, _, cx| {
                    voice.capture.toggle_mode();
                    voice.reconcile_capture(cx);
                    voice.changed(cx);
                })),
        ));
        if self.review_text.is_some() {
            root = root.child(
                Button::new("voice-use-draft")
                    .outline()
                    .label("Use as draft")
                    .on_click(cx.listener(|voice, _, _, cx| {
                        if let Some(text) = voice.review_text.take() {
                            voice.composer_visible = true;
                            cx.emit(ConversationEvent::ShowComposer);
                            cx.emit(ConversationEvent::UseDraft {
                                task_id: voice.task_id.clone(),
                                text,
                            });
                        }
                        voice.changed(cx);
                    })),
            );
        }
        root.into_any_element()
    }

    pub(crate) fn render_controls(
        &mut self,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let status = if self.awaiting_permission { "Waiting for microphone permission…" } else { active_voice_status(
            self.starting,
            self.mute_pending.map(|(muted, _)| muted),
            self.policy.muted(),
            self.policy.paused(),
            self.policy.speaking(),
        ) };
        let mut root = div()
            .flex()
            .flex_col()
            .gap_2()
            .px_3()
            .py_2()
            .w_full()
            .debug_selector(|| "local-voice-controls".into());
        let mut controls = div().flex().flex_wrap().items_center().gap_2();
        if self.active() {
            controls = controls
                .child(
                    div()
                        .id("voice-session-status")
                        .role(accesskit::Role::Status)
                        .aria_label(status)
                        .text_size(px(12.))
                        .text_color(rgb(palette.muted))
                        .child(status),
                )
                .when(self.running, |row| {
                    row.child(
                        Button::new("voice-toggle-mute")
                            .outline()
                            .disabled(self.mute_pending.is_some())
                            .label(if self.capture.explicit_muted {
                                "Unmute"
                            } else {
                                "Mute"
                            })
                            .accessibility_label(if self.capture.explicit_muted {
                                "Unmute microphone"
                            } else {
                                "Mute microphone"
                            })
                            .on_click(cx.listener(|voice, _, _, cx| voice.toggle_mute(cx))),
                    )
                    .child(
                        Button::new("voice-stop-speaking")
                            .outline()
                            .label("Stop speaking")
                            .accessibility_label("Stop speaking without stopping agent work")
                            .on_click(cx.listener(|voice, _, _, cx| voice.stop_speaking(cx))),
                    )
                })
                .child(
                    Button::new("voice-end-session")
                        .outline()
                        .label("End voice")
                        .accessibility_label("End voice conversation")
                        .on_click(cx.listener(|voice, _, _, cx| voice.end(cx))),
                );
        } else {
            controls = controls.child(
                Button::new("voice-start-session")
                    .outline()
                    .label(if self.presented {
                        "Resume"
                    } else {
                        "Start voice"
                    })
                    .accessibility_label("Start local voice conversation in this chat")
                    .on_click(cx.listener(|voice, _, _, cx| voice.start(cx))),
            );
        }
        root = root.child(controls);
        if let Some(error) = &self.error {
            root = root.child(
                div()
                    .id("voice-session-error")
                    .role(accesskit::Role::Alert)
                    .aria_label(error.clone())
                    .text_size(px(12.))
                    .text_color(rgb(palette.ink))
                    .child(error.clone()),
            );
        } else if !self.policy.partial_text.is_empty() {
            root = root.child(
                div()
                    .id("voice-session-transcript")
                    .role(accesskit::Role::Label)
                    .aria_label(self.policy.partial_text.clone())
                    .text_size(px(12.))
                    .text_color(rgb(palette.muted))
                    .child(self.policy.partial_text.clone()),
            );
        } else if let Some(delivery) = &self.delivery {
            root = root.child(
                div()
                    .id("voice-session-delivery")
                    .role(accesskit::Role::Label)
                    .aria_label(delivery.clone())
                    .text_size(px(12.))
                    .text_color(rgb(palette.muted))
                    .child(delivery.clone()),
            );
        }
        if self.review_text.is_some() {
            root = root.child(
                Button::new("voice-use-draft")
                    .outline()
                    .label("Use as draft")
                    .accessibility_label("Copy recognized speech into the composer for review")
                    .on_click(cx.listener(|voice, _, _, cx| {
                        if let Some(text) = voice.review_text.take() {
                            cx.emit(ConversationEvent::UseDraft {
                                task_id: voice.task_id.clone(),
                                text,
                            });
                        }
                        voice.changed(cx);
                    })),
            );
        }
        root.into_any_element()
    }
}

impl Drop for ChatConversation {
    fn drop(&mut self) {
        self.audio.take();
        if let Some(lease) = self.lease.take() {
            let service = self.service.clone();
            let _ = std::thread::Builder::new()
                .name("voice-drop-lease".into())
                .spawn(move || {
                    let _ = service.end_voice_session(&lease);
                });
        }
    }
}
