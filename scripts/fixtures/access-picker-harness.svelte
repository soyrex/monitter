<script lang="ts">
  import { onMount } from 'svelte';
  import AccessPicker from '../../src/lib/components/AccessPicker.svelte';
  import type { Sandbox } from '../../src/lib/types';

  let sandbox = $state<Sandbox>('read-only');
  const calls: Sandbox[] = [];
  let pendingResolve: (() => void) | null = null;
  let pendingReject: ((reason: Error) => void) | null = null;

  function change(next: Sandbox) {
    calls.push(next);
    return new Promise<void>((resolve, reject) => {
      pendingResolve = () => { sandbox = next; resolve(); };
      pendingReject = reject;
    });
  }

  onMount(() => {
    (window as any).__ACCESS_PICKER_QA__ = {
      calls,
      resolve: () => { pendingResolve?.(); pendingResolve = null; pendingReject = null; },
      reject: () => { pendingReject?.(new Error('Settings save failed')); pendingResolve = null; pendingReject = null; },
    };
  });
</script>

<main>
  <AccessPicker provider="codex" {sandbox} onchange={change}/>
  <button type="button">Other control</button>
</main>

<style>
  :global(html, body, #app) { margin: 0; height: 100%; }
  main { --panel: #fff; --soft: #edf0f0; --line: #ccd4d4; --ink: #162020; --muted: #5d6868; --accent: #3978d4; --accent-ink: #225aa8; --danger: #c44c79; font: 14px sans-serif; padding: 32px; }
</style>
