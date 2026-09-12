<script lang="ts">
  import { Check, ChevronLeft, ChevronRight, ShieldAlert, X } from '@lucide/svelte';
  import type { ApprovalRequest } from '$lib/types';
  import HarnessInput from './HarnessInput.svelte';

  let { requests, disabled = false, resolvingId = null, onresolve, oninput }: {
    requests: ApprovalRequest[];
    disabled?: boolean;
    resolvingId?: string | null;
    onresolve?: (request: ApprovalRequest, decision: 'approve_once' | 'deny') => void;
    oninput?: (request: ApprovalRequest, response: unknown) => void;
  } = $props();

  const instanceId = $props.id();
  let selected = $state(0);
  // Keep this defensive filter here as well as at the placement site: a stale
  // snapshot must never put a resolved approval back in front of the composer.
  const pendingRequests = $derived(requests.filter(item => item.status === 'pending'));
  const request = $derived(pendingRequests[selected] ?? null);
  const resolving = $derived(request?.id === resolvingId);
  const countLabel = $derived(`${pendingRequests.length} approval${pendingRequests.length === 1 ? '' : 's'} awaiting your response`);
  const action = $derived.by(() => {
    if (!request || request.provider !== 'codex') return null;
    try {
      const value = JSON.parse(request.detail);
      return value && typeof value === 'object' && !Array.isArray(value) ? value as Record<string, unknown> : null;
    } catch { return null; }
  });
  const command = $derived(typeof action?.command === 'string' ? action.command : Array.isArray(action?.command) ? action.command.join(' ') : '');
  const reason = $derived(typeof action?.reason === 'string' ? action.reason : '');
  const plainDetail = $derived(action ? '' : request?.detail ?? '');
  const inputFormId = $derived(`approval-input-${instanceId}-${request?.id ?? 'none'}`);
  const inputSubmitSupported = $derived.by(() => {
    const input = request?.input;
    if (!input) return false;
    if (input.kind === 'form') return Object.values((input.schema?.properties ?? {}) as Record<string, Record<string, unknown>>).every(field => ['string', 'number', 'integer', 'boolean'].includes(String(field.type)));
    if (input.kind !== 'url') return true;
    try { const url = new URL(input.url ?? ''); return ['https:', 'http:'].includes(url.protocol); } catch { return false; }
  });

  $effect(() => {
    if (selected >= pendingRequests.length) selected = Math.max(0, pendingRequests.length - 1);
  });

  function move(direction: -1 | 1) {
    if (pendingRequests.length < 2) return;
    selected = (selected + direction + pendingRequests.length) % pendingRequests.length;
  }
  function permissionSummary(value: unknown) {
    if (Array.isArray(value)) return value.map(String).join(', ');
    if (value && typeof value === 'object') {
      const record = value as Record<string, unknown>;
      for (const key of ['scopes', 'scope', 'paths', 'permissions']) {
        if (Array.isArray(record[key])) return record[key].map(String).join(', ');
        if (typeof record[key] === 'string') return record[key];
      }
      return 'See technical request details.';
    }
    return String(value);
  }
</script>

