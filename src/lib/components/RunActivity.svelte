<script lang="ts">
  import type { Snippet } from 'svelte';
  import { AlarmClock, Archive, Bot, BookOpen, Brain, ChevronRight, Download, Eye, FilePen, FileSearch, FolderOpen, Globe, Image, ListChecks, MessageCircle, Monitor, Plug, Search, ShieldCheck, SquareTerminal, Terminal, Users, Wrench, X } from '@lucide/svelte';
  import type { ApprovalRequest, AttachmentFileData, RunEvent } from '$lib/types';
  import { getBridge } from '$lib/bridge';
  import { contextCompactionId, contextCompactionPhase, isContextCompaction, nativeSubagentActivity, toolFamily, isShellActivity, reasoningSummary, readableToolDetail, toolFileChanges, toolImage, toolPresentation, type ToolPresentation } from '$lib/activity-grouping';
  import { floating } from '$lib/floating';
  import Markdown from './Markdown.svelte';
  import AnimatedTitle from './AnimatedTitle.svelte';
  import ThinkingStatus from './ThinkingStatus.svelte';
  import ImageLightbox from './ImageLightbox.svelte';
  type DetailLoader = (event: RunEvent, onChunk: (detail: string) => void) => Promise<string>;
  type ProcessGroup = { type: 'reasoning-group' | 'tool-group'; values: RunEvent[] };
  let { event, events = [], approvals = [], processGroups = [], compressed = false, processTree = false, running = false, active = true, avatar, onloaddetail, onapproval }: { event?: RunEvent; events?: RunEvent[]; approvals?: ApprovalRequest[]; processGroups?: ProcessGroup[]; compressed?: boolean; processTree?: boolean; running?: boolean; active?: boolean; avatar?: Snippet; onloaddetail?: DetailLoader; onapproval?: (request: ApprovalRequest) => void } = $props();
  const items = $derived(events.length ? events : event ? [event] : []);
  const primary = $derived(items[0]);
  const latest = $derived(items.at(-1));
  const grouped = $derived(items.length > 1);
  const hasReasoning = $derived(items.some(item => item.kind === 'reasoning'));
  const toolItems = $derived(items.filter(item => item.kind === 'tool' || item.kind === 'subagent'));
  const failedTools = $derived(toolItems.filter(item => toolPresentation(item, false).label === 'Tool failed').length);
  const reasoning = $derived(primary?.kind === 'reasoning');
  const summary = $derived(reasoning ? [...new Set(items.map(item=>reasoningSummary(item.detail)).filter(Boolean))].join('\n\n') : '');
  const summaryPreview = $derived(reasoning && summary ? summary.replace(/\s+/g, ' ').trim() : '');
  const emptyReasoning = $derived(reasoning && !summary);
  const family = $derived(primary && nativeSubagentActivity(primary) ? 'Subagent activity' : compressed && grouped ? 'Tool calls' : primary ? toolFamily(primary) : 'Tool activity');
  const shell = $derived(items.length > 0 && items.every(isShellActivity));
  const compaction = $derived(items.length > 0 && items.every(isContextCompaction));
  const compactionId = $derived(compaction ? contextCompactionId(primary!) : null);
  const compactionPhases = $derived(items.map(contextCompactionPhase));
  // The app-server gives us separate started and completed items. A pair is
  // the only durable timing evidence available at this frontend boundary.
  const compactionDurationKnown = $derived(
    compaction && compactionId !== null && items.length === 2 &&
    items.every(item => contextCompactionId(item) === compactionId) &&
    latest!.createdAt >= primary!.createdAt &&
    (compactionPhases.every(phase => phase === null) ||
      (compactionPhases.includes('started') && compactionPhases.includes('completed'))),
  );
  const compactionActive = $derived(
    compaction && compactionId !== null && running && items.length === 1 && compactionPhases[0] === 'started',
  );
  const labels = $derived([...new Set(items.map(item => toolPresentation(item, running).label))]);
  const primaryPresentation: ToolPresentation = $derived(
    primary ? toolPresentation(primary, running) : { icon: 'terminal', label: 'Tool activity' },
  );
  const description = $derived(compressed && grouped ? `${labels.join(', ')} · ${items.length} tool calls` : primaryPresentation.label);
  const elapsed = $derived.by(() => {
    const seconds = Math.max(0, ((latest?.createdAt ?? 0) - (primary?.createdAt ?? 0)) / 1000);
    if (seconds < 60) return `${seconds.toFixed(1)}s`;
    const minutes = Math.floor(seconds / 60);
    return minutes < 60 ? `${minutes}m ${Math.floor(seconds % 60)}s` : `${Math.floor(minutes / 60)}h ${minutes % 60}m`;
  });
  const compactionDescription = $derived(
    compactionActive ? 'Compacting context...' :
    compactionDurationKnown ? `Context compacted in: ${elapsed}` :
    'Context compaction activity',
  );
  let open = $state(false), anchor = $state<HTMLButtonElement>(), panel = $state<HTMLDivElement>();
  let loadedDetails = $state<Record<string, string>>({});
  let loadingDetails = $state<Record<string, boolean>>({});
  let detailErrors = $state<Record<string, string>>({});
  let imagePreviews = $state<Record<string, { src: string; name: string }>>({});
  let imageLoading = $state<Record<string, boolean>>({});
  let imageErrors = $state<Record<string, string>>({});
  let lightbox = $state<{ src: string; alt: string; title?: string } | null>(null);
  let lightboxOpener = $state<HTMLElement | null>(null);
  const formatTime = (at:number) => new Intl.DateTimeFormat(undefined, {hour:'2-digit',minute:'2-digit'}).format(at);
  function close(restoreFocus=true) { open=false; if(restoreFocus)anchor?.focus(); }
  const expandedEvent = (item: RunEvent): RunEvent => loadedDetails[item.id] === undefined ? item : { ...item, detail: loadedDetails[item.id] };
  const needsFullDetail = (item: RunEvent) => item.detail.trimEnd().endsWith('[truncated]');
  async function hydrateDetail(item: RunEvent) {
    if (!onloaddetail || !needsFullDetail(item) || loadedDetails[item.id] !== undefined || loadingDetails[item.id]) return;
    loadingDetails = { ...loadingDetails, [item.id]: true };
    detailErrors = { ...detailErrors, [item.id]: '' };
    try {
      const detail = await onloaddetail(item, partial => { loadedDetails = { ...loadedDetails, [item.id]: partial }; });
      loadedDetails = { ...loadedDetails, [item.id]: detail };
    } catch (reason) {
      detailErrors = { ...detailErrors, [item.id]: reason instanceof Error ? reason.message : String(reason) };
    } finally {
      loadingDetails = { ...loadingDetails, [item.id]: false };
    }
  }
  function previewDataUrl(file: AttachmentFileData): string {
    if (!/^image\/(png|jpeg|webp)$/i.test(file.mimeType)) throw new Error('The viewed file is not a PNG, JPEG, or WebP image.');
    if (!/^[a-zA-Z0-9+/=]+$/.test(file.dataBase64)) throw new Error('The image preview data is invalid.');
    return `data:${file.mimeType};base64,${file.dataBase64}`;
  }
  async function hydrateImage(item: RunEvent) {
    const image = toolImage(item);
    if (!image || imagePreviews[item.id] || imageLoading[item.id]) return;
    imageLoading = { ...imageLoading, [item.id]: true };
    imageErrors = { ...imageErrors, [item.id]: '' };
    try {
      const file = await getBridge().readAttachmentFile(image.path);
      imagePreviews = { ...imagePreviews, [item.id]: { src: previewDataUrl(file), name: image.name } };
    } catch (reason) {
      imageErrors = { ...imageErrors, [item.id]: reason instanceof Error ? reason.message : String(reason) };
    } finally {
      imageLoading = { ...imageLoading, [item.id]: false };
    }
  }
  function toggle() {
    open = !open;
    if (open) for (const item of items) { void hydrateDetail(item); void hydrateImage(item); }
  }
  const diffLines = (diff:string) => diff.split('\n').slice(0, 400);
  const diffKind = (line:string) => line.startsWith('+++') || line.startsWith('---') ? 'header' : line.startsWith('+') ? 'added' : line.startsWith('-') ? 'removed' : line.startsWith('@@') ? 'hunk' : 'context';
  function outside(event:PointerEvent) { if(compressed && grouped)return; if(open && event.target instanceof Node && !anchor?.contains(event.target) && !panel?.contains(event.target))close(false); }
  function keys(event:KeyboardEvent) {
    // The preview owns Escape while its modal lightbox is open. Otherwise this
    // popup would unmount before the lightbox can restore focus to its image.
    if (lightbox) return;
    if(!open)return;
    if(event.key==='Escape'){event.preventDefault();event.stopPropagation();close();}
    if(event.key==='Tab' && panel && !(compressed && grouped)){
      const nodes=Array.from(panel.querySelectorAll<HTMLElement>('button,summary,[tabindex="0"]'));
      if(event.shiftKey && document.activeElement===nodes[0]){event.preventDefault();nodes.at(-1)?.focus();}
      else if(!event.shiftKey && document.activeElement===nodes.at(-1)){event.preventDefault();nodes[0]?.focus();}
    }
  }
  const ICON_COMPONENT = {
    'square-terminal': SquareTerminal,
    eye: Eye,
    'file-pen': FilePen,
    'file-search': FileSearch,
    'folder-open': FolderOpen,
    globe: Globe,
    download: Download,
    image: Image,
    search: Search,
    'list-checks': ListChecks,
    bot: Bot,
    users: Users,
    'message-circle': MessageCircle,
    'book-open': BookOpen,
    monitor: Monitor,
    'alarm-clock': AlarmClock,
    plug: Plug,
    wrench: Wrench,
    terminal: Terminal,
  } as const;
  const TriggerIcon = $derived(ICON_COMPONENT[primaryPresentation.icon]);
  const processPreview = $derived.by(() => {
    const current = toolItems.at(-1) ?? primary;
    if (!current) return '';
    const detail = readableToolDetail(current).replace(/\s+/g, ' ').trim().replace(/^Command\s+/i, '');
    if (!detail) return '';
    return detail.length > 100 ? `${detail.slice(0, 100).trimEnd()}…` : detail;
  });
  const processLabel = $derived(`${hasReasoning ? 'Thinking' : 'Turn'} · ${toolItems.length} ${toolItems.length === 1 ? 'tool' : 'tools'}${approvals.length ? ` · ${approvals.length} approval${approvals.length === 1 ? '' : 's'}` : ''}${failedTools ? ` · ${failedTools} failed` : ''}${processPreview ? `: ${processPreview}` : ''}`);
  const approvalLabel = (request: ApprovalRequest) => request.status === 'approved' && request.ruleId ? `Approved by saved rule: ${request.summary || request.tool}` : `${request.status === 'approved' ? 'Approved' : request.status[0].toUpperCase() + request.status.slice(1)}: ${request.summary || request.tool}`;
  const treeGroups = $derived(processGroups.length ? processGroups : [{ type: 'tool-group' as const, values: items }]);
  function processGroupLabel(group: ProcessGroup): string {
    if (group.type === 'reasoning-group') return 'Thought process';
    const values = group.values.filter(item => item.kind === 'tool' || item.kind === 'subagent');
    const count = values.length;
    if (!count) return 'Activity';
    if (values.every(isShellActivity)) return `Ran ${count} ${count === 1 ? 'command' : 'commands'}`;
    const label = toolPresentation(values[0], false).label;
    if (count === 1) return label;
    if (/\ba file\b/i.test(label)) return label.replace(/\ba file\b/i, `${count} files`);
    if (/\ba command\b/i.test(label)) return label.replace(/\ba command\b/i, `${count} commands`);
    return `${label} · ${count} tools`;
  }
