<script lang="ts">
  import { Brain, ChevronRight, Terminal, SquareTerminal, X } from '@lucide/svelte';
  import type { RunEvent } from '$lib/types';
  import { toolFamily, isShellActivity } from '$lib/activity-grouping';
  import { floating } from '$lib/floating';
  import Markdown from './Markdown.svelte';
  let { event, events = [], compressed = false, running = false }: { event?: RunEvent; events?: RunEvent[]; compressed?: boolean; running?: boolean } = $props();
  const items = $derived(events.length ? events : event ? [event] : []);
  const primary = $derived(items[0]);
  const latest = $derived(items.at(-1));
  const grouped = $derived(items.length > 1);
  const reasoning = $derived(primary?.kind === 'reasoning');
  const family = $derived(compressed && grouped ? 'Tool calls' : primary ? toolFamily(primary) : 'Tool activity');
  const shell = $derived(items.length > 0 && items.every(isShellActivity));
  function action(event: RunEvent, inProgress: boolean) {
    const family = toolFamily(event);
    if (isShellActivity(event)) return inProgress ? 'Running commands' : 'Ran commands';
    if (family === 'gmail') return inProgress ? 'Working in Gmail' : 'Worked in Gmail';
    if (family.includes('file_change') || /(?:write|edit|patch|apply)/i.test(event.title)) return inProgress ? 'Editing files' : 'Edited files';
    if (family.includes('web')) return inProgress ? 'Searching the web' : 'Searched the web';
    if (/(?:read|view|find|list)/i.test(`${family} ${event.title}`)) return inProgress ? 'Reading files' : 'Read files';
    if (/search/i.test(`${family} ${event.title}`)) return inProgress ? `Searching ${family}` : `Searched ${family}`;
    const name = family.replaceAll(/[_-]/g, ' ');
    return inProgress ? `Using ${name}` : `Used ${name}`;
  }
  const actions = $derived([...new Set(items.map(item => action(item, running)))]);
  const description = $derived(compressed && grouped ? `${actions.join(', ')} · ${items.length} tool calls` : shell ? (grouped ? 'Ran commands' : 'Ran command') : latest?.title || 'Tool activity');
  const elapsed = $derived.by(() => {
    const seconds = Math.max(0, ((latest?.createdAt ?? 0) - (primary?.createdAt ?? 0)) / 1000);
    if (seconds < 60) return `${seconds.toFixed(1)}s`;
    const minutes = Math.floor(seconds / 60);
    return minutes < 60 ? `${minutes}m ${Math.floor(seconds % 60)}s` : `${Math.floor(minutes / 60)}h ${minutes % 60}m`;
  });
  let open = $state(false), anchor = $state<HTMLButtonElement>(), panel = $state<HTMLDivElement>();
  const formatTime = (at:number) => new Intl.DateTimeFormat(undefined, {hour:'2-digit',minute:'2-digit'}).format(at);
  function readableDetail(value:string) {
    if (!value || value === 'null') return 'The harness reported this tool event without additional detail.';
    try { return JSON.stringify(JSON.parse(value),null,2); } catch { return value; }
  }
  function close(restoreFocus=true) { open=false; if(restoreFocus)anchor?.focus(); }
  function outside(event:PointerEvent) { if(compressed && grouped)return; if(open && event.target instanceof Node && !anchor?.contains(event.target) && !panel?.contains(event.target))close(false); }
  function keys(event:KeyboardEvent) {
    if(!open)return;
    if(event.key==='Escape'){event.preventDefault();event.stopPropagation();close();}
    if(event.key==='Tab' && panel && !(compressed && grouped)){
      const nodes=Array.from(panel.querySelectorAll<HTMLElement>('button,summary,[tabindex="0"]'));
      if(event.shiftKey && document.activeElement===nodes[0]){event.preventDefault();nodes.at(-1)?.focus();}
      else if(!event.shiftKey && document.activeElement===nodes.at(-1)){event.preventDefault();nodes[0]?.focus();}
    }
  }
