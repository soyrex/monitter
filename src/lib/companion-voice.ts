import { sendNativeSpeechCommand, subscribeNativeSpeech, type NativeSpeechEvent } from '$lib/mobile-speech';

export interface CompanionVoiceState {
  phase: 'off' | 'arming' | 'waiting' | 'listening' | 'finishing' | 'error';
  transcript: string;
  error: string;
}

type Source = 'wake' | 'manual';
type VoiceOptions = {
  onstate: (state: CompanionVoiceState) => void;
  ontranscript: (text: string, taskId: string, source: Source, autoSendReady?: boolean) => void;
  onlevel?: (level: number) => void;
};

const ROTATE_MS = 45_000;
const POST_WAKE_TIMEOUT_MS = 15_000;
const SILENCE_MS = 1_800;
const AUTO_SEND_SILENCE_MS = 11_000;
const MAX_TEXT = 20_000;
const START_TIMEOUT_MS = 12_000;
const FINAL_TIMEOUT_MS = 9_000;
const MAX_NO_SPEECH_RETRIES = 2;

type VoiceWindow = Window & {
  webkit?: { messageHandlers?: {
    monitterSpeech?: { postMessage: (payload: object) => void };
    monitterCompanion?: { postMessage: (payload: { active: boolean }) => void };
  } };
};

export function isCompanionAvailable(): boolean {
  if (typeof window === 'undefined') return false;
  const handlers = (window as VoiceWindow).webkit?.messageHandlers;
  return !!handlers?.monitterSpeech && !!handlers?.monitterCompanion;
}

export function setCompanionActive(active: boolean): boolean {
  if (typeof window === 'undefined') return false;
  const handler = (window as VoiceWindow).webkit?.messageHandlers?.monitterCompanion;
  if (!handler) return false;
  try { handler.postMessage({ active }); return true; }
  catch { return false; }
}

