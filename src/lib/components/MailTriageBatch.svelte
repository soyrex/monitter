<script lang="ts">
  import { onDestroy } from 'svelte';
  import { ArrowLeft, ExternalLink, LoaderCircle, Mail, MessageSquare, ShieldCheck } from '@lucide/svelte';
  import { getBridge } from '$lib/bridge';
  import type { MailBatch, MailCard, MailDetail } from '$lib/types';

  let { batch, taskId, fullHeight = false, onchat = () => {} }: { batch: MailBatch; taskId: string; fullHeight?: boolean; onchat?: () => void } = $props();
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
    <button class="flip-chat" type="button" aria-label="Chat about this inbox" title="Chat about this inbox" onclick={onchat}><MessageSquare size={15}/><span>Chat</span></button>
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
  .flip-chat{display:inline-flex;align-items:center;gap:5px;flex:none;min-height:28px;padding:0 8px;border:1px solid var(--line);border-radius:7px;color:var(--muted);background:var(--panel);font:500 calc(9px * var(--interface-font-ratio,1)) var(--mono)}.flip-chat:hover,.flip-chat:focus-visible{color:var(--ink);background:var(--soft)}
</style>
