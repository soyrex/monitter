# Mo for iOS

Mo is Monitter's foreground desk companion in the paired iOS owner workspace.
Mo works with a selected non-archived chat and the existing desktop connection.
It does not run in the background or replace the normal mobile controller and
owner workspace.

## Voice behavior and privacy

Voice is available only in the paired iOS app, where the bundled main frame has
both native speech and companion handlers. Mo's large **Tap to talk** area
occupies at least half the idle portrait screen's height. The visible face is
60% of that area's width and height, centered inside it. Tap Mo's face, then tap it
again to stop listening and review. Mo's mouth reacts to the local microphone
level while listening, and idle motion respects the iPhone's Reduce Motion
setting. The **Hey Mo** toggle sits below the face, followed by the optional
pause-to-send control. These three controls fill the
main companion view. The toggle explicitly activates wake listening.
Recognition uses iOS on-device speech recognition
with partial results to detect “Hey Mo”; the common recognition “Hey Moe” is
accepted too. This is regular foreground speech
recognition, not a dedicated low-power keyword model. It rotates bounded
recognition sessions while waiting for the phrase. Ambient text before the
phrase is discarded. “A Mo” by itself does not trigger wake listening. If Mo
hears only “Hey Mo,” it waits for a following command; without one, the request
times out visibly and nothing is drafted or sent. Speech audio is not sent to
the desktop or a cloud speech service.

The idle screen has no empty text field or Send button. Recognized speech opens
an outgoing review bubble with **Edit** and **Send** controls. Manual review is
the default. Enable **Auto-send after a
10-second pause** to send after eleven seconds without new speech. Shorter
recognizer utterances are joined into one message; further speech resets that
timer. An unchanged partial/final result does not reset it. A successful wake
auto-send rearms listening for the next wake phrase. A failed or ambiguous send
is never automatically replayed.

Editing, switching chats, disconnecting, or disabling listening cancels any
automatic send and preserves recognized text for review. Existing unsent text
is combined with new speech and always requires explicit Send. Unsent drafts
survive closing and reopening Companion Mode within the same page session;
reloading the page clears them. A revised wake phrase is never sent as a message,
and a correction that removes the trigger discards its earlier partial command.

**Tap to talk** starts a manual on-device session. **Stop** always finishes into
the draft for review, including when auto-send is enabled. The Send button can
also send the text currently visible in the bubble. Speech cannot resolve
approvals or answer approval inputs; those retain their explicit controls.
The attention section appears only when eligible chats have pending requests.
Desktop final replies appear automatically as speech-bubble popups over Mo.
Opening Companion Mode shows the selected chat's latest reply once; new replies
from eligible chats then queue one at a time. Each popup identifies its source
chat, supports scrolling long Markdown, and stays visible until dismissed.
Repeated snapshots and reconnects do not replay dismissed replies during the
same Companion Mode visit. Popups never change the pinned voice target; an
explicit **Reply in this chat** action can select a different source chat.
Floating replies can be dismissed without changing the listening state. The
face and any outgoing review controls remain reachable alongside the popup.

The companion-active message is separate from the unchanged `monitterSpeech`
command and event contract. iOS accepts `monitterCompanion` only from the
paired main frame at `http://127.0.0.1:31977`, with a body containing only
`{ active: boolean }`. iOS keeps the screen awake while Companion Mode is
visible and the app is foregrounded. Leaving the foreground, an audio
interruption or input-route loss, navigation, page closure, closing Companion
Mode, or losing the desktop connection stops microphone capture. Native start
and finalization waits are bounded. Errors remain visible and require the user
to enable listening again; microphone or speech authorization failures do not
cause retry loops.

## Local checks and builds

Build the static assets before the WebKit UI check, which runs a local encrypted
test relay and mocks only the iOS speech callbacks:

```sh
npm run check
node scripts/companion-voice-test.mjs
node scripts/companion-replies-test.mjs
npx vite build
node scripts/companion-ui-run.mjs
```

For a direct browser development preview, use `npm run dev -- --host
127.0.0.1`; browser preview does not provide the paired iOS native handlers,
so native voice will report unavailable. Build the static Vite assets and copy
them into the iOS app with:

```sh
MONITTER_SKIP_WEB_BUILD=1 ./mobile-ios/build.sh
```

Compile the bundled Swift sources against the installed iOS Simulator SDK with:

```sh
MONITTER_SKIP_WEB_BUILD=1 ./mobile-ios/build-simulator.sh
```

This compiles native code and creates a local simulator artifact; it does not
launch a simulator or verify microphone behavior on an iPhone. Physical-device
acceptance remains necessary.

## Physical iPhone acceptance

Use a paired iPhone, an approved owner workspace, a reachable desktop, and a
real test chat. Confirm each behavior directly on the device:

1. Open Companion Mode, select a chat, and leave the foreground screen idle.
   The menu's Mo button sits beside Settings on the left. The companion has only
   a subtle connection dot and close control in its upper corners; the face is
   the primary talk control, with both voice toggles below it.
   Confirm the display stays awake while the mode is visible and the app remains
   foregrounded.
2. Enable wake listening and grant iOS microphone and speech recognition access
   when prompted. Speak ambient words without the phrase and confirm no draft or
   send occurs. Say “A Mo” and confirm it does not trigger. Say only “Hey Mo”
   and confirm Mo waits for a command, then times out without drafting or
   sending. Say “Hey Mo” (then repeat using “Hey Moe”) followed by a short
   command. Confirm it appears in the outgoing bubble and waits for Send.
3. Start wake listening for one chat, switch chats, and confirm recognition
   stops. Verify pending requests in other chats are represented by the visible
   count and are not affected by voice.
4. Tap Mo's face, speak a message, tap the face again, review or edit the draft,
   and send it with the large send control. Enable auto-send and speak two
   utterances less than ten seconds apart; confirm they stay in one message and
   send once only after a pause longer than ten seconds. Confirm Stop, editing,
   and an existing draft require review instead. Voice never resolves approvals.
5. During listening, background the app or interrupt/change the audio route;
   also test desktop disconnect and closing Companion Mode. Confirm capture
   stops, visible errors require user action to retry, and a failed send is not
   replayed automatically.
6. Receive a desktop agent's final reply and confirm it pops up over Mo. Dismiss
   it and confirm polling/reconnection does not show it again. Receive two
   replies, including one from another chat; verify the queue, readable source
   labels, scrolling, and that the pinned voice target changes only when you
   explicitly choose **Reply in this chat**.

These steps are a test checklist, not evidence that the current build has been
run on a physical device.
