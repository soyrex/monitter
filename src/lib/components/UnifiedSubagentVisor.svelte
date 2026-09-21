<script lang="ts">
  import { tick } from 'svelte';
  import { LoaderCircle, X } from '@lucide/svelte';
  import UnifiedSubagentItem from './UnifiedSubagentItem.svelte';
  import RunActivity from './RunActivity.svelte';
  import { groupConversationActivity } from '$lib/activity-grouping';
  import { unifiedSubagentDomId, unifiedSubagentStatus, type UnifiedSubagent } from '$lib/unified-subagents';
  import type { RunEvent } from '$lib/types';

  let {
    items, selectedId = null, open = false, idPrefix = 'subagent-visor',
    label = 'Subagent activity', transcriptLoading = false, transcriptError = '',
    onselect, onopenchange,
  }: {
    items: UnifiedSubagent[]; selectedId?: string | null; open?: boolean;
    idPrefix?: string; label?: string; transcriptLoading?: boolean; transcriptError?: string;
    onselect?: (item: UnifiedSubagent) => void; onopenchange?: (value: boolean) => void;
  } = $props();

  const selected = $derived(items.find(item => item.id === selectedId) ?? items[0] ?? null);
  let panel = $state<HTMLDivElement>();
  let followTranscript = $state(true);
  function setOpen(value: boolean) { onopenchange?.(value); }
  function choose(item: UnifiedSubagent) { if (item.id !== selected?.id) followTranscript = true; onselect?.(item); setOpen(true); }
  function selectByOffset(current: number, offset: number) {
    if (!items.length) return;
    const next = items[(current + offset + items.length) % items.length];
    choose(next);
    requestAnimationFrame(() => document.getElementById(unifiedSubagentDomId(`${idPrefix}-tab`, next.id))?.focus());
  }
  function keys(event: KeyboardEvent, index: number) {
    if (event.key === 'ArrowRight' || event.key === 'ArrowDown') { event.preventDefault(); selectByOffset(index, 1); }
    else if (event.key === 'ArrowLeft' || event.key === 'ArrowUp') { event.preventDefault(); selectByOffset(index, -1); }
    else if (event.key === 'Home') { event.preventDefault(); selectByOffset(0, 0); }
    else if (event.key === 'End') { event.preventDefault(); selectByOffset(items.length - 1, 0); }
    else if (event.key === 'Escape') { event.preventDefault(); setOpen(false); }
  }
  const time = (value: number) => value > 0 ? new Date(value).toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' }) : '';
  const fallbackEvents = $derived.by(() => (selected?.transcript ?? [])
    .filter(entry => entry.role === 'activity' || entry.role === 'reasoning')
    .map(entry => {
      const [title = 'Tool activity', ...rest] = entry.text.split(/\r?\n/);
      return { id: entry.id, taskId: selected?.taskId ?? selected?.id ?? 'subagent', kind: entry.role === 'reasoning' ? 'reasoning' : 'tool', title, detail: rest.join('\n') || entry.text, createdAt: entry.createdAt } as RunEvent;
    }));
  const transcriptEvents = $derived(selected?.transcriptEvents?.length ? selected.transcriptEvents : fallbackEvents);
  const activityItems = $derived(groupConversationActivity([], transcriptEvents, true));
  const launchDetails = $derived([selected?.projectName ?? 'No project', selected?.model ?? 'Harness default', selected?.launchedAt ? `Launched ${time(selected.launchedAt)}` : 'Launch time unavailable'].join(' · '));
  function trackScroll() {
    if (panel) followTranscript = panel.scrollHeight - panel.scrollTop - panel.clientHeight < 36;
  }
  $effect(() => {
    const signature = selected?.transcript.map(entry => entry.id).join('|') ?? '';
    if (!open || !followTranscript || !signature) return;
    void tick().then(() => { if (panel && followTranscript) panel.scrollTop = panel.scrollHeight; });
  });
</script>

