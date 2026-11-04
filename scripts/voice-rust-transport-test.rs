//! Standalone process-level test for the actual Rust voice transport module.
//! Its default model test never opens a microphone. The explicitly ignored
//! physical acceptance test opens capture only under the bounded local runner.

#[path = "../desktop-gpui/src/native_voice_session.rs"]
mod native_voice_session;

#[cfg(test)]
mod transport_tests {
    use crate::native_voice_session;
    use super::native_voice_session::NativeVoiceSession;
    use std::{thread, time::{Duration, Instant}};

    const LOAD_TIMEOUT: Duration = Duration::from_secs(80);
    const STOP_TIMEOUT: Duration = Duration::from_secs(5);

    fn wait_for_status(
        audio: &NativeVoiceSession,
        status: &str,
        timeout: Duration,
    ) -> native_voice_session::AudioEvent {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            match audio.poll() {
                Ok(Some(event)) if event.kind == "error" => {
                    panic!("helper returned error while waiting for {status}: code={:?}", event.code);
                }
                Ok(Some(event)) if event.kind == "ready" && event.status.as_deref() == Some(status) => {
                    return event;
                }
                Ok(Some(_)) | Ok(None) => thread::sleep(Duration::from_millis(10)),
                Err(error) => panic!("transport failed while waiting for {status}: {error}"),
            }
        }
        panic!("timed out waiting for helper status {status}");
    }

    #[test]
    fn actual_transport_loads_models_then_stops_without_starting_capture() {
        assert!(std::env::var_os("MONITTER_NATIVE_VOICE_HELPER").is_some());
        assert!(std::env::var_os("MONITTER_VOICE_MODELS").is_some());
        let session_id = uuid::Uuid::new_v4().to_string();
        let epoch = uuid::Uuid::new_v4().to_string();
        let audio = NativeVoiceSession::spawn(session_id.clone(), epoch.clone())
            .unwrap_or_else(|error| panic!("could not spawn built voice helper: {error}"));

        audio.command("load", None, None, None).unwrap();
        let loaded = wait_for_status(&audio, "loaded", LOAD_TIMEOUT);
        assert_eq!(loaded.session_id, session_id);
        assert_eq!(loaded.epoch, epoch);
        assert!(loaded.request_id.as_deref().is_some_and(|id| !id.is_empty()));

        // Deliberately skip `start`: the real AVAudioEngine/microphone path is
        // not part of this transport test.
        audio.command("stop", None, None, None).unwrap();
        let stopped = wait_for_status(&audio, "stopped", STOP_TIMEOUT);
        assert_eq!(stopped.session_id, session_id);
        assert_eq!(stopped.epoch, epoch);
        assert!(stopped.request_id.as_deref().is_some_and(|id| !id.is_empty()));
    }

    /// Explicit physical test: open capture briefly while the foreground
    /// consumer is deliberately absent. No transcripts or microphone PCM are
    /// printed, stored, or sent to an agent. Run under a process-group timeout.
    #[test]
    #[ignore = "opens the physical microphone; use the bounded local acceptance runner"]
    fn physical_capture_survives_a_stalled_frontend() {
        let audio = NativeVoiceSession::spawn(uuid::Uuid::new_v4().to_string(), uuid::Uuid::new_v4().to_string()).unwrap();
        audio.command("load", None, None, None).unwrap();
        wait_for_status(&audio, "loaded", LOAD_TIMEOUT);
        audio.command("start", None, None, None).unwrap();
        let started = wait_for_status(&audio, "started", Duration::from_secs(20));
        assert_eq!(started.voice_processing, Some(true));

        // At 10 input-level events/second this exceeds the former 64-event
        // channel, even in a quiet room. The background reader must keep the
        // helper's stdout pipe drained independently of foreground polling.
        thread::sleep(Duration::from_secs(20));
        let mut events = 0;
        let mut input_levels = 0;
        loop {
            match audio.poll() {
                Ok(Some(event)) => {
                    assert_ne!(event.kind, "error", "physical capture error code: {:?}", event.code);
                    if event.kind == "audioLevel" && event.source.as_deref() == Some("input") { input_levels += 1; }
                    events += 1;
                }
                Ok(None) => break,
                Err(error) => panic!("physical capture transport error: {error}"),
            }
        }
        audio.command("mute", None, None, Some(true)).unwrap();
        wait_for_status(&audio, "muted", Duration::from_secs(10));
        audio.command("stop", None, None, None).unwrap();
        wait_for_status(&audio, "stopped", STOP_TIMEOUT);
        assert!(input_levels > 0, "capture produced no input observations; a quiet or unavailable audio stream cannot establish backpressure recovery");
        println!("physical stalled-frontend probe: capture survived 20s without foreground polling; {events} retained events; clean mute and stop");
    }
}
