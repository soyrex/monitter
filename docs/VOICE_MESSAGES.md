# Voice messages (desktop and owner LAN)

The composer records up to 90 seconds and transcribes the recording on the
desktop Mac with whisper.cpp. Review the transcript before pressing Send. Only
the transcript enters the message and agent prompt: Monitter does not create a
WAV attachment or retain the recording. Transcription uses temporary files
that are removed when the command completes. Recording is available in task,
draft, and ordinary channel composers; project board notes remain text only.

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

For an owner LAN browser, recording uses that device's microphone; the WAV is
sent to the desktop Mac only for transcription. A browser-trusted HTTPS origin
and microphone permission are required. The LAN device does not need
whisper.cpp or the model. The transcript returns to the browser composer for
review, and only that text follows the normal agent message path. Ordinary
HTTP LAN pages cannot use browser microphone capture. Voice remains unavailable
to paired mobile controllers and shared visitor clients.

## LAN HTTPS setup

Use a hostname you control that resolves to the Mac's private LAN address from
the client devices. Obtain a browser-trusted certificate for that hostname;
[DNS-01 validation](https://letsencrypt.org/docs/challenge-types/) works without
exposing the Monitter server to the public Internet. Configure the installed
desktop app at `~/Library/Application Support/com.monitter.desktop/lan-tls.json`:

```json
{
  "hostname": "lan.example.com",
  "certPath": "/absolute/path/to/fullchain.pem",
  "keyPath": "/absolute/path/to/privkey.pem"
}
```

Keep the private key readable only by the desktop user. Monitter loads the
certificate and key at startup and advertises `https://lan.example.com:18437/`
in Settings → LAN access. Restart Monitter after certificate renewal. If the
configuration, certificate parsing or key loading fails, LAN startup reports
an error and does not expose a plain HTTP owner interface. Browsers reject
expired, untrusted or hostname-mismatched certificates. Port 18436 remains HTTP on loopback for
developer tooling while TLS is configured. Without TLS configuration, the
legacy HTTP LAN interface remains available, but remote browser recording is
blocked by browser secure-context rules. The LAN interface grants owner-level
workspace control to trusted LAN users; HTTPS provides encryption, not a
separate visitor permission level.
