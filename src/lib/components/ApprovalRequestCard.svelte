<script lang="ts">
  import { AlertTriangle, Check, ShieldAlert, X } from '@lucide/svelte';
  import type { ApprovalRequest } from '$lib/types';
  import HarnessInput from './HarnessInput.svelte';

  let { request, disabled = false, resolving = false, onresolve, oninput }: {
    request: ApprovalRequest;
    disabled?: boolean;
    resolving?: boolean;
    onresolve?: (request: ApprovalRequest, decision: 'approve_once' | 'deny') => void;
    oninput?: (request: ApprovalRequest, response: unknown) => void;
  } = $props();

  const isPending = $derived(request.status === 'pending');
  const action = $derived.by(() => {
    if (request.provider !== 'codex') return null;
    try { const value = JSON.parse(request.detail); return value && typeof value === 'object' && !Array.isArray(value) ? value as Record<string, unknown> : null; }
    catch { return null; }
  });
  const command = $derived(typeof action?.command === 'string' ? action.command : Array.isArray(action?.command) ? action.command.join(' ') : '');
  const reason = $derived(typeof action?.reason === 'string' ? action.reason : '');
  const outcome = $derived(
    request.status === 'approved' ? (request.input ? 'Response submitted' : 'Approved once')
      : request.status === 'denied' ? 'Denied'
      : request.status === 'expired' ? 'Expired before a decision'
      : request.status === 'unsupported' ? 'Unsupported by this backend'
      : 'Awaiting your decision',
  );
  const timestamp = $derived(new Intl.DateTimeFormat(undefined, { hour: '2-digit', minute: '2-digit' }).format(
    isPending ? request.createdAt : request.resolvedAt ?? request.createdAt,
  ));
</script>

<article class:pending={isPending} class:resolved={!isPending} class:high={request.risk === 'high'} class="approval-request" aria-label={isPending ? 'Approval request' : 'Approval outcome'}>
  <header>
    <span class="approval-icon" aria-hidden="true">
      {#if isPending || request.status === 'expired' || request.status === 'unsupported'}<ShieldAlert size={16}/>{:else if request.status === 'approved'}<Check size={16}/>{:else}<X size={16}/>{/if}
    </span>
    <span class="approval-title">{isPending ? (request.input ? 'Your input is needed' : 'Approval needed') : outcome}</span>
    <time>{timestamp}</time>
  </header>
  <p class="approval-summary">{request.summary || request.tool}</p>
  <dl class="approval-metadata">
    <div><dt>Provider</dt><dd>{request.provider}</dd></div>
    <div><dt>Tool</dt><dd>{request.tool}</dd></div>
    <div><dt>Risk</dt><dd>{request.risk}</dd></div>
  </dl>
  {#if action}
    {#if command}<pre class="command-preview">{command}</pre>{/if}
    {#if reason}<p class="approval-detail">{reason}</p>{/if}
    {#if action.permissions}<pre class="command-preview">{JSON.stringify(action.permissions, null, 2)}</pre>{/if}
    {#if action.changes}<pre class="command-preview">{JSON.stringify(action.changes, null, 2)}</pre>{/if}
    <details class="technical"><summary>Request details</summary><pre>{JSON.stringify(action, null, 2)}</pre></details>
  {:else if request.detail}<p class="approval-detail">{request.detail}</p>{/if}
  {#if isPending}
    {#if request.input}<HarnessInput input={request.input} disabled={disabled || resolving || !oninput} onsubmit={response => oninput?.(request, response)}/>{/if}
    <div class="approval-actions">
      {#if !request.input}<button class="approve" disabled={disabled || resolving || !onresolve} aria-label="Approve once approval request" onclick={() => onresolve?.(request, 'approve_once')}><Check size={15}/>Approve once</button>{/if}
      <button class="deny" disabled={disabled || resolving} aria-label="Deny approval request" onclick={() => onresolve?.(request, 'deny')}><X size={15}/>Deny</button>
      {#if resolving}<span class="resolving" role="status"><AlertTriangle size={13}/>Resolving…</span>{/if}
    </div>
  {/if}
</article>

<style>
  .approval-request { margin: 12px 0 24px; padding: 13px; border: 1px solid var(--line); border-radius: 9px; background: var(--panel); color: var(--ink); font-size: calc(12px * var(--interface-font-ratio, 1)); }
  .approval-request.pending { border-color: color-mix(in srgb, var(--accent) 62%, var(--line)); box-shadow: 0 2px 12px color-mix(in srgb, var(--accent) 12%, transparent); }
  .approval-request.pending.high { border-color: color-mix(in srgb, #b84c44 70%, var(--line)); }
  header { display: flex; align-items: center; gap: 7px; }
  .approval-icon { display: grid; place-items: center; color: var(--accent-ink); }
  .high .approval-icon { color: #b84c44; }
  .approval-title { font-weight: 650; }
  time { margin-left: auto; color: var(--muted); font: calc(10px * var(--interface-font-ratio, 1)) var(--mono); }
  .approval-summary { margin: 9px 0 8px; font-size: calc(13px * var(--interface-font-ratio, 1)); font-weight: 600; line-height: 1.35; }
  .approval-metadata { display: flex; flex-wrap: wrap; gap: 5px 13px; margin: 0; color: var(--muted); font: calc(10px * var(--interface-font-ratio, 1)) var(--mono); }
  .approval-metadata div { display: flex; gap: 4px; }
  dt { color: var(--muted); } dd { margin: 0; color: var(--ink); }
  .approval-detail { margin: 10px 0 0; color: var(--muted); white-space: pre-wrap; overflow-wrap: anywhere; line-height: 1.5; }
  .command-preview, .technical pre { white-space: pre-wrap; overflow-wrap: anywhere; max-height: 240px; overflow: auto; font: 12px/1.5 var(--mono); }
  .command-preview { padding: 10px; background: var(--bg); border: 1px solid var(--line); border-radius: 6px; }
  .technical { margin-top: 10px; color: var(--muted); font-size: 11px; }
  .approval-actions { display: flex; align-items: center; flex-wrap: wrap; gap: 7px; margin-top: 13px; }
  .approval-actions button { display: inline-flex; align-items: center; gap: 5px; min-height: 30px; padding: 6px 9px; border: 1px solid var(--line); border-radius: 6px; font: 600 calc(11px * var(--interface-font-ratio, 1)) var(--interface-font, "IBM Plex Sans", sans-serif); }
  .approval-actions .approve { border-color: var(--accent); color: var(--on-accent); background: var(--accent); }
  .approval-actions .deny { color: #b84c44; }
  .approval-actions button:disabled { opacity: .55; cursor: wait; }
  .resolving { display: inline-flex; align-items: center; gap: 4px; color: var(--muted); font: calc(10px * var(--interface-font-ratio, 1)) var(--mono); }
  .resolved { padding-block: 10px; opacity: .92; }
  .resolved .approval-summary { margin-bottom: 5px; font-size: calc(12px * var(--interface-font-ratio, 1)); }
  @media (forced-colors: active) { .approval-request.pending { border-color: Highlight; } .approval-actions .approve { color: HighlightText; background: Highlight; } }
</style>
