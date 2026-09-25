# Voice messages (desktop)

The desktop composer records up to 90 seconds, saves the recording as a WAV
attachment, and transcribes it locally with whisper.cpp. Review the transcript
before pressing Send. The recording remains attached, and sent audio can be
played from its attachment card. Recording is available in task, draft, and
ordinary channel composers; project board notes remain text only.

## macOS setup

Install the free [whisper.cpp Homebrew formula](https://formulae.brew.sh/formula/whisper.cpp):

```sh
brew install whisper.cpp
```

Download the multilingual `ggml-base.bin` model from the
[official whisper.cpp model repository](https://huggingface.co/ggerganov/whisper.cpp)
to `~/Library/Application Support/Monitter/whisper/models/ggml-base.bin`.
The model repository currently lists SHA-1
`465707469ff3a37a2b9b8d8f89f2f99de7299dac` for that file. Verify the
download before using it.

The app discovers Homebrew's `whisper-cli` and that model path automatically.
For other locations, set `MONITTER_WHISPER_CLI` and `MONITTER_WHISPER_MODEL`
in the desktop app's environment. Both must point to existing local files.
The model is not included in the app bundle.

On first use, macOS asks for microphone access. Whisper may also take longer
on its first run while Metal prepares its kernels. If microphone access was
denied, allow Monitter in System Settings → Privacy & Security → Microphone.

Recording and transcription run on the desktop. Once you press Send, the
transcript and attached WAV follow the normal agent message path. A task on
an SSH host stores its attachment in that task's remote workspace, and an
agent that reads the attachment may send its content to its model provider.
The voice commands are unavailable to LAN, mobile, and shared browser clients.
