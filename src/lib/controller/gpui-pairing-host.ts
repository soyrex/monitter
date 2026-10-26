import type { DesktopBridge } from './remote-client';
import type { Snapshot, TerminalRead, TerminalSession } from '$lib/types';

interface PairingHost { invoke(command: string, args?: Record<string, unknown>): Promise<unknown>; }
/** This adapter is supplied only by the isolated, bundled GPUI pairing WebView.
 * The Rust host pins the entry URL and enforces its own command allow-list. */
export function pairingHost(): PairingHost | null {
  if (typeof window === 'undefined' || window.location.origin !== 'http://127.0.0.1:19438'
      || !['/gpui-pairing','/gpui-pairing/'].includes(window.location.pathname)) return null;
  return (window as unknown as { __MONITTER_PAIRING_HOST__?: PairingHost }).__MONITTER_PAIRING_HOST__ ?? null;
}
export function pairingHostBridge(): DesktopBridge | null {
  const host = pairingHost();
  if (!host) return null;
  return {
    getSnapshot: () => host.invoke('get_snapshot') as Promise<Snapshot>,
    sendMessage: (taskId, text, attachmentIds) => host.invoke('send_message', { taskId, text, attachmentIds }) as Promise<Snapshot>,
    cancelTask: taskId => host.invoke('cancel_task', { taskId }) as Promise<Snapshot>,
    resumeTask: taskId => host.invoke('resume_task', { taskId }) as Promise<Snapshot>,
    listTerminals: () => host.invoke('list_terminals') as Promise<TerminalSession[]>,
    readTerminal: (id, afterSeq) => host.invoke('read_terminal', { id, afterSeq }) as Promise<TerminalRead>,
  };
}
