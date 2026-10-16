//! Explicit, owner-local macOS microphone capture for the native composer.
//! Native recorder objects stay on GPUI's foreground thread; only completed,
//! bounded WAV bytes may be handed to a worker.

#[cfg(target_os = "macos")]
mod macos {
    use block2::RcBlock;
    use objc2::{
        MainThreadMarker,
        rc::Retained,
        runtime::{AnyObject, Bool},
    };
    use objc2_av_foundation::{AVAuthorizationStatus, AVCaptureDevice, AVMediaTypeAudio};
    use objc2_avf_audio::{
        AVAudioRecorder, AVFormatIDKey, AVLinearPCMBitDepthKey, AVLinearPCMIsBigEndianKey,
        AVLinearPCMIsFloatKey, AVNumberOfChannelsKey, AVSampleRateKey,
    };
    use objc2_foundation::{NSBundle, NSDictionary, NSNumber, NSString, NSURL};
    use std::{
        fs::{self, OpenOptions},
        io::Read,
        marker::PhantomData,
        os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt},
        path::{Path, PathBuf},
        rc::Rc,
        sync::{
            atomic::{AtomicU64, Ordering},
            mpsc::{self, Receiver},
        },
    };

    const MAX_RECORDING_SECONDS: f64 = 90.0;
    const MAX_WAV_BYTES: usize = 3 * 1024 * 1024 + 128;
    static NEXT_RECORDING_ID: AtomicU64 = AtomicU64::new(1);

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum MicrophonePermission {
        NotDetermined,
        Authorized,
        Denied,
        Restricted,
    }

    fn media_type() -> Result<&'static objc2_av_foundation::AVMediaType, String> {
        // SAFETY: The framework provides this immutable NSString media-type constant.
        unsafe { AVMediaTypeAudio }
            .ok_or_else(|| "Native audio capture is unavailable.".to_string())
    }

    fn microphone_permission(_main: MainThreadMarker) -> Result<MicrophonePermission, String> {
        // SAFETY: AVMediaTypeAudio is a defined constant accepted by this API.
        let status = unsafe { AVCaptureDevice::authorizationStatusForMediaType(media_type()?) };
        Ok(match status {
            AVAuthorizationStatus::Authorized => MicrophonePermission::Authorized,
            AVAuthorizationStatus::NotDetermined => MicrophonePermission::NotDetermined,
            AVAuthorizationStatus::Denied => MicrophonePermission::Denied,
            _ => MicrophonePermission::Restricted,
        })
    }

    fn require_privacy_declaration() -> Result<(), String> {
        let purpose = NSBundle::mainBundle()
            .objectForInfoDictionaryKey(&NSString::from_str("NSMicrophoneUsageDescription"));
        if !purpose
            .as_deref()
            .and_then(|value| value.downcast_ref::<NSString>())
            .is_some_and(|text| !text.to_string().trim().is_empty())
        {
            return Err(
                "This app bundle does not declare microphone use. Recording is disabled.".into(),
            );
        }
        Ok(())
    }

    /// Call only in response to an explicit Record action. The async native
    /// callback transports a bool through a thread-safe channel, never an ObjC
    /// object or UI reference.
    pub(super) fn request_microphone_permission_on_main(
        main: MainThreadMarker,
    ) -> Result<Receiver<bool>, String> {
        require_privacy_declaration()?;
        let (sender, receiver) = mpsc::channel();
        match microphone_permission(main)? {
            MicrophonePermission::Authorized => {
                let _ = sender.send(true);
            }
            MicrophonePermission::Denied | MicrophonePermission::Restricted => {
                let _ = sender.send(false);
            }
            MicrophonePermission::NotDetermined => {
                let callback = RcBlock::new(move |granted: Bool| {
                    let _ = sender.send(granted.as_bool());
                });
                // SAFETY: AVFoundation copies the block for its async callback;
                // its only capture is the Send mpsc sender.
                unsafe {
                    AVCaptureDevice::requestAccessForMediaType_completionHandler(
                        media_type()?,
                        &callback,
                    );
                }
            }
        }
        Ok(receiver)
    }

    struct PrivateRecording {
        directory: PathBuf,
        wav: PathBuf,
    }

    impl PrivateRecording {
        fn new() -> Result<Self, String> {
            let temp = std::env::temp_dir();
            let recording = (0..32)
                .find_map(|_| {
                    let serial = NEXT_RECORDING_ID.fetch_add(1, Ordering::Relaxed);
                    let directory = temp.join(format!(
                        "monitter-native-voice-{}-{serial}",
                        std::process::id()
                    ));
                    match fs::DirBuilder::new().mode(0o700).create(&directory) {
                        Ok(()) => Some(Ok(Self {
                            wav: directory.join("capture.wav"),
                            directory,
                        })),
                        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => None,
                        Err(error) => Some(Err(format!(
                            "Cannot prepare private recording folder: {error}"
                        ))),
                    }
                })
                .ok_or("Cannot allocate a private recording folder.")??;

            OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW)
                .open(&recording.wav)
                .map_err(|error| format!("Cannot prepare private recording file: {error}"))?;
            Ok(recording)
        }
    }

    impl Drop for PrivateRecording {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.directory);
        }
    }

    fn read_validated_wav(path: &Path) -> Result<Vec<u8>, String> {
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(path)
            .map_err(|error| format!("Cannot read the completed recording: {error}"))?;
        let metadata = file
            .metadata()
            .map_err(|error| format!("Cannot inspect recording: {error}"))?;
        if !metadata.is_file() || metadata.len() > MAX_WAV_BYTES as u64 {
            return Err("Recording is not a bounded regular WAV file.".into());
        }
        file.set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|error| format!("Cannot protect the recording: {error}"))?;
        let mut bytes = Vec::new();
        file.take(MAX_WAV_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| format!("Cannot read recording: {error}"))?;
        if bytes.len() > MAX_WAV_BYTES {
            return Err("Recording exceeded the 90 second limit.".into());
        }
        super::validate_wav(&bytes)?;
        Ok(bytes)
    }

    fn settings() -> Result<Retained<NSDictionary<NSString, AnyObject>>, String> {
        // SAFETY: These framework constants are immutable string keys. The
        // values use AVAudioRecorder's documented Linear PCM settings types.
        let keys = unsafe {
            [
                AVFormatIDKey,
                AVSampleRateKey,
                AVNumberOfChannelsKey,
                AVLinearPCMBitDepthKey,
                AVLinearPCMIsBigEndianKey,
                AVLinearPCMIsFloatKey,
            ]
        }
        .into_iter()
        .map(|key| key.ok_or("Native WAV settings are unavailable."))
        .collect::<Result<Vec<_>, _>>()?;
        let values: Vec<Retained<AnyObject>> = vec![
            NSNumber::new_u32(u32::from_be_bytes(*b"lpcm")).into(),
            NSNumber::new_f64(16_000.).into(),
            NSNumber::new_u32(1).into(),
            NSNumber::new_u32(16).into(),
            NSNumber::new_bool(false).into(),
            NSNumber::new_bool(false).into(),
        ];
        Ok(NSDictionary::from_retained_objects(&keys, &values))
    }

    #[derive(Clone, Copy, Debug)]
    pub(crate) struct RecordingStatus {
        pub recording: bool,
        pub elapsed_seconds: f64,
    }

    /// RAII microphone owner. The marker and Rc phantom deliberately prevent
    /// this owner from crossing threads despite AVAudioRecorder's Send impl.
    pub(crate) struct NativeVoiceCapture {
        recorder: Option<Retained<AVAudioRecorder>>,
        files: PrivateRecording,
        _main: MainThreadMarker,
        _not_send_sync: PhantomData<Rc<()>>,
    }

    impl NativeVoiceCapture {
        pub(crate) fn begin() -> Result<Self, String> {
            let main = MainThreadMarker::new()
                .ok_or("Microphone capture must run on the macOS main thread.")?;
            require_privacy_declaration()?;
            if microphone_permission(main)? != MicrophonePermission::Authorized {
                return Err("Microphone permission is required before recording.".into());
            }
            let files = PrivateRecording::new()?;
            let path = files
                .wav
                .to_str()
                .ok_or("Private recording path is not valid Unicode.")?;
            let url = NSURL::fileURLWithPath(&NSString::from_str(path));
            // SAFETY: This is called on the marker's main thread with a private
            // WAV path and typed PCM settings. NSError is surfaced to the UI.
            let audio_settings = settings()?;
            let recorder = unsafe {
                AVAudioRecorder::initWithURL_settings_error(main.alloc(), &url, &audio_settings)
            }
            .map_err(|error| format!("Cannot initialize microphone recording: {error}"))?;
            let capture = Self {
                recorder: Some(recorder),
                files,
                _main: main,
                _not_send_sync: PhantomData,
            };
            let recorder = capture.recorder.as_ref().unwrap();
            if !unsafe {
                recorder.prepareToRecord() && recorder.recordForDuration(MAX_RECORDING_SECONDS)
            } {
                return Err("The microphone could not start recording.".into());
            }
            let file = OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW)
                .open(&capture.files.wav)
                .map_err(|error| format!("Cannot protect active recording: {error}"))?;
            file.set_permissions(fs::Permissions::from_mode(0o600))
                .map_err(|error| format!("Cannot protect active recording: {error}"))?;
            Ok(capture)
        }

        pub(crate) fn status(&self) -> RecordingStatus {
            let recorder = self.recorder.as_ref().unwrap();
            let (recording, time) = unsafe { (recorder.isRecording(), recorder.currentTime()) };
            RecordingStatus {
                recording,
                elapsed_seconds: if time.is_finite() {
                    time.clamp(0., MAX_RECORDING_SECONDS)
                } else {
                    0.
                },
            }
        }

        /// Releases the microphone before reading the bounded WAV.
        pub(crate) fn stop(mut self) -> Result<Vec<u8>, String> {
            self.stop_native();
            read_validated_wav(&self.files.wav)
        }

        pub(crate) fn cancel(mut self) {
            self.stop_native();
        }

        fn stop_native(&mut self) {
            if let Some(recorder) = self.recorder.take() {
                unsafe { recorder.stop() };
            }
        }
    }

    impl Drop for NativeVoiceCapture {
        fn drop(&mut self) {
            self.stop_native();
        }
    }
}