</script>
<svelte:window onpointerdown={outside} onkeydown={keys}/>
{#if primary && reasoning}
  <details class="activity reasoning"><summary aria-label="Reasoning summary"><ChevronRight size={13} class="chevron"/><Brain size={14}/><span>Reasoning summary</span><time>{formatTime(primary.createdAt)}</time></summary><div class="activity-body"><Markdown text={primary.detail}/></div></details>
{:else if primary && latest}
  <div class="activity" class:grouped class:compressed={compressed && grouped}>
    <button class="activity-trigger" bind:this={anchor} aria-haspopup={compressed && grouped ? undefined : 'dialog'} aria-expanded={open} aria-label={`Tool activity: ${description}, ${items.length} ${items.length===1?'entry':'entries'}`} onclick={()=>open=!open}>
      {#if shell}<SquareTerminal size={15}/>{:else}<ChevronRight size={13} class="chevron"/><Terminal size={14}/>{/if}<span>{description}</span>{#if grouped && compressed}<small title="Time between the first and latest recorded tool event">· {elapsed}</small>{:else if grouped}<small>{items.length} entries</small>{/if}<time>{formatTime(latest.createdAt)}</time>
    </button>
    {#if open && compressed && grouped}<div bind:this={panel} class="activity-expanded" role="region" aria-label="Expanded tool calls">
      <div class="calls" aria-label="Tool activity entries">
        {#each items as item (item.id)}<details class="call" open>
          <summary><ChevronRight size={12} class="call-chevron"/><span>{item.title || 'Tool activity'}</span><time>{formatTime(item.createdAt)}</time></summary>
          <!-- svelte-ignore a11y_no_noninteractive_tabindex (scrollable output must be keyboard reachable) -->
          <pre tabindex="0" aria-label={`Tool details: ${item.title}`}>{readableDetail(item.detail)}</pre>
        </details>{/each}
      </div>
    </div>{:else if open && anchor}<div bind:this={panel} class="activity-popup" role="dialog" aria-label={`${family} activity history`} tabindex="-1" use:floating={{anchor}}>
      <header><strong>{family.replaceAll('_',' ')} <small>{items.length} {items.length===1?'entry':'entries'}</small></strong><button aria-label="Close activity history" onclick={()=>close()}><X size={15}/></button></header>
      <div class="calls" aria-label="Tool activity entries">
        {#each items as item (item.id)}<details class="call" open>
          <summary><ChevronRight size={12} class="call-chevron"/><span>{item.title || 'Tool activity'}</span><time>{formatTime(item.createdAt)}</time></summary>
          <!-- svelte-ignore a11y_no_noninteractive_tabindex (scrollable output must be keyboard reachable) -->
          <pre tabindex="0" aria-label={`Tool details: ${item.title}`}>{readableDetail(item.detail)}</pre>
        </details>{/each}
      </div>
    </div>{/if}
  </div>
{/if}
<style>
  .activity{margin:12px 0 28px;font-size:calc(12px * var(--interface-font-ratio, 1))}.activity.reasoning{border:1px solid var(--line);border-radius:8px;background:var(--panel)}
  summary,.activity-trigger{display:flex;align-items:center;gap:8px;padding:11px 12px;color:var(--muted);cursor:pointer;list-style:none;text-align:left}
  .activity-trigger{width:100%;font:inherit;padding:8px 0;background:transparent}.activity-trigger:hover{color:var(--ink)}
  summary::-webkit-details-marker{display:none}
  summary span,.activity-trigger span{flex:1;min-width:0;overflow:hidden;white-space:nowrap;text-overflow:ellipsis}
  .compressed .activity-trigger span{flex:0 1 auto}.compressed time{margin-left:auto}
  time{flex-shrink:0;font:calc(10px * var(--interface-font-ratio, 1)) var(--mono)}:global(.activity svg){flex-shrink:0}
  .activity[open] :global(.chevron),.activity-trigger[aria-expanded=true] :global(.chevron),.call[open] :global(.call-chevron){transform:rotate(90deg)}
  .reasoning summary :global(svg){color:var(--accent-ink)}small{flex-shrink:0;color:var(--muted);font:calc(10px * var(--interface-font-ratio, 1)) var(--mono)}
  .activity-body{padding:0 14px 12px;overflow:auto;max-height:280px}
  .activity-popup{position:fixed;inset:auto;margin:0;box-sizing:border-box;padding:12px;width:520px;border:1px solid var(--line);border-radius:12px;background:var(--panel);color:var(--ink);box-shadow:0 12px 40px #0004;font-family:inherit;font-size:calc(12px * var(--interface-font-ratio, 1));overflow:auto;overscroll-behavior:contain}
  header{display:flex;align-items:center;gap:8px;margin-bottom:10px}header strong{flex:1;font-weight:500}header small{margin-left:6px}header button{display:grid;place-items:center;width:25px;height:25px;color:var(--muted);border-radius:5px}header button:hover{background:var(--soft)}
  .activity-expanded{margin-top:8px;padding:10px;border:1px solid var(--line);border-radius:8px;background:var(--panel)}
  .calls{display:grid;gap:6px;max-height:min(420px,65vh);overflow:auto;overscroll-behavior:contain}
  .call{border:1px solid var(--line);border-radius:6px}.call summary{padding:8px 9px;font-size:calc(11px * var(--interface-font-ratio, 1))}.call pre{padding:0 9px 9px;max-height:220px;overflow:auto}
  pre{margin:0;white-space:pre-wrap;overflow-wrap:anywhere;font:calc(11px * var(--interface-font-ratio, 1))/1.6 var(--mono)}
</style>
