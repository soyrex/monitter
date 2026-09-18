<script lang="ts">
  import { tick } from 'svelte';
  import { LoaderCircle, X } from '@lucide/svelte';
  import UnifiedSubagentItem from './UnifiedSubagentItem.svelte';
  import { unifiedSubagentDomId, unifiedSubagentStatus, type UnifiedSubagent } from '$lib/unified-subagents';

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
              {#if selected.activity}<small>{selected.activity}</small>{/if}
            </header>
            <div class="transcript" aria-live="polite" aria-label={`${selected.agentName} transcript`}>
              {#if selected.transcript.length}
                {#each selected.transcript as entry (entry.id)}
                  <article class:assistant={entry.role === 'assistant'} class:activity={entry.role === 'activity' || entry.role === 'reasoning'}>
                    <div class="entry-meta"><strong>{entry.role === 'user' ? 'Assignment' : entry.role === 'assistant' ? selected.agentName : entry.role === 'reasoning' ? 'Thinking' : 'Activity'}</strong>{#if time(entry.createdAt)}<time>{time(entry.createdAt)}</time>{/if}</div>
                    <div class="entry-text">{entry.text}</div>
                  </article>
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
  .subagent-visor{container-type:inline-size;position:relative;z-index:4;width:min(var(--chat-content-max-width,900px),calc(100% - (2 * var(--density-composer-margin-inline,12px))));max-width:100%;min-width:0;box-sizing:border-box;margin-inline:auto;color:var(--ink);font-size:calc(12px * var(--interface-font-ratio,1))}.visor-shell{position:relative;display:flex;flex-direction:column;width:100%;max-height:min(58dvh,520px);overflow:hidden;border:1px solid var(--line);border-radius:0 0 10px 10px;background:var(--panel);box-shadow:0 5px 18px #0001;transition:box-shadow var(--motion-navigation,180ms) var(--motion-ease,ease)}.open .visor-shell{box-shadow:0 12px 34px #0003}
  .visor-tabs{order:0;min-height:40px;padding:5px 7px;overflow:hidden;background:color-mix(in srgb,var(--paper) 72%,var(--panel));scrollbar-width:thin}.visor-tabs-inner,.visor-panel-content{width:100%;box-sizing:border-box;margin-inline:auto}.visor-tabs-inner{display:flex;align-items:center;gap:5px;min-width:0;overflow-x:auto}.open .visor-tabs{border-bottom:1px solid var(--line)}.close{display:grid;place-items:center;flex:none;width:28px;height:28px;margin-left:auto;padding:0;border:0;border-radius:6px;background:transparent;color:var(--muted);cursor:pointer}.close:hover{background:var(--soft)}.close:focus-visible{outline:2px solid var(--accent);outline-offset:1px}
  .visor-panel{order:1;min-height:0;max-height:0;overflow:hidden;background:var(--panel);opacity:0;transition:max-height var(--motion-navigation,180ms) var(--motion-ease,ease),opacity 120ms ease}.open .visor-panel{max-height:min(50dvh,450px);overflow:auto;overscroll-behavior:contain;opacity:1}.visor-panel:focus-visible{outline:2px solid var(--accent);outline-offset:-2px}
  header{position:sticky;top:0;z-index:1;display:flex;align-items:flex-start;gap:12px;padding:10px 12px;border-bottom:1px solid var(--line);background:color-mix(in srgb,var(--panel) 94%,transparent);backdrop-filter:blur(10px)}header div{display:flex;align-items:center;gap:7px;flex:1;min-width:0}header strong{overflow:hidden;text-overflow:ellipsis;white-space:nowrap;font-size:calc(12px * var(--interface-font-ratio,1));font-weight:600}header span{display:inline-flex;align-items:center;gap:5px;flex:none;color:var(--muted);font:calc(9px * var(--interface-font-ratio,1)) var(--mono);text-transform:uppercase}header span i{width:6px;height:6px;border-radius:50%;background:var(--accent);box-shadow:0 0 0 3px color-mix(in srgb,var(--accent) 17%,transparent)}header small{max-width:45%;overflow:hidden;color:var(--muted);font-size:calc(10px * var(--interface-font-ratio,1));text-overflow:ellipsis;white-space:nowrap}
  .transcript{display:grid;align-content:start;gap:10px;height:240px;min-height:0;box-sizing:border-box;padding:12px;overflow-x:hidden;overflow-y:auto;border:1px solid color-mix(in srgb,var(--terminal-foreground,#e5e7eb) 18%,var(--terminal-background,#090b0d));border-radius:6px;background:var(--terminal-background,#090b0d);color:var(--terminal-foreground,#e5e7eb);font-family:var(--terminal-font,var(--mono,monospace));box-shadow:inset 0 0 0 1px #0005}.transcript article{max-width:88%;padding:9px 10px;border:1px solid color-mix(in srgb,var(--terminal-foreground,#e5e7eb) 18%,transparent);border-radius:4px;background:color-mix(in srgb,var(--terminal-foreground,#e5e7eb) 5%,var(--terminal-background,#090b0d))}.transcript article.assistant{justify-self:end;border-color:color-mix(in srgb,var(--accent) 45%,var(--terminal-foreground,#e5e7eb));background:color-mix(in srgb,var(--accent) 12%,var(--terminal-background,#090b0d))}.transcript article.activity{max-width:100%;justify-self:stretch;border-style:dashed;background:transparent}.entry-meta{display:flex;align-items:center;gap:10px;margin-bottom:4px;color:color-mix(in srgb,var(--terminal-foreground,#e5e7eb) 68%,var(--terminal-background,#090b0d));font:calc(9px * var(--interface-font-ratio,1)) var(--terminal-font,var(--mono,monospace));text-transform:uppercase;letter-spacing:.04em}.entry-meta strong{font:inherit;color:inherit}.entry-meta time{margin-left:auto}.entry-text{overflow-wrap:anywhere;font-family:var(--terminal-font,var(--mono,monospace));font-size:calc(12px * var(--chat-font-ratio,1));line-height:1.48;white-space:pre-wrap}.empty,.refresh,.transcript-error{display:flex;align-items:center;gap:7px;margin:0;padding:8px;color:color-mix(in srgb,var(--terminal-foreground,#e5e7eb) 68%,var(--terminal-background,#090b0d));font-family:var(--terminal-font,var(--mono,monospace));font-size:calc(11px * var(--interface-font-ratio,1))}.refresh{justify-content:center;padding:2px}.transcript-error{color:#f0a0a0}footer{display:flex;justify-content:space-between;gap:12px;padding:7px 12px;border-top:1px solid var(--line);color:var(--muted);font:calc(9px * var(--interface-font-ratio,1)) var(--mono)}:global(.spin){animation:spin .9s linear infinite}@keyframes spin{to{transform:rotate(360deg)}}
  @container (max-width:450px){.visor-shell{width:100%}.transcript article{max-width:96%}header small{display:none}}
  @media(prefers-reduced-motion:reduce){.visor-panel{transition:none}:global(.spin){animation:none}}
</style>