/// Strict, small RIFF/WAVE parser for the exact local Whisper input contract.
/// The service repeats validation and remains the final authority.
pub(crate) fn validate_wav(wav: &[u8]) -> Result<(), String> {
    const MAX_PCM_BYTES: usize = 90 * 16_000 * 2;
    if wav.len() < 44 || &wav[..4] != b"RIFF" || &wav[8..12] != b"WAVE" {
        return Err("Voice audio must be a PCM16 WAV file.".into());
    }
    let declared = u32::from_le_bytes(wav[4..8].try_into().unwrap()) as usize;
    if declared.checked_add(8) != Some(wav.len()) || declared < 4 {
        return Err("Voice audio WAV header is invalid.".into());
    }
    let mut offset = 12usize;
    let mut format = None;
    let mut data_len = None;
    while offset < wav.len() {
        if wav.len() - offset < 8 {
            return Err("Voice audio WAV header is invalid.".into());
        }
        let size = u32::from_le_bytes(wav[offset + 4..offset + 8].try_into().unwrap()) as usize;
        let start = offset + 8;
        let end = start
            .checked_add(size)
            .ok_or("Voice audio WAV header is invalid.")?;
        if end > wav.len() {
            return Err("Voice audio WAV header is invalid.".into());
        }
        match &wav[offset..offset + 4] {
            b"fmt " if size >= 16 => {
                format = Some((
                    u16::from_le_bytes(wav[start..start + 2].try_into().unwrap()),
                    u16::from_le_bytes(wav[start + 2..start + 4].try_into().unwrap()),
                    u32::from_le_bytes(wav[start + 4..start + 8].try_into().unwrap()),
                    u32::from_le_bytes(wav[start + 8..start + 12].try_into().unwrap()),
                    u16::from_le_bytes(wav[start + 12..start + 14].try_into().unwrap()),
                    u16::from_le_bytes(wav[start + 14..start + 16].try_into().unwrap()),
                ));
            }
            b"data" => data_len = Some(size),
            _ => {}
        }
        offset = end
            .checked_add(size & 1)
            .ok_or("Voice audio WAV header is invalid.")?;
        if offset > wav.len() {
            return Err("Voice audio WAV header is invalid.".into());
        }
    }
    let (encoding, channels, rate, byte_rate, block_align, bits) =
        format.ok_or("Voice audio WAV header is invalid.")?;
    let data = data_len.ok_or("Voice audio WAV header is invalid.")?;
    if encoding != 1
        || channels != 1
        || rate != 16_000
        || byte_rate != 32_000
        || block_align != 2
        || bits != 16
        || data == 0
        || data > MAX_PCM_BYTES
        || data % 2 != 0
    {
        return Err("Voice audio must be 16 kHz mono PCM16 WAV under 90 seconds.".into());
    }
    Ok(())
}

