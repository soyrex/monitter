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
async function payload(response: Response): Promise<any> {
  if (!response.ok) {
    if (response.status === 429) throw new Error('Too many pairing attempts. Wait a minute and try again.');
    if (response.status === 404 || response.status === 410) throw new Error('This device key was used or expired. Create a new one on your desktop.');
    throw new Error('Pairing service unavailable. Try scanning the desktop QR code.');
  }
  const text = await response.text();
  if (text.length > 4096) throw new Error('Invalid pairing response.');
  return JSON.parse(text);
}
export async function registerPairingCode(invitationJson: string): Promise<PairingRegistration> {
  const invitation = JSON.parse(invitationJson);
  if (invitation.version !== 2 || 'secret' in invitation) throw new Error('Update the desktop to use device keys.');
  const value = await payload(await fetch(endpoint(invitation.relayUrl), {method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({invitation}),signal:AbortSignal.timeout(15000)}));
  if (!/^\d{9}$/.test(value.code) || typeof value.revocationToken !== 'string' || typeof value.expiresAt !== 'number') throw new Error('Invalid pairing response.');
  return value;
}
export async function redeemPairingCode(code: string, relayUrl = DEFAULT_RELAY): Promise<string> {
  const clean = code.replace(/[\s-]/g,'');
  if (!/^\d{9}$/.test(clean)) throw new Error('Enter the nine-digit device key shown on your desktop.');
  const url = endpoint(relayUrl); url.pathname += '/' + clean;
  const value = await payload(await fetch(url,{cache:'no-store',signal:AbortSignal.timeout(15000)}));
  if (!value.invitation || value.invitation.version !== 2 || 'secret' in value.invitation || value.invitation.relayUrl !== relayUrl || value.expiresAt <= Date.now()) throw new Error('Invalid or expired device key.');
  return JSON.stringify(value.invitation);
}
export async function revokePairingCode(relayUrl: string, registration: PairingRegistration): Promise<void> {
  const url = endpoint(relayUrl); url.pathname += '/' + registration.code;
  const response = await fetch(url,{method:'DELETE',headers:{Authorization:'Bearer '+registration.revocationToken},signal:AbortSignal.timeout(10000)});
  if (!response.ok && response.status !== 404 && response.status !== 410) throw new Error('Could not revoke device key. It expires automatically after five minutes.');
}
