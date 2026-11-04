//! Private child-process transport for the local Swift audio engine. The helper
//! receives speech text and an opaque lease epoch, never service credentials,
//! workspace paths, task tools, approval authority or backend access.

use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    fs,
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex, mpsc::{self, SyncSender}},
};

const MAX_LINE_BYTES: usize = 16 * 1024;
const COMMAND_QUEUE: usize = 16;
const EVENT_QUEUE: usize = 64;

fn voice_diagnostics_enabled() -> bool {
    std::env::var("MONITTER_VOICE_DIAGNOSTICS").as_deref() == Ok("1")
}

// Forward only the helper's fixed stage names and validated numeric/boolean
// fields. Framework diagnostics and arbitrary stderr may contain user data.
fn safe_helper_diagnostic(line: &[u8]) -> Option<String> {
    let mut fields = std::str::from_utf8(line).ok()?.split_whitespace();
    if fields.next()? != "voice-stage" { return None; }
    let time = fields.next()?;
    if !time.bytes().all(|c| c.is_ascii_digit() || c == b'.') || time.parse::<f64>().ok()?.is_sign_negative() { return None; }
    let stage = fields.next()?;
    if !matches!(stage,
        "load-command" | "asr-load-begin" | "asr-load-end" | "tts-load-begin" | "tts-load-end"
        | "asr-warm-begin" | "asr-warm-end" | "asr-reset-begin" | "asr-reset-end"
        | "load-complete" | "load-ready-written" | "capture-start-command" | "capture-started"
        | "permission-check-begin" | "permission-authorized" | "permission-request-begin"
        | "permission-request-granted" | "permission-request-denied" | "permission-denied"
        | "capture-lock-begin" | "capture-lock-acquired" | "engine-create-begin" | "engine-create-end"
        | "voice-processing-begin" | "voice-processing-end" | "engine-start-begin" | "engine-start-end"
        | "mute-drain-await-input" | "mute-drain-input-done" | "mute-drain-reset-done"
        | "unmute-drain-done" | "unmute-timeout" | "input-task-exit" | "asr-normalize-begin"
        | "asr-process-begin" | "asr-process-end" | "asr-process-cancelled"
        | "route-notification" | "capture-check" | "input-metrics" | "input-overflow"
    ) { return None; }
    let remaining: Vec<_> = fields.collect();
    if stage == "route-notification" {
        if remaining.len() != 3 || !remaining.iter().zip(["running=", "snapshotMatch=", "stop="]).all(|(field, prefix)| field.strip_prefix(prefix).is_some_and(|value| matches!(value, "true" | "false"))) { return None; }
    } else if stage == "capture-check" {
        if remaining.len() != 4 || !safe_numeric_fields(&remaining[..2], &["taps=", "accepted="])
            || !remaining[2..].iter().zip(["running=", "muted="]).all(|(field, key)| field.strip_prefix(key).is_some_and(|v| matches!(v, "true" | "false"))) { return None; }
    } else if stage == "input-metrics" {
        if !safe_numeric_fields(&remaining, &["rate=", "frames=", "queuedSeconds=", "oldestSeconds=", "count=", "processSeconds="]) { return None; }
    } else if stage == "input-overflow" {
        if !remaining.first().and_then(|value| value.strip_prefix("reason=")).is_some_and(|reason| matches!(reason, "invalidFormat" | "count" | "bytes" | "duration" | "age"))
            || !safe_numeric_fields(&remaining[1..], &["queuedSeconds=", "oldestSeconds=", "count=", "bytes=", "frames=", "rate=", "channels="]) { return None; }
    } else if !remaining.is_empty() { return None; }
    Some(format!("voice-stage {time} {stage}{}", if remaining.is_empty() { String::new() } else { format!(" {}", remaining.join(" ")) }))
}

fn safe_numeric_fields(fields: &[&str], keys: &[&str]) -> bool {
    fields.len() == keys.len() && fields.iter().zip(keys).all(|(field, key)| {
        field.strip_prefix(key).is_some_and(|value| {
            if matches!(*key, "count=" | "bytes=" | "frames=" | "channels=" | "taps=" | "accepted=") {
                !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()) && value.parse::<u64>().is_ok()
            } else {
                value.parse::<f64>().is_ok_and(|number| number.is_finite() && number >= 0.)
            }
        })
    })
}

