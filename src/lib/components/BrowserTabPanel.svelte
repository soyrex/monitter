<script lang="ts">
  import { untrack } from 'svelte';
  import { observeBrowserViewport } from '$lib/browser-viewport';
  import ArrowRight from "@lucide/svelte/icons/arrow-right";
  import ChevronLeft from "@lucide/svelte/icons/chevron-left";
  import ChevronRight from "@lucide/svelte/icons/chevron-right";
  import Globe from "@lucide/svelte/icons/globe";
  import Puzzle from "@lucide/svelte/icons/puzzle";
  import RefreshCw from "@lucide/svelte/icons/refresh-cw";
  import X from "@lucide/svelte/icons/x";
  import type { BrowserExtensionLoadResult } from '$lib/types';

  export type BrowserTabMetadata = { id: string; url: string; title: string; unloaded: boolean };
  export type BrowserViewport = { x: number; y: number; width: number; height: number; visible: boolean };

  let { tab, active = false, available = true, layoutRevision = 0, canGoBack = null, canGoForward = null, onNavigate, onBack, onForward, onRefreshState, onReload, onUnload, onLoadExtension, onLayout, onClose }:
    { tab: BrowserTabMetadata; active?: boolean; available?: boolean; layoutRevision?: string | number; canGoBack?: boolean | null; canGoForward?: boolean | null; onNavigate?: (url: string) => void; onBack?: () => void; onForward?: () => void; onRefreshState?: () => void; onReload?: () => void; onUnload?: () => void; onLoadExtension?: (path: string) => Promise<BrowserExtensionLoadResult>; onLayout?: (viewport: BrowserViewport) => void; onClose?: () => void } = $props();
  let url = $state('');
  let addressFocused = $state(false);
  let addressDirty = $state(false);
  let addressTabId = '';
  let viewport = $state<HTMLElement>();
  let extensionOpen = $state(false), extensionPath = $state(''), extensionLoading = $state(false), extensionMessage = $state(''), extensionError = $state('');

  $effect(() => {
    const currentTabId = tab.id;
    const currentUrl = tab.url;
    // Native state is polled while the toolbar remains mounted. Keep a user's
    // draft stable during that polling, but always start a newly selected tab
    // with its own current address.
    if (addressTabId !== currentTabId) {
      addressTabId = currentTabId;
      addressFocused = false;
      addressDirty = false;
      url = currentUrl;
    } else if (!addressFocused && !addressDirty) {
      url = currentUrl;
    }
  });
  $effect(() => {
    // Native page-load callbacks do not fire for SPA pushState/replaceState.
    // Poll only the one active child; WebKit still owns the real history stack.
    if (!active || tab.unloaded || !onRefreshState) return;
    const timer = window.setInterval(() => onRefreshState?.(), 1000);
    return () => window.clearInterval(timer);
  });
  let geometryObserver = $state.raw<ReturnType<typeof observeBrowserViewport>>();
  $effect(() => {
    const node = viewport;
    if (!node) return;
    const observer = observeBrowserViewport(node, next => untrack(() => onLayout?.(next)));
    geometryObserver = observer;
    return () => { observer.destroy(); geometryObserver = undefined; };
  });
  $effect(() => {
    const visible = active, unloaded = tab.unloaded, revision = layoutRevision;
    // Revisions invalidate measurement without tearing down the native view
    // or emitting an intermediate hide on every splitter movement.
    geometryObserver?.update(visible, unloaded, revision);
  });
  function navigate() {
    const value = url.trim();
    addressDirty = false;
    if (value) onNavigate?.(value);
  }
  function cancelAddressEdit(event: KeyboardEvent) {
    if (event.key !== 'Escape') return;
    event.preventDefault();
    event.stopPropagation();
    url = tab.url;
    addressDirty = false;
    addressFocused = false;
    (event.currentTarget as HTMLInputElement).blur();
  }
  async function loadExtension() {
    const path = extensionPath.trim(); extensionMessage = ''; extensionError = '';
    if (!path.startsWith('/')) { extensionError = 'Enter an absolute extension folder path.'; return; }
    if (!onLoadExtension) { extensionError = 'Extension loading is unavailable in this browser.'; return; }
    extensionLoading = true;
    try { const result = await onLoadExtension(path); extensionMessage = `${result.displayName ?? 'Extension'} loaded by WebKit from ${result.path}. Manifest loading does not confirm sign-in or Bitwarden compatibility.`; }
    catch (reason) { extensionError = String(reason); }
    finally { extensionLoading = false; }
  }
</script>

