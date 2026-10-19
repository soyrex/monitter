<script lang="ts">
  import type { Collaboration, Message, RunEvent } from '$lib/types';
  import type { UnifiedSubagent } from '$lib/unified-subagents';
  import { exchangeDirection, exchangeGroupState } from '$lib/transcript-notices';
  import AttachmentList from './AttachmentList.svelte';
  import Markdown from './Markdown.svelte';
  let { events, record, taskId, peer, peerName, messages = [], onopen }: { events: RunEvent[]; record?: Collaboration; taskId: string; peer?: UnifiedSubagent | null; peerName?: string; messages?: Message[]; onopen?: (item: UnifiedSubagent) => void } = $props();
  let expanded = $state(false);
  const direction = $derived(exchangeDirection(record, taskId));
  const exchangeState = $derived(exchangeGroupState(events, record));
  const name = $derived(peer?.agentName ?? peerName ?? 'Unknown peer');
  const route = $derived(direction === 'outgoing' ? `This pane → ${name}` : direction === 'incoming' ? `${name} → this pane` : `Direction unknown · ${name}`);
  const bodyId = $derived(`exchange-body-${events[0]?.id}`);
  const time = (at: number) => new Intl.DateTimeFormat(undefined, { hour: '2-digit', minute: '2-digit', second: '2-digit' }).format(at);
</script>
<div class="exchange" class:failed={exchangeState === 'failed'} data-exchange-direction={direction}>
  <button class="exchange-trigger" aria-expanded={expanded} aria-controls={bodyId} onclick={() => expanded = !expanded}>
    <span class="route-mark" class:inflight={exchangeState === 'running'} aria-hidden="true">{direction === 'outgoing' ? '●─→○' : direction === 'incoming' ? '○─→●' : '○──○'}</span>
    <span class="route">{route} · {events.length} {events.length === 1 ? 'event' : 'events'}</span>
    <span class="state">{exchangeState}</span><span class="caret" aria-hidden="true">{expanded ? '⌄' : '›'}</span>
  </button>
  {#if expanded}<div class="exchange-body" id={bodyId}>
    {#each events as event (event.id)}<div class="exchange-step"><time>{time(event.createdAt)}</time><span>{event.title}</span></div>{/each}
    {#if record?.text}<section aria-label="Exchange request"><Markdown text={record.text}/></section>{/if}
    {#if record?.result}<section aria-label="Exchange result"><Markdown text={record.result}/></section>{/if}
    {#if record?.error}<p role="status">{record.error}</p>{/if}
    {#each messages as message (message.id)}
      {#if (message.text !== record?.text && message.text !== record?.result) || message.attachments?.length}<section aria-label="Recorded exchange message">{#if message.text !== record?.text && message.text !== record?.result}<Markdown text={message.text}/>{/if}<AttachmentList attachments={message.attachments ?? []}/></section>{/if}
    {/each}
    {#if peer}<button class="open-peer" onclick={() => onopen?.(peer!)}>Open {peer.title}</button>{/if}
  </div>{/if}
</div>
<style>
  .exchange{margin:5px 0;color:var(--ink);font-size:calc(11px * var(--interface-font-ratio,1))}.exchange-trigger{display:flex;align-items:center;gap:8px;width:100%;padding:8px 0;background:transparent;color:var(--muted);text-align:left;font:inherit;cursor:pointer}.route-mark{flex:none;color:var(--accent-ink);font:10px var(--mono);white-space:nowrap}.route{flex:1;min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}.state{font-size:10px}.caret{flex:none}.failed .route-mark,.failed .state{color:var(--danger,#bd655b)}.exchange-body{margin:0 0 10px 14px;padding:4px 0 4px 14px;border-left:1px solid var(--line);overflow-wrap:anywhere;-webkit-user-select:text;user-select:text}.exchange-step{display:flex;gap:10px;padding:5px 0;color:var(--muted)}time{flex:none;font:10px var(--mono)}section{margin:8px 0}.open-peer{padding:6px 9px;border:1px solid var(--line);border-radius:5px;background:var(--panel);color:var(--ink);font:inherit;cursor:pointer}button:focus-visible{outline:2px solid var(--accent);outline-offset:2px}.inflight{animation:exchange-pulse 1.5s ease-in-out infinite}@keyframes exchange-pulse{50%{opacity:.45}}@media(prefers-reduced-motion:reduce){:global(:root:not([data-motion="on"])) .inflight{animation:none}}.exchange-trigger,.open-peer{box-sizing:border-box}@media(max-width:640px){.exchange-trigger,.open-peer{min-height:44px}}@media(max-width:420px){.state{max-width:80px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}.exchange-body{margin-left:8px;padding-left:10px}}
</style>
