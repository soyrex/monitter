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

No domain, public DNS record, or certificate service is required. At first LAN
startup, the desktop generates a private local certificate authority (CA) for
this installation and saves it in its app data. It creates an HTTPS server
certificate for the Mac's current private IP addresses on port 18437. Settings
→ LAN access shows the HTTPS addresses, offers the public root certificate for
download, and shows its SHA-256 fingerprint. The root's private key stays on
the desktop. Ordinary trusted-LAN HTTP remains available on port 18436 for
existing clients, but browser microphone capture requires HTTPS.

To use a browser microphone on another device:

1. In desktop Settings → LAN access, note an HTTPS address and the CA
   fingerprint. Export the **public** root certificate.
2. Transfer the public certificate to the device over a trusted channel. Before
   trusting it, compare its SHA-256 fingerprint with the desktop display. A
   download from an untrusted HTTP page alone does not prove identity.
3. Install and explicitly trust that root certificate on the device. On iOS,
   manually installed roots additionally need full trust enabled under Settings
   → General → About → Certificate Trust Settings.
4. Open the HTTPS IP address, allow microphone access, and record. The device
   sends the temporary audio to the Mac for transcription; the composer and
   harness receive only the transcript.

Trust is a per-device setup step. Installing a local root grants trust to
certificates signed by that root; only do this for your own desktop and devices.
Keep the private CA directory under the desktop app-data directory private and
back it up securely if you need the same identity after reinstalling. Replacing
the CA requires trusting the new root on client devices. The server regenerates
its IP certificate when Monitter starts; restart after the Mac's LAN IP changes
or after a year of continuous uptime. An invalid/incomplete stored CA causes a
visible LAN startup error rather than silent identity replacement.

For advanced setups, `lan-tls.json` can instead specify a certificate for a
hostname you control, such as `lan.monitter.com`. Configure it in
`~/Library/Application Support/com.monitter.desktop/lan-tls.json`:

```json
{
  "hostname": "lan.monitter.com",
  "certPath": "/absolute/path/to/fullchain.pem",
  "keyPath": "/absolute/path/to/privkey.pem"
}
```

A browser-trusted certificate and LAN DNS resolution are then your
responsibility. [DNS-01 validation](https://letsencrypt.org/docs/challenge-types/)
can issue a public certificate without exposing the server to the Internet.
When a custom TLS configuration is present, HTTP remains on loopback only.
A partial/invalid custom configuration fails LAN startup rather than falling
back to the owner-level HTTP LAN interface. Restart after custom certificate
renewal. Neither HTTPS option adds visitor permissions: trusted LAN users can
control the owner workspace, including terminals.
