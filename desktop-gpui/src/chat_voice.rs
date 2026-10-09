//! Bounded, owner-local voice controls for the native chat composer.
//! Recording starts only after an explicit click; the native recorder remains
//! on GPUI's foreground thread and only bounded WAV bytes reach the worker.

use crate::{design::Palette, native_voice_capture};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use futures::channel::oneshot;
use gpui_kit::{
    assets::IconName,
    component::{
        Icon,
        button::{Button, ButtonVariants},
    },
    prelude::*,
    *,
};
use monitter_lib::experimental_desktop::DesktopService;
use serde_json::json;
use std::{
    sync::mpsc::{Receiver, TryRecvError},
    time::{Duration, Instant},
};

const PERMISSION_TIMEOUT: Duration = Duration::from_secs(45);
const TIMER_STEP: Duration = Duration::from_millis(200);
const TRANSCRIBE_STACK_BYTES: usize = 8 * 1024 * 1024;
const STATUS_MAX_CHARS: usize = 180;

#[derive(Clone, Debug)]
pub(crate) enum VoiceComposerEvent {
    TranscriptReady {
        task_id: String,
        draft: String,
        text: String,
    },
    StatusChanged,
}

enum VoiceState {
    Idle,
    AwaitingPermission {
        receiver: Receiver<bool>,
        requested_at: Instant,
        task_id: String,
        draft: String,
    },
    Recording {
        task_id: String,
        draft: String,
        elapsed_seconds: f64,
    },
    Transcribing,
}

pub(crate) struct VoiceComposer {
    service: DesktopService,
    task_id: String,
    draft: String,
    engine: String,
    state: VoiceState,
    capture: Option<native_voice_capture::NativeVoiceCapture>,
    generation: u64,
    error: Option<String>,
}

impl EventEmitter<VoiceComposerEvent> for VoiceComposer {}

impl VoiceComposer {
    pub(crate) fn new(service: DesktopService, _cx: &mut Context<Self>) -> Self {
        Self {
            service,
            task_id: String::new(),
            draft: String::new(),
            engine: "whisper".into(),
            state: VoiceState::Idle,
            capture: None,
            generation: 0,
            error: None,
        }
    }

    /// Draft changes do not notify: the parent already redraws for editor
    /// changes. Task changes invalidate permission, recording and replies.
    pub(crate) fn set_context(
        &mut self,
        task_id: impl Into<String>,
        draft: impl Into<String>,
        cx: &mut Context<Self>,
    ) {
        let task_id = task_id.into();
        let changed_task = self.task_id != task_id;
        let draft = draft.into();
        let changed_draft = self.draft != draft;
        if !changed_task && !changed_draft {
            return;
        }
        if changed_task {
            self.generation = self.generation.wrapping_add(1);
            self.cancel_capture();
            self.error = None;
            self.task_id = task_id;
        }
        self.draft = draft;
        if changed_task {
            self.changed(cx);
        }
    }

    /// Only the existing Whisper owner IPC is supported. Engine changes
    /// invalidate any pending result and immediately release a live recorder.
    pub(crate) fn set_engine(&mut self, engine: &str, cx: &mut Context<Self>) {
        let engine = if engine == "apple" {
            "apple"
        } else {
            "whisper"
        };
        if self.engine == engine {
            return;
        }
        self.engine = engine.into();
        self.generation = self.generation.wrapping_add(1);
        self.cancel_capture();
        self.error = (engine == "apple").then(|| {
            "Apple on-device transcription is not available in the GPUI desktop yet.".into()
        });
        self.changed(cx);
    }

    pub(crate) fn start(&mut self, cx: &mut Context<Self>) {
        if !matches!(&self.state, VoiceState::Idle) {
            return;
        }
        if self.task_id.is_empty() {
            self.error = Some("Choose a chat before recording.".into());
            self.changed(cx);
            return;
        }
        if self.engine == "apple" {
            self.error = Some(
                "Apple on-device transcription is not available in the GPUI desktop yet.".into(),
            );
            self.changed(cx);
            return;
        }
        // Freeze attribution at the Record click, before macOS can show its
        // permission prompt and the user can edit the composer.
        let task_id = self.task_id.clone();
        let draft = self.draft.clone();
        match native_voice_capture::request_microphone_permission() {
            Ok(receiver) => {
                self.generation = self.generation.wrapping_add(1);
                self.state = VoiceState::AwaitingPermission {
                    receiver,
                    requested_at: Instant::now(),
                    task_id,
                    draft,
                };
                self.error = None;
                self.spawn_permission_poll(self.generation, cx);
            }
            Err(error) => self.error = Some(error),
        }
        self.changed(cx);
    }

    pub(crate) fn stop(&mut self, cx: &mut Context<Self>) {
        let VoiceState::Recording { task_id, draft, .. } = &self.state else {
            return;
        };
        self.finish_recording(task_id.clone(), draft.clone(), cx);
    }

