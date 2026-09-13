import { invoke } from '@tauri-apps/api/core';

export const isLanBrowser = () => typeof document !== 'undefined' && (
  (import.meta.env.DEV && import.meta.env.MODE === 'monitter-web') ||
  document.documentElement.hasAttribute('data-monitter-lan') || !!document.querySelector('meta[name="monitter-lan"]')
);
const storageKey = 'monitter.lan.access';
export function accessKey(): string { return sessionStorage.getItem(storageKey) ?? ''; }
export function setAccessKey(key: string) { sessionStorage.setItem(storageKey, key.trim()); }
export async function lanInvoke<T>(command: string, args: Record<string, unknown> = {}): Promise<T> {
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
export interface LanServerInfo { urls: string[]; token: string; error: string | null; accessCodeRequired: boolean; }
export const getLanServerInfo = () => invoke<LanServerInfo>('get_lan_server_info');

// randomUUID requires HTTPS in browsers; getRandomValues also works on LAN HTTP.
export function localUuid(): string {
  const bytes = crypto.getRandomValues(new Uint8Array(16));
  bytes[6] = (bytes[6] & 15) | 64;
  bytes[8] = (bytes[8] & 63) | 128;
  const hex = Array.from(bytes, byte => byte.toString(16).padStart(2, '0')).join('');
  return `${hex.slice(0,8)}-${hex.slice(8,12)}-${hex.slice(12,16)}-${hex.slice(16,20)}-${hex.slice(20)}`;
}
