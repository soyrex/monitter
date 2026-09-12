import { bytesToBase64url, generatePairingKeyPair, randomBytes } from './secure-session';

export interface RememberedController {
  publicKey: string;
  name: string;
  approvedAt: number;
  lastAccessAt: number;
}
export interface DesktopPairingRecord {
  version: 1;
  relayUrl: string;
  identity: { keyPair: CryptoKeyPair; room: string };
  enabled: boolean;
  /** Sliding inactivity timeout. Null means remembered until explicitly revoked. */
  inactivityDays: number | null;
  devices: RememberedController[];
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

export function deviceExpiresAt(device: RememberedController, days: number | null): number | null {
  return days === null ? null : device.lastAccessAt + days * DAY;
}
export function isRememberedController(record: DesktopPairingRecord, publicKey: string, now = Date.now()): boolean {
  const device = record.devices.find(item => item.publicKey === publicKey);
  if (!record.enabled || !device) return false;
  const expiry = deviceExpiresAt(device, record.inactivityDays);
  return expiry === null || now < expiry;
}
export async function newDesktopPairing(relayUrl: string): Promise<DesktopPairingRecord> {
  return {
    version: 1, relayUrl,
    identity: { keyPair: await generatePairingKeyPair(), room: bytesToBase64url(randomBytes(18)) },
    enabled: true, inactivityDays: null, devices: [],
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
function validateDesktop(value: unknown): asserts value is DesktopPairingRecord {
  const record = value as DesktopPairingRecord | undefined;
  if (!record || record.version !== 1 || typeof record.relayUrl !== 'string'
    || !validKeys(record.identity?.keyPair) || typeof record.identity.room !== 'string'
    || !/^[A-Za-z0-9_-]{22,128}$/.test(record.identity.room)
    || typeof record.enabled !== 'boolean'
    || !(record.inactivityDays === null || Number.isInteger(record.inactivityDays) && record.inactivityDays >= 1 && record.inactivityDays <= MAX_INACTIVITY_DAYS)
    || !Array.isArray(record.devices) || record.devices.length > 32
    || record.devices.some(device => typeof device.publicKey !== 'string' || !/^[A-Za-z0-9_-]{87}$/.test(device.publicKey)
      || typeof device.name !== 'string' || device.name.length > 80
      || !Number.isFinite(device.approvedAt) || !Number.isFinite(device.lastAccessAt))) {
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
  const value = await readRecord('desktop');
  if (value === undefined) return null;
  validateDesktop(value); return value;
}
export async function loadMobilePairing(): Promise<MobilePairingRecord | null> {
  const value = await readRecord('mobile');
  if (value === undefined) return null;
  validateMobile(value); return value;
}
export function saveDesktopPairing(record: DesktopPairingRecord): Promise<void> {
  validateDesktop(record);
  // Copy ordinary fields so Svelte proxies never reach structured clone.
  return writeRecord('desktop', {
    version: 1, relayUrl: record.relayUrl, enabled: record.enabled, inactivityDays: record.inactivityDays,
    identity: { room: record.identity.room, keyPair: { privateKey: record.identity.keyPair.privateKey, publicKey: record.identity.keyPair.publicKey } },
    devices: record.devices.map(device => ({ publicKey: device.publicKey, name: device.name, approvedAt: device.approvedAt, lastAccessAt: device.lastAccessAt })),
  });
}
export function saveMobilePairing(record: MobilePairingRecord): Promise<void> {
  validateMobile(record);
  return writeRecord('mobile', {
    version: 1, invitation: record.invitation, savedAt: record.savedAt,
    identity: { keyPair: { privateKey: record.identity.keyPair.privateKey, publicKey: record.identity.keyPair.publicKey } },
  });
}
export function clearDesktopPairing(): Promise<void> { return writeRecord('desktop'); }
export function clearMobilePairing(): Promise<void> { return writeRecord('mobile'); }
