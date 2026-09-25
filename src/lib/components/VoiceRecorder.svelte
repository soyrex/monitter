<script lang="ts">
  import { onDestroy } from 'svelte';
  import { Mic, Square, X, LoaderCircle } from '@lucide/svelte';
  import { audioBufferToWav16k } from '$lib/voice-wav';

  type RecordedFile = { filename: string; mimeType: string; dataBase64: string };
  let { disabled = false, onrecorded }: { disabled?: boolean; onrecorded: (file: RecordedFile) => Promise<void> } = $props();

  const maxSeconds = 90;
  let recording = $state(false);
  let requesting = $state(false);
  let processing = $state(false);
  let elapsed = $state(0);
  let error = $state('');
  let stream: MediaStream | undefined;
  let recorder: MediaRecorder | undefined;
  let chunks: Blob[] = [];
  let startedAt = 0;
  let ticker: ReturnType<typeof setInterval> | undefined;
  let cancelled = false;
  let destroyed = false;

  const timeLabel = $derived(`${Math.floor(elapsed / 60)}:${String(elapsed % 60).padStart(2, '0')}`);

  function releaseStream() {
    stream?.getTracks().forEach(track => track.stop());
    stream = undefined;
  }

  function clearTicker() {
    if (ticker) clearInterval(ticker);
    ticker = undefined;
  }

  async function start() {
    if (requesting || recording || processing) return;
    error = '';
    elapsed = 0;
    cancelled = false;
    if (!navigator.mediaDevices?.getUserMedia || typeof MediaRecorder === 'undefined' || typeof AudioContext === 'undefined') {
      error = 'Voice recording is not supported in this browser.';
      return;
    }
    requesting = true;
    try {
      stream = await navigator.mediaDevices.getUserMedia({ audio: true });
      if (destroyed) { releaseStream(); return; }
      chunks = [];
      recorder = new MediaRecorder(stream);
      recorder.ondataavailable = event => { if (event.data.size) chunks.push(event.data); };
      recorder.onerror = () => { error = 'Recording failed. Check microphone access and try again.'; recording = false; clearTicker(); releaseStream(); };
      recorder.onstop = () => { void finishRecording(); };
      recorder.start();
      startedAt = Date.now();
      recording = true;
      ticker = setInterval(() => {
        elapsed = Math.min(maxSeconds, Math.floor((Date.now() - startedAt) / 1000));
        if (elapsed >= maxSeconds && recorder?.state === 'recording') recorder.stop();
      }, 200);
    } catch (reason) {
      releaseStream();
      if (reason instanceof DOMException && (reason.name === 'NotAllowedError' || reason.name === 'SecurityError')) {
        error = 'Microphone access was denied. Allow microphone access in your browser settings and try again.';
      } else if (reason instanceof DOMException && reason.name === 'NotFoundError') {
        error = 'No microphone was found.';
      } else {
        error = `Could not start recording: ${reason instanceof Error ? reason.message : String(reason)}`;
      }
    } finally {
      requesting = false;
    }
  }

  function stop() {
    if (recorder?.state === 'recording') recorder.stop();
  }

  function cancel() {
    cancelled = true;
    error = '';
    if (recorder?.state === 'recording') recorder.stop();
    else {
      clearTicker();
      releaseStream();
      recording = false;
      processing = false;
    }
  }

  async function finishRecording() {
    clearTicker();
    recording = false;
    releaseStream();
    if (cancelled) {
      chunks = [];
      return;
    }
    if (!chunks.length) {
      error = 'No audio was captured. Try recording again.';
      return;
    }

    processing = true;
    let context: AudioContext | undefined;
    try {
      const source = new Blob(chunks, { type: recorder?.mimeType || chunks[0].type });
      const bytes = await source.arrayBuffer();
      context = new AudioContext();
      const decoded = await context.decodeAudioData(bytes);
      if (cancelled) return;
      const wav = audioBufferToWav16k(decoded);
      const dataBase64 = await blobToBase64(wav);
      if (cancelled) return;
      const filename = `voice-message-${new Date().toISOString().replace(/[:.]/g, '-')}.wav`;
      await onrecorded({ filename, mimeType: 'audio/wav', dataBase64 });
    } catch (reason) {
      error = `Could not prepare the recording: ${reason instanceof Error ? reason.message : String(reason)}`;
    } finally {
      await context?.close().catch(() => {});
      chunks = [];
      processing = false;
      recorder = undefined;
    }
  }

  function blobToBase64(blob: Blob): Promise<string> {
    return new Promise((resolve, reject) => {
      const reader = new FileReader();
      reader.onload = () => {
        const result = reader.result;
        if (typeof result !== 'string') return reject(new Error('Could not read the WAV audio.'));
        resolve(result.slice(result.indexOf(',') + 1));
      };
      reader.onerror = () => reject(reader.error ?? new Error('Could not read the WAV audio.'));
      reader.readAsDataURL(blob);
    });
  }

  onDestroy(() => {
    destroyed = true;
    cancelled = true;
    clearTicker();
    if (recorder?.state === 'recording') recorder.stop();
    releaseStream();
  });
