<script lang="ts">
  import { onDestroy, onMount, tick } from 'svelte';
  import Mic from '@lucide/svelte/icons/mic';
  import Square from '@lucide/svelte/icons/square';
  import X from '@lucide/svelte/icons/x';
  import LoaderCircle from '@lucide/svelte/icons/loader-circle';
  import { isOwnerRelayBrowser, localUuid } from '$lib/lan';
  import { nativeSpeechPlatform, sendNativeSpeechCommand, subscribeNativeSpeech } from '$lib/mobile-speech';

  let { disabled = false, contextKey = '', ontranscript }: { disabled?: boolean; contextKey?: string; ontranscript: (text: string, contextKey: string) => void | Promise<void> } = $props();
  let available = $state(false);
  let checking = $state(true);
  let requesting = $state(false);
  let recording = $state(false);
  let finishing = $state(false);
  let elapsed = $state(0);
  let error = $state('');
  let partial = $state('');
  let reviewText = $state('');
  let reviewContextKey = '';
  let reviewAdded = $state(false);
  let reviewPending = $state(false);
  let transcriptBody = $state<HTMLElement>();
  let statusId = '';
  let sessionId = '';
  let startedContextKey = '';
  let startedAt = 0;
  let ticker: ReturnType<typeof setInterval> | undefined;
  let unsubscribe: (() => void) | undefined;
  let destroyed = false;

  const platform = nativeSpeechPlatform();
  const ownerWorkspace = isOwnerRelayBrowser();
  const timeLabel = $derived(`${Math.floor(elapsed / 60)}:${String(elapsed % 60).padStart(2, '0')}`);
  const showingTranscript = $derived(requesting || recording || finishing || !!reviewText);

  $effect(() => {
    partial;
    reviewText;
    void tick().then(() => { if (transcriptBody) transcriptBody.scrollTop = transcriptBody.scrollHeight; });
  });

  function clearTicker() {
    if (ticker) clearInterval(ticker);
    ticker = undefined;
  }

  function reset() {
    clearTicker();
    sessionId = '';
    startedContextKey = '';
    requesting = false;
    recording = false;
    finishing = false;
    partial = '';
    elapsed = 0;
  }

  function start() {
    if (disabled || !available || requesting || recording || finishing) return;
    error = '';
    partial = '';
    reviewText = '';
    reviewContextKey = '';
    reviewAdded = false;
    sessionId = localUuid();
    startedContextKey = contextKey;
    requesting = true;
    if (!sendNativeSpeechCommand('start', sessionId)) {
      error = 'On-device speech is unavailable in this app.';
      reset();
    }
  }

  function stop() {
    if (!sessionId || finishing) return;
    finishing = true;
    recording = false;
    clearTicker();
    sendNativeSpeechCommand('stop', sessionId);
  }

  function cancel() {
    if (sessionId) sendNativeSpeechCommand('cancel', sessionId);
    error = '';
    reset();
  }

  function addTranscript(text: string, key: string) {
    if (reviewPending) return;
    reviewPending = true;
    void Promise.resolve(ontranscript(text, key))
      .then(() => { if (!destroyed) reviewAdded = true; })
      .catch(reason => { if (!destroyed) error = `Could not add the transcript: ${reason instanceof Error ? reason.message : String(reason)}`; })
      .finally(() => { if (!destroyed) { reviewPending = false; reset(); } });
  }

  onMount(() => {
    statusId = localUuid();
    unsubscribe = subscribeNativeSpeech(value => {
      if (value.type === 'availability' && value.sessionId === statusId) {
        available = value.available === true;
        checking = false;
        return;
      }
      if (!sessionId || value.sessionId !== sessionId) return;
      if (value.type === 'started') {
        requesting = false;
        recording = true;
        startedAt = Date.now();
        ticker = setInterval(() => {
          elapsed = Math.min(90, Math.floor((Date.now() - startedAt) / 1000));
          if (elapsed >= 90) stop();
        }, 200);
      } else if (value.type === 'partial') {
        partial = typeof value.text === 'string' ? value.text.slice(0, 20_000) : '';
      } else if (value.type === 'final') {
        const transcript = typeof value.text === 'string' ? value.text.trim().slice(0, 20_000) : '';
        clearTicker();
        recording = false;
        requesting = false;
        finishing = true;
        if (!transcript) { error = 'No speech was recognized. Try again.'; reset(); return; }
        reviewText = transcript;
        reviewContextKey = startedContextKey;
        addTranscript(transcript, reviewContextKey);
      } else if (value.type === 'error') {
        error = typeof value.error === 'string' && value.error ? value.error.slice(0, 300) : 'On-device transcription failed.';
        reset();
      } else if (value.type === 'stopped') {
        if (recording || finishing) error = 'No speech was recognized. Try again.';
        reset();
      }
    });
    if (!sendNativeSpeechCommand('status', statusId)) checking = false;
    return () => { unsubscribe?.(); };
  });

  onDestroy(() => {
    destroyed = true;
    if (sessionId) sendNativeSpeechCommand('cancel', sessionId);
    clearTicker();
    unsubscribe?.();
  });
</script>