<section class="browser-tab-panel" aria-label={`Browser tab: ${tab.title}`}>
  <form class="browser-toolbar" onsubmit={event => { event.preventDefault(); navigate(); }}>
    <Globe size={16} aria-hidden="true" />
    <button type="button" aria-label="Back" title="Back" disabled={!available || canGoBack !== true} onclick={() => onBack?.()}><ChevronLeft size={16}/></button>
    <button type="button" aria-label="Forward" title="Forward" disabled={!available || canGoForward !== true} onclick={() => onForward?.()}><ChevronRight size={16}/></button>
    <input aria-label="Browser address" bind:value={url} onfocus={() => addressFocused = true} onblur={() => addressFocused = false} oninput={() => addressDirty = true} onkeydown={cancelAddressEdit} autocomplete="off" autocapitalize="off" spellcheck="false" placeholder="https://example.com" disabled={!available}/>
    <button type="submit" aria-label="Navigate" title="Navigate" disabled={!available}><ArrowRight size={15}/></button>
    <button type="button" aria-label="Reload browser tab" title="Reload browser tab" disabled={!available || !tab.url} onclick={() => onReload?.()}><RefreshCw size={15}/></button>
    <button type="button" aria-label="Unload browser tab" title="Unload native browser view; keep this tab" disabled={!available || tab.unloaded} onclick={() => onUnload?.()}><Globe size={15}/></button>
    <button type="button" aria-label="Load unpacked extension" title="Load unpacked extension folder" disabled={!available} aria-expanded={extensionOpen} onclick={() => { extensionOpen = !extensionOpen; extensionMessage = ''; extensionError = ''; }}><Puzzle size={15}/></button>
    <button type="button" aria-label="Close browser tab" title="Close browser tab" onclick={() => onClose?.()}><X size={16}/></button>
  </form>
  {#if extensionOpen}<form class="extension-loader" onsubmit={event => { event.preventDefault(); void loadExtension(); }}><label>Unpacked extension folder <input aria-label="Unpacked extension folder" bind:value={extensionPath} placeholder="/absolute/path/to/extension" autocomplete="off" spellcheck="false" disabled={extensionLoading}/></label><button type="submit" disabled={extensionLoading}>{extensionLoading ? 'Loading…' : 'Load'}</button><p class="extension-warning">Extensions may read or change browser pages. Load only folders you trust; website permissions are not granted automatically.</p>{#if extensionMessage}<p class="extension-success" role="status">{extensionMessage}</p>{/if}{#if extensionError}<p class="extension-error" role="alert">{extensionError}</p>{/if}</form>{/if}
  <div class="browser-native-viewport" bind:this={viewport} data-browser-tab-id={tab.id} data-browser-unloaded={tab.unloaded} aria-label={tab.unloaded ? 'Native browser view waiting to load' : 'Native browser view'}>
    <div><Globe size={24}/><strong>{tab.title || 'Browser'}</strong><p>{!available ? 'Browser tabs require the native desktop app.' : tab.unloaded ? 'Enter or reload an address to attach the native browser view.' : 'Native browser view is attached by the desktop runtime.'}</p></div>
  </div>
</section>

<style>
  .browser-tab-panel { min-width:0; min-height:0; height:100%; display:flex; flex-direction:column; background:var(--paper); }
  .browser-toolbar { display:flex; align-items:center; gap:7px; min-width:0; padding:7px 10px; border-bottom:1px solid var(--line); color:var(--muted); background:var(--panel); }
  input { min-width:0; flex:1; padding:6px 8px; border:1px solid var(--line); border-radius:5px; color:var(--ink); background:var(--paper); }
  button { display:grid; place-items:center; width:29px; height:29px; border-radius:5px; color:var(--muted); background:transparent; cursor:pointer; }
  button:hover { color:var(--ink); background:var(--soft); }
  .browser-native-viewport { min-width:0; min-height:0; flex:1 1 auto; position:relative; overflow:hidden; background:var(--paper); }
  .extension-loader { display:flex; flex-wrap:wrap; align-items:end; gap:7px; padding:7px 10px; border-bottom:1px solid var(--line); background:var(--panel); font-size:12px; }
  .extension-loader label { display:grid; flex:1 1 260px; gap:3px; color:var(--muted); } .extension-loader input { width:100%; box-sizing:border-box; }
  .extension-loader button { width:auto; padding:0 10px; border:1px solid var(--line); color:var(--ink); }
  .extension-loader p { flex-basis:100%; margin:0; } .extension-warning { color:var(--muted); } .extension-success { color:var(--accent-ink); } .extension-error { color:#d95b5b; }
  .browser-native-viewport > div { position:absolute; inset:0; display:grid; place-content:center; justify-items:center; gap:8px; color:var(--muted); text-align:center; }
  strong { color:var(--ink); } p { max-width:300px; margin:0; font-size:12px; }
</style>