{#if request}
  <section class="approval-dock" aria-label="Pending approvals" data-request-id={request.id}>
    <p class="sr-only" aria-live="polite">{countLabel}</p>
    <header class="dock-header">
      <span class:high={request.risk === 'high'} class="dock-icon" aria-hidden="true"><ShieldAlert size={17}/></span>
      <div class="heading"><strong>{request.input ? 'Your input is needed' : 'Approval needed'}</strong><span>{request.provider} · {request.tool} · {request.risk} risk</span></div>
      {#if pendingRequests.length > 1}
        <div class="request-switcher" aria-label="Pending approval navigation">
          <button type="button" aria-label="Previous approval" onclick={() => move(-1)}><ChevronLeft size={16}/></button>
          <span aria-label={`Approval ${selected + 1} of ${pendingRequests.length}`}>{selected + 1} of {pendingRequests.length}</span>
          <button type="button" aria-label="Next approval" onclick={() => move(1)}><ChevronRight size={16}/></button>
        </div>
      {/if}
    </header>

    <div class="dock-body">
      <p class="summary">{request.summary || request.tool}</p>
      {#if command}<pre class="command-preview">{command}</pre>{/if}
      {#if reason}<p class="detail">{reason}</p>{:else if plainDetail}<p class="detail">{plainDetail}</p>{/if}
      {#if action?.permissions}<p class="detail">Permissions requested: {permissionSummary(action.permissions)}</p>{/if}
      {#if action?.changes}<p class="detail">Proposed changes are included in the request.</p>{/if}
      {#if action}<details><summary>Technical request details</summary><pre>{JSON.stringify(action, null, 2)}</pre></details>{/if}
      {#if request.input}
        {#key request.id}<HarnessInput input={request.input} formId={inputFormId} hideSubmit={true} disabled={disabled || resolving || !oninput} onsubmit={response => oninput?.(request, response)}/>{/key}
      {/if}
    </div>

    <div class="dock-actions">
      {#if request.input}<button class="approve" type="submit" form={inputFormId} disabled={disabled || resolving || !oninput || !inputSubmitSupported}><Check size={17}/>Submit response</button>
      {:else}<button class="approve" disabled={disabled || resolving || !onresolve} onclick={() => onresolve?.(request, 'approve_once')}><Check size={17}/>Approve once</button>{/if}
      <button class="deny" disabled={disabled || resolving || !onresolve} onclick={() => onresolve?.(request, 'deny')}><X size={17}/>Deny</button>
      {#if resolving}<span class="resolving" role="status">Sending your response…</span>{/if}
    </div>
  </section>
{/if}

<style>
  .approval-dock { flex: none; display: flex; flex-direction: column; box-sizing: border-box; width: min(var(--chat-content-max-width, 900px), calc(100% - 2 * var(--chat-side-padding, 20px))); max-height: min(420px, calc(100% - var(--approval-dock-reserve, 148px)), calc(100dvh - var(--approval-dock-reserve, 148px))); margin: 0 auto 8px; border: 1px solid color-mix(in srgb, var(--accent) 70%, var(--line)); border-radius: 12px; background: color-mix(in srgb, var(--accent) 8%, var(--panel)); box-shadow: 0 -4px 18px color-mix(in srgb, var(--accent) 11%, transparent); color: var(--ink); overflow: hidden; }
  .dock-header { display: flex; align-items: center; gap: 9px; padding: 10px 12px; border-bottom: 1px solid color-mix(in srgb, var(--accent) 25%, var(--line)); }
  .dock-icon { display: grid; place-items: center; color: var(--accent-ink); }.dock-icon.high { color: #b84c44; }
  .heading { min-width: 0; display: grid; gap: 1px; }.heading strong { font-size: calc(13px * var(--interface-font-ratio, 1)); }.heading span { color: var(--muted); font: calc(10px * var(--interface-font-ratio, 1)) var(--mono); text-transform: capitalize; }
  .request-switcher { margin-left: auto; display: flex; align-items: center; gap: 4px; white-space: nowrap; color: var(--muted); font: calc(10px * var(--interface-font-ratio, 1)) var(--mono); }.request-switcher button { display: grid; place-items: center; width: 32px; height: 32px; border: 1px solid var(--line); border-radius: 6px; background: var(--panel); color: var(--ink); }
  .dock-body { flex: 1 1 auto; min-height: 0; max-height: min(220px, calc(100dvh - var(--approval-dock-reserve, 148px) - 108px)); padding: 0 12px; overflow: auto; overscroll-behavior: contain; }.summary { margin: 11px 0 7px; font-weight: 650; line-height: 1.35; }.detail { margin: 8px 0; color: var(--muted); white-space: pre-wrap; overflow-wrap: anywhere; line-height: 1.45; }.command-preview, details pre { margin: 8px 0; padding: 8px; border: 1px solid var(--line); border-radius: 6px; background: var(--bg); white-space: pre-wrap; overflow-wrap: anywhere; font: 12px/1.45 var(--mono); }.command-preview { max-height: 110px; overflow: auto; } details { margin: 10px 0; color: var(--muted); font-size: 11px; } details pre { max-height: 180px; overflow: auto; }
  .dock-actions { display: flex; align-items: center; flex-wrap: wrap; gap: 8px; padding: 10px 12px; border-top: 1px solid color-mix(in srgb, var(--accent) 25%, var(--line)); background: color-mix(in srgb, var(--accent) 6%, var(--panel)); }.dock-actions button { display: inline-flex; align-items: center; justify-content: center; gap: 6px; min-height: 44px; padding: 9px 13px; border: 1px solid var(--line); border-radius: 7px; font: 650 calc(12px * var(--interface-font-ratio, 1)) var(--interface-font, "IBM Plex Sans", sans-serif); }.approve { border-color: var(--accent) !important; background: var(--accent); color: var(--on-accent); }.deny { color: #a63f38; background: var(--panel); }.dock-actions button:disabled { opacity: .55; cursor: wait; }.resolving { color: var(--muted); font: calc(10px * var(--interface-font-ratio, 1)) var(--mono); }
  .sr-only { position: absolute; width: 1px; height: 1px; padding: 0; margin: -1px; overflow: hidden; clip: rect(0, 0, 0, 0); white-space: nowrap; border: 0; }
  @media (max-width: 560px) { .approval-dock { width: min(var(--chat-content-max-width, 900px), calc(100% - 2 * var(--chat-side-padding, 12px))); margin-bottom: 6px; }.dock-header { padding-inline: 10px; }.heading span { font-size: 9px; }.dock-actions { padding: 8px 10px; }.dock-actions button { flex: 1; }.request-switcher button { width: 30px; height: 30px; } }
  @media (forced-colors: active) { .approval-dock { border-color: Highlight; }.approve { color: HighlightText; background: Highlight; } }
</style>
