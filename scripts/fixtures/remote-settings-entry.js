import { mount } from 'svelte';
import Harness from './remote-settings-harness.svelte';
export { createMobileSession } from '../../src/lib/controller/remote-client.ts';
export { generatePairingKeyPair, bytesToBase64url, exportPairingPublicKey } from '../../src/lib/controller/secure-session.ts';
export { clearDesktopPairing, newDesktopPairing, saveDesktopPairing } from '../../src/lib/controller/pairing-store.ts';
export function mountHarness() { mount(Harness,{target:document.getElementById('app')}); }