{#if items.length}
  <section class:open class="subagent-visor" aria-label={label}>
    <div class="visor-shell">
      <div class="visor-tabs" role="tablist" aria-label="Subagent tasks">
        <div class="visor-tabs-inner">
          {#each items as item, index (item.id)}
            <UnifiedSubagentItem item={item} compact tab selected={item.id === selected?.id} buttonId={unifiedSubagentDomId(`${idPrefix}-tab`, item.id)} controls={unifiedSubagentDomId(`${idPrefix}-panel`, item.id)} onkeydown={event => keys(event, index)} onclick={choose}/>
          {/each}
          {#if open}<button class="close" aria-label="Close subagent activity" onclick={() => setOpen(false)}><X size={15}/></button>{/if}
        </div>
      </div>
      {#if selected}
        <div bind:this={panel} class="visor-panel" id={unifiedSubagentDomId(`${idPrefix}-panel`, selected.id)} role="tabpanel" aria-labelledby={unifiedSubagentDomId(`${idPrefix}-tab`, selected.id)} aria-hidden={!open} tabindex={open ? 0 : -1} onscroll={trackScroll}>
          <div class="visor-panel-content">
            <header>
              <div><strong>{selected.agentName}</strong><span><i></i>{unifiedSubagentStatus(selected.status)}</span></div>
              <small class="subagent-meta">{launchDetails}</small>
              {#if selected.activity}<small class="subagent-activity">{selected.activity}</small>{/if}
            </header>
            <div class="transcript" aria-live="polite" aria-label={`${selected.agentName} transcript`}>
              {#if selected.transcript.length}
                {#each selected.transcript.filter(entry => entry.role !== 'activity' && entry.role !== 'reasoning') as entry (entry.id)}
                  <article class:user={entry.role === 'user'} class:assistant={entry.role === 'assistant'}>
                    <div class="entry-meta"><strong>{entry.role === 'user' ? 'Assignment' : selected.agentName}</strong>{#if time(entry.createdAt)}<time>{time(entry.createdAt)}</time>{/if}</div>
                    <div class="entry-text">{entry.text}</div>
                  </article>
                {/each}
                {#each activityItems as activity (activity.type === 'process-group' || activity.type === 'tool-group' || activity.type === 'reasoning-group' ? `${activity.type}:${activity.values[0]?.id}` : activity.value.id)}
                  {#if activity.type === 'process-group'}<RunActivity events={activity.values} processGroups={activity.groups} processTree/>
                  {:else if activity.type === 'tool-group'}<RunActivity events={activity.values} compressed/>
                  {:else if activity.type === 'reasoning-group'}<RunActivity events={activity.values}/>{/if}
                {/each}
              {:else if transcriptLoading}
                <p class="empty"><LoaderCircle class="spin" size={14}/>Loading live transcript…</p>
              {:else}
                {#if selected.prompt}<article><div class="entry-meta"><strong>Assignment</strong></div><div class="entry-text">{selected.prompt}</div></article>{/if}
                <p class="empty">Waiting for transcript activity…</p>
              {/if}
              {#if transcriptLoading && selected.transcript.length}<p class="refresh"><LoaderCircle class="spin" size={12}/>Updating…</p>{/if}
              {#if transcriptError}<p class="transcript-error">Transcript unavailable: {transcriptError}</p>{/if}
            </div>
            <footer>
              {#if selected.model}<span>{selected.model}{selected.reasoningEffort ? ` · ${selected.reasoningEffort}` : ''}</span>{/if}
              <span>{selected.transcript.length} {selected.transcript.length === 1 ? 'entry' : 'entries'}</span>
            </footer>
          </div>
        </div>
      {/if}
    </div>
  </section>
{/if}

<style>
  .subagent-visor{container-type:inline-size;position:relative;z-index:4;width:100%;max-width:none;min-width:0;box-sizing:border-box;margin:0;color:var(--ink);font-size:calc(12px * var(--interface-font-ratio,1))}.visor-shell{position:relative;display:flex;flex-direction:column;width:100%;max-height:min(58dvh,520px);overflow:hidden;border-block:1px solid var(--line);background:var(--panel);box-shadow:0 5px 18px #0001;transition:box-shadow var(--motion-navigation,180ms) var(--motion-ease,ease)}.open .visor-shell{box-shadow:0 12px 34px #0003}
  .visor-tabs{order:0;min-height:40px;padding:5px 0;overflow:hidden;background:var(--panel);scrollbar-width:thin}.visor-tabs-inner,.visor-panel-content{width:min(var(--chat-content-max-width,900px),calc(100% - (2 * var(--density-composer-margin-inline,12px))));max-width:100%;box-sizing:border-box;margin-inline:auto}.visor-tabs-inner{display:flex;align-items:center;gap:5px;min-width:0;overflow-x:auto}.open .visor-tabs{border-bottom:1px solid var(--line)}.close{display:grid;place-items:center;flex:none;width:28px;height:28px;margin-left:auto;padding:0;border:0;border-radius:6px;background:transparent;color:var(--muted);cursor:pointer}.close:hover{background:var(--soft)}.close:focus-visible{outline:2px solid var(--accent);outline-offset:1px}
  .visor-panel{order:1;min-height:0;max-height:0;overflow:hidden;background:var(--panel);opacity:0;transition:max-height var(--motion-navigation,180ms) var(--motion-ease,ease),opacity 120ms ease}.open .visor-panel{max-height:min(50dvh,450px);overflow:auto;overscroll-behavior:contain;opacity:1}.visor-panel:focus-visible{outline:2px solid var(--accent);outline-offset:-2px}
  header{position:sticky;top:0;z-index:1;display:flex;align-items:flex-start;gap:12px;padding:10px 12px;border-bottom:1px solid var(--line);background:color-mix(in srgb,var(--panel) 94%,transparent);backdrop-filter:blur(10px)}header div{display:flex;align-items:center;gap:7px;flex:1;min-width:0}header strong{overflow:hidden;text-overflow:ellipsis;white-space:nowrap;font-size:calc(12px * var(--interface-font-ratio,1));font-weight:600}header span{display:inline-flex;align-items:center;gap:5px;flex:none;color:var(--muted);font:calc(9px * var(--interface-font-ratio,1)) var(--mono);text-transform:uppercase}header span i{width:6px;height:6px;border-radius:50%;background:var(--accent);box-shadow:0 0 0 3px color-mix(in srgb,var(--accent) 17%,transparent)}header small{max-width:45%;overflow:hidden;color:var(--muted);font-size:calc(10px * var(--interface-font-ratio,1));text-overflow:ellipsis;white-space:nowrap}
  .transcript{display:grid;align-content:start;gap:8px;height:240px;min-height:0;box-sizing:border-box;padding:12px;overflow-x:hidden;overflow-y:auto;border:1px solid color-mix(in srgb,var(--terminal-foreground,#e5e7eb) 18%,var(--terminal-background,#090b0d));border-radius:6px;background:var(--terminal-background,#090b0d);color:var(--terminal-foreground,#e5e7eb);box-shadow:inset 0 0 0 1px #0005}.transcript article{max-width:88%;padding:8px 10px;border:1px solid color-mix(in srgb,var(--terminal-foreground,#e5e7eb) 18%,transparent);border-radius:9px 9px 9px 3px;background:color-mix(in srgb,var(--terminal-foreground,#e5e7eb) 5%,var(--terminal-background,#090b0d))}.transcript article.user{justify-self:end;border-color:color-mix(in srgb,var(--accent) 45%,var(--terminal-foreground,#e5e7eb));border-radius:9px 9px 3px 9px;background:color-mix(in srgb,var(--accent) 12%,var(--terminal-background,#090b0d))}.transcript article.assistant{justify-self:start}.entry-meta{display:flex;align-items:center;gap:10px;margin-bottom:4px;color:color-mix(in srgb,var(--terminal-foreground,#e5e7eb) 68%,var(--terminal-background,#090b0d));font:calc(8px * var(--interface-font-ratio,1)) var(--terminal-font,var(--mono,monospace));text-transform:uppercase;letter-spacing:.04em}.entry-meta strong{font:inherit;color:inherit}.entry-meta time{margin-left:auto}.entry-text{overflow-wrap:anywhere;font-family:var(--chat-font,"IBM Plex Sans",system-ui,sans-serif);font-size:calc(10px * var(--chat-font-ratio,1));line-height:1.5;white-space:pre-wrap}.tool-call,.thinking-block{width:100%;box-sizing:border-box;border:1px solid color-mix(in srgb,var(--terminal-foreground,#e5e7eb) 18%,transparent);border-radius:6px;background:color-mix(in srgb,var(--terminal-foreground,#e5e7eb) 4%,var(--terminal-background,#090b0d));font-size:calc(10px * var(--interface-font-ratio,1))}.tool-call summary,.thinking-block summary{display:flex;align-items:center;gap:7px;padding:7px 9px;color:color-mix(in srgb,var(--terminal-foreground,#e5e7eb) 78%,var(--terminal-background,#090b0d));cursor:pointer;list-style:none;font-family:var(--terminal-font,var(--mono,monospace))}.tool-call summary::-webkit-details-marker,.thinking-block summary::-webkit-details-marker{display:none}.tool-call summary span,.thinking-block summary span{min-width:0;overflow:hidden;flex:1;text-overflow:ellipsis;white-space:nowrap}.tool-call summary time,.thinking-block summary time{flex:none;color:color-mix(in srgb,var(--terminal-foreground,#e5e7eb) 55%,var(--terminal-background,#090b0d));font-size:calc(8px * var(--interface-font-ratio,1))}.tool-call[open] :global(.disclosure-chevron),.thinking-block[open] :global(.disclosure-chevron){transform:rotate(90deg)}:global(.disclosure-chevron){flex:none;transition:transform 120ms ease}.tool-detail{padding:8px 10px 10px;border-top:1px solid color-mix(in srgb,var(--terminal-foreground,#e5e7eb) 12%,transparent);overflow-wrap:anywhere;color:color-mix(in srgb,var(--terminal-foreground,#e5e7eb) 82%,var(--terminal-background,#090b0d));font:calc(10px * var(--interface-font-ratio,1))/1.5 var(--terminal-font,var(--mono,monospace));white-space:pre-wrap}.empty,.refresh,.transcript-error{display:flex;align-items:center;gap:7px;margin:0;padding:8px;color:color-mix(in srgb,var(--terminal-foreground,#e5e7eb) 68%,var(--terminal-background,#090b0d));font-family:var(--terminal-font,var(--mono,monospace));font-size:calc(9px * var(--interface-font-ratio,1))}.refresh{justify-content:center;padding:2px}.transcript-error{color:#f0a0a0}footer{display:flex;justify-content:space-between;gap:12px;padding:7px 12px;border-top:1px solid var(--line);color:var(--muted);font:calc(9px * var(--interface-font-ratio,1)) var(--mono)}:global(.spin){animation:spin .9s linear infinite}@keyframes spin{to{transform:rotate(360deg)}}
  .subagent-meta{flex:0 1 auto}.subagent-activity{flex:0 1 28%;font-family:var(--terminal-font,var(--mono,monospace))}
  @container (max-width:450px){.visor-tabs-inner,.visor-panel-content{width:calc(100% - (2 * var(--density-composer-margin-inline,12px)))}.transcript article{max-width:96%}header small.subagent-meta{display:block;max-width:52%;font-size:calc(8px * var(--interface-font-ratio,1))}header small.subagent-activity{display:none}}
  @media(prefers-reduced-motion:reduce){.visor-panel{transition:none}:global(.spin){animation:none}}
</style>
