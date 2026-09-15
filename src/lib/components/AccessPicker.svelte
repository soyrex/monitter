<script lang="ts">
  import {Check,ChevronDown,LoaderCircle,Shield,X} from '@lucide/svelte';
  import {floating} from '$lib/floating';
  import type {Provider,Sandbox} from '$lib/types';
  let {provider,sandbox,disabled=false,appliesNextTurn=false,onchange}:{provider:Provider;sandbox:Sandbox;disabled?:boolean;appliesNextTurn?:boolean;onchange:(sandbox:Sandbox)=>Promise<void>|void}=$props();
  const choices=$derived(provider==='codex'
    ? [
        {id:'read-only' as Sandbox,label:'Read only',description:'Can inspect the workspace, but cannot make changes.'},
        {id:'workspace-write' as Sandbox,label:'Workspace write',description:'Can edit files within the selected workspace.'},
        {id:'yolo' as Sandbox,label:'YOLO',description:'Skips permission prompts and sandbox restrictions. Use only when explicitly trusted.'},
      ]
    : provider==='claude' || provider==='acp'
      ? [
          {id:'harness-configured' as Sandbox,label:'Harness permissions',description:'Uses the permission policy configured by Claude Code.'},
          {id:'yolo' as Sandbox,label:'YOLO',description:provider==='acp' ? 'Requests the ACP agent’s advertised bypassPermissions mode. Use only when explicitly trusted.' : 'Skips permission prompts. Use only when explicitly trusted.'},
        ]
      : [{id:'harness-configured' as Sandbox,label:'Harness permissions',description:'Uses the permission policy configured by this harness.'}]);
  const value=$derived(choices.find(choice=>choice.id===sandbox)??choices[0]);
  const title=$derived(disabled ? 'Permissions can be changed when this run finishes.' : appliesNextTurn ? 'Applies to the next turn.' : provider==='codex' ? 'Permissions for this chat' : 'Harness permission policy for this chat');
  let open=$state(false),saving=$state(false),error=$state('');
  let anchor=$state<HTMLButtonElement>(),root=$state<HTMLDivElement>(),panel=$state<HTMLDivElement>();

  async function choose(next:Sandbox) {
    if(saving||disabled||next===value.id)return;
    saving=true;error='';
    try { await onchange(next); open=false; anchor?.focus(); }
    catch(reason) { error=reason instanceof Error ? reason.message : String(reason); }
    finally { saving=false; }
  }
  function close(){open=false;anchor?.focus();}
  function outside(event:PointerEvent){if(open&&event.target instanceof Node&&!root?.contains(event.target))close();}
  function keys(event:KeyboardEvent){
    if(!open)return;
    if(event.key==='Escape'){event.preventDefault();event.stopPropagation();close();return;}
    if(event.key==='Tab'&&panel){
      const items=Array.from(panel.querySelectorAll<HTMLElement>('button:not(:disabled),[tabindex="0"]'));
      const first=items[0],last=items.at(-1);
      if(event.shiftKey&&document.activeElement===first){event.preventDefault();last?.focus();}
      else if(!event.shiftKey&&document.activeElement===last){event.preventDefault();first?.focus();}
    }
  }
</script>

<svelte:window onpointerdown={outside} onkeydown={keys}/>
<div class="access-picker" bind:this={root}>
  <button class="access-trigger" bind:this={anchor} aria-label={`Permissions: ${value.label}`} aria-haspopup="dialog" aria-expanded={open} title={title} onclick={()=>{open=!open;}}>
    {#if saving}<LoaderCircle size={14} class="spin"/>{:else}<Shield size={14}/>{/if}
    <span>{value.label}</span>
    <ChevronDown size={12}/>
  </button>
  {#if open&&anchor}
    <div bind:this={panel} class="access-menu" role="dialog" aria-label="Access permissions" tabindex="-1" use:floating={{anchor,side:'above'}}>
      <header><strong>Access permissions</strong><button aria-label="Close permission picker" onclick={close}><X size={14}/></button></header>
      {#if disabled}<p class="explanation">Available after this run finishes.</p>{:else if appliesNextTurn}<p class="explanation">Applies to the next turn.</p>{/if}
      {#if error}<p class="error" role="alert">{error}</p>{/if}
      <div class="access-list" role="radiogroup" aria-label="Permission options">
        {#each choices as choice (choice.id)}
          <button class:chosen={value.id===choice.id} role="radio" aria-checked={value.id===choice.id} disabled={disabled||saving} onclick={()=>void choose(choice.id)}>
            <span><b>{choice.label}</b><small>{choice.description}</small></span>
            {#if value.id===choice.id}<Check size={14}/>{/if}
          </button>
        {/each}
      </div>
    </div>
  {/if}
</div>

<style>
  .access-picker{position:relative;min-width:0}.access-trigger{display:flex;align-items:center;gap:6px;min-width:0;max-width:220px;padding:5px 7px;border-radius:6px;color:var(--muted);font-size:calc(11px * var(--interface-font-ratio, 1))}.access-trigger:hover{background:var(--soft);color:var(--ink)}.access-trigger span{overflow:hidden;text-overflow:ellipsis;white-space:nowrap}.access-trigger :global(svg){flex:none}
  .access-menu{position:fixed;inset:auto;margin:0;padding:12px;width:300px;max-height:min(420px,80vh);overflow:auto;border:1px solid var(--line);border-radius:12px;background:var(--panel);color:var(--ink);box-shadow:0 12px 40px #0004;font-family:inherit;font-size:calc(12px * var(--interface-font-ratio, 1))}.access-menu header{display:flex;align-items:center;gap:4px;margin-bottom:10px}.access-menu header strong{flex:1;font-weight:500}.access-menu header button{display:grid;place-items:center;width:25px;height:25px;border-radius:5px;color:var(--muted)}.access-menu button:hover{background:var(--soft)}
  .access-list{display:grid;gap:2px}.access-list button{display:flex;align-items:center;gap:10px;width:100%;padding:9px;border-radius:7px;text-align:left}.access-list button>span{display:grid;gap:4px;flex:1;min-width:0}.access-list b{font-size:calc(12px * var(--interface-font-ratio, 1));font-weight:500}.access-list small,.explanation,.error{display:block;color:var(--muted);font-size:calc(11px * var(--interface-font-ratio, 1));line-height:1.4}.access-list .chosen{background:var(--soft)}.access-list :global(svg){color:var(--accent-ink);flex-shrink:0}.explanation,.error{line-height:1.5;margin:5px 0 10px}.error{color:var(--danger,#c44c79)}:global(.spin){animation:spin .8s linear infinite}@keyframes spin{to{transform:rotate(360deg)}}@media(prefers-reduced-motion:reduce){:global(.spin){animation:none}}
  @container (width < 520px){.access-trigger{width:28px;height:28px;padding:0;justify-content:center}.access-trigger span,.access-trigger :global(svg:last-child){display:none}}
</style>
