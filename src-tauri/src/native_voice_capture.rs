//! Native owner microphone prototype. Currently unreferenced: wiring and bundle
//! privacy declarations must be reviewed before enabling the Record control.
//! The recorder never crosses threads; only completed validated WAV bytes may.
const MAX_WAV_BYTES: usize = 3 * 1024 * 1024 + 128;

#[cfg(target_os = "macos")]
mod macos {
    use super::MAX_WAV_BYTES;
    use block2::RcBlock;
    use objc2::{
        rc::Retained,
        runtime::{AnyObject, Bool},
        MainThreadMarker,
    };
    use objc2_av_foundation::{AVAuthorizationStatus, AVCaptureDevice, AVMediaTypeAudio};
    use objc2_avf_audio::{
        AVAudioRecorder, AVFormatIDKey, AVLinearPCMBitDepthKey, AVLinearPCMIsBigEndianKey,
        AVLinearPCMIsFloatKey, AVNumberOfChannelsKey, AVSampleRateKey,
    };
    use objc2_foundation::{NSBundle, NSDictionary, NSNumber, NSString, NSURL};
    use std::{
        fs::{self, File, OpenOptions},
        io::Read,
        marker::PhantomData,
        os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt},
        path::{Path, PathBuf},
        rc::Rc,
        sync::mpsc::{self, Receiver},
    };

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(crate) enum MicrophonePermission {
        NotDetermined,
        Authorized,
        Denied,
        Restricted,
    }
    /// No permission prompt, microphone access, or device capture.
    pub(crate) fn microphone_permission(
        _main: MainThreadMarker,
    ) -> Result<MicrophonePermission, String> {
        // SAFETY: A defined audio media-type constant is the only accepted argument.
        let status = unsafe {
            AVCaptureDevice::authorizationStatusForMediaType(
                AVMediaTypeAudio.ok_or("Native audio capture is unavailable.")?,
            )
        };
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
    /// Invoke only after the user's explicit Record action. The callback passes
    /// a bool across a channel; no native object or UI reference leaves main.
    pub(crate) fn request_microphone_permission(
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
                // SAFETY: AVFoundation copies the block for its asynchronous
                // callback, whose capture contains only a thread-safe sender.
                unsafe {
                    AVCaptureDevice::requestAccessForMediaType_completionHandler(
                        AVMediaTypeAudio.ok_or("Native audio capture is unavailable.")?,
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
            let directory = std::env::temp_dir()
                .join(format!("monitter-native-voice-{}", uuid::Uuid::new_v4()));
            fs::DirBuilder::new()
                .mode(0o700)
                .create(&directory)
                .map_err(|error| format!("Cannot prepare private recording folder: {error}"))?;
            let recording = Self {
                wav: directory.join("capture.wav"),
                directory,
            };
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
        crate::voice::validate_wav(&bytes)?;
        Ok(bytes)
    }
    fn settings() -> Result<Retained<NSDictionary<NSString, AnyObject>>, String> {
        // SAFETY: These framework constants are immutable NSString keys. The
        // value types match AVAudioRecorder's documented audio-settings contract.
        let keys = unsafe {
            [
                AVFormatIDKey,
                AVSampleRateKey,
                AVNumberOfChannelsKey,
                AVLinearPCMBitDepthKey,
                AVLinearPCMIsBigEndianKey,
                AVLinearPCMIsFloatKey,
            ]
        };
        let keys = keys
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
    /// Main-thread-only RAII microphone owner. There is deliberately no Send
    /// or Sync implementation even though generated AVAudioRecorder has them.
    pub(crate) struct NativeVoiceCapture {
        recorder: Option<Retained<AVAudioRecorder>>,
        files: PrivateRecording,
        _main: MainThreadMarker,
        _not_send_sync: PhantomData<Rc<()>>,
    }
    impl NativeVoiceCapture {
        /// Permission must already have completed after an explicit Record click.
        /// Constructing the UI or checking permission never calls this method.
        pub(crate) fn begin(main: MainThreadMarker) -> Result<Self, String> {
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
            // SAFETY: Main-thread owner, a private .wav URL, and correctly typed
            // PCM settings; NSError is surfaced rather than ignored.
            let recorder = unsafe {
                AVAudioRecorder::initWithURL_settings_error(main.alloc(), &url, &settings()?)
            }
            .map_err(|error| format!("Cannot initialize microphone recording: {error}"))?;
            let capture = Self {
                recorder: Some(recorder),
                files,
                _main: main,
                _not_send_sync: PhantomData,
            };
            let recorder = capture.recorder.as_ref().unwrap();
            // Native duration limit remains effective if the GPUI timer stalls.
            if !unsafe { recorder.prepareToRecord() && recorder.recordForDuration(90.) } {
                return Err("The microphone could not start recording.".into());
            }
            // Preserve restrictive permissions even if AVFoundation replaced
            // the precreated file. The containing folder is always private.
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
                    time.clamp(0., 90.)
                } else {
                    0.
                },
            }
        }
        /// Release the microphone before opening/validating bytes. RAII removes
        /// the private WAV on success, error, cancellation, and window teardown.
        pub(crate) fn stop(mut self) -> Result<Vec<u8>, String> {
            self.stop_native();
            read_validated_wav(&self.files.wav)
        }
        pub(crate) fn cancel(mut self) {
            self.stop_native();
        }
        fn stop_native(&mut self) {
            if let Some(recorder) = self.recorder.take() {
                unsafe {
                    recorder.stop();
                }
            }
        }
    }
    impl Drop for NativeVoiceCapture {
        fn drop(&mut self) {
            self.stop_native();
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        fn silent_wav() -> Vec<u8> {
            let mut bytes = Vec::new();
            bytes.extend(b"RIFF");
            bytes.extend(38u32.to_le_bytes());
            bytes.extend(b"WAVEfmt ");
            bytes.extend(16u32.to_le_bytes());
            bytes.extend(1u16.to_le_bytes());
            bytes.extend(1u16.to_le_bytes());
            bytes.extend(16_000u32.to_le_bytes());
            bytes.extend(32_000u32.to_le_bytes());
            bytes.extend(2u16.to_le_bytes());
            bytes.extend(16u16.to_le_bytes());
            bytes.extend(b"data");
            bytes.extend(2u32.to_le_bytes());
            bytes.extend([0, 0]);
            bytes
        }
        #[test]
        fn private_recording_cleanup_and_validated_bytes() {
            let files = PrivateRecording::new().unwrap();
            let directory = files.directory.clone();
            assert_eq!(
                fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
                0o700
            );
            fs::write(&files.wav, silent_wav()).unwrap();
            assert_eq!(read_validated_wav(&files.wav).unwrap(), silent_wav());
            assert_eq!(
                fs::metadata(&files.wav).unwrap().permissions().mode() & 0o777,
                0o600
            );
            drop(files);
            assert!(!directory.exists());
        }
        #[test]
        fn rejects_symlink_oversize_and_invalid_wav_without_capture() {
            let files = PrivateRecording::new().unwrap();
            let file = File::create(&files.wav).unwrap();
            file.set_len(MAX_WAV_BYTES as u64 + 1).unwrap();
            assert!(read_validated_wav(&files.wav).is_err());
            drop(file);
            fs::write(&files.wav, b"invalid").unwrap();
            assert!(read_validated_wav(&files.wav).is_err());
            fs::remove_file(&files.wav).unwrap();
            let target = files.directory.join("target.wav");
            fs::write(&target, silent_wav()).unwrap();
            std::os::unix::fs::symlink(&target, &files.wav).unwrap();
            assert!(read_validated_wav(&files.wav).is_err());
        }
    }
}
#[cfg(target_os = "macos")]
pub(crate) use macos::{
    microphone_permission, request_microphone_permission, MicrophonePermission, NativeVoiceCapture,
    RecordingStatus,
};
