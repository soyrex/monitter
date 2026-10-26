import type { PairingRegistration } from './pairing-code';
import { bytesToBase64url, generatePairingKeyPair, randomBytes } from './secure-session';

export interface RememberedController {
  publicKey: string;
  name: string;
  approvedAt: number;
  lastAccessAt: number;
}
export interface DesktopPairingRecord {
  version: 2;
  relayUrl: string;
  identity: { keyPair: CryptoKeyPair; room: string };
  enabled: boolean;
  /** Sliding inactivity timeout. Null means remembered until explicitly revoked. */
  inactivityDays: number | null;
  devices: RememberedController[];
  /** Deliberately separate from controller access; never inferred from devices. */
  ownerDevices: RememberedController[];
  /** Stable locator and its private management token; never sent to phones. */
  pairingRegistration?: PairingRegistration;
  pairingManagementToken?: string;
}
export interface MobilePairingRecord {
  version: 1;
  invitation: string;
  identity: { keyPair: CryptoKeyPair };
  savedAt: number;
}

const DB_NAME = 'monitter-paired-devices-v1';
const STORE = 'pairings';
const DAY = 86_400_000;
export const MAX_INACTIVITY_DAYS = 36_500;
let writes: Promise<void> = Promise.resolve();
let mobileIdentity: Promise<MobilePairingRecord['identity']> | null = null;

// Native Keychain storage is exposed only by the bundled, trusted main frame.
function nativePairing(): { postMessage(message: Record<string, unknown>): Promise<any> } | undefined {
  return typeof window === 'undefined' ? undefined : (window as any).webkit?.messageHandlers?.monitterPairing;
}
function decodeNativeKey(value: string): Uint8Array {
  return Uint8Array.from(atob(value), char => char.charCodeAt(0));
}
let nativeIdentity: Promise<MobilePairingRecord['identity']> | null = null;
function getNativeIdentity(): Promise<MobilePairingRecord['identity']> {
  if (!nativeIdentity) {
    nativeIdentity = (async () => {
      const value = await nativePairing()!.postMessage({ action: 'identity' });
      const privateKey = await crypto.subtle.importKey('pkcs8', decodeNativeKey(value.privateKeyPkcs8), { name: 'ECDH', namedCurve: 'P-256' }, false, ['deriveBits']);
      const publicKey = await crypto.subtle.importKey('raw', decodeNativeKey(value.publicKeyRaw), { name: 'ECDH', namedCurve: 'P-256' }, true, []);
      return { keyPair: { privateKey, publicKey } };
    })();
    void nativeIdentity.catch(() => { nativeIdentity = null; });
  }
  return nativeIdentity;
}

export function deviceExpiresAt(device: RememberedController, days: number | null): number | null {
  return days === null ? null : device.lastAccessAt + days * DAY;
}
export function isRememberedController(record: DesktopPairingRecord, publicKey: string, now = Date.now()): boolean {
  const device = record.devices.find(item => item.publicKey === publicKey);
  if (!record.enabled || !device) return false;
  const expiry = deviceExpiresAt(device, record.inactivityDays);
  return expiry === null || now < expiry;
}
export function isRememberedOwnerWorkspace(record: DesktopPairingRecord, publicKey: string, now = Date.now()): boolean {
  const device = record.ownerDevices.find(item => item.publicKey === publicKey);
  if (!record.enabled || !device) return false;
  const expiry = deviceExpiresAt(device, record.inactivityDays);
  return expiry === null || now < expiry;
}
export async function newDesktopPairing(relayUrl: string): Promise<DesktopPairingRecord> {
  return {
    version: 2, relayUrl,
    identity: { keyPair: await generatePairingKeyPair(), room: bytesToBase64url(randomBytes(18)) },
    enabled: true, inactivityDays: null, devices: [], ownerDevices: [],
  };
}

