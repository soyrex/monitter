<script lang="ts">
  import { Brain, Check, ChevronDown, LoaderCircle, RefreshCw, X } from '@lucide/svelte';
  import { getBridge } from '$lib/bridge';
  import { floating } from '$lib/floating';
  import type { Agent, HarnessModel, ModelCatalog } from '$lib/types';

  let { draft, saved, disabled = false, onchange }: {
    draft: Agent;
    /** The persisted configuration the native catalog command will actually use. */
    saved: Agent | null;
    disabled?: boolean;
    onchange: (model: string) => void;
  } = $props();

  const bridge = getBridge();
  let catalog = $state<ModelCatalog | null>(null);
  let loading = $state(false);
  let error = $state('');
  let open = $state(false);
  let query = $state('');
  let customModel = $state('');
  let anchor = $state<HTMLButtonElement>();
  let root = $state<HTMLDivElement>();
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
  const selected = $derived(catalog?.models.find(model => model.id === draft.model));
  const label = $derived(selected?.name || draft.model || 'Harness default');
  const visibleModels = $derived((catalog?.models ?? []).filter(model => `${model.name} ${model.id} ${model.description}`.toLowerCase().includes(query.toLowerCase())));

  $effect(() => {
    const key = configurationKey;
    if (key === observedKey) return;
    observedKey = key;
    revision += 1;
    catalog = null;
    error = '';
    open = false;
    query = '';
    customModel = draft.model;
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

  function choose(model: HarnessModel | null) {
    if (disabled) return;
    onchange(model?.id ?? '');
    open = false;
    anchor?.focus();
  }
  function chooseCustom() {
    const model = customModel.trim();
    if (!model || disabled) return;
    onchange(model);
    open = false;
    anchor?.focus();
  }
  function close() { open = false; anchor?.focus(); }
  function outside(event: PointerEvent) { if (open && event.target instanceof Node && !root?.contains(event.target)) close(); }
  function keys(event: KeyboardEvent) { if (open && event.key === 'Escape') { event.preventDefault(); close(); } }
</script>

<svelte:window onpointerdown={outside} onkeydown={keys}/>
<div class="agent-model-picker" bind:this={root}>
  <button class="model-trigger" bind:this={anchor} aria-label={`Model: ${label}`} aria-haspopup="dialog" aria-expanded={open} disabled={disabled || !persisted} title={!persisted ? 'Save the harness, host, folder, account, or ACP launch changes before choosing a model' : 'Choose the model for new chats'} onclick={() => { open = !open; query = ''; }}>
    {#if loading}<LoaderCircle size={14} class="spin"/>{:else}<Brain size={15}/>{/if}<span>{label}</span><ChevronDown size={13}/>
  </button>
  {#if !persisted}<small class="status">Save this agent’s runtime settings to load its available models.</small>{/if}
  {#if open && anchor}<div class="model-menu" role="dialog" aria-label="Choose agent model" tabindex="-1" use:floating={{anchor,side:'below'}}>
    <header><strong>Model for new chats</strong><button aria-label="Refresh models" disabled={loading} onclick={() => refresh(true)}><RefreshCw size={14}/></button><button aria-label="Close model selector" onclick={close}><X size={14}/></button></header>
    {#if error}<p class="error" role="alert">{error}</p>{/if}
    {#if loading && !catalog}<p class="status" role="status">Reading harness models…</p>{:else if catalog}
      {#if catalog.warning}<p class="status">{catalog.warning}</p>{/if}
      <input aria-label="Find a model" placeholder="Find a model by name or ID…" bind:value={query}/>
      <div class="model-list" role="radiogroup" aria-label="Available models">
        <button class:chosen={!draft.model} role="radio" aria-checked={!draft.model} disabled={disabled} onclick={() => choose(null)}><span><b>Harness default</b><small>Let this harness choose its configured default model.</small></span>{#if !draft.model}<Check size={14}/>{/if}</button>
        {#if draft.model && !selected}<button class="chosen" role="radio" aria-checked="true" disabled={disabled} onclick={() => { customModel = draft.model; }}><span><b>Current saved model</b><code>{draft.model}</code><small>This ID is not in the harness catalog.</small></span><Check size={14}/></button>{/if}
        {#each visibleModels as model (model.id)}<button class:chosen={draft.model === model.id} role="radio" aria-checked={draft.model === model.id} disabled={disabled} onclick={() => choose(model)}><span><b>{model.name}</b><code>{model.id}</code>{#if model.description}<small>{model.description}</small>{/if}</span>{#if draft.model === model.id}<Check size={14}/>{/if}</button>{/each}
      </div>
      {#if !catalog.models.length}<p class="status">This harness does not provide a discoverable model catalog. You can keep its default or enter the exact model ID it documents.</p>{:else if !visibleModels.length}<p class="status">No models match that search.</p>{/if}
      {#if !catalog.models.length}<form class="custom-model" onsubmit={event => { event.preventDefault(); chooseCustom(); }}><label for="custom-model">Exact model ID</label><div><input id="custom-model" aria-label="Exact model ID" bind:value={customModel} placeholder="provider/model"/><button type="submit" disabled={disabled || !customModel.trim()}>Use ID</button></div></form>{/if}
    {:else if error}
      <button disabled={disabled} onclick={() => choose(null)}>Use harness default</button>
    {/if}
  </div>{/if}
</div>

<style>
  .agent-model-picker{position:relative;min-width:0}.model-trigger{display:flex;align-items:center;gap:6px;width:100%;min-height:33px;padding:7px 9px;border:1px solid var(--line);border-radius:6px;color:var(--ink);text-align:left;background:var(--field,var(--panel))}.model-trigger:disabled{opacity:.6;cursor:not-allowed}.model-trigger span{overflow:hidden;text-overflow:ellipsis;white-space:nowrap;flex:1}.model-trigger :global(svg){flex:none}.status,.error{display:block;margin:5px 0 0;color:var(--muted);font-size:calc(11px * var(--interface-font-ratio,1));line-height:1.4}.error{color:var(--danger,#c44c79)}.model-menu{position:fixed;inset:auto;margin:0;padding:12px;width:350px;max-height:min(520px,80vh);overflow:auto;border:1px solid var(--line);border-radius:12px;background:var(--panel);color:var(--ink);box-shadow:0 12px 40px #0004;font-family:inherit;font-size:calc(12px * var(--interface-font-ratio,1))}.model-menu header{display:flex;align-items:center;gap:4px;margin-bottom:10px}.model-menu header strong{flex:1;font-weight:500}.model-menu header button{display:grid;place-items:center;width:25px;height:25px;border-radius:5px;color:var(--muted)}.model-menu button:hover{background:var(--soft)}.model-menu input{width:100%;padding:7px;margin-bottom:8px}.model-list{max-height:300px;overflow:auto;overscroll-behavior:contain}.model-list button{display:flex;align-items:center;gap:10px;width:100%;padding:9px;border-radius:7px;text-align:left}.model-list button>span{display:grid;gap:3px;flex:1;min-width:0}.model-list b{font-size:calc(12px * var(--interface-font-ratio,1));font-weight:500}.model-list small{color:var(--muted);font-size:calc(11px * var(--interface-font-ratio,1));line-height:1.35}.model-list code{color:var(--muted);font-size:calc(10px * var(--interface-font-ratio,1));overflow:hidden;text-overflow:ellipsis;white-space:nowrap}.model-list .chosen{background:var(--soft)}.model-list :global(svg){color:var(--accent-ink);flex-shrink:0}.custom-model{border-top:1px solid var(--line);margin-top:10px;padding-top:10px}.custom-model label{display:block;margin-bottom:5px;color:var(--muted);font-size:calc(11px * var(--interface-font-ratio,1))}.custom-model div{display:flex;gap:6px}.custom-model input{margin:0;min-width:0;flex:1}.custom-model button{padding:0 8px;border-radius:6px;background:var(--soft);color:var(--ink);white-space:nowrap}
</style>
