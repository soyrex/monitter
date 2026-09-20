<script lang="ts">
  import { onDestroy } from 'svelte';
  import { ExternalLink, LoaderCircle, Mail, ShieldCheck, X } from '@lucide/svelte';
  import { getBridge } from '$lib/bridge';
  import type { MailBatch, MailCard, MailDetail } from '$lib/types';

  let { batch, taskId }: { batch: MailBatch; taskId: string } = $props();
  const bridge = getBridge();
  const items = $derived([...batch.items].sort((a, b) => b.importanceScore - a.importanceScore || b.receivedAt - a.receivedAt));
  let selected = $state<MailCard | null>(null);
  let detail = $state<MailDetail | null>(null);
  let loading = $state(false);
  let error = $state('');
  let poll: ReturnType<typeof setTimeout> | null = null;
  let generation = 0;

  const label = (value: string) => value.replaceAll('_', ' ');
  const received = (value: number) => new Date(value).toLocaleString([], { dateStyle: 'medium', timeStyle: 'short' });
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

<section class="mail-batch" aria-label={`Mail triage from ${batch.accountLabel}`}>
  <header>
    <span class="mail-icon"><Mail size={16}/></span>
    <span><strong>{batch.queryLabel}</strong><small>{batch.accountLabel} · {batch.source} · {batch.items.length} message{batch.items.length === 1 ? '' : 's'}</small></span>
    <span class:pending={batch.classifier.mode === 'pending'} class:degraded={batch.classifier.mode !== 'jev' && batch.classifier.mode !== 'pending'} class="classifier">{batch.classifier.mode === 'jev' ? 'Jev' : batch.classifier.mode === 'pending' ? 'Jev pending' : `${batch.classifier.mode} classifier`}</span>
  </header>
  {#if batch.classifier.mode === 'pending'}<p class="pending-note" role="status">Showing immediate provisional labels while Jev classifies this batch in the background.</p>
  {:else if batch.classifier.mode !== 'jev'}<p class="fallback" role="status">Jev was unavailable; these labels use the local low-confidence fallback. {batch.classifier.fallbackReason ?? ''}</p>{/if}
  <div class="mail-list">
    {#each items as item (item.id)}
      <button class="mail-card" onclick={() => void open(item)} aria-label={`Open ${item.subject || 'email'} from ${item.from}`}>
        <span class={`importance ${importanceClass(item.importance)}`}>{item.importanceScore}</span>
        <span class="mail-copy">
          <span class="mail-row"><strong>{item.subject || '(no subject)'}</strong><time>{received(item.receivedAt)}</time></span>
          <span class="sender">{item.from}</span>
          <span class="snippet">{item.snippet || 'No snippet available.'}</span>
          <span class="signals"><em>{label(item.intent)}</em><em>reply {label(item.replyRequired)}</em><em>owner {label(item.suggestedOwner)}</em><em>{label(item.suggestedAction)}</em><em>{item.confidence}% confidence</em></span>
        </span>
        <ExternalLink class="open-icon" size={14} aria-hidden="true"/>
      </button>
    {/each}
  </div>
  <footer><ShieldCheck size={13}/><span>Read-only triage. Jev receives bounded envelope snippets; full bodies load only after a click.</span></footer>
</section>

{#if selected}
  <div class="mail-overlay" role="presentation" onclick={(event) => { if (event.currentTarget === event.target) close(); }}>
    <div class="mail-detail" role="dialog" aria-modal="true" aria-label={`Email: ${selected.subject || 'No subject'}`}>
      <header>
        <span><small>{batch.accountLabel} · {batch.source}</small><strong>{selected.subject || '(no subject)'}</strong></span>
        <button class="close" aria-label="Close email" onclick={close}><X size={18}/></button>
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
    </div>
  </div>
{/if}

<style>
  .mail-batch{margin:0 0 24px;border:1px solid color-mix(in srgb,var(--accent) 24%,var(--line));border-radius:12px;overflow:hidden;background:color-mix(in srgb,var(--accent) 3%,var(--panel));box-shadow:0 8px 24px #0000000b}.mail-batch>header{display:flex;align-items:center;gap:10px;padding:12px 14px;border-bottom:1px solid var(--line)}.mail-batch>header>span:nth-child(2){display:grid;min-width:0;gap:2px}.mail-batch>header strong{overflow:hidden;text-overflow:ellipsis;white-space:nowrap;font-size:calc(13px * var(--interface-font-ratio,1))}.mail-batch>header small{color:var(--muted);font:calc(9px * var(--interface-font-ratio,1)) var(--mono);text-transform:uppercase;letter-spacing:.035em}.mail-icon{display:grid;place-items:center;width:29px;height:29px;border-radius:8px;color:var(--accent);background:color-mix(in srgb,var(--accent) 12%,var(--panel))}.classifier{margin-left:auto;flex:none;padding:3px 7px;border-radius:999px;color:#2e7650;background:#4f9d6918;font:calc(9px * var(--interface-font-ratio,1)) var(--mono);text-transform:uppercase}.classifier.pending{color:#386e9f;background:#4f87c319}.classifier.degraded{color:#966d2e;background:#c6923519}.pending-note,.fallback{margin:0;padding:8px 14px;border-bottom:1px solid var(--line);font-size:calc(11px * var(--interface-font-ratio,1))}.pending-note{color:#386e9f;background:#4f87c30d}.fallback{color:#8a672d;background:#c692350d}.mail-list{display:grid}.mail-card{display:grid;grid-template-columns:36px minmax(0,1fr) 18px;align-items:start;gap:10px;width:100%;padding:12px 14px;border:0;border-bottom:1px solid var(--line);color:var(--ink);background:transparent;text-align:left}.mail-card:last-child{border-bottom:0}.mail-card:hover,.mail-card:focus-visible{background:color-mix(in srgb,var(--accent) 7%,var(--panel))}.open-icon{margin-top:3px;color:var(--muted)}.importance{display:grid;place-items:center;width:32px;height:32px;border-radius:9px;font:600 calc(11px * var(--interface-font-ratio,1)) var(--mono)}.importance-critical{color:#9f3340;background:#d84c5d18}.importance-high{color:#a05d28;background:#df843319}.importance-normal{color:#386e9f;background:#4f87c319}.importance-low{color:var(--muted);background:var(--soft)}.mail-copy{display:grid;min-width:0;gap:3px}.mail-row{display:flex;align-items:baseline;gap:12px}.mail-row strong{min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;font-size:calc(12.5px * var(--interface-font-ratio,1))}.mail-row time{margin-left:auto;flex:none;color:var(--muted);font:calc(9px * var(--interface-font-ratio,1)) var(--mono)}.sender{overflow:hidden;text-overflow:ellipsis;white-space:nowrap;color:var(--muted);font-size:calc(11px * var(--interface-font-ratio,1))}.snippet{display:-webkit-box;overflow:hidden;color:color-mix(in srgb,var(--ink) 78%,var(--muted));font-size:calc(11.5px * var(--interface-font-ratio,1));line-height:1.45;-webkit-box-orient:vertical;-webkit-line-clamp:2;line-clamp:2}.signals{display:flex;flex-wrap:wrap;gap:5px;margin-top:3px}.signals em{padding:2px 5px;border:1px solid var(--line);border-radius:5px;color:var(--muted);font:normal calc(8.5px * var(--interface-font-ratio,1)) var(--mono);text-transform:uppercase}.mail-batch>footer,.mail-detail>footer{display:flex;align-items:center;gap:6px;padding:8px 14px;border-top:1px solid var(--line);color:var(--muted);font-size:calc(10px * var(--interface-font-ratio,1))}.mail-overlay{position:fixed;z-index:120;inset:0;display:grid;place-items:center;padding:22px;background:#0a0d12a8;backdrop-filter:blur(8px)}.mail-detail{display:flex;flex-direction:column;width:min(760px,calc(100vw - 32px));max-height:min(780px,calc(100vh - 40px));overflow:hidden;border:1px solid var(--line);border-radius:13px;color:var(--ink);background:var(--panel);box-shadow:0 24px 80px #0007}.mail-detail>header{display:flex;gap:16px;align-items:flex-start;padding:16px 18px;border-bottom:1px solid var(--line)}.mail-detail>header>span{display:grid;min-width:0;gap:4px}.mail-detail>header small{color:var(--muted);font:calc(9px * var(--interface-font-ratio,1)) var(--mono);text-transform:uppercase}.mail-detail>header strong{font-size:calc(16px * var(--interface-font-ratio,1))}.close{display:grid;place-items:center;width:31px;height:31px;margin-left:auto;border-radius:7px}.close:hover{background:var(--soft)}dl{display:grid;gap:5px;margin:0;padding:12px 18px;border-bottom:1px solid var(--line);font-size:calc(11px * var(--interface-font-ratio,1))}dl div{display:grid;grid-template-columns:70px 1fr;gap:10px}dt{color:var(--muted)}dd{min-width:0;margin:0;overflow-wrap:anywhere}.mail-detail pre{min-height:180px;margin:0;padding:18px;overflow:auto;white-space:pre-wrap;overflow-wrap:anywhere;color:var(--ink);background:var(--paper);font:calc(12px * var(--interface-font-ratio,1))/1.6 var(--interface-font,"IBM Plex Sans",sans-serif)}.detail-state{display:flex;min-height:220px;align-items:center;justify-content:center;gap:9px;padding:24px;color:var(--muted);text-align:center}.detail-state.error{color:#b4544b}.spin{animation:spin 1s linear infinite}@keyframes spin{to{transform:rotate(360deg)}}@media(max-width:640px){.mail-card{grid-template-columns:32px minmax(0,1fr);padding:11px}.open-icon{display:none}.mail-row{display:grid;gap:2px}.mail-row time{margin:0}.mail-overlay{padding:8px}.mail-detail{width:100%;max-height:calc(100vh - 16px)}dl div{grid-template-columns:54px 1fr}}
</style>