#[cfg(target_os = "macos")]
pub(crate) use macos::{NativeVoiceCapture, RecordingStatus};

#[cfg(target_os = "macos")]
pub(crate) fn request_microphone_permission() -> Result<std::sync::mpsc::Receiver<bool>, String> {
    let main = objc2::MainThreadMarker::new()
        .ok_or("Microphone permission must be requested on the macOS main thread.")?;
    macos::request_microphone_permission_on_main(main)
}

#[cfg(not(target_os = "macos"))]
pub(crate) struct NativeVoiceCapture;

#[cfg(not(target_os = "macos"))]
#[derive(Clone, Copy, Debug)]
pub(crate) struct RecordingStatus {
    pub recording: bool,
    pub elapsed_seconds: f64,
}

#[cfg(not(target_os = "macos"))]
impl NativeVoiceCapture {
    pub(crate) fn begin() -> Result<Self, String> {
        Err("Native voice recording is available only in the macOS GPUI app.".into())
    }

    pub(crate) fn status(&self) -> RecordingStatus {
        RecordingStatus {
            recording: false,
            elapsed_seconds: 0.,
        }
    }

    pub(crate) fn stop(self) -> Result<Vec<u8>, String> {
        Err("Native voice recording is available only in the macOS GPUI app.".into())
    }

