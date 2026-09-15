<script lang="ts">
  import { invoke, isTauri } from '@tauri-apps/api/core';
  import { listen, type UnlistenFn } from '@tauri-apps/api/event';
  import { onMount } from 'svelte';
  import RemoteControl from "$lib/components/RemoteControl.svelte";
  import ShareControl from "$lib/components/ShareControl.svelte";
  import AppSurface from "$lib/components/AppSurface.svelte";
  import LanGate from '$lib/components/LanGate.svelte';
  import { isLanBrowser } from '$lib/lan';
  import { remoteControlOpen, workspaceShareOpen, workspaceShareTaskId } from '$lib/workspace-panels';

  let hotUi = $state(false);
  let devUiError = $state('');
  let returningToPackagedUi = false;

  function returnToPackagedUi() {
    if (returningToPackagedUi) return;
    returningToPackagedUi = true;
    void invoke('use_packaged_ui').catch(reason => {
      returningToPackagedUi = false;
      devUiError = `Could not restore the packaged interface: ${String(reason)}`;
    });
  }

  onMount(() => {
    const native = isTauri();
    hotUi = native
      && window.location.origin === 'http://127.0.0.1:18420'
      && new URLSearchParams(window.location.search).get('monitter-dev-ui') === '1';
    // The packaged UI can surface native menu failures. The remote developer
    // renderer deliberately receives no event capability.
    if (isLanBrowser()) return;
    let stopped = false;
    let stopErrorListener: UnlistenFn | undefined;
    void listen<string>('monitter-dev-ui-error', event => { devUiError = event.payload; })
      .then(stop => { if (stopped) stop(); else stopErrorListener = stop; });
    return () => {
      stopped = true;
      stopErrorListener?.();
    };
  });
</script>
{#if hotUi}
  <aside class="dev-ui-badge" aria-label="Hot-reload developer interface">
    <span><i></i>DEV UI · HMR</span>
    <button type="button" onclick={returnToPackagedUi}>Use packaged UI</button>
  </aside>
{/if}
{#if devUiError}
  <div class="dev-ui-error" role="alert"><span>{devUiError}</span><button type="button" aria-label="Dismiss developer interface error" onclick={()=>devUiError=''}>×</button></div>
{/if}
<LanGate>
<AppSurface />

{#if !isLanBrowser()}
<RemoteControl bind:open={$remoteControlOpen} />
<ShareControl bind:open={$workspaceShareOpen} taskId={$workspaceShareTaskId} />
{/if}
</LanGate>

<style>
  .dev-ui-badge { position:fixed; right:10px; bottom:10px; z-index:10000; display:flex; align-items:center; gap:8px; padding:5px 6px 5px 9px; border:1px solid color-mix(in srgb,var(--accent) 55%,var(--line)); border-radius:8px; background:color-mix(in srgb,var(--panel) 94%,transparent); color:var(--ink); box-shadow:0 5px 18px #0003; backdrop-filter:blur(12px); font:600 9px var(--mono); letter-spacing:.05em; }
  .dev-ui-badge span { display:flex; align-items:center; gap:6px; }
  .dev-ui-badge i { width:6px; height:6px; border-radius:50%; background:#45ad78; box-shadow:0 0 8px #45ad78; }
  .dev-ui-badge button { padding:4px 7px; border-radius:5px; color:var(--ink); background:var(--soft); font:500 10px var(--interface-font); }
  .dev-ui-error { position:fixed; right:12px; top:12px; z-index:10001; display:flex; align-items:flex-start; gap:10px; max-width:min(440px,calc(100vw - 24px)); padding:9px 10px; border:1px solid #bd655b; border-radius:8px; background:var(--panel); color:#bd655b; box-shadow:0 6px 22px #0003; font-size:11px; line-height:1.4; }
  .dev-ui-error button { flex:none; color:inherit; font-size:16px; line-height:1; }
</style>
