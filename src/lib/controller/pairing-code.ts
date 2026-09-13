/** Public rendezvous only. Private keys and transport secrets never enter this API. */
export const DEFAULT_RELAY = 'wss://api.monitter.com/relay';
export interface PairingRegistration { code: string; expiresAt: number; revocationToken: string; }
function endpoint(relayUrl: string): URL {
  const url = new URL(relayUrl);
  if (url.protocol !== 'wss:' && !(url.protocol === 'ws:' && ['localhost','127.0.0.1','[::1]'].includes(url.hostname))) throw new Error('A secure relay is required.');
  url.protocol = url.protocol === 'wss:' ? 'https:' : 'http:';
  url.pathname = '/pairing'; url.search = ''; url.hash = '';
  return url;
}
async function request<T>(url: URL, options: RequestInit, timeoutMs: number, read: (response: Response) => Promise<T>): Promise<T> {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), timeoutMs);
  try {
    // Keep the deadline active while reading the body too. Do not retry a
    // create/claim automatically: the first request may already have succeeded.
    return await read(await fetch(url, { ...options, signal: controller.signal }));
  } catch (reason) {
    const name = reason && typeof reason === 'object' && 'name' in reason ? reason.name : undefined;
    if (controller.signal.aborted || name === 'AbortError' || name === 'TimeoutError') {
      throw new Error(`The pairing service at ${url.hostname} timed out. Check your connection and try again.`);
    }
    if (reason instanceof TypeError) {
      throw new Error(`Could not reach the pairing service at ${url.hostname}. Check your network connection and try again.`);
    }
    throw reason;
  } finally { clearTimeout(timer); }
}
async function payload(response: Response): Promise<any> {
  if (!response.ok) {
    if (response.status === 429) throw new Error('Too many pairing attempts. Wait a minute and try again.');
    if (response.status === 404 || response.status === 410) throw new Error('This device key was used or expired. Create a new one on your desktop.');
    throw new Error('Pairing service unavailable. Try scanning the desktop QR code.');
  }
  const text = await response.text();
  if (text.length > 4096) throw new Error('Invalid pairing response.');
  try {
    const value = JSON.parse(text);
    if (!value || typeof value !== 'object' || Array.isArray(value)) throw new Error();
    return value;
  } catch { throw new Error('Invalid pairing response.'); }
}
export async function registerPairingCode(invitationJson: string): Promise<PairingRegistration> {
  const invitation = JSON.parse(invitationJson);
  if (invitation.version !== 2 || 'secret' in invitation) throw new Error('Update the desktop to use device keys.');
  const value = await request(endpoint(invitation.relayUrl), {method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({invitation})}, 15000, payload);
  if (!/^\d{9}$/.test(value.code) || typeof value.revocationToken !== 'string' || typeof value.expiresAt !== 'number') throw new Error('Invalid pairing response.');
  return value;
}
export async function redeemPairingCode(code: string, relayUrl = DEFAULT_RELAY): Promise<string> {
  const clean = code.replace(/[\s-]/g,'');
  if (!/^\d{9}$/.test(clean)) throw new Error('Enter the nine-digit device key shown on your desktop.');
  const url = endpoint(relayUrl); url.pathname += '/' + clean;
  const value = await request(url, {cache:'no-store'}, 15000, payload);
  if (!value.invitation || value.invitation.version !== 2 || 'secret' in value.invitation || value.invitation.relayUrl !== relayUrl || value.expiresAt <= Date.now()) throw new Error('Invalid or expired device key.');
  return JSON.stringify(value.invitation);
}
export async function revokePairingCode(relayUrl: string, registration: PairingRegistration): Promise<void> {
  const url = endpoint(relayUrl); url.pathname += '/' + registration.code;
  try {
    await request(url, {method:'DELETE',headers:{Authorization:'Bearer '+registration.revocationToken}}, 10000, async response => {
      if (!response.ok && response.status !== 404 && response.status !== 410) throw new Error('Could not revoke device key.');
    });
  } catch (reason) {
    throw new Error(`${reason instanceof Error ? reason.message : 'Could not revoke device key.'} Unclaimed device keys expire automatically after five minutes.`);
  }
}
