import { writable } from 'svelte/store';

export type MobileSpeechMode = 'desktop' | 'on-device';

const preferenceKey = 'monitter:mobile-speech-mode:v1';

function storedMode(): MobileSpeechMode {
  if (typeof window === 'undefined') return 'desktop';
  try { return window.localStorage.getItem(preferenceKey) === 'on-device' ? 'on-device' : 'desktop'; }
  catch { return 'desktop'; }
}

// This belongs to the phone, not the desktop workspace or its shared Settings.
// The existing desktop Whisper path stays selected on upgraded phones.
export const mobileSpeechMode = writable<MobileSpeechMode>(storedMode());

if (typeof window !== 'undefined') {
  window.addEventListener('storage', event => {
    if (event.key === preferenceKey) mobileSpeechMode.set(event.newValue === 'on-device' ? 'on-device' : 'desktop');
  });
}

export function setMobileSpeechMode(mode: MobileSpeechMode): void {
  mobileSpeechMode.set(mode);
  if (typeof window === 'undefined') return;
  try { window.localStorage.setItem(preferenceKey, mode); } catch { /* private storage may be unavailable */ }
}

type NativeSpeechPlatform = 'ios' | 'android';
export type NativeSpeechEventType = 'availability' | 'started' | 'partial' | 'final' | 'error' | 'stopped';
export interface NativeSpeechEvent {
  sessionId: string;
  type: NativeSpeechEventType;
  available?: boolean;
  text?: string;
  error?: string;
}

type NativeSpeechWindow = Window & {
  webkit?: { messageHandlers?: { monitterSpeech?: { postMessage: (payload: object) => void } } };
  monitterSpeechAndroid?: { postMessage: (payload: string) => void };
};

export function nativeSpeechPlatform(): NativeSpeechPlatform | null {
  if (typeof window === 'undefined') return null;
  const host = window as NativeSpeechWindow;
  if (host.webkit?.messageHandlers?.monitterSpeech) return 'ios';
  if (host.monitterSpeechAndroid) return 'android';
  return null;
}

export function sendNativeSpeechCommand(action: 'status' | 'start' | 'stop' | 'cancel', sessionId: string, locale?: string): boolean {
  if (typeof window === 'undefined') return false;
  const host = window as NativeSpeechWindow;
  const payload = { action, sessionId, ...(locale ? { locale } : {}) };
  if (host.webkit?.messageHandlers?.monitterSpeech) {
    host.webkit.messageHandlers.monitterSpeech.postMessage(payload);
    return true;
  }
  if (host.monitterSpeechAndroid) {
    host.monitterSpeechAndroid.postMessage(JSON.stringify(payload));
    return true;
  }
  return false;
}

const eventTypes = new Set<NativeSpeechEventType>(['availability', 'started', 'partial', 'final', 'error', 'stopped']);

export function subscribeNativeSpeech(listener: (event: NativeSpeechEvent) => void): () => void {
  if (typeof window === 'undefined') return () => {};
  const handle = (event: Event) => {
    const detail = (event as CustomEvent<unknown>).detail;
    if (!detail || typeof detail !== 'object') return;
    const value = detail as Partial<NativeSpeechEvent>;
    if (typeof value.sessionId !== 'string' || value.sessionId.length > 100 ||
        typeof value.type !== 'string' || !eventTypes.has(value.type as NativeSpeechEventType)) return;
    listener(value as NativeSpeechEvent);
  };
  window.addEventListener('monitter:native-speech', handle);
  return () => window.removeEventListener('monitter:native-speech', handle);
}