    pub(crate) fn cancel(self) {}
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn request_microphone_permission() -> Result<std::sync::mpsc::Receiver<bool>, String> {
    Err("Native voice recording is available only in the macOS GPUI app.".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn silent_wav(samples: usize) -> Vec<u8> {
        let data_size = samples * 2;
        let mut wav = Vec::with_capacity(44 + data_size);
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&((36 + data_size) as u32).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&16_000u32.to_le_bytes());
        wav.extend_from_slice(&32_000u32.to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&(data_size as u32).to_le_bytes());
        wav.resize(44 + data_size, 0);
        wav
    }

    #[::core::prelude::v1::test]
    fn accepts_only_bounded_mono_sixteen_kilohertz_pcm16_wav() {
        assert!(validate_wav(&silent_wav(16_000)).is_ok());
        assert!(validate_wav(&silent_wav(90 * 16_000)).is_ok());
        assert!(validate_wav(&silent_wav(90 * 16_000 + 1)).is_err());
        assert!(validate_wav(&silent_wav(0)).is_err());
        assert!(validate_wav(b"not audio").is_err());
        let mut mismatched_riff_size = silent_wav(4);
        mismatched_riff_size[4..8].copy_from_slice(&0u32.to_le_bytes());
        assert!(validate_wav(&mismatched_riff_size).is_err());
        let mut invalid_alignment = silent_wav(4);
        invalid_alignment[32..34].copy_from_slice(&4u16.to_le_bytes());
        assert!(validate_wav(&invalid_alignment).is_err());
    }
}