fn drain_helper_diagnostics(reader: &mut impl BufRead, mut emit: impl FnMut(String)) {
    loop {
        match read_line_bounded(reader) {
            Ok(Some(line)) => {
                if let Some(safe) = safe_helper_diagnostic(&line) { emit(safe); }
            }
            Ok(None) => break,
            Err(_) => {
                // Framework stderr is not our event protocol. Discard an
                // oversized line without closing the pipe or losing later
                // diagnostics. The retained buffer stays bounded.
                match reader.skip_until(b'\n') {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
            }
        }
    }
}

fn audio_progress_diagnostic(event: &AudioEvent) -> Option<String> {
    if event.kind != "audioProgress" { return None; }
    let status = event.status.as_deref()?;
    if !matches!(status, "started" | "paused" | "resumed" | "completed" | "cancelled" | "alreadyFinished" | "generated") { return None; }
    Some(format!("voice-audio-progress status={status} generation={} samplesPlayed={}", event.generation.map_or_else(|| "none".into(), |v| v.to_string()), event.samples_played.map_or_else(|| "none".into(), |v| v.to_string())))
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AudioEvent {
    #[serde(rename = "type")]
    pub kind: String,
    pub session_id: String,
    pub epoch: String,
    pub request_id: Option<String>,
    pub status: Option<String>,
    pub generation: Option<u64>,
    pub utterance_id: Option<String>,
    pub revision: Option<u64>,
    pub text: Option<String>,
    pub is_final: Option<bool>,
    pub active: Option<bool>,
    pub level: Option<f64>,
    pub source: Option<String>,
    pub voice_processing: Option<bool>,
    pub samples_played: Option<u64>,
    pub code: Option<String>,
    pub message: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AudioCommand<'a> {
    #[serde(rename = "type")]
    kind: &'a str,
    session_id: &'a str,
    epoch: &'a str,
    request_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    generation: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    muted: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    models_path: Option<String>,
}

// The stdout reader must keep draining even while GPUI is temporarily busy.
// Blocking it on a full channel can block the helper's synchronous event writer
// on the audio actor, leaving microphone buffers unprocessed. Keep only the
// newest meter, playback position and provisional text, while retaining ordered
// control edges and final transcripts. Memory remains bounded at EVENT_QUEUE.
#[derive(Default)]
struct EventInbox {
    pending: VecDeque<AudioEvent>,
    closed: Option<String>,
}

impl AudioEvent {
    fn replaceable(&self) -> bool {
        (self.kind == "audioLevel" && self.source.is_some())
            || (self.kind == "audioProgress" && self.status.as_deref() == Some("progress") && self.generation.is_some())
            || (self.kind == "transcript" && self.is_final == Some(false) && self.utterance_id.is_some())
    }

    fn superseded_by(&self, newer: &Self) -> bool {
        self.replaceable() && self.session_id == newer.session_id && self.epoch == newer.epoch && self.kind == newer.kind
            && match self.kind.as_str() {
                "audioLevel" => self.source == newer.source,
                "audioProgress" => newer.status.as_deref() == Some("progress") && self.generation == newer.generation,
                // A final replaces the queued provisional text, never the other
                // way around. Other utterances remain in chronological order.
                "transcript" => self.utterance_id == newer.utterance_id,
                _ => false,
            }
    }
}

impl EventInbox {
    fn push(&mut self, event: AudioEvent) -> bool {
        if self.closed.is_some() { return false; }
        self.pending.retain(|old| !old.superseded_by(&event));
        if self.pending.len() == EVENT_QUEUE {
            if let Some(index) = self.pending.iter().position(AudioEvent::replaceable) {
                self.pending.remove(index);
            } else if event.replaceable() {
                // A meter/partial update must never displace a final or command
                // acknowledgement, nor stop the helper's pipe from draining.
                return true;
            } else {
                self.close("Voice controls fell behind. Voice has stopped.".into());
                return false;
            }
        }
        self.pending.push_back(event);
        true
    }

    fn close(&mut self, error: String) {
        if self.closed.is_none() { self.closed = Some(error); }
    }

    fn poll(&mut self) -> Result<Option<AudioEvent>, String> {
        if let Some(event) = self.pending.pop_front() { return Ok(Some(event)); }
        match &self.closed {
            Some(error) => Err(error.clone()),
            None => Ok(None),
        }
    }
}

pub(crate) struct NativeVoiceSession {
    child: Option<Child>,
    commands: Option<SyncSender<Vec<u8>>>,
    events: Arc<Mutex<EventInbox>>,
    session_id: String,
    epoch: String,
}

fn voice_models_path(override_path: Option<std::ffi::OsString>, home: Option<std::ffi::OsString>) -> Result<PathBuf, String> {
    let path = match override_path {
        Some(path) => PathBuf::from(path),
        None => PathBuf::from(home.ok_or("Cannot locate the local voice model cache.")?)
            .join("Library/Caches/Monitter/VoiceModels"),
    };
    if !path.is_absolute() {
        return Err("Local voice model directory must be an absolute path.".into());
    }
    Ok(path)
}

fn configured_voice_models() -> Result<PathBuf, String> {
    let path = voice_models_path(std::env::var_os("MONITTER_VOICE_MODELS"), std::env::var_os("HOME"))?;
    if !path.join("parakeet-eou-320").is_dir()
        || !path.join("phonon-31m-en-v0.6.0/phonon-31m-en").is_dir()
    {
        return Err("Local voice models are missing. Run the local voice model setup first.".into());
    }
    Ok(path)
}

fn executable_path() -> Result<PathBuf, String> {
    #[cfg(not(target_os = "macos"))]
    return Err("Local conversation is available on macOS.".into());
    #[cfg(target_os = "macos")]
    {
        let path = if let Some(path) = std::env::var_os("MONITTER_NATIVE_VOICE_HELPER") {
            PathBuf::from(path)
        } else {
            std::env::current_exe()
                .map_err(|_| "Cannot locate local voice.")?
                .parent()
                .ok_or("Cannot locate local voice.")?
                .join("monitter-native-voice")
        };
        validate_executable(&path)?;
        Ok(path)
    }
}

fn validate_executable(path: &Path) -> Result<(), String> {
    if !path.is_absolute() {
        return Err("Local voice helper path must be absolute.".into());
    }
    let metadata = fs::symlink_metadata(path).map_err(
        |_| "Local voice helper is missing. Reinstall Monitter with local voice support.",
    )?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err("Local voice helper is not a regular executable.".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = metadata.permissions().mode();
        if mode & 0o111 == 0 || mode & 0o022 != 0 {
            return Err(
                "Local voice helper must be executable and not writable by other users.".into(),
            );
        }
    }
    Ok(())
}

impl NativeVoiceSession {
    /// Blocking process setup: call on a worker, not the GPUI foreground thread.
    /// Creating the child does not open a microphone. `start` is a separate,
    /// explicit command after the backend lease and warmed models are ready.
    pub(crate) fn spawn(session_id: String, epoch: String) -> Result<Self, String> {
        Self::spawn_at(&executable_path()?, session_id, epoch)
    }

    fn spawn_at(path: &Path, session_id: String, epoch: String) -> Result<Self, String> {
        validate_executable(path)?;
        uuid::Uuid::parse_str(&session_id).map_err(|_| "Invalid audio session.")?;
        uuid::Uuid::parse_str(&epoch).map_err(|_| "Invalid audio epoch.")?;
        let mut command = Command::new(path);
        let diagnostics = voice_diagnostics_enabled();
        command
            .env_clear()
            .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(if diagnostics { Stdio::piped() } else { Stdio::null() });
        if diagnostics { command.env("MONITTER_VOICE_DIAGNOSTICS", "1"); }
        // Swift/Foundation cache and temporary-file discovery needs these. No
        // cloud-provider tokens or arbitrary parent environment are inherited.
        for key in ["HOME", "TMPDIR", "LANG"] {
            if let Some(value) = std::env::var_os(key) {
                command.env(key, value);
            }
        }
        let mut child = command
            .spawn()
            .map_err(|e| format!("Cannot start local audio: {e}"))?;
        let mut stdin = child
            .stdin
            .take()
            .ok_or("Local audio input pipe is unavailable.")?;
        let stdout = child
            .stdout
            .take()
            .ok_or("Local audio output pipe is unavailable.")?;
        if let Some(stderr) = child.stderr.take() {
            if let Err(error) = std::thread::Builder::new().name("voice-diagnostic-reader".into()).spawn(move || {
                let mut reader = BufReader::new(stderr);
                drain_helper_diagnostics(&mut reader, |safe| eprintln!("{safe}"));
            }) {
                let _ = child.kill(); let _ = child.wait();
                return Err(format!("Cannot read local audio diagnostics: {error}"));
            }
        }
        let (commands, receiver) = mpsc::sync_channel::<Vec<u8>>(COMMAND_QUEUE);
        let events = Arc::new(Mutex::new(EventInbox::default()));
        let writer_errors = events.clone();
        if let Err(error) = std::thread::Builder::new()
            .name("voice-command-writer".into())
            .spawn(move || {
                while let Ok(bytes) = receiver.recv() {
                    if stdin.write_all(&bytes).and_then(|_| stdin.flush()).is_err() {
                        if let Ok(mut inbox) = writer_errors.lock() {
                            inbox.close("Local audio stopped accepting commands.".into());
                        }
                        break;
                    }
                }
            })
        {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("Cannot start local audio transport: {error}"));
        }
        let reader_events = events.clone();
        let expected_session = session_id.clone();
        let expected_epoch = epoch.clone();
        if let Err(error) = std::thread::Builder::new()
            .name("voice-event-reader".into())
            .spawn(move || {
                let mut reader = BufReader::new(stdout);
                loop {
                    match read_line_bounded(&mut reader) {
                        Ok(Some(bytes)) => {
                            match serde_json::from_slice::<AudioEvent>(&bytes) {
                                Ok(event) => {
                                    if diagnostics && event.kind == "ready" && event.status.as_deref().is_some_and(|s| matches!(s, "loaded" | "started" | "stopped" | "muted" | "unmuted" | "inputFinished")) {
                                        eprintln!("voice-transport ready={} identityMatch={}", event.status.as_deref().unwrap(), event.session_id == expected_session && event.epoch == expected_epoch);
                                    }
                                    if event.session_id != expected_session || event.epoch != expected_epoch { continue; }
                                    let Ok(mut inbox) = reader_events.lock() else { break; };
                                    if !inbox.push(event) { break; }
                                }
                                Err(_) => {
                                    if let Ok(mut inbox) = reader_events.lock() { inbox.close("Local audio returned an invalid event.".into()); }
                                    break;
                                }
                            }
                        }
                        Ok(None) => {
                            if let Ok(mut inbox) = reader_events.lock() { inbox.close("Local audio stopped.".into()); }
                            break;
                        }
                        Err(error) => {
                            if let Ok(mut inbox) = reader_events.lock() { inbox.close(error); }
                            break;
                        }
                    }
                }
            })
        {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("Cannot read local audio events: {error}"));
        }
        Ok(Self {
            child: Some(child),
            commands: Some(commands),
            events,
            session_id,
            epoch,
        })
    }

    pub(crate) fn command(
        &self,
        kind: &str,
        generation: Option<u64>,
        text: Option<&str>,
        muted: Option<bool>,
    ) -> Result<(), String> {
        let models_path = if kind == "load" {
            let path = configured_voice_models()?;
            Some(
                path.to_str()
                    .ok_or("Local voice model path is invalid.")?
                    .to_owned(),
            )
        } else {
            None
        };
        let mut bytes = serde_json::to_vec(&AudioCommand {
            kind,
            session_id: &self.session_id,
            epoch: &self.epoch,
            request_id: uuid::Uuid::new_v4().to_string(),
            generation,
            text,
            muted,
            models_path,
        })
        .map_err(|_| "Cannot encode local audio command.")?;
        if bytes.len() > MAX_LINE_BYTES {
            return Err("Local audio command is too large.".into());
        }
        bytes.push(b'\n');
        self.commands
            .as_ref()
            .ok_or("Local audio has ended.")?
            .try_send(bytes)
            .map_err(|_| "Local audio is not responding. Voice has stopped.".into())
    }

    pub(crate) fn poll(&self) -> Result<Option<AudioEvent>, String> {
        // Discard stale events even if an old helper flushes its pipe after the
        // native controller has already moved to another session or epoch.
        for _ in 0..EVENT_QUEUE {
            let next = self.events.lock().map_err(|_| "Local audio event reader failed.".to_owned())?.poll();
            match next {
                Ok(Some(event))
                    if event.session_id == self.session_id && event.epoch == self.epoch =>
                {
                    if voice_diagnostics_enabled() {
                        if event.kind == "ready" && event.status.as_deref().is_some_and(|s| matches!(s, "loaded" | "started" | "stopped" | "muted" | "unmuted" | "inputFinished")) {
                            eprintln!("voice-audio-ready status={}", event.status.as_deref().unwrap());
                        }
                        if let Some(line) = audio_progress_diagnostic(&event) { eprintln!("{line}"); }
                        if event.kind == "error" && event.code.as_deref().is_some_and(|code| matches!(code,
                            "inputOverflow" | "inputUnavailable" | "asrFailed" | "inputFinishFailed" | "inputFinishTimeout" | "recognizerBusy" | "synthesisFailed" | "routeChanged" | "loadFailed" | "startFailed"
                        )) {
                            eprintln!("voice-audio-error code={}", event.code.as_deref().unwrap());
                        }
                    }
                    return Ok(Some(event));
                }
                Ok(Some(_)) => continue,
                Err(error) => return Err(error),
                Ok(None) => return Ok(None),
            }
        }
        Ok(None)
    }
}

