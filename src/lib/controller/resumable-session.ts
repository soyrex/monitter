import {
  createDesktopSession, createMobileSession,
  type DesktopBridge, type DesktopSession, type MobileSession,
  type PersistentDesktopIdentity, type PersistentMobileIdentity, type RemoteConnectionState,
} from './remote-client';
import type { ControllerSendReceipt } from './protocol';
import type { Snapshot, TerminalRead, TerminalSession } from '../types';

export interface ResumableDesktopOptions {
  readonly relayUrl: string;
  readonly bridge: DesktopBridge;
  readonly identity: PersistentDesktopIdentity;
  readonly isTrustedController?: (publicKey: string) => boolean | Promise<boolean>;
  readonly onControllerAccess?: (publicKey: string) => void | Promise<void>;
}
export interface ResumableMobileOptions { readonly invitation: string; readonly identity: PersistentMobileIdentity; }
export interface ResumableDesktopSession extends DesktopSession { reconnectNow(): Promise<void>; }
export interface ResumableMobileSession extends MobileSession { reconnectNow(): Promise<void>; }

/** Memory-only reconnect orchestration. Identity/trust persistence belongs to the caller. */
export async function createResumableDesktopSession(options: ResumableDesktopOptions): Promise<ResumableDesktopSession> {
  let current: DesktopSession | null = null, permanentlyClosed = false, attempt = 0, opening: Promise<void> | null = null;
  let retryTimer: ReturnType<typeof setTimeout> | null = null, generation = 0;
  let unsubscribeState: (() => void) | null = null, unsubscribePending: (() => void) | null = null;
  const listeners = new Set<(state: RemoteConnectionState) => void>(), pending = new Set<() => void>();
  const notify = (state: RemoteConnectionState) => listeners.forEach(listener => listener(state));
  const cancelRetry = () => { if (retryTimer) clearTimeout(retryTimer); retryTimer = null; };
  const detach = () => { unsubscribeState?.(); unsubscribePending?.(); unsubscribeState = null; unsubscribePending = null; };
  const scheduleRetry = (ownedGeneration: number) => {
    if (permanentlyClosed || retryTimer || ownedGeneration !== generation) return;
    const wait = Math.min(5_000, 250 * 2 ** Math.min(attempt++, 5));
    retryTimer = setTimeout(() => {
      retryTimer = null;
      if (!permanentlyClosed && ownedGeneration === generation) void open().catch(reason => {
        if (permanentlyClosed) return;
        notify({ status: 'error', error: reason instanceof Error ? reason.message : 'Controller reconnection failed.' });
        scheduleRetry(generation);
      });
    }, wait);
  };
  const open = async (): Promise<void> => {
    if (permanentlyClosed) return;
    if (opening) return opening;
    const ownedGeneration = ++generation;
    opening = (async () => {
      const next = await createDesktopSession(options.relayUrl, options.bridge, { identity: options.identity, isTrustedController: options.isTrustedController, onControllerAccess: options.onControllerAccess });
      if (permanentlyClosed || ownedGeneration !== generation) return next.close();
      const previous = current;
      detach(); current = null; previous?.close(); current = next;
      unsubscribeState = next.subscribe(state => {
        if (ownedGeneration !== generation) return;
        notify(state);
        if (state.status === 'connected') { attempt = 0; cancelRetry(); }
        else if (state.status === 'closed' || state.status === 'error') scheduleRetry(ownedGeneration);
      });
      unsubscribePending = next.onPendingPeer(() => { if (ownedGeneration === generation) pending.forEach(listener => listener()); });
    })().finally(() => { opening = null; });
    return opening;
  };
  await open();
  const invitation = (current as DesktopSession | null)?.invitation;
  if (!invitation) throw new Error('Controller connection is unavailable.');
  return {
    get invitation() { return invitation; },
    getStatus: () => current?.getStatus() ?? 'closed', getVerificationCode: () => current?.getVerificationCode() ?? null,
    getPeer: () => current?.getPeer() ?? null, getPeerControllerPublicKey: () => current?.getPeerControllerPublicKey() ?? null,
    subscribe(listener) { listeners.add(listener); listener({ status: current?.getStatus() ?? 'closed', ...(current?.getVerificationCode() ? { verificationCode: current.getVerificationCode()! } : {}) }); return () => listeners.delete(listener); },
    onPendingPeer(listener) { pending.add(listener); return () => pending.delete(listener); },
    approve: () => current?.approve() ?? Promise.reject(new Error('Controller connection is unavailable.')),
    reject: () => current?.reject() ?? Promise.reject(new Error('Controller connection is unavailable.')),
    reconnectNow: async () => {
      cancelRetry();
      // Visibility and pageshow may request the same foreground reconnect in
      // one tick. Share the active factory instead of invalidating it.
      if (opening) return opening;
      generation += 1; detach(); const previous = current; current = null; previous?.close(); await open();
    },
    close: () => { permanentlyClosed = true; generation += 1; cancelRetry(); detach(); const previous = current; current = null; previous?.close(); },
  };
}

