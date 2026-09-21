<script lang="ts">
  import { onDestroy } from 'svelte';
  import { ArrowLeft, ExternalLink, LoaderCircle, Mail, ShieldCheck } from '@lucide/svelte';
  import { getBridge } from '$lib/bridge';
  import type { MailBatch, MailCard, MailDetail } from '$lib/types';

  let { batch, taskId, fullHeight = false }: { batch: MailBatch; taskId: string; fullHeight?: boolean } = $props();
  const bridge = getBridge();
  const items = $derived([...batch.items].sort((a, b) => b.importanceScore - a.importanceScore || b.receivedAt - a.receivedAt));
  const activeItems = $derived(items.filter(item => (item.state ?? 'active') === 'active'));
  const historyItems = $derived(items.filter(item => item.state === 'history'));
  const needsReview = $derived(activeItems.filter(item => item.confidence < 60 || item.replyRequired === 'unclear'));
  const actionable = $derived(activeItems.filter(item => !needsReview.includes(item) && (item.importance === 'critical' || item.importance === 'high' || item.replyRequired === 'yes' || ['action_request', 'decision_needed', 'scheduling'].includes(item.intent) || ['reply', 'schedule', 'delegate', 'track'].includes(item.suggestedAction))));
  const reference = $derived(activeItems.filter(item => !needsReview.includes(item) && !actionable.includes(item)));
  const groups = $derived([
    { key: 'action', label: 'Needs action', items: actionable },
    { key: 'review', label: 'Needs review', items: needsReview },
    { key: 'reference', label: 'For reference', items: reference },
  ].filter(group => group.items.length > 0));
  const newCount = $derived(activeItems.filter(item => item.isNew).length);
  let selected = $state<MailCard | null>(null);
  let detail = $state<MailDetail | null>(null);
  let loading = $state(false);
  let error = $state('');
  let poll: ReturnType<typeof setTimeout> | null = null;
  let generation = 0;

  const label = (value: string) => value.replaceAll('_', ' ');
  const received = (value: number) => new Date(value).toLocaleString([], { dateStyle: 'medium', timeStyle: 'short' });
  const checked = $derived(received(batch.updatedAt || batch.createdAt));
  const importanceClass = (value: MailCard['importance']) => `importance-${value}`;

  function stopPoll() { if (poll) clearTimeout(poll); poll = null; }
  function close() { generation += 1; stopPoll(); selected = null; detail = null; loading = false; error = ''; }

  async function waitForDetail(mailId: string, current: number, attempts = 0): Promise<void> {
    if (current !== generation || !selected) return;
    try {
      const found = await bridge.getMailDetail(taskId, mailId);
      if (current !== generation) return;
      if (found) { detail = found; loading = false; return; }
    } catch (reason) {
      if (current !== generation) return;
      error = String(reason);
      loading = false;
      return;
    }
    if (attempts >= 180) {
      error = 'The Gmail reader did not return this message within three minutes. Close this view and try again.';
      loading = false;
      return;
    }
    poll = setTimeout(() => { void waitForDetail(mailId, current, attempts + 1); }, 1000);
  }

  async function open(card: MailCard) {
    generation += 1;
    const current = generation;
    stopPoll();
    selected = card; detail = null; error = ''; loading = true;
    try {
      const cached = await bridge.getMailDetail(taskId, card.id);
      if (current !== generation) return;
      if (cached) { detail = cached; loading = false; return; }
      await bridge.requestMailDetail(taskId, card.id);
      if (current !== generation) return;
      await waitForDetail(card.id, current);
    } catch (reason) {
      if (current !== generation) return;
      error = String(reason);
      loading = false;
    }
  }

  onDestroy(stopPoll);
</script>