impl Drop for NativeVoiceSession {
    fn drop(&mut self) {
        // A hung model cannot keep capture or playback alive after End, a task
        // switch or a dropped window. Reaping never blocks the UI thread.
        self.commands.take();
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = std::thread::Builder::new()
                .name("voice-helper-reaper".into())
                .spawn(move || {
                    let _ = child.wait();
                });
        }
    }
}

fn read_line_bounded(reader: &mut impl BufRead) -> Result<Option<Vec<u8>>, String> {
    let mut bytes = Vec::new();
    loop {
        let available = reader
            .fill_buf()
            .map_err(|_| "Cannot read local audio output.")?;
        if available.is_empty() {
            return if bytes.is_empty() {
                Ok(None)
            } else {
                Err("Local audio returned an incomplete event.".into())
            };
        }
        let end = available.iter().position(|b| *b == b'\n');
        let count = end.map_or(available.len(), |i| i + 1);
        if bytes.len().saturating_add(count) > MAX_LINE_BYTES {
            return Err("Local audio event exceeded its limit.".into());
        }
        bytes.extend_from_slice(&available[..count]);
        reader.consume(count);
        if end.is_some() {
            return Ok(Some(bytes));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn model_path_defaults_to_user_cache_and_honors_absolute_override() {
        assert_eq!(voice_models_path(None, Some("/Users/example".into())).unwrap(), PathBuf::from("/Users/example/Library/Caches/Monitter/VoiceModels"));
        assert_eq!(voice_models_path(Some("/tmp/custom-models".into()), None).unwrap(), PathBuf::from("/tmp/custom-models"));
        assert!(voice_models_path(Some("relative".into()), Some("/Users/example".into())).is_err());
        assert!(voice_models_path(None, None).is_err());
    }

    #[test]
    fn oversized_framework_diagnostics_do_not_close_the_pipe_or_hide_later_stages() {
        let mut input = vec![b'x'; MAX_LINE_BYTES * 3];
        input.extend_from_slice(b"\nprivate framework text\nvoice-stage 12.5 asr-load-end\nvoice-stage 12.6 tts-load-begin\ntruncated");
        let mut reader = BufReader::with_capacity(97, std::io::Cursor::new(input));
        let mut forwarded = Vec::new();
        drain_helper_diagnostics(&mut reader, |line| forwarded.push(line));
        assert_eq!(forwarded, ["voice-stage 12.5 asr-load-end", "voice-stage 12.6 tts-load-begin"]);
    }

    fn event(kind: &str, fields: serde_json::Value) -> AudioEvent {
        let mut value = serde_json::json!({"type":kind, "sessionId":"session", "epoch":"epoch"});
        value.as_object_mut().unwrap().extend(fields.as_object().unwrap().clone());
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn slow_consumer_keeps_latest_provisional_state_and_ordered_controls() {
        let mut inbox = EventInbox::default();
        inbox.push(event("inputActivity", serde_json::json!({"active":true})));
        for revision in 1..1000 {
            assert!(inbox.push(event("audioLevel", serde_json::json!({"source":"input", "level":0.2}))));
            assert!(inbox.push(event("transcript", serde_json::json!({"utteranceId":"a", "revision":revision, "isFinal":false, "text":"provisional"}))));
        }
        assert_eq!(inbox.pending.len(), 3);
        inbox.push(event("inputActivity", serde_json::json!({"active":false})));
        inbox.push(event("transcript", serde_json::json!({"utteranceId":"a", "revision":1000, "isFinal":true, "text":"final"})));
        inbox.push(event("ready", serde_json::json!({"status":"inputFinished"})));
        let events: Vec<_> = std::iter::from_fn(|| inbox.poll().unwrap()).collect();
        assert_eq!(events.iter().map(|e| e.kind.as_str()).collect::<Vec<_>>(), ["inputActivity", "audioLevel", "inputActivity", "transcript", "ready"]);
        assert_eq!(events[0].active, Some(true));
        assert_eq!(events[2].active, Some(false));
        assert_eq!(events[3].is_final, Some(true));
        assert_eq!(events[3].revision, Some(1000));
    }

    #[test]
    fn playback_progress_precedes_completion_and_never_crosses_generations() {
        let mut inbox = EventInbox::default();
        for generation in [1, 2] {
            inbox.push(event("audioProgress", serde_json::json!({"status":"started", "generation":generation})));
            for samples in 1..100 {
                inbox.push(event("audioProgress", serde_json::json!({"status":"progress", "generation":generation, "samplesPlayed":samples})));
            }
            inbox.push(event("audioProgress", serde_json::json!({"status":"completed", "generation":generation})));
        }
        assert_eq!(inbox.pending.len(), 6);
        for generation in [1, 2] {
            assert_eq!(inbox.poll().unwrap().unwrap().status.as_deref(), Some("started"));
            let progress = inbox.poll().unwrap().unwrap();
            assert_eq!(progress.generation, Some(generation));
            assert_eq!(progress.samples_played, Some(99));
            assert_eq!(inbox.poll().unwrap().unwrap().status.as_deref(), Some("completed"));
        }
    }

    #[test]
    fn final_transcripts_are_never_silently_evicted_when_controls_fill_the_limit() {
        let mut inbox = EventInbox::default();
        for i in 0..EVENT_QUEUE {
            assert!(inbox.push(event("transcript", serde_json::json!({"utteranceId":i.to_string(), "isFinal":true, "text":"final"}))));
        }
        assert!(inbox.push(event("audioLevel", serde_json::json!({"source":"input", "level":0.2}))));
        assert_eq!(inbox.pending.len(), EVENT_QUEUE);
        assert!(!inbox.push(event("ready", serde_json::json!({"status":"stopped"}))));
        for i in 0..EVENT_QUEUE {
            let final_event = inbox.poll().unwrap().unwrap();
            assert_eq!(final_event.is_final, Some(true));
            assert_eq!(final_event.utterance_id.as_deref(), Some(i.to_string().as_str()));
        }
        assert!(inbox.poll().unwrap_err().contains("controls fell behind"));
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn a_stalled_ui_does_not_block_helper_telemetry() {
        use std::{os::unix::fs::PermissionsExt, thread, time::{Duration, Instant}};
        let root = std::env::temp_dir().join(format!("monitter-voice-transport-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let helper = root.join("helper.py");
        let marker = root.join("drained");
        let session = uuid::Uuid::new_v4().to_string();
        let epoch = uuid::Uuid::new_v4().to_string();
        let event = serde_json::json!({"type":"audioLevel", "sessionId":session, "epoch":epoch, "source":"input", "level":0.2}).to_string() + "\n";
        fs::write(&helper, format!("#!/usr/bin/python3\nimport sys, pathlib\nline = {}\nfor _ in range(5000):\n    sys.stdout.write(line)\nsys.stdout.flush()\npathlib.Path({}).write_text('drained')\nsys.stdin.read()\n", serde_json::to_string(&event).unwrap(), serde_json::to_string(marker.to_str().unwrap()).unwrap())).unwrap();
        fs::set_permissions(&helper, fs::Permissions::from_mode(0o700)).unwrap();
        let audio = NativeVoiceSession::spawn_at(&helper, session, epoch).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        // Deliberately do not poll: a temporarily busy UI must not block the
        // helper's stdout writer and consequently its microphone consumer.
        while !marker.exists() && Instant::now() < deadline { thread::sleep(Duration::from_millis(10)); }
        let drained = marker.exists();
        drop(audio);
        fs::remove_dir_all(root).unwrap();
        assert!(drained, "UI event backlog blocked the helper output pipe");
    }
    #[test]
    fn diagnostics_forward_only_safe_stages_and_progress_fields() {
        assert_eq!(safe_helper_diagnostic(b"voice-stage 12.5 asr-warm-begin\n").as_deref(), Some("voice-stage 12.5 asr-warm-begin"));
        assert!(safe_helper_diagnostic(b"voice-stage 12.5 asr-warm-begin secret\n").is_none());
        assert!(safe_helper_diagnostic(b"voice-stage 12.5 /private/path\n").is_none());
        assert!(safe_helper_diagnostic(b"voice-stage 12.5 route-notification running=true snapshotMatch=false stop=true\n").is_some());
        assert!(safe_helper_diagnostic(b"voice-stage 12.5 route-notification running=true snapshotMatch=false stop=secret\n").is_none());
        assert!(safe_helper_diagnostic(b"voice-stage 12.5 capture-check taps=30 accepted=30 running=true muted=false\n").is_some());
        assert!(safe_helper_diagnostic(b"voice-stage 12.5 capture-check taps=30 accepted=30 running=true muted=private\n").is_none());
        assert!(safe_helper_diagnostic(b"voice-stage 12.5 input-metrics rate=48000 frames=4096 queuedSeconds=0.25 oldestSeconds=0.2 count=3 processSeconds=1.2\n").is_some());
        assert!(safe_helper_diagnostic(b"voice-stage 12.5 input-overflow reason=age queuedSeconds=3 oldestSeconds=4.1 count=9 bytes=1024 frames=4096 rate=48000 channels=1\n").is_some());
        for invalid in [
            "input-metrics rate=48000 frames=4096 queuedSeconds=NaN oldestSeconds=0 count=3 processSeconds=1",
            "input-metrics rate=48000 frames=4096 queuedSeconds=0 oldestSeconds=0 count=3.5 processSeconds=1",
            "input-metrics rate=48000 frames=4096 queuedSeconds=0 oldestSeconds=0 count=3 processSeconds=private",
            "input-metrics rate=48000 frames=4096 queuedSeconds=0 oldestSeconds=0 count=3 processSeconds=1 secret",
            "input-overflow",
            "input-overflow reason=private queuedSeconds=3 oldestSeconds=4 count=9 bytes=1 frames=1 rate=48000 channels=1",
        ] {
            assert!(safe_helper_diagnostic(format!("voice-stage 12.5 {invalid}\n").as_bytes()).is_none());
        }
        let event: AudioEvent = serde_json::from_value(serde_json::json!({"type":"audioProgress","sessionId":"secret","epoch":"secret","status":"completed","generation":7,"samplesPlayed":42,"text":"private"})).unwrap();
        assert_eq!(audio_progress_diagnostic(&event).as_deref(), Some("voice-audio-progress status=completed generation=7 samplesPlayed=42"));
    }
    #[test]
    fn line_reader_rejects_oversize_and_truncated_events() {
        assert!(
            read_line_bounded(&mut std::io::Cursor::new(vec![b'x'; MAX_LINE_BYTES + 1])).is_err()
        );
        assert!(read_line_bounded(&mut std::io::Cursor::new(b"{}".as_slice())).is_err());
        let mut stream = std::io::Cursor::new(b"{}\n{}\n".as_slice());
        assert_eq!(
            read_line_bounded(&mut stream).unwrap(),
            Some(b"{}\n".to_vec())
        );
        assert_eq!(
            read_line_bounded(&mut stream).unwrap(),
            Some(b"{}\n".to_vec())
        );
        assert!(read_line_bounded(&mut stream).unwrap().is_none());
    }
}
