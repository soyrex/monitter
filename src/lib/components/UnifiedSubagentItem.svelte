<script lang="ts">
  import { CircleAlert, CircleCheck, CirclePause, CircleStop } from '@lucide/svelte';
  import { activeUnifiedSubagent, unifiedSubagentStatus, type UnifiedSubagent } from '$lib/unified-subagents';

  let {
    item,
    selected = false,
    compact = false,
    titlePrefix = '',
    onclick,
    onkeydown,
    buttonId,
    controls,
    tab = false,
  }: {
    item: UnifiedSubagent;
    selected?: boolean;
    compact?: boolean;
    titlePrefix?: string;
    onclick?: (item: UnifiedSubagent) => void;
    onkeydown?: (event: KeyboardEvent) => void;
    buttonId?: string;
    controls?: string;
    tab?: boolean;
  } = $props();

  const live = $derived(activeUnifiedSubagent(item));
  const statusText = $derived(unifiedSubagentStatus(item.status));
</script>

<button
  class="subagent-item"
  class:selected
  class:compact
  class:live
  class:failed={item.status === 'error'}
  class:finished={item.status === 'completed'}
  id={buttonId}
  role={tab ? 'tab' : undefined}
  aria-selected={tab ? selected : undefined}
  aria-current={!tab && selected ? 'page' : undefined}
  aria-controls={controls}
  tabindex={tab && !selected ? -1 : undefined}
  title={`${titlePrefix}${item.title} · ${item.agentName} · ${statusText}`}
  onclick={() => onclick?.(item)}
  onkeydown={onkeydown}
>
  <span class="agent-mark" aria-hidden="true">{item.agentInitials}</span>
  <span class="copy">
    <span class="heading"><strong>{item.title}</strong><span class="status"><i></i>{statusText}</span></span>
    {#if !compact}<small>{item.agentName}{item.activity ? ` · ${item.activity}` : ''}</small>{/if}
  </span>
  {#if item.status === 'completed'}<CircleCheck class="outcome" size={14} aria-label="Finished"/>
  {:else if item.status === 'error'}<CircleAlert class="outcome" size={14} aria-label="Failed"/>
  {:else if item.status === 'interrupted'}<CircleStop class="outcome" size={14} aria-label="Stopped"/>
  {:else if item.status === 'idle'}<CirclePause class="outcome" size={14} aria-label="Waiting"/>{/if}
</button>

<style>
  .subagent-item{display:flex;align-items:center;gap:8px;min-width:0;width:100%;padding:8px 9px;border:1px solid transparent;border-radius:8px;background:transparent;color:var(--ink);text-align:left;cursor:pointer;font:inherit}
  .subagent-item:hover{background:var(--soft)}.subagent-item:focus-visible{outline:2px solid var(--accent);outline-offset:2px}.subagent-item.selected{border-color:color-mix(in srgb,var(--accent) 38%,var(--line));background:color-mix(in srgb,var(--accent) 9%,var(--panel))}
  .agent-mark{display:grid;place-items:center;flex:none;width:22px;height:22px;border-radius:6px;color:var(--on-accent);background:var(--accent);font:600 calc(9px * var(--interface-font-ratio,1))/1 var(--mono)}
  .copy{display:grid;gap:2px;flex:1;min-width:0}.heading{display:flex;align-items:center;gap:7px;min-width:0}.heading strong{min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;font-size:calc(12px * var(--interface-font-ratio,1));font-weight:550}.copy small{overflow:hidden;color:var(--muted);font-size:calc(10px * var(--interface-font-ratio,1));text-overflow:ellipsis;white-space:nowrap}
  .status{display:inline-flex;align-items:center;gap:4px;flex:none;color:var(--muted);font:calc(9px * var(--interface-font-ratio,1)) var(--mono);text-transform:uppercase;letter-spacing:.03em}.status i{width:5px;height:5px;border-radius:50%;background:var(--line)}.live .status i{background:var(--accent);box-shadow:0 0 0 3px color-mix(in srgb,var(--accent) 17%,transparent)}.failed .status i{background:var(--danger,#c44c4c)}.finished .status i{background:#6f9278}:global(.outcome){flex:none;color:var(--muted)}.failed :global(.outcome){color:var(--danger,#c44c4c)}.finished :global(.outcome){color:#6f9278}
  .compact{width:auto;max-width:min(260px,42vw);padding:6px 8px}.compact .agent-mark{width:19px;height:19px;border-radius:5px;font-size:calc(8px * var(--interface-font-ratio,1))}.compact .heading strong{font-size:calc(11px * var(--interface-font-ratio,1))}.compact .status{gap:0;font-size:0}.compact .status i{width:6px;height:6px}
  @container (max-width: 420px){.status{display:none}.subagent-item:not(.compact){padding:7px}.agent-mark{width:20px;height:20px}}
</style>
