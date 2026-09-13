import { mount } from 'svelte';
import Harness from './share-control-harness.svelte';
export { createMobileSession } from '../../src/lib/controller/remote-client.ts';
export function mountHarness() { mount(Harness, { target: document.getElementById('app') }); }
