<script lang="ts">
  import Check from "@lucide/svelte/icons/check";
  import Circle from "@lucide/svelte/icons/circle";
  import CircleDot from "@lucide/svelte/icons/circle-dot";
  import ListChecks from "@lucide/svelte/icons/list-checks";
  import Minus from "@lucide/svelte/icons/minus";
  import OctagonAlert from "@lucide/svelte/icons/octagon-alert";
  import type { WorkPlan, WorkPlanItem } from '$lib/types';

  let { plans }: { plans: WorkPlan[] } = $props();

  const ordered = $derived([...plans].sort((left, right) =>
    right.updatedAt - left.updatedAt || right.createdAt - left.createdAt || right.id.localeCompare(left.id)
  ));
  const current = $derived(ordered.find(plan => plan.status === 'active') ?? ordered[0]);
  const previous = $derived(ordered.filter(plan => plan.id !== current?.id));
  const completed = $derived(current?.items.filter(item => item.status === 'completed').length ?? 0);
  const inProgress = $derived(current?.items.filter(item => item.status === 'in_progress').length ?? 0);
  const blocked = $derived(current?.items.filter(item => item.status === 'blocked').length ?? 0);
  const skipped = $derived(current?.items.filter(item => item.status === 'skipped').length ?? 0);
  const progressSummary = $derived(current
    ? `${completed} of ${current.items.length} done${inProgress ? ` · ${inProgress} in progress` : ''}${blocked ? ` · ${blocked} blocked` : ''}${skipped ? ` · ${skipped} skipped` : ''}`
    : '');

  const statusLabel: Record<WorkPlanItem['status'], string> = {
    pending: 'Pending',
    in_progress: 'In progress',
    completed: 'Done',
    blocked: 'Blocked',
    skipped: 'Skipped',
  };

  function completedCount(plan: WorkPlan): number {
    return plan.items.filter(item => item.status === 'completed').length;
  }
</script>