function validKeys(value: unknown): value is CryptoKeyPair {
  const pair = value as CryptoKeyPair | undefined;
  return !!pair?.privateKey && !!pair.publicKey
    && pair.privateKey.type === 'private' && !pair.privateKey.extractable
    && pair.privateKey.algorithm.name === 'ECDH'
    && (pair.privateKey.algorithm as EcKeyAlgorithm).namedCurve === 'P-256'
    && pair.privateKey.usages.includes('deriveBits')
    && pair.publicKey.type === 'public' && pair.publicKey.algorithm.name === 'ECDH'
    && (pair.publicKey.algorithm as EcKeyAlgorithm).namedCurve === 'P-256';
}
function validDevices(value: unknown): value is RememberedController[] {
  return Array.isArray(value) && value.length <= 32
    && value.every(device => typeof device.publicKey === 'string' && /^[A-Za-z0-9_-]{87}$/.test(device.publicKey)
      && typeof device.name === 'string' && device.name.length <= 80
      && Number.isFinite(device.approvedAt) && Number.isFinite(device.lastAccessAt));
}
function validateDesktop(value: unknown): asserts value is DesktopPairingRecord {
  const record = value as DesktopPairingRecord | undefined;
  if (!record || record.version !== 2 || typeof record.relayUrl !== 'string'
    || !validKeys(record.identity?.keyPair) || typeof record.identity.room !== 'string'
    || !/^[A-Za-z0-9_-]{22,128}$/.test(record.identity.room)
    || typeof record.enabled !== 'boolean'
    || !(record.inactivityDays === null || Number.isInteger(record.inactivityDays) && record.inactivityDays >= 1 && record.inactivityDays <= MAX_INACTIVITY_DAYS)
    || (record.pairingManagementToken !== undefined && !/^[A-Za-z0-9_-]{43}$/.test(record.pairingManagementToken))
    || (record.pairingRegistration !== undefined && (!/^\d{9}$/.test(record.pairingRegistration.code) || record.pairingRegistration.persistent !== true || record.pairingRegistration.expiresAt !== null || typeof record.pairingRegistration.revocationToken !== 'string' || record.pairingRegistration.revocationToken.length < 32))
    || !validDevices(record.devices) || !validDevices(record.ownerDevices)) {
    throw new Error('Saved desktop pairing is invalid. Revoke saved devices and pair again.');
  }
}
function validateMobile(value: unknown): asserts value is MobilePairingRecord {
  const record = value as MobilePairingRecord | undefined;
  if (!record || record.version !== 1 || !validKeys(record.identity?.keyPair)
    || typeof record.invitation !== 'string' || record.invitation.length > 4096
    || !Number.isFinite(record.savedAt)) {
    throw new Error('Saved phone pairing is invalid. Forget this desktop and pair again.');
  }
}
function openDatabase(): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    if (typeof indexedDB === 'undefined') return reject(new Error('Secure device storage is unavailable. Approval cannot be remembered.'));
    const request = indexedDB.open(DB_NAME, 1);
    request.onupgradeneeded = () => { request.result.createObjectStore(STORE); };
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(new Error(`Could not open saved device storage: ${request.error?.message ?? 'unknown error'}`));
    request.onblocked = () => reject(new Error('Saved device storage is blocked. Close other Monitter windows and try again.'));
  });
}
async function readRecord(key: string): Promise<unknown> {
  await writes;
  const db = await openDatabase();
  try {
    return await new Promise((resolve, reject) => {
      const request = db.transaction(STORE, 'readonly').objectStore(STORE).get(key);
      request.onsuccess = () => resolve(request.result);
      request.onerror = () => reject(new Error(`Could not read saved device: ${request.error?.message ?? 'unknown error'}`));
    });
  } finally { db.close(); }
}
function writeRecord(key: string, value?: unknown): Promise<void> {
  // Preserve save/forget order even when opening IndexedDB is asynchronous.
  const operation = writes.then(async () => {
    const db = await openDatabase();
    try {
      await new Promise<void>((resolve, reject) => {
        const transaction = db.transaction(STORE, 'readwrite');
        transaction.oncomplete = () => resolve();
        transaction.onabort = transaction.onerror = () => reject(new Error(`Could not save device approval: ${transaction.error?.message ?? 'storage unavailable'}`));
        const store = transaction.objectStore(STORE);
        if (value === undefined) store.delete(key); else store.put(value, key);
      });
    } finally { db.close(); }
  });
  writes = operation.catch(() => undefined);
  return operation;
}
export async function loadDesktopPairing(): Promise<DesktopPairingRecord | null> {
  let value = await readRecord('desktop');
  if (value === undefined) return null;
  // v1 had only controller trust. Migration intentionally grants no owner scope.
  if ((value as { version?: unknown }).version === 1) {
    const legacy = value as Omit<DesktopPairingRecord, 'version' | 'ownerDevices'>;
    value = { ...legacy, version: 2, ownerDevices: [] };
  }
  validateDesktop(value); return value;
}
/** Device identity survives forgetting a desktop; grants remain desktop-owned. */
export function getOrCreateMobileIdentity(): Promise<MobilePairingRecord['identity']> {
  if (nativePairing()) return getNativeIdentity();
  if (mobileIdentity) return mobileIdentity;
  const operation = writes.then(async () => {
    const db = await openDatabase();
    try {
      const stored = await new Promise<CryptoKeyPair | null>((resolve, reject) => {
        const transaction = db.transaction(STORE, 'readonly');
        const store = transaction.objectStore(STORE);
        const identity = store.get('mobile-identity'), pairing = store.get('mobile');
        transaction.oncomplete = () => {
          if (identity.result !== undefined) {
            if (!validKeys(identity.result?.keyPair)) { reject(new Error('Saved phone identity is invalid.')); return; }
            resolve(identity.result.keyPair);
          } else if (pairing.result !== undefined) {
            try { validateMobile(pairing.result); resolve(pairing.result.identity.keyPair); }
            catch (reason) { reject(reason); }
          } else resolve(null);
        };
        transaction.onabort = transaction.onerror = () => reject(new Error('Could not read this phone identity.'));
      });
      // Generate only when neither persisted identity exists, outside a transaction.
      const candidate = stored ?? await generatePairingKeyPair();
      return await new Promise<MobilePairingRecord['identity']>((resolve, reject) => {
        const transaction = db.transaction(STORE, 'readwrite');
        const store = transaction.objectStore(STORE);
        let identity: MobilePairingRecord['identity'];
        transaction.oncomplete = () => resolve(identity);
        transaction.onabort = transaction.onerror = () => reject(new Error('Could not persist this phone identity. Pairing has not started.'));
        const existing = store.get('mobile-identity');
        existing.onsuccess = () => {
          if (existing.result !== undefined) {
            if (!validKeys(existing.result?.keyPair)) { transaction.abort(); return; }
            identity = existing.result;
            return;
          }
          // The transaction serializes migration/creation across tabs too.
          const saved = store.get('mobile');
          saved.onsuccess = () => {
            if (saved.result !== undefined) {
              try { validateMobile(saved.result); } catch { transaction.abort(); return; }
              identity = saved.result.identity;
            } else identity = { keyPair: candidate };
            identity = { keyPair: { privateKey: identity.keyPair.privateKey, publicKey: identity.keyPair.publicKey } };
            try { store.put(identity, 'mobile-identity'); } catch { transaction.abort(); }
          };
        };
      });
    } finally { db.close(); }
  });
  writes = operation.then(() => undefined, () => undefined);
  mobileIdentity = operation;
  void operation.catch(() => { if (mobileIdentity === operation) mobileIdentity = null; });
  return operation;
}
export async function loadMobilePairing(): Promise<MobilePairingRecord | null> {
  const bridge = nativePairing();
  if (bridge) {
    const saved = await bridge.postMessage({ action: 'load' });
    if (saved) {
      const record = { version: 1 as const, invitation: saved.invitation, savedAt: saved.savedAt, identity: await getNativeIdentity() };
      validateMobile(record);
      return record;
    }
    // Legacy private keys are nonextractable. Keep their existing identity and
    // locator together; a newly scanned pairing can adopt the native identity.
    const legacy = await readRecord('mobile');
    if (legacy === undefined) return null;
    validateMobile(legacy);
    return legacy;
  }
  const value = await readRecord('mobile');
  if (value === undefined) return null;
  validateMobile(value);
  const identity = await getOrCreateMobileIdentity();
  return { ...value, identity };
}
export function saveDesktopPairing(record: DesktopPairingRecord): Promise<void> {
  validateDesktop(record);
  // Copy ordinary fields so Svelte proxies never reach structured clone.
  return writeRecord('desktop', {
    ...(record.pairingManagementToken ? { pairingManagementToken: record.pairingManagementToken } : {}),
    ...(record.pairingRegistration ? { pairingRegistration: { ...record.pairingRegistration } } : {}),
    version: 2, relayUrl: record.relayUrl, enabled: record.enabled, inactivityDays: record.inactivityDays,
    identity: { room: record.identity.room, keyPair: { privateKey: record.identity.keyPair.privateKey, publicKey: record.identity.keyPair.publicKey } },
    devices: record.devices.map(device => ({ publicKey: device.publicKey, name: device.name, approvedAt: device.approvedAt, lastAccessAt: device.lastAccessAt })),
    ownerDevices: record.ownerDevices.map(device => ({ publicKey: device.publicKey, name: device.name, approvedAt: device.approvedAt, lastAccessAt: device.lastAccessAt })),
  });
}
export async function saveMobilePairing(record: MobilePairingRecord): Promise<void> {
  validateMobile(record);
  const bridge = nativePairing();
  if (bridge) {
    const native = await getNativeIdentity();
    const actual = bytesToBase64url(new Uint8Array(await crypto.subtle.exportKey('raw', record.identity.keyPair.publicKey)));
    const expected = bytesToBase64url(new Uint8Array(await crypto.subtle.exportKey('raw', native.keyPair.publicKey)));
    if (actual === expected) {
      await bridge.postMessage({ action: 'save', invitation: record.invitation, savedAt: record.savedAt });
      return;
    }
  }
  return writeRecord('mobile', {
    version: 1, invitation: record.invitation, savedAt: record.savedAt,
    identity: { keyPair: { privateKey: record.identity.keyPair.privateKey, publicKey: record.identity.keyPair.publicKey } },
  });
}
export function clearDesktopPairing(): Promise<void> { return writeRecord('desktop'); }
export async function clearMobilePairing(): Promise<void> {
  if (nativePairing()) await nativePairing()!.postMessage({ action: 'forget' });
  else await getOrCreateMobileIdentity();
  await writeRecord('mobile');
}
