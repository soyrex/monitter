<script lang="ts">
  import UnifiedSubagentItem from './UnifiedSubagentItem.svelte';
  import { unifiedSubagentDomId, type UnifiedSubagent } from '$lib/unified-subagents';

  let { items, selectedId = null, panelPrefix = 'subagent-visor-panel', label = 'Subagent tasks', onselect }: {
    items: UnifiedSubagent[];
    selectedId?: string | null;
    panelPrefix?: string;
    label?: string;
    onselect?: (item: UnifiedSubagent) => void;
  } = $props();

  const selected = $derived(selectedId ?? items[0]?.id ?? null);

  function selectByOffset(current: number, offset: number) {
    if (!items.length) return;
    const next = items[(current + offset + items.length) % items.length];
    onselect?.(next);
    requestAnimationFrame(() => document.getElementById(unifiedSubagentDomId('subagent-mini-tab', next.id))?.focus());
  }

  function keys(event: KeyboardEvent, index: number) {
    if (event.key === 'ArrowRight' || event.key === 'ArrowDown') { event.preventDefault(); selectByOffset(index, 1); }
    else if (event.key === 'ArrowLeft' || event.key === 'ArrowUp') { event.preventDefault(); selectByOffset(index, -1); }
    else if (event.key === 'Home') { event.preventDefault(); selectByOffset(0, 0); }
    else if (event.key === 'End') { event.preventDefault(); selectByOffset(items.length - 1, 0); }
  }
</script>

{#if items.length}
  <div class="subagent-tabs" role="tablist" aria-label={label}>
    {#each items as item, index (item.id)}
      <UnifiedSubagentItem item={item} compact tab selected={item.id === selected} buttonId={unifiedSubagentDomId('subagent-mini-tab', item.id)} controls={unifiedSubagentDomId(panelPrefix, item.id)} onkeydown={event => keys(event, index)} onclick={onselect}/>
    {/each}
  </div>
{/if}

<style>
  .subagent-tabs{container-type:inline-size;display:flex;align-items:stretch;gap:6px;max-width:100%;padding:7px max(0px,calc((100% - var(--chat-content-max-width,900px))/2));overflow-x:auto;overscroll-behavior-inline:contain;scrollbar-width:thin;border-top:1px solid var(--line);background:color-mix(in srgb,var(--paper) 88%,transparent)}
  @container (max-width: 500px){.subagent-tabs{gap:4px;padding-block:5px}}
</style>