<div class="native-speech-recorder" class:owner-workspace={ownerWorkspace}>
  {#if showingTranscript}
    <section class="transcript-panel" aria-label="On-device speech transcript">
      <header>
        <strong>{reviewText ? 'Transcript' : finishing ? 'Finishing transcript' : recording ? 'Listening' : 'Starting microphone'}</strong>
        {#if recording}<span class="elapsed"><span class="dot" aria-hidden="true"></span>{timeLabel} / 1:30</span>{/if}
        {#if reviewText}<button type="button" class="dismiss" aria-label="Dismiss transcript preview" onclick={() => reviewText = ''}><X size={14}/></button>{/if}
      </header>
      <div class="transcript-body" bind:this={transcriptBody} aria-live="off">{reviewText || partial || (recording ? 'Speak now. Your words will appear here.' : finishing ? 'Waiting for the final words…' : 'Waiting for microphone permission…')}</div>
      {#if reviewText}<small>{reviewAdded ? 'Added to your editable message. Review it before sending.' : reviewPending ? 'Adding to your message…' : 'Could not add this text. You can try again.'}</small>{/if}
      {#if reviewText && !reviewAdded && !reviewPending}<button type="button" class="retry" onclick={() => addTranscript(reviewText, reviewContextKey)}>Add to message</button>{/if}
    </section>
  {/if}
  {#if recording}
    <button type="button" class="control stop" aria-label="Stop recording" title="Stop and transcribe on this device" onclick={stop}><Square size={15} fill="currentColor"/></button>
    <button type="button" class="control cancel" aria-label="Cancel recording" title="Discard recording" onclick={cancel}><X size={15}/></button>
  {:else if requesting || finishing}
    <span class="spinner" role="status" aria-label={finishing ? 'Transcribing on this device' : 'Waiting for microphone'}><LoaderCircle size={14}/></span>
    <button type="button" class="control cancel" aria-label="Cancel recording" title="Cancel transcription" onclick={cancel}><X size={15}/></button>
  {:else}
    <button type="button" class="control record" aria-label="Record voice message on this device" title={available ? 'Record voice message on this device' : 'On-device speech is unavailable for this language or device'} disabled={disabled || checking || !available || !platform} onclick={start}><Mic size={16}/></button>
  {/if}
  {#if error}<small class="error" role="alert">{error}</small>{/if}
  {#if !checking && !available}<small class="error" role="status">On-device speech is unavailable for this language or device.</small>{/if}
</div>

<style>
  .native-speech-recorder{position:relative;display:inline-flex;align-items:center;gap:6px;flex:none;min-height:30px;font:11px/1.35 var(--interface-font,sans-serif);color:var(--muted)}
  .control{width:30px;height:30px;min-width:30px;display:grid;place-items:center;border:1px solid var(--line);border-radius:8px;background:var(--panel);color:var(--ink);cursor:pointer}
  .control:hover:not(:disabled){border-color:var(--accent);color:var(--accent)}
  .control:disabled{opacity:.45;cursor:not-allowed}
  .stop{color:#df665e;border-color:#df665e}
  .cancel{color:var(--muted)}
  .owner-workspace{position:static}
  .transcript-panel{position:absolute;left:0;bottom:calc(100% + 18px);z-index:30;display:flex;flex-direction:column;gap:9px;box-sizing:border-box;width:min(360px,calc(100vw - 24px));max-height:min(42dvh,320px);padding:12px 14px;border:1px solid var(--line);border-radius:11px;background:var(--panel);color:var(--ink);box-shadow:0 12px 38px #0006;text-align:left;pointer-events:auto}
  .owner-workspace .transcript-panel{left:0;right:0;bottom:calc(100% + 8px);width:100%}
  .transcript-panel header{display:flex;align-items:center;gap:10px;flex:none;color:var(--ink);font:600 12px/1.4 var(--interface-font,sans-serif)}
  .transcript-panel header strong{flex:1}
  .elapsed{display:inline-flex;align-items:center;gap:5px;color:var(--muted);font:10px var(--mono,monospace)}
  .dismiss{display:grid;place-items:center;flex:none;width:24px;height:24px;border:0;border-radius:5px;background:transparent;color:var(--muted);cursor:pointer}
  .dismiss:hover{background:var(--soft);color:var(--ink)}
  .transcript-body{min-height:44px;overflow:auto;overscroll-behavior:contain;white-space:pre-wrap;overflow-wrap:anywhere;color:var(--ink);font:var(--chat-font-weight,400) max(16px,var(--chat-font-size,13px))/1.5 var(--chat-font,"IBM Plex Sans",system-ui,sans-serif)}
  .transcript-panel small{flex:none;color:var(--muted);font:10px/1.4 var(--interface-font,sans-serif)}
  .retry{align-self:flex-start;padding:5px 8px;border:1px solid var(--line);border-radius:6px;background:var(--soft);color:var(--ink);cursor:pointer}
  .dot{width:7px;height:7px;border-radius:50%;background:#df665e;animation:pulse 1.2s infinite}
  .spinner{animation:spin 1s linear infinite}
  .error{max-width:260px;color:#d46b62;line-height:1.3}
  @keyframes pulse{50%{opacity:.35}}
  @keyframes spin{to{transform:rotate(360deg)}}
</style>
