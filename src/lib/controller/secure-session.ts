/**
 * Small, deliberately in-memory cryptographic session primitive for the paired
 * controller.  A new key is derived after every socket connection, so callers
 * must never resume or replay a mutation after a disconnect.
 */
const encoder = new TextEncoder();
const decoder = new TextDecoder();
function source(bytes: Uint8Array): ArrayBuffer { return new Uint8Array(bytes).buffer as ArrayBuffer; }

export interface SecureEnvelope { readonly type: 'secure'; readonly seq: number; readonly ciphertext: string; }

export function bytesToBase64url(bytes: Uint8Array): string {
  let binary = '';
  for (const value of bytes) binary += String.fromCharCode(value);
  return btoa(binary).replaceAll('+', '-').replaceAll('/', '_').replaceAll('=', '');
}

export function base64urlToBytes(value: string): Uint8Array {
  if (!/^[A-Za-z0-9_-]+$/.test(value)) throw new Error('Invalid base64url value.');
  const padded = value.replaceAll('-', '+').replaceAll('_', '/') + '='.repeat((4 - value.length % 4) % 4);
  const binary = atob(padded);
  return Uint8Array.from(binary, char => char.charCodeAt(0));
}

export function randomBytes(length: number): Uint8Array {
  const result = new Uint8Array(length);
  crypto.getRandomValues(result);
  return result;
}

function sequenceIV(direction: number, sequence: number): Uint8Array {
  if (!Number.isSafeInteger(sequence) || sequence < 0) throw new Error('Invalid encrypted message sequence.');
  const iv = new Uint8Array(12);
  iv[3] = direction;
  new DataView(iv.buffer).setBigUint64(4, BigInt(sequence));
  return iv;
}

/** Derive independent directional AES-GCM keys from invitation secret + both ephemeral nonces. */
export async function deriveDirectionalKeys(secret: Uint8Array, desktopNonce: Uint8Array, mobileNonce: Uint8Array, side: 'desktop' | 'mobile', protocol = 1) {
  if (secret.byteLength !== 32 || desktopNonce.byteLength !== 16 || mobileNonce.byteLength !== 16) throw new Error('Invalid secure-session key material.');
  const material = await crypto.subtle.importKey('raw', source(secret), 'HKDF', false, ['deriveKey']);
  const salt = new Uint8Array(32); salt.set(desktopNonce); salt.set(mobileNonce, 16);
  const derive = (label: string) => crypto.subtle.deriveKey(
    { name: 'HKDF', hash: 'SHA-256', salt: source(salt), info: source(encoder.encode(`monitter-controller-v${protocol}/${label}`)) },
    material, { name: 'AES-GCM', length: 256 }, false, ['encrypt', 'decrypt'],
  );
  const desktopToMobile = await derive('desktop-to-mobile');
  const mobileToDesktop = await derive('mobile-to-desktop');
  return side === 'desktop'
    ? { send: desktopToMobile, receive: mobileToDesktop, sendDirection: 1, receiveDirection: 2 }
    : { send: mobileToDesktop, receive: desktopToMobile, sendDirection: 2, receiveDirection: 1 };
}

/** ECDH P-256 material is ephemeral and never serialized with a v2 invitation. */
export async function generatePairingKeyPair(): Promise<CryptoKeyPair> {
  return crypto.subtle.generateKey({ name: 'ECDH', namedCurve: 'P-256' }, false, ['deriveBits']);
}

export async function exportPairingPublicKey(key: CryptoKey): Promise<Uint8Array> {
  return new Uint8Array(await crypto.subtle.exportKey('raw', key));
}

export async function deriveEcdhSecret(privateKey: CryptoKey, peerPublicKey: Uint8Array): Promise<Uint8Array> {
  if (peerPublicKey.byteLength !== 65 || peerPublicKey[0] !== 4) throw new Error('Invalid P-256 public key.');
  const peer = await crypto.subtle.importKey('raw', source(peerPublicKey), { name: 'ECDH', namedCurve: 'P-256' }, false, []);
  return new Uint8Array(await crypto.subtle.deriveBits({ name: 'ECDH', public: peer }, privateKey, 256));
}

/** Human-verifiable short authentication string for the ordered v2 handshake transcript. */
export async function verificationCode(desktopPublic: Uint8Array, mobilePublic: Uint8Array, desktopNonce: Uint8Array, mobileNonce: Uint8Array, sharedSecret: Uint8Array): Promise<string> {
  if ([desktopPublic, mobilePublic].some(value => value.byteLength !== 65) || desktopNonce.byteLength !== 16 || mobileNonce.byteLength !== 16 || sharedSecret.byteLength !== 32) throw new Error('Invalid verification transcript.');
  const transcript = new Uint8Array(194);
  let offset = 0; for (const value of [desktopPublic, mobilePublic, desktopNonce, mobileNonce, sharedSecret]) { transcript.set(value, offset); offset += value.byteLength; }
  const digest = new Uint8Array(await crypto.subtle.digest('SHA-256', source(transcript)));
  const numeric = new DataView(digest.buffer).getUint32(0) % 1_000_000;
  return numeric.toString().padStart(6, '0');
}

/** Commit to a v2 participant's immutable public key and random nonce before reveal. */
export async function pairingCommitment(role: 'desktop' | 'mobile', publicKey: Uint8Array, nonce: Uint8Array): Promise<Uint8Array> {
  if (publicKey.byteLength !== 65 || nonce.byteLength !== 16) throw new Error('Invalid pairing commitment material.');
  const domain = encoder.encode(`monitter-controller-v2/commit/${role}\0`);
  const input = new Uint8Array(domain.byteLength + publicKey.byteLength + nonce.byteLength);
  input.set(domain); input.set(publicKey, domain.byteLength); input.set(nonce, domain.byteLength + publicKey.byteLength);
  return new Uint8Array(await crypto.subtle.digest('SHA-256', source(input)));
}

export class SecureChannel {
  private outbound = 0;
  private inbound = 0;
  constructor(private readonly keys: Awaited<ReturnType<typeof deriveDirectionalKeys>>) {}

  async seal(value: unknown): Promise<SecureEnvelope> {
    const seq = this.outbound++;
    const plaintext = encoder.encode(JSON.stringify(value));
    const ciphertext = await crypto.subtle.encrypt({ name: 'AES-GCM', iv: source(sequenceIV(this.keys.sendDirection, seq)) }, this.keys.send, source(plaintext));
    return { type: 'secure', seq, ciphertext: bytesToBase64url(new Uint8Array(ciphertext)) };
  }

  async open(envelope: SecureEnvelope): Promise<unknown> {
    // Strictly increasing sequence numbers reject both replays and reordering.
    if (!Number.isSafeInteger(envelope.seq) || envelope.seq !== this.inbound || typeof envelope.ciphertext !== 'string') throw new Error('Rejected replayed or out-of-order encrypted message.');
    const plaintext = await crypto.subtle.decrypt(
      { name: 'AES-GCM', iv: source(sequenceIV(this.keys.receiveDirection, envelope.seq)) }, this.keys.receive, source(base64urlToBytes(envelope.ciphertext)),
    );
    this.inbound += 1;
    return JSON.parse(decoder.decode(plaintext));
  }
}