    /// During transcription this discards the eventual insertion; the owner
    /// IPC has no cancellation command, so its bounded worker may finish.
    pub(crate) fn cancel(&mut self, cx: &mut Context<Self>) {
        self.generation = self.generation.wrapping_add(1);
        self.cancel_capture();
        self.error = None;
        self.changed(cx);
    }

    fn changed(&mut self, cx: &mut Context<Self>) {
        cx.emit(VoiceComposerEvent::StatusChanged);
        cx.notify();
    }

    fn cancel_capture(&mut self) {
        self.state = VoiceState::Idle;
        if let Some(capture) = self.capture.take() {
            capture.cancel();
        }
    }

    pub(crate) fn status(&self) -> Option<String> {
        let message = if let Some(error) = &self.error {
            Some(error.as_str())
        } else if self.engine == "apple" {
            Some("Apple on-device transcription is not available in the GPUI desktop yet.")
        } else {
            None
        }?;
        Some(message.chars().take(STATUS_MAX_CHARS).collect())
    }

    fn spawn_permission_poll(&self, generation: u64, cx: &mut Context<Self>) {
        let weak = cx.weak_entity();
        cx.spawn(async move |_, cx| {
            loop {
                cx.background_executor().timer(TIMER_STEP).await;
                let finished = weak
                    .update(cx, |composer, cx| composer.poll_permission(generation, cx))
                    .unwrap_or(true);
                if finished {
                    break;
                }
            }
        })
        .detach();
    }

    fn poll_permission(&mut self, generation: u64, cx: &mut Context<Self>) -> bool {
        if self.generation != generation {
            return true;
        }
        let outcome = match &self.state {
            VoiceState::AwaitingPermission {
                receiver,
                requested_at,
                ..
            } => match receiver.try_recv() {
                Ok(granted) => Some(Ok(granted)),
                Err(TryRecvError::Disconnected) => Some(Err(
                    "The microphone permission request ended unexpectedly.".to_string(),
                )),
                Err(TryRecvError::Empty) if requested_at.elapsed() >= PERMISSION_TIMEOUT => {
                    Some(Err("Microphone permission request timed out.".to_string()))
                }
                Err(TryRecvError::Empty) => None,
            },
            _ => return true,
        };
        let Some(outcome) = outcome else {
            return false;
        };
        let (task_id, draft) = match &self.state {
            VoiceState::AwaitingPermission { task_id, draft, .. } => {
                (task_id.clone(), draft.clone())
            }
            _ => return true,
        };
        match outcome {
            Ok(true) if self.engine == "whisper" && self.task_id == task_id => {
                match native_voice_capture::NativeVoiceCapture::begin() {
                    Ok(capture) => {
                        self.capture = Some(capture);
                        self.state = VoiceState::Recording {
                            task_id,
                            draft,
                            elapsed_seconds: 0.,
                        };
                        self.error = None;
                        self.spawn_recording_tick(generation, cx);
                    }
                    Err(error) => {
                        self.state = VoiceState::Idle;
                        self.error = Some(error);
                    }
                }
            }
            Ok(true) => {
                self.state = VoiceState::Idle;
            }
            Ok(false) => {
                self.state = VoiceState::Idle;
                self.error = Some(
                    "Microphone access was denied. Check macOS Privacy & Security settings.".into(),
                );
            }
            Err(error) => {
                self.state = VoiceState::Idle;
                self.error = Some(error);
            }
        }
        self.changed(cx);
        true
    }

    fn spawn_recording_tick(&self, generation: u64, cx: &mut Context<Self>) {
        let weak = cx.weak_entity();
        cx.spawn(async move |_, cx| {
            loop {
                cx.background_executor().timer(TIMER_STEP).await;
                let keep_running = weak
                    .update(cx, |composer, cx| composer.tick_recording(generation, cx))
                    .unwrap_or(false);
                if !keep_running {
                    break;
                }
            }
        })
        .detach();
    }

    fn tick_recording(&mut self, generation: u64, cx: &mut Context<Self>) -> bool {
        if self.generation != generation || !matches!(&self.state, VoiceState::Recording { .. }) {
            return false;
        }
        let Some(capture) = self.capture.as_ref() else {
            self.state = VoiceState::Idle;
            self.error = Some("The microphone recorder is unavailable.".into());
            self.changed(cx);
            return false;
        };
        let status = capture.status();
        if !status.recording || status.elapsed_seconds >= 90. {
            let (task_id, draft) = match &self.state {
                VoiceState::Recording { task_id, draft, .. } => (task_id.clone(), draft.clone()),
                _ => return false,
            };
            self.finish_recording(task_id, draft, cx);
            return false;
        }
        if let VoiceState::Recording {
            elapsed_seconds, ..
        } = &mut self.state
        {
            *elapsed_seconds = status.elapsed_seconds;
        }
        cx.notify();
        true
    }

