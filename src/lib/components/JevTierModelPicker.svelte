<script lang="ts">
  import RefreshCw from '@lucide/svelte/icons/refresh-cw';
  import { getBridge } from '$lib/bridge';
  import type { Agent, JevModelTiers, ModelCatalog } from '$lib/types';

  let { draft, saved, disabled = false, onchange }: {
    draft: Agent;
    saved: Agent | null;
    disabled?: boolean;
    onchange: (tier: keyof JevModelTiers, modelId: string) => void;
  } = $props();

  const tiers = [
    { id: 'fast', label: 'Fast model' },
    { id: 'balanced', label: 'Balanced model' },
    { id: 'strong', label: 'Strong model' },
    { id: 'frontier', label: 'Frontier model' },
  ] as const;
  const bridge = getBridge();
  let catalog = $state<ModelCatalog | null>(null);
  let loading = $state(false);
  let error = $state('');
  let revision = 0;
  let observedKey = '';

  const launchKey = (agent: Agent) => JSON.stringify({
    provider: agent.provider,
    hostId: agent.hostId,
    cwd: agent.cwd,
    codexHome: agent.codexHome ?? null,
    acp: agent.acp ?? null,
  });
  const persisted = $derived(Boolean(draft.id && saved && draft.id === saved.id && launchKey(draft) === launchKey(saved)));
  const configurationKey = $derived(`${draft.id}:${persisted ? launchKey(draft) : ''}`);

  $effect(() => {
    const key = configurationKey;
    if (key === observedKey) return;
    observedKey = key;
    revision += 1;
    catalog = null;
    error = '';
    if (persisted) void refresh(false, key);
  });

  async function refresh(force = false, expectedKey = configurationKey) {
    if (!persisted) return;
    const version = ++revision;
    loading = true;
    error = '';
    try {
      const result = await bridge.getModelCatalog({ agentId: draft.id, ...(force ? { refresh: true } : {}) });
      if (version === revision && expectedKey === configurationKey) catalog = result;
    } catch (reason) {
      if (version === revision && expectedKey === configurationKey) error = reason instanceof Error ? reason.message : String(reason);
    } finally {
      if (version === revision) loading = false;
    }
  }

  const tierValue = (tier: keyof JevModelTiers) => draft.jevModelTiers?.[tier] ?? '';
  const advertised = (value: string) => catalog?.models.some(model => model.id === value) ?? false;
</script>

<div class="jev-tier-picker">
  <div class="picker-heading"><span>Model mappings</span><button type="button" aria-label="Refresh available models" title="Refresh available models" disabled={disabled || !persisted || loading} onclick={() => refresh(true)}><RefreshCw size={14}/></button></div>
  {#if !persisted}<p class="status">Save this agent’s harness, host, folder, account, or ACP launch settings to load its available models.</p>{/if}
  {#if loading && !catalog}<p class="status" role="status">Reading harness models…</p>{/if}
  {#if error}<p class="error" role="alert">Could not load models: {error}</p>{/if}
  {#if catalog?.warning}<p class="status">{catalog.warning}</p>{/if}
  <div class="tier-grid">
    {#each tiers as tier (tier.id)}
      {@const value = tierValue(tier.id)}
      <label>{tier.label}
        {#if catalog && catalog.models.length}
          <select value={value} disabled={disabled} onchange={(event) => onchange(tier.id, event.currentTarget.value)}>
            <option value="">Use normal model</option>
            {#if value && !advertised(value)}<option value={value}>{value} (not in current catalog)</option>{/if}
            {#each catalog.models as model (model.id)}<option value={model.id}>{model.name} · {model.id}</option>{/each}
          </select>
          {#if value && !advertised(value)}<small class="warning">Choose an advertised model to replace this unverified ID.</small>{/if}
        {:else if catalog && !catalog.models.length}
          <input value={value} disabled={disabled} oninput={(event) => onchange(tier.id, event.currentTarget.value)} placeholder="Use normal model" />
        {:else}
          <select disabled><option>{value || 'Use normal model'}</option></select>
        {/if}
      </label>
    {/each}
  </div>
  {#if catalog && !catalog.models.length}<p class="status">This harness did not advertise models. Enter only exact model IDs it supports, or leave a tier blank to use its normal model.</p>{/if}
</div>

<style>
  .jev-tier-picker{display:grid;gap:7px;min-width:0}
  .picker-heading{display:flex;align-items:center;justify-content:space-between;gap:8px;color:var(--muted);font-size:calc(11px * var(--interface-font-ratio,1))}
  .picker-heading button{display:grid;place-items:center;width:25px;height:25px;border-radius:5px;color:var(--muted)}
  .picker-heading button:hover:not(:disabled){background:var(--soft)}
  .picker-heading button:disabled{opacity:.5}
  .tier-grid{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:9px 15px}
  .tier-grid label{display:grid;gap:5px;min-width:0;color:var(--muted);font-size:calc(11px * var(--interface-font-ratio,1))}
  .tier-grid select,.tier-grid input{width:100%;min-width:0;min-height:35px;padding:6px 9px;border:1px solid var(--line);border-radius:6px;background:var(--field,var(--panel));color:var(--ink);font:inherit}
  .tier-grid select:disabled,.tier-grid input:disabled{opacity:.6}
  .status,.error,.warning{margin:0;color:var(--muted);font-size:calc(11px * var(--interface-font-ratio,1));line-height:1.4}
  .error,.warning{color:var(--danger,#c44c79)}
  @media(max-width:560px){.tier-grid{grid-template-columns:minmax(0,1fr)}}
</style>