</script>

<div class="voice-recorder">
  {#if recording}
    <span class="recording-status" role="status"><span class="dot"></span>Recording {timeLabel} / 1:30</span>
    <button type="button" class="control stop" aria-label="Stop recording" title="Stop recording" onclick={stop}><Square size={15} fill="currentColor"/></button>
    <button type="button" class="control cancel" aria-label="Cancel recording" title="Cancel recording" onclick={cancel}><X size={15}/></button>
  {:else if processing}
    <span class="recording-status" role="status"><span class="spinner"><LoaderCircle size={14}/></span>Preparing audio…</span>
    <button type="button" class="control cancel" aria-label="Cancel recording" title="Discard recording" onclick={cancel}><X size={15}/></button>
  {:else if requesting}
    <span class="recording-status" role="status"><span class="spinner"><LoaderCircle size={14}/></span>Waiting for microphone…</span>
  {:else}
    <button type="button" class="control record" aria-label="Record voice message" title="Record voice message (up to 90 seconds)" disabled={disabled} onclick={start}><Mic size={16}/></button>
    <span class="hint">Voice message</span>
  {/if}
  {#if error}<span class="error" role="alert">{error}</span>{/if}
</div>

<style>
  .voice-recorder{display:inline-flex;align-items:center;gap:6px;min-width:0;min-height:30px;font:11px/1.35 var(--interface-font,sans-serif);color:var(--muted)}
  .control{display:grid;place-items:center;width:28px;height:28px;padding:0;border:1px solid var(--line);border-radius:7px;background:var(--panel);color:var(--ink);cursor:pointer;flex:none}
  .control:hover:not(:disabled){background:var(--soft);color:var(--accent-ink,var(--accent))}
  .control:focus-visible{outline:2px solid var(--accent);outline-offset:2px}
  .control:disabled{opacity:.45;cursor:not-allowed}
  .record{color:var(--accent-ink,var(--accent))}.stop{color:var(--danger,#c44)}.cancel{color:var(--muted)}
  .recording-status{display:inline-flex;align-items:center;gap:6px;white-space:nowrap;font-variant-numeric:tabular-nums;color:var(--ink)}
  .dot{width:7px;height:7px;border-radius:50%;background:var(--danger,#c44);animation:pulse 1s ease-in-out infinite}
  .hint{white-space:nowrap}.error{max-width:min(320px,50vw);color:var(--danger,#b84c44);overflow-wrap:anywhere}
  .spinner{display:inline-flex;animation:spin 1s linear infinite}
  @keyframes pulse{50%{opacity:.35}}@keyframes spin{to{transform:rotate(360deg)}}
  @media(prefers-reduced-motion:reduce){.dot,.spinner{animation:none}}
</style>