    fn finish_recording(&mut self, task_id: String, draft: String, cx: &mut Context<Self>) {
        let Some(capture) = self.capture.take() else {
            self.state = VoiceState::Idle;
            self.changed(cx);
            return;
        };
        self.state = VoiceState::Transcribing;
        self.error = None;
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        let weak = cx.weak_entity();
        let service = self.service.clone();
        let wav = match capture.stop() {
            Ok(wav) => wav,
            Err(error) => {
                self.state = VoiceState::Idle;
                self.error = Some(error);
                self.changed(cx);
                return;
            }
        };
        let payload = json!({"audioBase64": STANDARD.encode(wav)});
        let (sender, receiver) = oneshot::channel::<Result<String, String>>();
        let worker = std::thread::Builder::new()
            .name("monitter-voice-transcription".into())
            .stack_size(TRANSCRIBE_STACK_BYTES)
            .spawn(move || {
                let result = service
                    .invoke("transcribe_voice_message", payload)
                    .and_then(|response| {
                        response["text"]
                            .as_str()
                            .map(str::to_owned)
                            .ok_or_else(|| "Transcription service returned no text.".to_string())
                    })
                    .and_then(|text| {
                        let text = text.trim().to_owned();
                        if text.is_empty() {
                            Err("No speech was recognized.".into())
                        } else {
                            Ok(text)
                        }
                    });
                let _ = sender.send(result);
            });
        if let Err(error) = worker {
            self.state = VoiceState::Idle;
            self.error = Some(format!("Could not start transcription worker: {error}"));
            self.changed(cx);
            return;
        }
        cx.spawn(async move |_, cx| {
            let result = receiver.await.unwrap_or_else(|_| {
                Err("Transcription worker stopped before returning a result.".into())
            });
            let _ = weak.update(cx, |composer, cx| {
                if composer.generation != generation {
                    return;
                }
                composer.state = VoiceState::Idle;
                match result {
                    Ok(text) => cx.emit(VoiceComposerEvent::TranscriptReady {
                        task_id,
                        draft,
                        text,
                    }),
                    Err(error) => composer.error = Some(error),
                }
                composer.changed(cx);
            });
        })
        .detach();
        self.changed(cx);
    }

    pub(crate) fn render_controls(
        &mut self,
        _palette: Palette,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut root = div().flex().items_center().gap_2();
        match &self.state {
            VoiceState::Idle => {
                root = root.child(
                    Button::new("record-voice-message")
                        .compact()
                        .outline()
                        .icon(Icon::new(IconName::Mic))
                        .accessibility_label("Record voice message")
                        .tooltip("Record a voice message for local transcription")
                        .on_click(cx.listener(|voice, _, _, cx| voice.start(cx))),
                )
            }
            VoiceState::AwaitingPermission { .. } => {
                root = root.child(
                    Button::new("cancel-voice-permission")
                        .compact()
                        .outline()
                        .icon(Icon::new(IconName::X))
                        .accessibility_label("Cancel microphone permission request")
                        .tooltip("Cancel recording")
                        .on_click(cx.listener(|voice, _, _, cx| voice.cancel(cx))),
                )
            }
            VoiceState::Recording {
                elapsed_seconds, ..
            } => {
                root = root
                    .child(div().child(format!(
                        "{:02}:{:02}",
                        *elapsed_seconds as u64 / 60,
                        *elapsed_seconds as u64 % 60
                    )))
                    .child(
                        Button::new("stop-voice-recording")
                            .compact()
                            .outline()
                            .icon(Icon::new(IconName::Square))
                            .accessibility_label("Stop and transcribe voice recording")
                            .tooltip("Stop recording")
                            .on_click(cx.listener(|voice, _, _, cx| voice.stop(cx))),
                    )
                    .child(
                        Button::new("cancel-voice-recording")
                            .compact()
                            .outline()
                            .icon(Icon::new(IconName::X))
                            .accessibility_label("Cancel voice recording")
                            .tooltip("Discard recording")
                            .on_click(cx.listener(|voice, _, _, cx| voice.cancel(cx))),
                    );
            }
            VoiceState::Transcribing => {
                root = root.child(
                    Button::new("discard-voice-transcript")
                        .compact()
                        .outline()
                        .icon(Icon::new(IconName::X))
                        .accessibility_label("Discard transcript when transcription finishes")
                        .tooltip("Discard transcript when transcription finishes")
                        .on_click(cx.listener(|voice, _, _, cx| voice.cancel(cx))),
                )
            }
        }
        root.into_any_element()
    }
}

impl Drop for VoiceComposer {
    fn drop(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.cancel_capture();
    }
}
