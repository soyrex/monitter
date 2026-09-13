<script lang="ts">
  import { LoaderCircle, RotateCcw, ShieldCheck } from '@lucide/svelte';
  import type { Agent, ApprovalRule, Host } from '$lib/types';

  let { rules, agents, hosts, agentId, hostId, cwd, onrevoke, revokingId = null, compact = false }: {
    rules: ApprovalRule[];
    agents: Agent[];
    hosts: Host[];
    agentId?: string;
    hostId?: string;
    cwd?: string;
    onrevoke?: (rule: ApprovalRule) => void;
    revokingId?: string | null;
    compact?: boolean;
  } = $props();

  const visibleRules = $derived(rules.filter(rule =>
    (agentId === undefined || rule.agentId === agentId) &&
    (hostId === undefined || rule.hostId === hostId) &&
    (cwd === undefined || rule.cwd === cwd),
  ));
  const agentName = (id: string) => agents.find(agent => agent.id === id)?.name ?? 'Unknown agent';
  const hostName = (id: string) => hosts.find(host => host.id === id)?.name ?? 'Unknown host';
  function when(value: number | null) {
    if (!value) return 'Never used';
    return `Used ${new Intl.DateTimeFormat(undefined, { dateStyle: 'medium', timeStyle: 'short' }).format(value)}`;
  }
</script>

<div class:compact class="saved-rules" aria-label="Saved approval rules">
  {#each visibleRules as rule (rule.id)}
    <article class="saved-rule" data-approval-rule-id={rule.id}>
      <span class="rule-icon" aria-hidden="true"><ShieldCheck size={15}/></span>
      <div class="rule-copy">
        <strong>{rule.summary || rule.tool}</strong>
        <span title={`${agentName(rule.agentId)} · ${hostName(rule.hostId)} · ${rule.provider}`}>{agentName(rule.agentId)} · {hostName(rule.hostId)} · {rule.provider}</span>
        <small title={rule.cwd}>Folder: {rule.cwd || 'Unknown folder'} · {rule.scopeDescription || rule.tool} · {when(rule.lastUsedAt)}{#if rule.useCount > 0} · {rule.useCount} use{rule.useCount === 1 ? '' : 's'}{/if}</small>
        {#if rule.detail}<details><summary>Action details</summary><pre>{rule.detail}</pre></details>{/if}
      </div>
      <button type="button" class="revoke" disabled={!onrevoke || revokingId !== null} onclick={() => onrevoke?.(rule)} aria-label={`Revoke saved approval for ${rule.summary || rule.tool}`}>
        {#if revokingId === rule.id}<LoaderCircle class="spin" size={14}/>Revoking…{:else}<RotateCcw size={14}/>Revoke{/if}
      </button>
    </article>
  {:else}
    <p class="rules-empty">No saved approvals for this scope.</p>
  {/each}
</div>

<style>
  .saved-rules { display: grid; gap: 8px; }
  .saved-rule { display:flex; align-items:start; gap:9px; padding:10px; border:1px solid var(--line); border-radius:8px; background:color-mix(in srgb, var(--accent) 4%, var(--panel)); }
  .rule-icon { display:grid; place-items:center; margin-top:2px; color:var(--accent-ink); }
  .rule-copy { min-width:0; flex:1; display:grid; gap:2px; }
  .rule-copy strong { overflow:hidden; text-overflow:ellipsis; white-space:nowrap; font-size:calc(12px * var(--interface-font-ratio, 1)); }
  .rule-copy span, .rule-copy small { overflow:hidden; text-overflow:ellipsis; white-space:nowrap; color:var(--muted); font:calc(10px * var(--interface-font-ratio, 1)) var(--mono); }.rule-copy details { color:var(--muted); font-size:calc(10px * var(--interface-font-ratio, 1)); }.rule-copy summary { cursor:pointer; width:max-content; }.rule-copy pre { max-height:160px; margin:5px 0 0; padding:7px; overflow:auto; border:1px solid var(--line); border-radius:5px; background:var(--bg); white-space:pre-wrap; overflow-wrap:anywhere; font:calc(10px * var(--interface-font-ratio, 1)) / 1.4 var(--mono); }
  .revoke { flex:none; display:inline-flex; align-items:center; gap:4px; min-height:44px; padding:6px 8px; border:1px solid var(--line); border-radius:6px; background:var(--panel); color:var(--ink); font:600 calc(10px * var(--interface-font-ratio, 1)) var(--interface-font, sans-serif); }
  .revoke:disabled { opacity:.55; cursor:wait; }.rules-empty { margin:0; color:var(--muted); font-size:calc(12px * var(--interface-font-ratio, 1)); }
  .compact .saved-rule { padding:8px; }
  @media (max-width:560px) { .saved-rule { align-items:center; }.rule-copy span, .rule-copy small { white-space:normal; } }
</style>
