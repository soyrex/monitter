import { invokeCommand } from './command-invoke';
import { createCommandGuard } from './command-protocol';
import { OwnerRelayDisconnectedError } from './owner-relay-error.js';
export { OwnerRelayDisconnectedError, isOwnerRelayDisconnectedError } from './owner-relay-error.js';
export type { LanServerInfo } from './types';

/** Installed mobile shells serve their bundled UI from fixed private origins. */
export const isIOSBundleOrigin = () => typeof window !== 'undefined' &&
  window.location.origin === 'http://127.0.0.1:31977';
export const isAndroidBundleOrigin = () => typeof window !== 'undefined' &&
  window.location.origin === 'https://appassets.androidplatform.net';
export const isMobileBundleOrigin = () => isIOSBundleOrigin() || isAndroidBundleOrigin();
/** The bundled root is always an owner-relay workspace, even if a WebView drops the query. */
export const isOwnerRelayBrowser = () => isMobileBundleOrigin() && window.location.pathname === '/';

export const isLanBrowser = () => typeof document !== 'undefined' && (
  (import.meta.env.DEV && (import.meta.env.MODE === 'monitter-web' || import.meta.env.MODE === 'monitter-app-ui')) ||
  document.documentElement.hasAttribute('data-monitter-lan') || !!document.querySelector('meta[name="monitter-lan"]') ||
  isOwnerRelayBrowser()
);
type OwnerRelayInvoker = (command: string, args: Record<string, unknown>) => Promise<unknown>;
let ownerRelayInvoker: OwnerRelayInvoker | null = null;
/** Only the owner relay gate registers this provider after an approved session. */
export function setOwnerRelayInvoker(invoke: OwnerRelayInvoker | null): void { ownerRelayInvoker = invoke; }
export const isOwnerRelayReconnecting = () => isOwnerRelayBrowser() && !ownerRelayInvoker;
/** A phone's connection to its paired desktop drops and recovers routinely on
 * cellular/Wi-Fi handoffs. Surfacing every transient drop as a popup is noisy;
 * only a run of consecutive failed reconnects is worth interrupting the user
 * about. OwnerRelayGate reports each attempt's outcome as it happens. */
const OWNER_RELAY_RECONNECT_ALERT_THRESHOLD = 5;
let ownerRelayReconnectFailures = 0;
export function noteOwnerRelayReconnectOutcome(connected: boolean): void {
  ownerRelayReconnectFailures = connected ? 0 : ownerRelayReconnectFailures + 1;
}
export const isOwnerRelayReconnectPersistent = () => ownerRelayReconnectFailures >= OWNER_RELAY_RECONNECT_ALERT_THRESHOLD;
const storageKey = 'monitter.lan.access';
export function accessKey(): string { return sessionStorage.getItem(storageKey) ?? ''; }
export function setAccessKey(key: string) { sessionStorage.setItem(storageKey, key.trim()); }
const requireCompatibleCommand = createCommandGuard(() => lanRequest('get_command_capabilities', {}));
export async function lanInvoke<T>(command: string, args: Record<string, unknown> = {}): Promise<T> {
  await requireCompatibleCommand(command);
  return lanRequest<T>(command, args);
}
async function lanRequest<T>(command: string, args: Record<string, unknown>): Promise<T> {
  if (isOwnerRelayBrowser()) {
    const invoke = ownerRelayInvoker;
    if (!invoke) throw new OwnerRelayDisconnectedError();
    try { return await invoke(command, args) as T; }
    catch (reason) {
      // A request can lose its transport after dispatch. Classify that failure
      // for quiet background refreshes; never retry the command here.
      if (invoke !== ownerRelayInvoker) throw new OwnerRelayDisconnectedError();
      throw reason;
    }
  }
  const response = await fetch('/api/invoke', {
    method: 'POST', headers: { 'Content-Type': 'application/json', Authorization: `Bearer ${accessKey()}` },
    body: JSON.stringify({ command, args }),
  });
  if (response.status === 401) {
    window.dispatchEvent(new Event('monitter-lan-unauthorized'));
    throw new Error('Incorrect or expired access code. Check Monitter Settings → LAN access.');
  }
  const data = await response.json();
  if (!response.ok || !data.ok) throw new Error(data.error || 'Monitter could not complete this request.');
  return data.result as T;
}
export async function lanAccessCodeRequired(): Promise<boolean> {
  try {
    const response = await fetch('/api/access', { cache: 'no-store' });
    if (!response.ok) return true;
    return (await response.json()).required !== false;
  } catch { return true; }
}
export const getLanServerInfo = () => invokeCommand('get_lan_server_info');

// randomUUID requires HTTPS in browsers; getRandomValues also works on LAN HTTP.
export function localUuid(): string {
  const bytes = crypto.getRandomValues(new Uint8Array(16));
  bytes[6] = (bytes[6] & 15) | 64;
  bytes[8] = (bytes[8] & 63) | 128;
  const hex = Array.from(bytes, byte => byte.toString(16).padStart(2, '0')).join('');
  return `${hex.slice(0,8)}-${hex.slice(8,12)}-${hex.slice(12,16)}-${hex.slice(16,20)}-${hex.slice(20)}`;
}
