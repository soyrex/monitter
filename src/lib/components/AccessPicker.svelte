<script lang="ts">
  import {Check,ChevronDown,LoaderCircle,Shield,X} from '@lucide/svelte';
  import {floating} from '$lib/floating';
  import type {Provider,Sandbox} from '$lib/types';
  import {
    capabilityFor,
    denormalizePermission,
    isLevelSupported,
    normalizeStoredSandbox,
    NORMALIZED_PERMISSIONS,
    permissionDescription,
    friendlyPermissionLabel,
    type NormalizedPermission,
  } from '$lib/agent-permissions';

  let {provider,sandbox,disabled=false,appliesNextTurn=false,onchange}:{
    provider:Provider;
    sandbox:Sandbox;
    disabled?:boolean;
    appliesNextTurn?:boolean;
    onchange:(sandbox:Sandbox)=>Promise<void>|void;
  } = $props();

  // The per-chat picker shows the same three-level vocabulary as the agent
  // editor so users never have to learn harness-specific names. The
  // stored `Sandbox` value is mapped on save through the same translation
  // layer used by the agent editor.
  const choices = $derived.by(() => {
    const agentStub = { provider, sandbox, acp: null } as Parameters<typeof normalizeStoredSandbox>[0];
    return NORMALIZED_PERMISSIONS.map(level => ({
      level,
      label: friendlyPermissionLabel(level),
      description: permissionDescription(level),
      supported: isLevelSupported(agentStub, level),
    }));
  });
  const selectedLevel = $derived(normalizeStoredSandbox({ provider, sandbox, acp: null } as Parameters<typeof normalizeStoredSandbox>[0]));
  const value = $derived(choices.find(choice => choice.level === selectedLevel) ?? choices[0]);
  const caps = $derived(capabilityFor({ provider, acp: null }));
  const title = $derived(disabled
    ? 'Permissions can be changed when this run finishes.'
    : appliesNextTurn
      ? 'Applies to the next turn.'
      : provider === 'codex'
        ? 'Permissions for this chat'
        : 'Permissions for this chat');

  let open = $state(false), saving = $state(false), error = $state('');
  let anchor = $state<HTMLButtonElement>(), root = $state<HTMLDivElement>(), panel = $state<HTMLDivElement>();

  async function choose(next: NormalizedPermission) {
    if (saving || disabled || next === selectedLevel) return;
    saving = true; error = '';
    try {
      await onchange(denormalizePermission(provider, next));
      open = false;
      anchor?.focus();
    }
    catch (reason) { error = reason instanceof Error ? reason.message : String(reason); }
    finally { saving = false; }
  }
  function close() { open = false; anchor?.focus(); }
  function outside(event: PointerEvent) { if (open && event.target instanceof Node && !root?.contains(event.target)) close(); }
  function keys(event: KeyboardEvent) {
    if (!open) return;
    if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); close(); return; }
    if (event.key === 'Tab' && panel) {
      const items = Array.from(panel.querySelectorAll<HTMLElement>('button:not(:disabled),[tabindex="0"]'));
      const first = items[0], last = items.at(-1);
      if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last?.focus(); }
      else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first?.focus(); }
    }
  }
</script>

<svelte:window onpointerdown={outside} onkeydown={keys}/>
<div class="access-picker" bind:this={root}>
  <button class="access-trigger" bind:this={anchor} aria-label={`Permissions: ${value.label}`} aria-haspopup="dialog" aria-expanded={open} title={title} onclick={() => { open = !open; }}>
    {#if saving}<LoaderCircle size={14} class="spin"/>{:else}<Shield size={14}/>{/if}
    <span>{value.label}</span>
    <ChevronDown size={12}/>
  </button>
  {#if open && anchor}
    <div bind:this={panel} class="access-menu" role="dialog" aria-label="Permissions" tabindex="-1" use:floating={{anchor,side:'above'}}>
      <header><strong>Permissions</strong><button aria-label="Close permission picker" onclick={close}><X size={14}/></button></header>
      {#if disabled}<p class="explanation">Available after this run finishes.</p>{:else if appliesNextTurn}<p class="explanation">Applies to the next turn.</p>{/if}
      {#if error}<p class="error" role="alert">{error}</p>{/if}
      <div class="access-list" role="radiogroup" aria-label="Permission options">
        {#each choices as choice (choice.level)}
          <button
            class:chosen={selectedLevel === choice.level}
            role="radio"
            aria-checked={selectedLevel === choice.level}
            disabled={disabled || saving || !choice.supported}
            aria-disabled={!choice.supported || undefined}
            title={!choice.supported ? (caps.hint || undefined) : undefined}
            onclick={() => void choose(choice.level)}
          >
            <span><b>{choice.label}</b><small>{choice.description}</small>{#if !choice.supported}<small class="disabled-note">{caps.hint}</small>{/if}</span>
            {#if selectedLevel === choice.level}<Check size={14}/>{/if}
          </button>
        {/each}
      </div>
      <p class="explanation">{caps.hint}</p>
    </div>
  {/if}
</div>

<style>
  .access-picker{position:relative;min-width:0}.access-trigger{display:flex;align-items:center;gap:6px;min-width:0;max-width:220px;padding:5px 7px;border-radius:6px;color:var(--muted);font-size:calc(11px * var(--interface-font-ratio, 1))}.access-trigger:hover{background:var(--soft);color:var(--ink)}.access-trigger span{overflow:hidden;text-overflow:ellipsis;white-space:nowrap}.access-trigger :global(svg){flex:none}
  .access-menu{position:fixed;inset:auto;margin:0;padding:12px;width:300px;max-height:min(420px,80vh);overflow:auto;border:1px solid var(--line);border-radius:12px;background:var(--panel);color:var(--ink);box-shadow:0 12px 40px #0004;font-family:inherit;font-size:calc(12px * var(--interface-font-ratio, 1))}.access-menu header{display:flex;align-items:center;gap:4px;margin-bottom:10px}.access-menu header strong{flex:1;font-weight:500}.access-menu header button{display:grid;place-items:center;width:25px;height:25px;border-radius:5px;color:var(--muted)}.access-menu button:hover{background:var(--soft)}
  .access-list{display:grid;gap:2px}.access-list button{display:flex;align-items:center;gap:10px;width:100%;padding:9px;border-radius:7px;text-align:left}.access-list button[aria-disabled="true"]{opacity:.55;cursor:not-allowed}.access-list button>span{display:grid;gap:4px;flex:1;min-width:0}.access-list b{font-size:calc(12px * var(--interface-font-ratio, 1));font-weight:500}.access-list small,.explanation,.error{display:block;color:var(--muted);font-size:calc(11px * var(--interface-font-ratio, 1));line-height:1.4}.access-list .disabled-note{font-style:italic}.access-list .chosen{background:var(--soft)}.access-list :global(svg){color:var(--accent-ink);flex-shrink:0}.explanation,.error{line-height:1.5;margin:5px 0 0}.error{color:var(--danger,#c44c79)}:global(.spin){animation:spin .8s linear infinite}@keyframes spin{to{transform:rotate(360deg)}}@media(prefers-reduced-motion:reduce){:global(.spin){animation:none}}
  @container (width < 520px){.access-trigger{width:28px;height:28px;padding:0;justify-content:center}.access-trigger span,.access-trigger :global(svg:last-child){display:none}}
</style>