export async function createResumableMobileSession(options: ResumableMobileOptions): Promise<ResumableMobileSession> {
  let current: MobileSession | null = null, permanentlyClosed = false, attempt = 0, opening: Promise<void> | null = null;
  let retryTimer: ReturnType<typeof setTimeout> | null = null, generation = 0;
  let unsubscribeState: (() => void) | null = null;
  const listeners = new Set<(state: RemoteConnectionState) => void>();
  const notify = (state: RemoteConnectionState) => listeners.forEach(listener => listener(state));
  const cancelRetry = () => { if (retryTimer) clearTimeout(retryTimer); retryTimer = null; };
  const scheduleRetry = (ownedGeneration: number) => {
    if (permanentlyClosed || retryTimer || ownedGeneration !== generation) return;
    const wait = Math.min(5_000, 250 * 2 ** Math.min(attempt++, 5));
    retryTimer = setTimeout(() => {
      retryTimer = null;
      if (!permanentlyClosed && ownedGeneration === generation) void open().catch(reason => {
        if (permanentlyClosed) return;
        notify({ status: 'error', error: reason instanceof Error ? reason.message : 'Controller reconnection failed.' });
        scheduleRetry(generation);
      });
    }, wait);
  };
  const open = async (): Promise<void> => {
    if (permanentlyClosed) return;
    if (opening) return opening;
    const ownedGeneration = ++generation;
    opening = (async () => {
      const next = await createMobileSession(options.invitation, undefined, { identity: options.identity });
      if (permanentlyClosed || ownedGeneration !== generation) return next.close();
      const previous = current;
      unsubscribeState?.(); unsubscribeState = null; current = null; previous?.close(); current = next;
      unsubscribeState = next.subscribe(state => {
        if (ownedGeneration !== generation) return;
        notify(state);
        if (state.status === 'connected') { attempt = 0; cancelRetry(); }
        else if (state.status === 'rejected') cancelRetry();
        else if (state.status === 'closed' || state.status === 'error') scheduleRetry(ownedGeneration);
      });
    })().finally(() => { opening = null; }); return opening;
  };
  const requireCurrent = () => current ?? (() => { throw new Error('Controller connection is unavailable.'); })();
  await open();
  return {
    getStatus: () => current?.getStatus() ?? 'closed', getVerificationCode: () => current?.getVerificationCode() ?? null,
    supportsRememberedDevices: () => current?.supportsRememberedDevices() ?? false,
    subscribe(listener) { listeners.add(listener); listener({ status: current?.getStatus() ?? 'closed', ...(current?.getVerificationCode() ? { verificationCode: current.getVerificationCode()! } : {}) }); return () => listeners.delete(listener); },
    reconnectNow: async () => {
      cancelRetry();
      if (opening) return opening;
      generation += 1; unsubscribeState?.(); unsubscribeState = null; const previous = current; current = null; previous?.close(); await open();
    },
    close: () => { permanentlyClosed = true; generation += 1; cancelRetry(); unsubscribeState?.(); unsubscribeState = null; const previous = current; current = null; previous?.close(); },
    getSnapshot: () => requireCurrent().getSnapshot(), sendMessage: (id, text) => requireCurrent().sendMessage(id, text) as Promise<Snapshot | ControllerSendReceipt>,
    cancelTask: id => requireCurrent().cancelTask(id), resumeTask: id => requireCurrent().resumeTask(id), listTerminals: () => requireCurrent().listTerminals(), readTerminal: (id, seq) => requireCurrent().readTerminal(id, seq),
  };
}