export function createCompanionVoice(options: VoiceOptions) {
  let state: CompanionVoiceState = { phase: 'off', transcript: '', error: '' };
  let mode: 'wake' | 'manual' | null = null;
  let taskId = '';
  let sessionId = '';
  let triggered = false;
  let delivered = false;
  let command = '';
  let segment = '';
  let settledCommand = '';
  let autoSend = false;
  let quietElapsed = false;
  let completion: 'manual' | 'auto' | null = null;
  let bareCommandSession = false;
  let noSpeechRetries = 0;
  let destroyed = false;
  let unsubscribe: (() => void) | undefined;
  let rotationTimer: ReturnType<typeof setTimeout> | undefined;
  let commandTimer: ReturnType<typeof setTimeout> | undefined;
  let silenceTimer: ReturnType<typeof setTimeout> | undefined;
  let watchdogTimer: ReturnType<typeof setTimeout> | undefined;
  let visibilityHandler: (() => void) | undefined;

  function publish(phase: CompanionVoiceState['phase'], transcript = state.transcript, error = '') {
    state = { phase, transcript, error };
    options.onstate({ ...state });
  }

  function clearSessionTimers() {
    if (rotationTimer) clearTimeout(rotationTimer);
    if (commandTimer) clearTimeout(commandTimer);
    if (watchdogTimer) clearTimeout(watchdogTimer);
    rotationTimer = commandTimer = watchdogTimer = undefined;
  }

  function clearTimers() {
    clearSessionTimers();
    if (silenceTimer) clearTimeout(silenceTimer);
    silenceTimer = undefined;
  }

  function detach() {
    clearTimers();
    unsubscribe?.(); unsubscribe = undefined;
    if (typeof document !== 'undefined' && visibilityHandler) document.removeEventListener('visibilitychange', visibilityHandler);
    visibilityHandler = undefined;
  }

  function cancelSession() {
    const retired = sessionId;
    sessionId = '';
    if (retired) sendNativeSpeechCommand('cancel', retired);
  }

  function stop() {
    cancelSession();
    detach();
    mode = null; taskId = ''; triggered = false; delivered = false; command = '';
    segment = ''; settledCommand = ''; completion = null; quietElapsed = false;
    options.onlevel?.(0);
    publish('off', '');
  }

  function fail(message: string) {
    cancelSession(); detach(); mode = null;
    options.onlevel?.(0);
    publish('error', state.transcript, message.slice(0, 300));
  }

  function freshId(): string {
    try { return crypto.randomUUID(); }
    catch { return `voice-${Date.now()}-${Math.random().toString(36).slice(2)}`; }
  }

  function startSession() {
    if (!mode) return;
    // Keep a message's silence deadline across recognizer utterance boundaries.
    clearSessionTimers();
    sessionId = freshId();
    options.onlevel?.(0);
    const current = sessionId;
    publish(mode === 'manual' ? 'arming' : triggered ? 'listening' : 'arming', command);
    watchdogTimer = setTimeout(() => {
      if (sessionId === current && mode) fail('Native speech did not start. Re-enable listening to try again.');
    }, START_TIMEOUT_MS);
    if (!sendNativeSpeechCommand('start', current)) {
      fail('On-device speech is unavailable in this app.');
      return;
    }
    if (mode === 'wake' && !triggered) {
      rotationTimer = setTimeout(() => {
        if (sessionId !== current || !mode || triggered) return;
        sessionId = '';
        sendNativeSpeechCommand('cancel', current);
        startSession();
      }, ROTATE_MS);
    }
    if (mode === 'wake' && triggered) {
      commandTimer = setTimeout(() => {
        if (sessionId === current && mode && triggered && !delivered && !command) fail('No command followed “Hey Mo”. Listening stopped.');
      }, POST_WAKE_TIMEOUT_MS);
    }
  }

  function normalizeWords(value: string): string {
    return value.toLocaleLowerCase().replace(/[’']/g, '').replace(/[^\p{L}\p{N}]+/gu, ' ').trim().replace(/\s+/g, ' ');
  }

  function wakeRemainder(value: string, allowRevision = false): { found: boolean; remainder: string } {
    // "A Mo" is only a correction of an already observed wake phrase. It must
    // never activate listening by itself or become the command after activation.
    const match = value.match(/(?:^|[^\p{L}\p{N}])hey[\s,.;:!?-]+moe?(?=$|[^\p{L}\p{N}])/iu)
      ?? (allowRevision ? value.match(/^a[\s,.;:!?-]+moe?(?=$|[^\p{L}\p{N}])/iu) : null);
    if (!match || match.index === undefined) return { found: false, remainder: '' };
    const end = match.index + match[0].length;
    return { found: true, remainder: value.slice(end).replace(/^[\s,.;:!?-]+/, '').trim() };
  }

  function clearCommand() {
    segment = '';
    command = settledCommand;
    if (!command) {
      if (silenceTimer) clearTimeout(silenceTimer);
      silenceTimer = undefined;
      quietElapsed = false;
    }
    publish('listening', command);
  }

  function acceptCommand(value: string) {
    const clean = value.trim().replace(/\s+/g, ' ').slice(0, MAX_TEXT);
    if (!clean) return;
    const changed = clean !== segment;
    segment = clean;
    command = [settledCommand, segment].filter(Boolean).join(' ').slice(0, MAX_TEXT);
    publish('listening', command);
    if (!changed || (!autoSend && mode === 'manual')) return;
    quietElapsed = false;
    if (silenceTimer) clearTimeout(silenceTimer);
    silenceTimer = setTimeout(() => {
      if (!mode || delivered || !command) return;
      quietElapsed = autoSend;
      if (autoSend && !segment && settledCommand) deliver(command, mode, true);
      else finishCapture(autoSend ? 'auto' : 'manual');
    }, autoSend ? AUTO_SEND_SILENCE_MS : SILENCE_MS);
  }

  function completeUtterance() {
    if (!mode) return;
    if (!command) { fail('No speech was recognized. Try again.'); return; }
    if (!autoSend || completion === 'manual') {
      deliver(command, mode);
    } else if (quietElapsed) {
      deliver(command, mode, true);
    } else {
      // Speech recognizers often finalize after a short pause. Continue listening
      // for the rest of this message until its full eleven-second pause elapses.
      settledCommand = command;
      segment = '';
      bareCommandSession = true;
      completion = null;
      cancelSession();
      startSession();
    }
  }

  function receiveText(value: string, final: boolean) {
    if (!mode) return;
    const text = value.trim().slice(0, MAX_TEXT);
    if (mode === 'manual') {
      if (text) acceptCommand(text);
      else if (final) clearCommand();
      if (final) completeUtterance();
      return;
    }
    if (!triggered) {
      const parsed = wakeRemainder(text);
      if (parsed.found) {
        triggered = true;
        bareCommandSession = false;
        if (commandTimer) clearTimeout(commandTimer);
        commandTimer = setTimeout(() => {
          if (mode && triggered && !delivered && !command) fail('No command followed “Hey Mo”. Listening stopped.');
        }, POST_WAKE_TIMEOUT_MS);
        const normalized = normalizeWords(parsed.remainder);
        if (normalized === 'cancel') { stop(); return; }
        if (parsed.remainder) acceptCommand(parsed.remainder);
        else {
          publish('listening', '');
          if (final) {
            const old = sessionId;
            sessionId = '';
            sendNativeSpeechCommand('cancel', old);
            bareCommandSession = true;
            startSession();
          }
        }
      }
      // Pre-trigger ambient speech is deliberately discarded.
      return;
    }
    const repeated = wakeRemainder(text, true);
    if (!bareCommandSession && !repeated.found) {
      // Native results revise the entire utterance. A revision that no longer
      // contains the wake phrase is not a new command, nor proof of the old one.
      clearCommand();
      if (final) {
        triggered = false;
        rotateAfterFinal();
      }
      return;
    }
    const subsequent = repeated.found ? repeated.remainder : text;
    const normalized = normalizeWords(subsequent);
    if (normalized === 'cancel') { stop(); return; }
    if (subsequent) acceptCommand(subsequent);
    else clearCommand();
    if (final) {
      if (command) completeUtterance();
      else {
        // The recognizer commonly emits a partial wake phrase before ending
        // that utterance. Continue in a fresh session for the spoken command.
        cancelSession();
        bareCommandSession = true;
        startSession();
      }
    }
  }

  function deliver(text: string, source: Source, autoSendReady = false) {
    const finalText = text.trim().slice(0, MAX_TEXT);
    if (!finalText || delivered || !taskId) { if (!finalText && mode === 'manual') fail('No speech was recognized. Try again.'); return; }
    delivered = true;
    const target = taskId;
    detach();
    cancelSession();
    mode = null;
    options.onlevel?.(0);
    publish('off', finalText);
    if (autoSendReady) options.ontranscript(finalText, target, source, true);
    else options.ontranscript(finalText, target, source);
  }

  function rotateAfterFinal() {
    const retired = sessionId;
    sessionId = '';
    if (retired) sendNativeSpeechCommand('cancel', retired);
    startSession();
  }

  function handleEvent(event: NativeSpeechEvent) {
    if (!sessionId || event.sessionId !== sessionId || !mode) return;
    if (event.type === 'started') {
      if (watchdogTimer) clearTimeout(watchdogTimer);
      watchdogTimer = undefined;
      publish(mode === 'wake' && !triggered ? 'waiting' : 'listening', command);
    } else if (event.type === 'level') {
      if (typeof event.level === 'number' && Number.isFinite(event.level) && event.level >= 0 && event.level <= 1) options.onlevel?.(event.level);
    } else if (event.type === 'partial') {
      receiveText(typeof event.text === 'string' ? event.text : '', false);
    } else if (event.type === 'final') {
      if (mode === 'wake' && !triggered) {
        receiveText(typeof event.text === 'string' ? event.text : '', true);
        if (mode && !triggered) rotateAfterFinal();
        else if (mode && triggered && command) completeUtterance();
      } else receiveText(typeof event.text === 'string' ? event.text : '', true);
    } else if (event.type === 'error') {
      const message = typeof event.error === 'string' && event.error ? event.error : 'On-device speech failed. Re-enable listening to try again.';
      const noSpeech = message === 'Speech recognition did not return a final transcript.' || message === 'No speech detected';
      if (autoSend && completion !== 'manual' && settledCommand && !segment && noSpeech) {
        // An empty recognizer segment during the pause doesn't lose the prior
        // finalized text or restart its silence clock.
        rotateAfterFinal();
      } else if (mode === 'wake' && !triggered && noSpeech && noSpeechRetries < MAX_NO_SPEECH_RETRIES) {
        noSpeechRetries += 1;
        const old = sessionId;
        sessionId = '';
        sendNativeSpeechCommand('cancel', old);
        startSession();
      } else fail(message);
    } else if (event.type === 'stopped') {
      if (mode === 'manual' && state.phase === 'finishing' && command) deliver(command, 'manual');
      else if (mode === 'wake' && !triggered) rotateAfterFinal();
      else if (mode) fail('Speech recognition stopped. Re-enable listening to try again.');
    }
  }

  function begin(nextMode: 'wake' | 'manual', target: string, auto = false) {
    if (destroyed || !target || (typeof document !== 'undefined' && document.visibilityState === 'hidden')) return;
    stop();
    mode = nextMode; taskId = target; triggered = nextMode === 'manual'; delivered = false; command = '';
    autoSend = auto;
    bareCommandSession = false; noSpeechRetries = 0;
    unsubscribe = subscribeNativeSpeech(handleEvent);
    visibilityHandler = () => { if (document.visibilityState === 'hidden') stop(); };
    if (typeof document !== 'undefined') document.addEventListener('visibilitychange', visibilityHandler);
    startSession();
  }

  function finishCapture(reason: 'manual' | 'auto') {
    if (!sessionId || !mode) return;
    if (reason === 'manual' && mode === 'wake' && !command) { stop(); return; }
    completion = reason;
    if (settledCommand && !segment) {
      deliver(command, mode, reason === 'auto' && quietElapsed);
      return;
    }
    clearTimers();
    publish('finishing', command);
    const current = sessionId;
    watchdogTimer = setTimeout(() => {
      if (sessionId === current && mode) fail('Native speech did not finish. Re-enable listening to try again.');
    }, FINAL_TIMEOUT_MS);
    sendNativeSpeechCommand('stop', sessionId);
  }

  return {
    listen(target: string, auto = false) { if (isCompanionAvailable()) begin('wake', target, auto); else fail('Companion voice is available only in the paired iOS app.'); },
    record(target: string, auto = false) { if (isCompanionAvailable()) begin('manual', target, auto); else fail('On-device speech is unavailable in this app.'); },
    finish() { finishCapture('manual'); },
    stop,
    destroy() { stop(); destroyed = true; },
  };
}
