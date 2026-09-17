<script lang="ts">
  import UnifiedSubagentItem from './UnifiedSubagentItem.svelte';
  import { activeUnifiedSubagent, type UnifiedSubagent } from '$lib/unified-subagents';

  let { items, selectedId = null, label = 'Subagent tasks', onselect }: {
    items: UnifiedSubagent[];
    selectedId?: string | null;
    label?: string;
    onselect?: (item: UnifiedSubagent) => void;
  } = $props();

  const active = $derived(items.filter(activeUnifiedSubagent));
  const recent = $derived(items.filter(item => !activeUnifiedSubagent(item)));
</script>

<aside class="subagent-sidebar" aria-label={label}>
  <section aria-labelledby="subagent-active-heading">
    <h2 id="subagent-active-heading">Active <span>{active.length}</span></h2>
    {#if active.length}{#each active as item (item.id)}<UnifiedSubagentItem {item} selected={item.id === selectedId} onclick={onselect}/>{/each}
    {:else}<p>No active subagents.</p>{/if}
  </section>
  <section aria-labelledby="subagent-recent-heading">
    <h2 id="subagent-recent-heading">Recent <span>{recent.length}</span></h2>
    {#if recent.length}{#each recent as item (item.id)}<UnifiedSubagentItem {item} selected={item.id === selectedId} onclick={onselect}/>{/each}
    {:else}<p>No recent subagent work.</p>{/if}
  </section>
</aside>

<style>
  .subagent-sidebar{container-type:inline-size;display:grid;align-content:start;gap:18px;min-width:0;overflow:auto;padding:12px;background:var(--sidebar,var(--panel));color:var(--ink)}section{display:grid;gap:3px;min-width:0}h2{display:flex;align-items:center;gap:7px;margin:0 4px 5px;color:var(--muted);font:600 calc(10px * var(--interface-font-ratio,1)) var(--mono);text-transform:uppercase;letter-spacing:.08em}h2 span{font-weight:400}p{margin:0;padding:7px 9px;color:var(--muted);font-size:calc(11px * var(--interface-font-ratio,1));line-height:1.4}@container (max-width:250px){.subagent-sidebar{padding:8px}.subagent-sidebar :global(.status){display:none}}
</style>
