# Local conversational voice

Status: the isolated prototype has been exercised, and Alex reports that it is
working well. Integration into the normal desktop package is in progress.
This feedback supports proceeding with integration; it is not a claim that
all devices, interruption scenarios, or the new package have been verified.

## Behavior and scope

The native GPUI voice control connects a selected local Codex chat to a separate
Swift audio helper. Parakeet EOU 120M recognises speech locally; Phonon 31M speaks
new assistant commentary and answers. The existing Codex agent generates the
responses through its existing account. Speech processing uses no hosted API;
this does not make the conversation agent itself offline.

Voice is available on Apple-silicon macOS 15 or later. It is enabled by default
on supported builds; `MONITTER_LOCAL_VOICE=0` disables the control. Existing
composer dictation remains separate. Microphone capture begins only when the
user starts or resumes voice. Open microphone and push-to-talk are separate
modes. Type instead retains the orb and exposes the composer. Starting voice
in another pane stops the old audio session and leaves that pane resumable.
End voice does not cancel the agent's work.

Input onset pauses playback. Substantive final interjections cancel obsolete
speech and enter normal chat admission; short backchannels can resume playback.
Questions and approvals continue to require their existing explicit UI controls.
Provisional transcripts do not execute tools or become committed user turns.

The backend owns renewable task-bound leases and durable deduplication receipts.
Canonical user-message or queue state and admission receipts commit atomically.
The client serialises admission, bounds pending finals, and does not blindly
retry an uncertain mutation. Delivered means persisted in chat, not provider
acknowledgement. A crash between persistence and agent launch is not exactly-once
execution. Playback freshness uses canonical message timestamps; it cannot prove
semantic causality within a provider turn.

## Build and local models

Requirements are the normal GPUI build prerequisites plus Xcode/Swift on
Apple-silicon macOS 15 or later. From the repository root:

```sh
python3 scripts/voice-native-download.py
npm run check:gpui
npm run build:gpui
```

The downloader verifies pinned public assets. Its default destination is
`~/Library/Caches/Monitter/VoiceModels`. Set `MONITTER_VOICE_MODELS` or pass
`--models-dir` to the downloader to use another absolute directory; the runtime
uses the same environment override. Model acquisition is explicit, not a build
or application-start side effect. Local model data and build outputs are not
committed to Git.

The normal supported macOS package builds the pinned Swift helper, includes its
resource bundle, and signs the finished app. Debug and release packages use
matching helper configurations. Unsupported macOS hosts still build the desktop
without this voice helper; Linux packaging is unchanged. Build metadata records
whether native voice was included. Packaging does not install or restart the GUI
or persistent backend.

`MONITTER_NATIVE_VOICE_HELPER` can select another absolute helper executable for
development. Normally the helper is found alongside the GUI executable inside
the app. See the [native helper guide](../desktop-gpui/native-voice/README.md) for
asset revisions, licenses, protocol details and standalone probes.

## Isolated preview

```sh
node scripts/run-local-voice-prototype.mjs
```

The runner creates separately named and signed preview bundles, with a private
backend profile under `artifacts/local-voice-preview/profile/`. It does not
replace the installed app or use production data. Select a local Codex chat,
then start voice. `--no-build` is only for an already verified current build;
`--prepare-only` stages the preview without opening it.

The preview wrapper pins the profile and speech paths, including when launched
through LaunchServices, and removes inherited OpenAI/Codex API-key variables.
The normal app retains its existing account configuration. Optional
`--voice-diagnostics` records allowlisted numerical and lifecycle diagnostics;
it does not record transcript text, microphone PCM, or arbitrary helper stderr.
Diagnostics are disabled by default.

## Verification

Run source-level checks from the repository root:

```sh
npm run check:gpui
node scripts/gpui-toolchain.mjs cargo test --manifest-path desktop-gpui/Cargo.toml --locked -p monitter --lib voice_
node scripts/gpui-toolchain.mjs cargo test --manifest-path desktop-gpui/Cargo.toml --locked --lib voice_
swift test --package-path desktop-gpui/native-voice --disable-automatic-resolution --jobs 2
```

Backend tests cover admission, stale leases, durable receipt identity, queued
conversion and edited-message detection. Conversation-policy tests cover
history filtering, revisions, bounds, interruption and backchannels. Transport
tests cover bounded parsing, event coalescing, critical event order, and a real
child process writing while the foreground consumer is stalled. Permission
checks cover denial, cancellation, disconnected channels and expired grants.

Model and physical-device probes have separate opt-in commands in the helper
guide. A synthetic sink cannot be accepted as actual audio by the controller.
Microphone buffer progress and playback callbacks are component evidence, not
proof of a natural sustained conversation. Live acceptance checks should cover:

- Several spoken turns with returned audio and correct chat admission.
- An interjection during speech, with obsolete playback stopped.
- Push-to-talk release, mute/unmute, End, and subsequent Resume.
- Pane handoff with no duplicate admission or old queued audio.
- Type instead retaining both the orb and an editable composer.
- Existing dictation and agent operation remaining intact.

Alex has reported successful use of the prototype and authorised integration.
Earlier testing also observed intermittent open-mic backlog, variable cold
model loading, and audio-engine configuration changes. The prototype added
bounded/coalesced transport, asynchronous stdin reads, a main-thread permission
gate, capture watchdogs, and a CPU/ANE inference configuration. Route/engine
changes still stop capture visibly and require Resume. The engine notification
does not always identify an actual device change. Automatic recovery and
universal Bluetooth reliability are not established.

## Later improvements

UI refinement follows integration. Streaming provisional transcripts into a
cancellable draft-only response path is a proposal, not implemented behavior.
It would require revision matching and final commitment before speech or tool
execution. Moshi is excluded for this machine's memory constraints.