<section class="mail-batch" class:full-height={fullHeight} aria-label={`Actionable inbox from ${batch.accountLabel}`}>
  <header>
    <span class="mail-icon"><Mail size={16}/></span>
    <span><strong>Actionable inbox</strong><small>{batch.accountLabel} · {batch.queryLabel} · checked {checked}</small></span>
    <span class:pending={batch.classifier.mode === 'pending'} class:degraded={!['jev', 'pending', 'not_needed'].includes(batch.classifier.mode)} class="classifier">{batch.classifier.mode === 'jev' ? 'Jev' : batch.classifier.mode === 'pending' ? 'Jev pending' : batch.classifier.mode === 'not_needed' ? 'Checked' : `${batch.classifier.mode} classifier`}</span>
  </header>
  <div class="inbox-summary" aria-label="Inbox summary">
    <strong>{actionable.length}</strong><span>need action</span><strong>{needsReview.length}</strong><span>need review</span>{#if newCount}<strong class="new-count">{newCount}</strong><span>new</span>{/if}
  </div>
  {#if batch.classifier.mode === 'pending'}<p class="pending-note" role="status">Showing immediate provisional labels while Jev classifies this batch in the background.</p>
  {:else if !['jev', 'not_needed'].includes(batch.classifier.mode)}<p class="fallback" role="status">Jev was unavailable; these labels use the local low-confidence fallback. {batch.classifier.fallbackReason ?? ''}</p>{/if}
  <div class="mail-content">
    {#if selected}
      <article class="mail-expanded" role="region" aria-label={`Email: ${selected.subject || 'No subject'}`}>
        <header class="expanded-row">
          <button class="back" aria-label="Back to inbox" onclick={close}><ArrowLeft size={17}/><span>Inbox</span></button>
          <span class={`importance ${importanceClass(selected.importance)}`}>{selected.importanceScore}</span>
          <span class="mail-copy">
            <span class="mail-row"><strong>{selected.subject || '(no subject)'}</strong>{#if selected.isNew}<em class="new-pill">New</em>{/if}<time>{received(selected.receivedAt)}</time></span>
            <span class="sender">{selected.from}</span>
            <span class="signals"><em>{label(selected.intent)}</em><em>reply {label(selected.replyRequired)}</em><em>owner {label(selected.suggestedOwner)}</em><em>{label(selected.suggestedAction)}</em><em>{selected.confidence}% confidence</em></span>
          </span>
        </header>
        <dl>
          <div><dt>From</dt><dd>{selected.from}</dd></div>
          <div><dt>To</dt><dd>{selected.to.join(', ') || 'Not supplied'}</dd></div>
          {#if selected.cc.length}<div><dt>Cc</dt><dd>{selected.cc.join(', ')}</dd></div>{/if}
          <div><dt>Received</dt><dd>{received(selected.receivedAt)}</dd></div>
        </dl>
        {#if loading}<div class="detail-state" role="status"><LoaderCircle class="spin" size={18}/><span>Asking this Codex agent to read the full message through its Gmail connector…</span></div>
        {:else if error}<div class="detail-state error" role="alert">{error}</div>
        {:else if detail}<pre>{detail.bodyText}</pre>{/if}
        <footer><ShieldCheck size={13}/><span>Email content is displayed as untrusted plain text. No mailbox action is available here.</span></footer>
      </article>
    {:else}
      <div class="mail-scroll" aria-label="Email message list">
      {#if groups.length === 0}<div class="empty-inbox"><strong>Nothing currently needs attention.</strong><span>This list will update on the next scheduled check.</span></div>{/if}
      {#each groups as group (group.key)}
        <section class="mail-group" aria-label={group.label}>
          <h3>{group.label}<span>{group.items.length}</span></h3>
          <div class="mail-list">
            {#each group.items as item (item.id)}
              <button class:new-mail={item.isNew} class="mail-card" onclick={() => void open(item)} aria-label={`Open ${item.subject || 'email'} from ${item.from}`}>
                <span class={`importance ${importanceClass(item.importance)}`}>{item.importanceScore}</span>
                <span class="mail-copy">
                  <span class="mail-row"><strong>{item.subject || '(no subject)'}</strong>{#if item.isNew}<em class="new-pill">New</em>{/if}<time>{received(item.receivedAt)}</time></span>
                  <span class="sender">{item.from}</span>
                  <span class="snippet">{item.snippet || 'No snippet available.'}</span>
                  <span class="signals"><em>{label(item.intent)}</em><em>reply {label(item.replyRequired)}</em><em>owner {label(item.suggestedOwner)}</em><em>{label(item.suggestedAction)}</em><em>{item.confidence}% confidence</em></span>
                </span>
                <ExternalLink class="open-icon" size={14} aria-hidden="true"/>
              </button>
            {/each}
          </div>
        </section>
      {/each}
      {#if historyItems.length}
        <details class="mail-history"><summary>No longer matching <span>{historyItems.length}</span></summary><div class="mail-list history-list">
          {#each historyItems as item (item.id)}<button class="mail-card" onclick={() => void open(item)}><span class={`importance ${importanceClass(item.importance)}`}>{item.importanceScore}</span><span class="mail-copy"><span class="mail-row"><strong>{item.subject || '(no subject)'}</strong><time>{received(item.receivedAt)}</time></span><span class="sender">{item.from}</span></span><ExternalLink class="open-icon" size={14} aria-hidden="true"/></button>{/each}
        </div></details>
      {/if}
      </div>
    {/if}
  </div>
  <div class="sync-receipt" role="status">Checked {checked} · {batch.lastAdded ?? 0} added · {batch.lastUpdated ?? 0} refreshed{#if batch.lastMovedToHistory}<span>· {batch.lastMovedToHistory} moved to history</span>{/if}</div>
  <footer><ShieldCheck size={13}/><span>Read-only local inbox. Jev receives bounded envelope snippets; full bodies load only after a click.</span></footer>
</section>

<style>
  .mail-batch.full-height{display:flex;height:100%;min-height:0;box-sizing:border-box;margin:0;flex-direction:column}.mail-content{display:block;min-width:0;min-height:0}.mail-scroll{min-width:0;min-height:0}.mail-batch.full-height .mail-content{flex:1;overflow:hidden}.mail-batch.full-height .mail-scroll{height:100%;overflow-y:auto;overscroll-behavior:contain;scrollbar-gutter:stable}.mail-batch.full-height>header,.mail-batch.full-height>.inbox-summary,.mail-batch.full-height>.pending-note,.mail-batch.full-height>.fallback,.mail-batch.full-height>.sync-receipt,.mail-batch.full-height>footer{flex:none}
  .mail-batch{margin:0 0 24px;border:1px solid color-mix(in srgb,var(--accent) 24%,var(--line));border-radius:12px;overflow:hidden;background:color-mix(in srgb,var(--accent) 3%,var(--panel));box-shadow:0 8px 24px #0000000b}.mail-batch>header{display:flex;align-items:center;gap:10px;padding:12px 14px;border-bottom:1px solid var(--line)}.mail-batch>header>span:nth-child(2){display:grid;min-width:0;gap:2px}.mail-batch>header strong{overflow:hidden;text-overflow:ellipsis;white-space:nowrap;font-size:calc(13px * var(--interface-font-ratio,1))}.mail-batch>header small{color:var(--muted);font:calc(9px * var(--interface-font-ratio,1)) var(--mono);text-transform:uppercase;letter-spacing:.035em}.mail-icon{display:grid;place-items:center;width:29px;height:29px;border-radius:8px;color:var(--accent);background:color-mix(in srgb,var(--accent) 12%,var(--panel))}.classifier{margin-left:auto;flex:none;padding:3px 7px;border-radius:999px;color:#2e7650;background:#4f9d6918;font:calc(9px * var(--interface-font-ratio,1)) var(--mono);text-transform:uppercase}.classifier.pending{color:#386e9f;background:#4f87c319}.classifier.degraded{color:#966d2e;background:#c6923519}.inbox-summary{display:flex;align-items:baseline;gap:5px;padding:9px 14px;border-bottom:1px solid var(--line);color:var(--muted);font-size:calc(10px * var(--interface-font-ratio,1))}.inbox-summary strong{margin-left:8px;color:var(--ink);font:600 calc(12px * var(--interface-font-ratio,1)) var(--mono)}.inbox-summary strong:first-child{margin-left:0}.inbox-summary .new-count{color:var(--accent)}.pending-note,.fallback{margin:0;padding:8px 14px;border-bottom:1px solid var(--line);font-size:calc(11px * var(--interface-font-ratio,1))}.pending-note{color:#386e9f;background:#4f87c30d}.fallback{color:#8a672d;background:#c692350d}.mail-group{border-bottom:1px solid var(--line)}.mail-group h3{display:flex;align-items:center;gap:6px;margin:0;padding:8px 14px;color:var(--muted);background:color-mix(in srgb,var(--soft) 55%,transparent);font:600 calc(9px * var(--interface-font-ratio,1)) var(--mono);letter-spacing:.04em;text-transform:uppercase}.mail-group h3 span,.mail-history summary span{display:grid;place-items:center;min-width:18px;height:18px;border-radius:9px;background:var(--soft);font-size:8px}.mail-list{display:grid}.mail-card{display:grid;grid-template-columns:36px minmax(0,1fr) 18px;align-items:start;gap:10px;width:100%;padding:12px 14px;border:0;border-bottom:1px solid var(--line);color:var(--ink);background:transparent;text-align:left}.mail-card:last-child{border-bottom:0}.mail-card:hover,.mail-card:focus-visible{background:color-mix(in srgb,var(--accent) 7%,var(--panel))}.mail-card.new-mail{box-shadow:inset 3px 0 color-mix(in srgb,var(--accent) 70%,transparent)}.open-icon{margin-top:3px;color:var(--muted)}.importance{display:grid;place-items:center;width:32px;height:32px;border-radius:9px;font:600 calc(11px * var(--interface-font-ratio,1)) var(--mono)}.importance-critical{color:#9f3340;background:#d84c5d18}.importance-high{color:#a05d28;background:#df843319}.importance-normal{color:#386e9f;background:#4f87c319}.importance-low{color:var(--muted);background:var(--soft)}.mail-copy{display:grid;min-width:0;gap:3px}.mail-row{display:flex;align-items:baseline;gap:8px}.mail-row strong{min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;font-size:calc(12.5px * var(--interface-font-ratio,1))}.mail-row time{margin-left:auto;flex:none;color:var(--muted);font:calc(9px * var(--interface-font-ratio,1)) var(--mono)}.new-pill{padding:2px 5px;border-radius:999px;color:var(--accent);background:color-mix(in srgb,var(--accent) 12%,transparent);font:600 8px var(--mono);text-transform:uppercase}.sender{overflow:hidden;text-overflow:ellipsis;white-space:nowrap;color:var(--muted);font-size:calc(11px * var(--interface-font-ratio,1))}.snippet{display:-webkit-box;overflow:hidden;color:color-mix(in srgb,var(--ink) 78%,var(--muted));font-size:calc(11.5px * var(--interface-font-ratio,1));line-height:1.45;-webkit-box-orient:vertical;-webkit-line-clamp:2;line-clamp:2}.signals{display:flex;flex-wrap:wrap;gap:5px;margin-top:3px}.signals em{padding:2px 5px;border:1px solid var(--line);border-radius:5px;color:var(--muted);font:normal calc(8.5px * var(--interface-font-ratio,1)) var(--mono);text-transform:uppercase}.empty-inbox{display:grid;gap:4px;padding:22px 14px;text-align:center}.empty-inbox strong{font-size:calc(12px * var(--interface-font-ratio,1))}.empty-inbox span{color:var(--muted);font-size:calc(10px * var(--interface-font-ratio,1))}.mail-history{border-bottom:1px solid var(--line)}.mail-history summary{display:flex;align-items:center;gap:7px;padding:9px 14px;color:var(--muted);cursor:pointer;font:calc(9px * var(--interface-font-ratio,1)) var(--mono);text-transform:uppercase}.history-list{opacity:.72}.sync-receipt{padding:7px 14px;color:var(--muted);background:color-mix(in srgb,var(--soft) 40%,transparent);font:calc(9px * var(--interface-font-ratio,1)) var(--mono)}.sync-receipt span{margin-left:.4em}.mail-batch>footer,.mail-expanded>footer{display:flex;align-items:center;gap:6px;padding:8px 14px;border-top:1px solid var(--line);color:var(--muted);font-size:calc(10px * var(--interface-font-ratio,1))}.mail-expanded{display:flex;min-width:0;min-height:0;max-height:min(720px,calc(100vh - 140px));flex-direction:column;overflow:hidden;color:var(--ink);background:var(--panel);animation:expand-mail .18s ease-out both;transform-origin:top}.mail-batch.full-height .mail-expanded{height:100%;max-height:none}.expanded-row{display:grid;grid-template-columns:auto 36px minmax(0,1fr);align-items:start;gap:10px;padding:12px 14px;border-bottom:1px solid var(--line);background:color-mix(in srgb,var(--accent) 7%,var(--panel))}.back{display:flex;align-items:center;gap:4px;min-height:32px;padding:0 7px;border:1px solid var(--line);border-radius:7px;color:var(--muted);background:var(--panel);font:500 calc(9px * var(--interface-font-ratio,1)) var(--mono)}.back:hover,.back:focus-visible{color:var(--ink);background:var(--soft)}dl{display:grid;gap:5px;margin:0;padding:12px 18px;border-bottom:1px solid var(--line);font-size:calc(11px * var(--interface-font-ratio,1))}dl div{display:grid;grid-template-columns:70px 1fr;gap:10px}dt{color:var(--muted)}dd{min-width:0;margin:0;overflow-wrap:anywhere}.mail-expanded pre{min-height:0;flex:1;margin:0;padding:18px;overflow:auto;white-space:pre-wrap;overflow-wrap:anywhere;color:var(--ink);background:var(--paper);font:calc(12px * var(--interface-font-ratio,1))/1.6 var(--interface-font,"IBM Plex Sans",sans-serif)}.detail-state{display:flex;min-height:180px;flex:1;align-items:center;justify-content:center;gap:9px;padding:24px;color:var(--muted);text-align:center}.detail-state.error{color:#b4544b}.spin{animation:spin 1s linear infinite}@keyframes spin{to{transform:rotate(360deg)}}@keyframes expand-mail{from{opacity:.75;transform:translateY(8px) scaleY(.985)}to{opacity:1;transform:none}}@media(max-width:640px){.mail-card{grid-template-columns:32px minmax(0,1fr);padding:11px}.open-icon{display:none}.mail-row{display:grid;gap:2px}.mail-row time{margin:0}.expanded-row{grid-template-columns:auto 32px minmax(0,1fr);padding:11px}.back span{display:none}dl div{grid-template-columns:54px 1fr}}
</style>
