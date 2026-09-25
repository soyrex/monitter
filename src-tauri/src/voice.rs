use serde::Serialize;
use std::{
    env,
    fs::{self, OpenOptions},
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

const MAX_AUDIO_BYTES: usize = 3 * 1024 * 1024 + 128;
const MAX_TEXT_BYTES: u64 = 64 * 1024;
const MAX_SECONDS: u32 = 90;
const RUN_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct VoiceTranscript {
    text: String,
}

fn decode_base64(input: &str) -> Result<Vec<u8>, String> {
    if input.len() > MAX_AUDIO_BYTES * 4 / 3 + 8 {
        return Err("Voice audio exceeds the 90 second limit.".into());
    }
    let bytes = input.as_bytes();
    if bytes.is_empty() || bytes.len() % 4 != 0 {
        return Err("Voice audio is not valid base64.".into());
    }
    fn val(b: u8) -> Option<u8> {
        match b {
            b'A'..=b'Z' => Some(b - b'A'),
            b'a'..=b'z' => Some(b - b'a' + 26),
            b'0'..=b'9' => Some(b - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    for (i, quad) in bytes.chunks_exact(4).enumerate() {
        let last = i + 1 == bytes.len() / 4;
        let a = val(quad[0]).ok_or("Voice audio is not valid base64.")? as u32;
        let b = val(quad[1]).ok_or("Voice audio is not valid base64.")? as u32;
        let c = if quad[2] == b'=' {
            0
        } else {
            val(quad[2]).ok_or("Voice audio is not valid base64.")? as u32
        };
        let d = if quad[3] == b'=' {
            0
        } else {
            val(quad[3]).ok_or("Voice audio is not valid base64.")? as u32
        };
        if (!last && (quad[2] == b'=' || quad[3] == b'='))
            || (quad[2] == b'=' && quad[3] != b'=')
            || (quad[2] == b'=' && b & 0x0f != 0)
            || (quad[3] == b'=' && quad[2] != b'=' && c & 0x03 != 0)
        {
            return Err("Voice audio is not valid base64.".into());
        }
        let bits = (a << 18) | (b << 12) | (c << 6) | d;
        out.push((bits >> 16) as u8);
        if quad[2] != b'=' {
            out.push((bits >> 8) as u8);
        }
        if quad[3] != b'=' {
            out.push(bits as u8);
        }
        if out.len() > MAX_AUDIO_BYTES {
            return Err("Voice audio exceeds the 90 second limit.".into());
        }
    }
    Ok(out)
}

pub(crate) fn validate_wav(wav: &[u8]) -> Result<(), String> {
    if wav.len() < 44 || &wav[..4] != b"RIFF" || &wav[8..12] != b"WAVE" {
        return Err("Voice audio must be a PCM16 WAV file.".into());
    }
    let declared = u32::from_le_bytes(wav[4..8].try_into().unwrap()) as usize;
    if declared.saturating_add(8) != wav.len() || declared.saturating_add(8) < 12 {
        return Err("Voice audio WAV header is invalid.".into());
    }
    let mut offset = 12;
    let mut fmt = None;
    let mut data_len = None;
    while offset + 8 <= declared + 8 {
        let size = u32::from_le_bytes(wav[offset + 4..offset + 8].try_into().unwrap()) as usize;
        let start = offset + 8;
        let end = start
            .checked_add(size)
            .ok_or("Voice audio WAV header is invalid.")?;
        if end > declared + 8 || end > wav.len() {
            return Err("Voice audio WAV header is invalid.".into());
        }
        match &wav[offset..offset + 4] {
            b"fmt " if size >= 16 => {
                fmt = Some((
                    u16::from_le_bytes(wav[start..start + 2].try_into().unwrap()),
                    u16::from_le_bytes(wav[start + 2..start + 4].try_into().unwrap()),
                    u32::from_le_bytes(wav[start + 4..start + 8].try_into().unwrap()),
                    u16::from_le_bytes(wav[start + 14..start + 16].try_into().unwrap()),
                ))
            }
            b"data" => data_len = Some(size),
            _ => {}
        }
        offset = end + (size & 1);
    }
    let (format, channels, rate, bits) = fmt.ok_or("Voice audio WAV header is invalid.")?;
    let data = data_len.ok_or("Voice audio WAV header is invalid.")?;
    if format != 1 || channels != 1 || rate != 16_000 || bits != 16 || data % 2 != 0 {
        return Err("Voice audio must be 16 kHz mono PCM16 WAV.".into());
    }
    if data / 32_000 > MAX_SECONDS as usize || data > MAX_SECONDS as usize * 32_000 {
        return Err("Voice audio exceeds the 90 second limit.".into());
    }
    Ok(())
}

fn configured_executable() -> Result<PathBuf, String> {
    if let Some(value) = env::var_os("MONITTER_WHISPER_CLI") {
        let path = PathBuf::from(value);
        let available = if path.components().count() > 1 {
            path.is_file()
        } else {
            env::split_paths(&env::var_os("PATH").unwrap_or_default())
                .any(|dir| dir.join(&path).is_file())
        };
        return if available {
            Ok(path)
        } else {
            Err("Configured Whisper executable is unavailable. Check MONITTER_WHISPER_CLI.".into())
        };
    }
    let candidates = [
        PathBuf::from("/opt/homebrew/bin/whisper-cli"),
        PathBuf::from("/usr/local/bin/whisper-cli"),
        PathBuf::from("whisper-cli"),
    ];
    for path in candidates {
        if path.components().count() > 1 && path.is_file() {
            return Ok(path);
        }
        if path.components().count() == 1
            && env::split_paths(&env::var_os("PATH").unwrap_or_default())
                .any(|dir| dir.join(&path).is_file())
        {
            return Ok(path);
        }
    }
    Err(
        "Whisper is unavailable. Install whisper.cpp or set MONITTER_WHISPER_CLI to whisper-cli."
            .into(),
    )
}

fn configured_model() -> Result<PathBuf, String> {
    if let Some(path) = env::var_os("MONITTER_WHISPER_MODEL").map(PathBuf::from) {
        if path.is_file() {
            return Ok(path);
        }
        return Err(
            "Whisper model is unavailable. Set MONITTER_WHISPER_MODEL to a local model file."
                .into(),
        );
    }
    let mut candidates = Vec::new();
    if let Some(home) = env::var_os("HOME") {
        let home = PathBuf::from(home);
        candidates
            .push(home.join("Library/Application Support/Monitter/whisper/models/ggml-base.bin"));
        candidates.push(home.join(".cache/whisper.cpp/ggml-base.bin"));
        candidates.push(home.join("Library/Application Support/whisper.cpp/ggml-base.bin"));
    }
    candidates.extend([
        PathBuf::from("/opt/homebrew/share/whisper.cpp/models/ggml-base.bin"),
        PathBuf::from("/usr/local/share/whisper.cpp/models/ggml-base.bin"),
    ]);
    candidates.into_iter().find(|p| p.is_file()).ok_or_else(|| {
        "Whisper model is unavailable. Set MONITTER_WHISPER_MODEL to a local model file.".into()
    })
}

struct TempFiles {
    dir: PathBuf,
}
impl Drop for TempFiles {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

pub(crate) fn transcribe(audio_base64: String) -> Result<VoiceTranscript, String> {
    let wav = decode_base64(&audio_base64)?;
    validate_wav(&wav)?;
    let cli = configured_executable()?;
    let model = configured_model()?;
    let dir = env::temp_dir().join(format!("monitter-voice-{}", uuid::Uuid::new_v4()));
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        let mut builder = fs::DirBuilder::new();
        builder.mode(0o700);
        builder
            .create(&dir)
            .map_err(|_| "Could not prepare temporary voice transcription files.".to_string())?;
    }
    #[cfg(not(unix))]
    fs::create_dir(&dir)
        .map_err(|_| "Could not prepare temporary voice transcription files.".to_string())?;
    let files = TempFiles { dir: dir.clone() };
    let input = dir.join("audio.wav");
    let output = dir.join("transcript");
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&input)
        .map_err(|_| "Could not prepare temporary voice transcription files.".to_string())?;
    file.write_all(&wav)
        .map_err(|_| "Could not prepare temporary voice transcription files.".to_string())?;
    drop(file);
    let mut child = Command::new(cli)
        .args(["--model"])
        .arg(model)
        .args(["--file"])
        .arg(&input)
        .args(["--language", "auto", "--output-txt", "--output-file"])
        .arg(&output)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "Could not start Whisper transcription.".to_string())?;
    let deadline = Instant::now() + RUN_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break,
            Ok(Some(_)) => return Err("Whisper could not transcribe this voice message.".into()),
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(100)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("Whisper transcription timed out.".into());
            }
        }
    }
    let text_path = output.with_extension("txt");
    let metadata =
        fs::metadata(&text_path).map_err(|_| "Whisper returned no transcript.".to_string())?;
    if metadata.len() > MAX_TEXT_BYTES {
        return Err("Whisper transcript exceeded the output limit.".into());
    }
    let text = fs::read_to_string(&text_path)
        .map_err(|_| "Whisper returned an invalid transcript.".to_string())?;
    drop(files);
    Ok(VoiceTranscript {
        text: text.trim().to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn wav(data_size: usize) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend(b"RIFF");
        v.extend(((36 + data_size) as u32).to_le_bytes());
        v.extend(b"WAVEfmt ");
        v.extend(16u32.to_le_bytes());
        v.extend(1u16.to_le_bytes());
        v.extend(1u16.to_le_bytes());
        v.extend(16_000u32.to_le_bytes());
        v.extend(32_000u32.to_le_bytes());
        v.extend(2u16.to_le_bytes());
        v.extend(16u16.to_le_bytes());
        v.extend(b"data");
        v.extend((data_size as u32).to_le_bytes());
        v.resize(44 + data_size, 0);
        v
    }
    #[test]
    fn accepts_pcm16_and_rejects_invalid_or_long_wav() {
        assert!(validate_wav(&wav(32_000)).is_ok());
        assert!(validate_wav(b"nope").is_err());
        let mut trailing_bytes = wav(32_000);
        trailing_bytes.push(0);
        assert!(validate_wav(&trailing_bytes).is_err());
        assert!(validate_wav(&wav(90 * 32_000 + 2)).is_err());
    }
    #[test]
    fn decodes_base64_and_rejects_bad_padding() {
        assert_eq!(decode_base64("UklGRg==").unwrap(), b"RIFF");
        assert!(decode_base64("UklGR===").is_err());
    }
    #[test]
    fn config_missing_model_is_explicit() {
        let saved = env::var_os("MONITTER_WHISPER_MODEL");
        env::set_var("MONITTER_WHISPER_MODEL", "/definitely/missing/model");
        assert!(configured_model()
            .unwrap_err()
            .contains("MONITTER_WHISPER_MODEL"));
        match saved {
            Some(v) => env::set_var("MONITTER_WHISPER_MODEL", v),
            None => env::remove_var("MONITTER_WHISPER_MODEL"),
        }
    }

    #[test]
    fn config_missing_explicit_cli_is_not_silently_ignored() {
        let saved = env::var_os("MONITTER_WHISPER_CLI");
        env::set_var("MONITTER_WHISPER_CLI", "/definitely/missing/whisper-cli");
        assert!(configured_executable()
            .unwrap_err()
            .contains("MONITTER_WHISPER_CLI"));
        match saved {
            Some(v) => env::set_var("MONITTER_WHISPER_CLI", v),
            None => env::remove_var("MONITTER_WHISPER_CLI"),
        }
    }
}