</script>
{#snippet imagePreview(item: RunEvent)}
  {#if imagePreviews[item.id]}
    {@const preview=imagePreviews[item.id]}
    <button class="tool-image-preview" aria-label={`View full-size ${preview.name}`} onclick={event=>{lightboxOpener=event.currentTarget;lightbox={src:preview.src,alt:`Preview of ${preview.name}`,title:preview.name}}}>
      <img src={preview.src} alt={`Preview of ${preview.name}`}/>
    </button>
  {:else if imageLoading[item.id]}<p class="detail-state">Loading image preview…</p>
  {:else if imageErrors[item.id]}<p class="detail-state error">Image preview unavailable: {imageErrors[item.id]}</p>
  {/if}
{/snippet}
{#snippet processStep(item: RunEvent)}
  {@const isThought=item.kind === 'reasoning'}
  {@const presentation=isThought ? {icon:'book-open' as const,label:'Thought process'} : toolPresentation(item, running)}
  {@const displayItem=expandedEvent(item)}
  <details class="process-step">
    <summary><ChevronRight size={12} class="call-chevron"/><span class:failed={presentation.label === 'Tool failed'}><svelte:component this={ICON_COMPONENT[presentation.icon]} size={14}/>{presentation.label}</span><time>{formatTime(item.createdAt)}</time></summary>
    {#if isThought}<div class="process-thought"><Markdown text={reasoningSummary(displayItem.detail)}/></div>
    {:else}<pre class="detail-summary" tabindex="0" aria-label={`Tool details: ${item.title}`}>{readableToolDetail(displayItem)}</pre>{/if}
    {@render imagePreview(displayItem)}
    {#if loadingDetails[item.id]}<p class="detail-state">Loading full detail…</p>{/if}
    {#if detailErrors[item.id]}<p class="detail-state error">{detailErrors[item.id]}</p>{/if}
  </details>
{/snippet}
<svelte:window onpointerdown={outside} onkeydown={keys}/>
{#if primary && emptyReasoning}
  <ThinkingStatus {running} {active} {avatar} startedAt={primary.createdAt}/>
{:else if primary && reasoning}
  <details class="activity reasoning"><summary aria-label="Reasoning summary"><ChevronRight size={13} class="chevron"/><Brain size={14}/><span>Reasoning: {summaryPreview}</span><time>{formatTime(latest?.createdAt ?? primary.createdAt)}</time></summary><div class="activity-body"><Markdown text={summary}/></div></details>
{:else if processTree && primary && (hasReasoning || items.length > 1 || approvals.length)}
  <details class="activity process-tree">
    <summary aria-label={processLabel}><ChevronRight size={13} class="chevron"/><Brain size={14}/><span>{processLabel}</span><time>{formatTime(latest?.createdAt ?? primary.createdAt)}</time></summary>
    <div class="process-steps" aria-label="Process steps">
      {#each treeGroups as group, index (group.values[0]?.id ?? index)}
        <details class="process-branch">
          <summary><ChevronRight size={12} class="call-chevron"/><span>{processGroupLabel(group)}</span><time>{formatTime(group.values.at(-1)?.createdAt ?? primary.createdAt)}</time></summary>
          <div class="process-branch-steps">
            {#each group.values as item (item.id)}{@render processStep(item)}{/each}
          </div>
        </details>
      {/each}
      {#each approvals as approval (approval.id)}
        <button class="process-approval" type="button" onclick={() => onapproval?.(approval)} title="Open approval history">
          <ShieldCheck size={14}/><span>{approvalLabel(approval)}</span><time>{formatTime(approval.resolvedAt ?? approval.createdAt)}</time>
        </button>
      {/each}
    </div>
  </details>
{:else if primary && latest}
  <div class="activity" class:grouped class:compressed={compressed && grouped} class:compaction>
    <button class="activity-trigger" bind:this={anchor} aria-haspopup={compressed && grouped ? undefined : 'dialog'} aria-expanded={open} aria-label={compaction ? compactionDescription : `Tool activity: ${description}, ${items.length} ${items.length===1?'entry':'entries'}`} onclick={toggle}>
      {#if compaction}<Archive size={15}/><span><AnimatedTitle text={compactionDescription} active={compactionActive} activeTooltip="Compacting context..."/></span>
      {:else}<ChevronRight size={13} class="chevron"/><TriggerIcon size={14}/>{/if}{#if !compaction}<span>{description}</span>{#if grouped && compressed}<small title="Time between the first and latest recorded tool event">· {elapsed}</small>{:else if grouped}<small>{items.length} entries</small>{/if}<time>{formatTime(latest.createdAt)}</time>{/if}
    </button>
    {#if open && compressed && grouped}<div bind:this={panel} class="activity-expanded" role="region" aria-label="Expanded tool calls">
      <div class="calls" aria-label="Tool activity entries">
        {#each items as item (item.id)}{@const displayItem=expandedEvent(item)}<details class="call" open>
          <summary><ChevronRight size={12} class="call-chevron"/><span>{nativeSubagentActivity(item) ? toolPresentation(item, false).label : item.title || 'Tool activity'}</span><time>{formatTime(item.createdAt)}</time></summary>
          <!-- svelte-ignore a11y_no_noninteractive_tabindex (scrollable output must be keyboard reachable) -->
          <pre class="detail-summary" tabindex="0" aria-label={`Tool details: ${item.title}`}>{readableToolDetail(displayItem)}</pre>
          {@render imagePreview(displayItem)}
          {#if loadingDetails[item.id]}<p class="detail-state">Loading full detail…</p>{/if}
          {#if detailErrors[item.id]}<p class="detail-state error">{detailErrors[item.id]}</p>{/if}
          {#each toolFileChanges(displayItem) as change}
            {#if change.diff}<section class="diff" aria-label={`Changes to ${change.path}`}><strong>{change.path}</strong><code>
              {#each diffLines(change.diff) as line}<span class:added={diffKind(line)==='added'} class:removed={diffKind(line)==='removed'} class:hunk={diffKind(line)==='hunk'} class:header={diffKind(line)==='header'}>{line || ' '}</span>{/each}
              {#if change.diff.split('\n').length > 400}<em>…diff truncated</em>{/if}
            </code></section>{/if}
          {/each}
        </details>{/each}
      </div>
    </div>{:else if open && anchor}<div bind:this={panel} class="activity-popup" role="dialog" aria-label={`${family} activity history`} tabindex="-1" use:floating={{anchor}}>
      <header><strong>{family.replaceAll('_',' ')} <small>{items.length} {items.length===1?'entry':'entries'}</small></strong><button aria-label="Close activity history" onclick={()=>close()}><X size={15}/></button></header>
      <div class="calls" aria-label="Tool activity entries">
        {#each items as item (item.id)}{@const displayItem=expandedEvent(item)}<details class="call" open>
          <summary><ChevronRight size={12} class="call-chevron"/><span>{nativeSubagentActivity(item) ? toolPresentation(item, false).label : item.title || 'Tool activity'}</span><time>{formatTime(item.createdAt)}</time></summary>
          <!-- svelte-ignore a11y_no_noninteractive_tabindex (scrollable output must be keyboard reachable) -->
          <pre class="detail-summary" tabindex="0" aria-label={`Tool details: ${item.title}`}>{readableToolDetail(displayItem)}</pre>
          {@render imagePreview(displayItem)}
          {#if loadingDetails[item.id]}<p class="detail-state">Loading full detail…</p>{/if}
          {#if detailErrors[item.id]}<p class="detail-state error">{detailErrors[item.id]}</p>{/if}
          {#each toolFileChanges(displayItem) as change}
            {#if change.diff}<section class="diff" aria-label={`Changes to ${change.path}`}><strong>{change.path}</strong><code>
              {#each diffLines(change.diff) as line}<span class:added={diffKind(line)==='added'} class:removed={diffKind(line)==='removed'} class:hunk={diffKind(line)==='hunk'} class:header={diffKind(line)==='header'}>{line || ' '}</span>{/each}
              {#if change.diff.split('\n').length > 400}<em>…diff truncated</em>{/if}
            </code></section>{/if}
          {/each}
        </details>{/each}
      </div>
    </div>{/if}
  </div>
{/if}
<style>
  .activity{margin:4px 0 10px;font-size:calc(12px * var(--interface-font-ratio, 1))}.activity.reasoning{margin-bottom:1em;border:1px solid var(--line);border-radius:8px;background:var(--panel)}
  summary,.activity-trigger{display:flex;align-items:center;gap:8px;padding:11px 12px;color:var(--muted);cursor:pointer;list-style:none;text-align:left}
  .activity-trigger{box-sizing:border-box;width:100%;font:inherit;padding:8px 4px 8px 0;background:transparent}.activity-trigger:hover{color:var(--ink)}
  summary::-webkit-details-marker{display:none}
  summary span,.activity-trigger span{flex:1;min-width:0;overflow:hidden;white-space:nowrap;text-overflow:ellipsis}
  .compressed .activity-trigger span{flex:0 1 auto}.compressed time{margin-left:auto}
  .compaction .activity-trigger{color:var(--accent-ink)}.compaction .activity-trigger:hover{color:var(--accent-ink)}
  .compaction .activity-trigger span{flex:1;min-width:0;overflow:visible}
  time{flex-shrink:0;margin-left:auto;padding-right:2px;font:calc(10px * var(--interface-font-ratio, 1)) var(--mono)}:global(.activity svg){flex-shrink:0}
  .activity[open] :global(.chevron),.activity-trigger[aria-expanded=true] :global(.chevron),.call[open] :global(.call-chevron){transform:rotate(90deg)}
  .reasoning summary :global(svg){color:var(--accent-ink)}small{flex-shrink:0;color:var(--muted);font:calc(10px * var(--interface-font-ratio, 1)) var(--mono)}
  .activity-body{padding:0 14px 12px;overflow:auto;max-height:280px}
  .process-tree{margin:10px 0}
  .process-tree > summary{padding:9px 10px;color:var(--ink);font-size:calc(11px * var(--interface-font-ratio,1))}
  .process-steps{position:relative;display:grid;gap:2px;margin:0 10px 10px 24px;padding-left:12px;border-left:1px solid var(--line)}
  .process-branch{position:relative;min-width:0}.process-branch > summary{gap:6px;padding:6px 5px;color:var(--muted);font-size:calc(11px * var(--interface-font-ratio,1))}.process-branch > summary span{display:flex;align-items:center;gap:6px}.process-branch > summary time{font-size:calc(9px * var(--interface-font-ratio,1))}.process-branch[open] > summary{color:var(--ink);background:color-mix(in srgb,var(--soft) 55%,transparent)}
  .process-branch-steps{display:grid;gap:2px;margin:0 0 5px 17px;padding-left:12px;border-left:1px solid var(--line)}
  .process-step{position:relative;min-width:0;border:0;border-radius:5px}
  .process-step::before{content:"";position:absolute;left:-13px;top:16px;width:12px;border-top:1px solid var(--line)}
  .process-step > summary{gap:6px;padding:6px 5px;color:var(--muted);font-size:calc(11px * var(--interface-font-ratio,1))}
  .process-step > summary span{display:flex;align-items:center;gap:6px}.process-step > summary span.failed{color:var(--danger,#c44c79)}
  .process-step > summary :global(svg){flex:none}.process-step > summary time{font-size:calc(9px * var(--interface-font-ratio,1))}
  .process-step[open] > summary{color:var(--ink);background:color-mix(in srgb,var(--soft) 55%,transparent)}
  .process-step .detail-summary,.process-thought{margin:0 5px 7px 23px;padding:7px 8px;border-left:1px solid var(--line);color:var(--muted);font-size:calc(10px * var(--interface-font-ratio,1));line-height:1.45}
  .process-approval{display:flex;align-items:center;gap:6px;width:100%;padding:6px 5px;color:var(--muted);border:0;border-radius:5px;background:transparent;text-align:left;font:inherit;font-size:calc(11px * var(--interface-font-ratio,1));cursor:pointer}.process-approval:hover,.process-approval:focus-visible{color:var(--ink);background:color-mix(in srgb,var(--soft) 55%,transparent)}.process-approval > span{min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}.process-approval :global(svg){flex:none;color:var(--accent-ink)}.process-approval time{font-size:calc(9px * var(--interface-font-ratio,1))}
  .process-thought :global(.markdown){font-size:inherit}.process-thought :global(.markdown p:last-child){margin-bottom:0}
  .activity-popup{position:fixed;inset:auto;margin:0;box-sizing:border-box;padding:12px;width:520px;border:1px solid var(--line);border-radius:12px;background:var(--panel);color:var(--ink);box-shadow:0 12px 40px #0004;font-family:inherit;font-size:calc(12px * var(--interface-font-ratio, 1));overflow:auto;overscroll-behavior:contain}
  header{display:flex;align-items:center;gap:8px;margin-bottom:10px}header strong{flex:1;font-weight:500}header small{margin-left:6px}header button{display:grid;place-items:center;width:25px;height:25px;color:var(--muted);border-radius:5px}header button:hover{background:var(--soft)}
  .activity-expanded{margin-top:8px;padding:10px;border:1px solid var(--line);border-radius:8px;background:var(--panel)}
  .calls{display:grid;gap:6px;max-height:min(420px,65vh);padding-right:3px;overflow:auto;overscroll-behavior:contain;scrollbar-gutter:stable}
  .call{min-width:0;border:1px solid var(--line);border-radius:6px}.call summary{padding:8px 9px;font-size:calc(11px * var(--interface-font-ratio, 1))}.call pre{padding:0 9px 9px;max-height:220px;overflow:auto}
  .diff{margin:0 9px 9px;min-width:0;overflow:hidden;border:1px solid var(--line);border-radius:6px;background:color-mix(in srgb,var(--panel) 86%,var(--soft))}.diff strong{display:block;padding:6px 9px;overflow:hidden;color:var(--muted);border-bottom:1px solid var(--line);font:500 calc(10px * var(--interface-font-ratio, 1)) var(--mono);text-overflow:ellipsis;white-space:nowrap}.diff code{display:block;padding:5px 0;font:calc(10px * var(--interface-font-ratio, 1))/1.45 var(--mono)}.diff code span,.diff code em{display:block;min-width:0;padding:0 9px;white-space:pre-wrap;overflow-wrap:anywhere}.diff .added{color:#238636;background:rgba(46,160,67,.12)}.diff .removed{color:#cf222e;background:rgba(248,81,73,.12)}.diff .hunk{color:var(--accent-ink);background:var(--soft)}.diff .header{color:var(--muted)}.diff em{color:var(--muted);font-style:normal}
  .detail-state{margin:0;padding:0 9px 9px;color:var(--muted);font-size:calc(10px * var(--interface-font-ratio, 1))}.detail-state.error{color:var(--danger)}
  .tool-image-preview{display:block;width:min(220px,calc(100% - 18px));margin:0 9px 9px;padding:0;border:1px solid var(--line);border-radius:6px;background:var(--soft);cursor:zoom-in;overflow:hidden}.tool-image-preview:focus-visible{outline:2px solid var(--accent);outline-offset:2px}.tool-image-preview img{display:block;width:100%;max-height:150px;object-fit:contain;background:var(--paper)}
  pre{margin:0;white-space:pre-wrap;overflow-wrap:anywhere;font:calc(11px * var(--interface-font-ratio, 1))/1.6 var(--mono)}
  @media (max-width:640px){.activity{margin:2px 0 7px}.activity-trigger{padding:6px 0}}
</style>

<ImageLightbox bind:image={lightbox} returnFocus={lightboxOpener}/>