<section class="work-plan" aria-label="Agent work plan">
  <header class="work-plan-heading">
    <span><ListChecks size={14} aria-hidden="true"/> Work plan</span>
    {#if current}<span class="plan-state" class:closed={current.status === 'closed'}>{current.status === 'active' ? 'Active' : 'Closed'}</span>{/if}
  </header>

  {#if current}
    <div class="plan-head">
      <h3>{current.title}</h3>
      <p>{progressSummary}</p>
    </div>
    <div class="plan-progress" role="progressbar" aria-label="Completed work-plan items" aria-valuemin="0" aria-valuemax={current.items.length} aria-valuenow={completed}>
      {#each current.items as item (item.id)}<span data-status={item.status} title={`${item.title}: ${statusLabel[item.status]}`}></span>{/each}
    </div>
    <ol class="plan-items">
      {#each current.items as item (item.id)}
        <li data-status={item.status} aria-label={`${item.title}: ${statusLabel[item.status]}`}>
          <span class="item-icon" aria-hidden="true">
            {#if item.status === 'completed'}<Check size={13}/>
            {:else if item.status === 'in_progress'}<CircleDot size={14}/>
            {:else if item.status === 'blocked'}<OctagonAlert size={14}/>
            {:else if item.status === 'skipped'}<Minus size={14}/>
            {:else}<Circle size={13}/>{/if}
          </span>
          <span class="item-body"><span class="item-title">{item.title}</span>{#if item.note}<span class="item-note">{item.note}</span>{/if}</span>
          <small>{statusLabel[item.status]}</small>
        </li>
      {/each}
    </ol>
    {#if current.summary}<p class="plan-summary"><b>Outcome</b>{current.summary}</p>{/if}
    {#if previous.length}
      <details class="plan-history">
        <summary>Earlier plans <span>{previous.length}</span></summary>
        <ol>{#each previous as plan (plan.id)}<li><b>{plan.title}</b><small>{completedCount(plan)} / {plan.items.length} done · {plan.status}</small>{#if plan.summary}<p>{plan.summary}</p>{/if}</li>{/each}</ol>
      </details>
    {/if}
  {:else}
    <p class="plan-empty">No work plan published for this chat yet.</p>
  {/if}
</section>

<style>
  .work-plan{min-width:0;padding:11px;border:1px solid color-mix(in srgb,var(--line) 75%,transparent);border-radius:7px;background:color-mix(in srgb,var(--paper) 64%,var(--sidebar));}
  .work-plan-heading{display:flex;align-items:center;justify-content:space-between;gap:8px;color:var(--muted);font:600 calc(10px * var(--interface-font-ratio,1))/1.2 var(--mono);letter-spacing:.06em;text-transform:uppercase}
  .work-plan-heading>span:first-child{display:flex;align-items:center;gap:6px;min-width:0}.work-plan-heading :global(svg){color:var(--accent-ink)}
  .plan-state{padding:3px 6px;border-radius:999px;background:color-mix(in srgb,var(--accent) 12%,transparent);color:var(--accent-ink);font:600 calc(9px * var(--interface-font-ratio,1))/1 var(--mono);letter-spacing:0;text-transform:none}
  .plan-state.closed{background:color-mix(in srgb,var(--muted) 13%,transparent);color:var(--muted)}
  .plan-head h3{margin:10px 0 2px;color:var(--ink);font:600 calc(12px * var(--interface-font-ratio,1))/1.35 var(--sans,system-ui);overflow-wrap:anywhere}
  .plan-head p{margin:0;color:var(--muted);font:calc(10px * var(--interface-font-ratio,1))/1.4 var(--sans,system-ui);overflow-wrap:anywhere}
  .plan-progress{display:flex;gap:2px;height:5px;margin:11px 0 10px;overflow:hidden;border-radius:999px;background:color-mix(in srgb,var(--muted) 15%,transparent)}
  .plan-progress span{flex:1;min-width:0;background:color-mix(in srgb,var(--muted) 19%,transparent)}
  .plan-progress span[data-status=completed]{background:#3eaa7a}.plan-progress span[data-status=in_progress]{background:var(--accent)}.plan-progress span[data-status=blocked]{background:var(--danger,#c96058)}.plan-progress span[data-status=skipped]{background:color-mix(in srgb,var(--muted) 55%,transparent)}
  .plan-items,.plan-history ol{list-style:none;margin:0;padding:0}
  .plan-items{display:grid;gap:2px}
  .plan-items li{display:flex;align-items:flex-start;gap:8px;min-width:0;padding:6px 0;border-top:1px solid color-mix(in srgb,var(--line) 55%,transparent)}
  .item-icon{display:grid;place-items:center;flex:none;width:18px;height:18px;margin-top:1px;border:1px solid color-mix(in srgb,var(--muted) 35%,transparent);border-radius:6px;color:var(--muted)}
  li[data-status=completed] .item-icon{border-color:color-mix(in srgb,#3eaa7a 40%,transparent);background:color-mix(in srgb,#3eaa7a 13%,transparent);color:#3eaa7a}
  li[data-status=in_progress] .item-icon{border-color:color-mix(in srgb,var(--accent) 45%,transparent);background:color-mix(in srgb,var(--accent) 12%,transparent);color:var(--accent-ink)}
  li[data-status=blocked] .item-icon{border-color:color-mix(in srgb,var(--danger,#c96058) 45%,transparent);background:color-mix(in srgb,var(--danger,#c96058) 12%,transparent);color:var(--danger,#c96058)}
  li[data-status=skipped] .item-icon{background:color-mix(in srgb,var(--muted) 10%,transparent)}
  .item-body{display:grid;gap:3px;flex:1;min-width:0}.item-title{color:var(--ink);font:calc(11px * var(--interface-font-ratio,1))/1.35 var(--sans,system-ui);overflow-wrap:anywhere}
  li[data-status=completed] .item-title,li[data-status=skipped] .item-title{color:var(--muted)}
  .item-note{color:var(--muted);font:calc(10px * var(--interface-font-ratio,1))/1.4 var(--sans,system-ui);white-space:pre-wrap;overflow-wrap:anywhere}
  .plan-items small{flex:none;max-width:65px;margin-top:3px;color:var(--muted);font:calc(9px * var(--interface-font-ratio,1))/1.25 var(--mono);text-align:right}
  li[data-status=in_progress] small{color:var(--accent-ink)}li[data-status=blocked] small{color:var(--danger,#c96058)}
  .plan-summary{display:grid;gap:3px;margin:9px 0 0;padding:8px;border-radius:5px;background:color-mix(in srgb,var(--accent) 7%,transparent);color:var(--ink);font:calc(10px * var(--interface-font-ratio,1))/1.45 var(--sans,system-ui);white-space:pre-wrap;overflow-wrap:anywhere}
  .plan-summary b{color:var(--muted);font:600 calc(9px * var(--interface-font-ratio,1))/1 var(--mono);letter-spacing:.05em;text-transform:uppercase}
  .plan-history{margin-top:9px;padding-top:8px;border-top:1px solid var(--line)}.plan-history summary{display:flex;justify-content:space-between;cursor:pointer;color:var(--muted);font:calc(10px * var(--interface-font-ratio,1))/1.4 var(--sans,system-ui)}.plan-history summary:hover{color:var(--ink)}.plan-history ol{display:grid;gap:6px;margin-top:9px}.plan-history li{display:grid;gap:2px;padding:6px;border-radius:5px;background:color-mix(in srgb,var(--muted) 5%,transparent);overflow-wrap:anywhere}.plan-history b{font:600 calc(10px * var(--interface-font-ratio,1))/1.3 var(--sans,system-ui)}.plan-history small,.plan-history p{margin:0;color:var(--muted);font:calc(9px * var(--interface-font-ratio,1))/1.4 var(--sans,system-ui)}
  .plan-empty{margin:9px 0 0;color:var(--muted);font:calc(10px * var(--interface-font-ratio,1))/1.45 var(--sans,system-ui)}
</style>
