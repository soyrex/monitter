<script lang="ts">
  import {Brain,Check,ChevronDown,LoaderCircle,RefreshCw,X,Zap} from '@lucide/svelte';
  import {getBridge} from '$lib/bridge';
  import {floating} from '$lib/floating';
  import type {ModelCatalog,ModelSettings,ModelTarget,HarnessModel} from '$lib/types';
  let {target,settings=null,fallbackModel='',disabled=false,onchange}:{target:ModelTarget;settings?:ModelSettings|null;fallbackModel?:string;disabled?:boolean;onchange:(settings:ModelSettings)=>Promise<void>}=$props();
  const bridge=getBridge(), id=$props.id();
  let catalog=$state<ModelCatalog|null>(null),loading=$state(false),saving=$state(false),error=$state(''),open=$state(false),query=$state('');
  let anchor=$state<HTMLButtonElement>(),root=$state<HTMLDivElement>(),panel=$state<HTMLDivElement>();
  let revision=0, observedTarget='';
  const targetKey=$derived(JSON.stringify(target));
  const current=$derived(settings?.model ? {...settings,reasoningEffort:settings.reasoningEffort ?? (catalog?.current.model===settings.model?catalog.current.reasoningEffort:null),fastMode:settings.fastMode ?? (catalog?.current.model===settings.model?catalog.current.fastMode:null)} : catalog?.current??{model:fallbackModel,reasoningEffort:null,fastMode:null});
  const selected=$derived(catalog?.models.find(model=>model.id===current.model));
  const label=$derived(selected?.name || current.model || 'Harness default');
  const efforts=$derived(selected?.reasoningEfforts??[]);
  const effort=$derived(current.reasoningEffort ?? selected?.defaultEffort ?? efforts[0]?.id ?? '');
  const effortIndex=$derived(Math.max(0,efforts.findIndex(level=>level.id===effort)));
  const visibleModels=$derived((catalog?.models??[]).filter(model=>`${model.name} ${model.id} ${model.description}`.toLowerCase().includes(query.toLowerCase())));
  $effect(()=>{const key=targetKey;JSON.stringify(settings);if(key!==observedTarget){observedTarget=key;open=false;query='';catalog=null;}void refresh(key);return()=>{revision+=1;};});
  async function refresh(key=targetKey) {
    const version=++revision;loading=true;error='';
    try{const result=await bridge.getModelCatalog(JSON.parse(key));if(version===revision)catalog=result;}
    catch(reason){if(version===revision)error=reason instanceof Error?reason.message:String(reason);}
    finally{if(version===revision)loading=false;}
  }
  async function save(next:ModelSettings) {
    if(saving||disabled)return;
    const key=targetKey;saving=true;error='';
    try{await onchange(next);if(key===targetKey && catalog){catalog={...catalog,current:next};if(!next.model)await refresh(key);}}
    catch(reason){if(key===targetKey)error=reason instanceof Error?reason.message:String(reason);}
    finally{saving=false;}
  }
  function choose(model:HarnessModel){void save({model:model.id,reasoningEffort:model.id===current.model?current.reasoningEffort:model.defaultEffort,fastMode:model.supportsFast?current.fastMode:null});}
  function close(){open=false;anchor?.focus();}
  function outside(event:PointerEvent){if(open&&event.target instanceof Node&&!root?.contains(event.target))close();}
  function keys(event:KeyboardEvent){
    if(!open)return;
    if(event.key==='Escape'){event.preventDefault();event.stopPropagation();close();return;}
    if(event.key==='Tab'&&panel){const items=Array.from(panel.querySelectorAll<HTMLElement>('button:not(:disabled),input:not(:disabled),[tabindex="0"]'));const first=items[0],last=items.at(-1);if(event.shiftKey&&document.activeElement===first){event.preventDefault();last?.focus();}else if(!event.shiftKey&&document.activeElement===last){event.preventDefault();first?.focus();}}
  }
</script>
<svelte:window onpointerdown={outside} onkeydown={keys}/>
<div class="model-picker" bind:this={root}>
  <button class="model-trigger" bind:this={anchor} aria-label={`Model: ${label}`} aria-haspopup="dialog" aria-expanded={open} title={disabled?'Model can be changed when this run finishes':'Choose model and reasoning effort'} onclick={()=>{open=!open;query='';}}>
    {#if loading||saving}<LoaderCircle size={13} class="spin"/>{:else}<Brain size={14}/>{/if}{#if current.fastMode && selected?.supportsFast}<Zap class="fast-icon" size={11}/>{/if}<span>{label}</span>{#if effort}<small>{effort}</small>{/if}<ChevronDown size={12}/>
  </button>
  {#if open&&anchor}<div bind:this={panel} class="model-menu" role="dialog" aria-label="Model and reasoning" tabindex="-1" use:floating={{anchor,side:'above'}}>
    <header><strong>Model</strong><button aria-label="Refresh models" disabled={loading||saving} onclick={()=>refresh()}><RefreshCw size={14}/></button><button aria-label="Close model picker" onclick={close}><X size={14}/></button></header>
    {#if disabled}<p class="explanation">Available after this run finishes.</p>{/if}
    {#if error}<p class="error" role="alert">{error}</p>{/if}
    {#if loading&&!catalog}<p class="explanation" role="status">Reading harness models…</p>{:else if catalog}
      {#if catalog.warning}<p class="explanation">{catalog.warning}</p>{/if}
      {#if catalog.models.length>5}<input aria-label="Find a model" placeholder="Find a model…" bind:value={query}/>{/if}
      <div class="model-list" role="radiogroup" aria-label="Available models">
        {#each visibleModels as model (model.id)}<button class:chosen={current.model===model.id} role="radio" aria-checked={current.model===model.id} disabled={disabled||saving} onclick={()=>choose(model)}><span><b>{model.name}</b><small>{model.description}</small></span>{#if current.model===model.id}<Check size={14}/>{/if}</button>{/each}
      </div>
      {#if !catalog.models.length}<p class="explanation">This harness does not provide a model catalog.{#if target.taskId && current.model} Clear the saved choice below to recover with its default model.{/if}</p>{/if}
      {#if efforts.length}<div class="effort"><label for="effort-{id}">Reasoning effort <b>{effort}</b></label><input id="effort-{id}" aria-label="Reasoning effort" type="range" min="0" max={Math.max(0,efforts.length-1)} step="1" value={effortIndex} aria-valuetext={effort} disabled={disabled||saving||efforts.length<2} onchange={event=>save({...current,reasoningEffort:efforts[Number(event.currentTarget.value)].id})}/><small>{efforts[effortIndex]?.description}</small></div>{/if}
      {#if selected?.supportsFast}<label class="fast-row"><span><b>Fast mode</b><small>{selected.fastDescription || 'Faster responses with increased usage.'}</small></span><input type="checkbox" role="switch" aria-label="Fast mode" checked={current.fastMode===true} disabled={disabled||saving} onchange={event=>save({...current,fastMode:event.currentTarget.checked})}/></label>{/if}
      {#if target.taskId}<button class="reset" disabled={disabled||saving||!current.model} onclick={()=>save({model:'',reasoningEffort:null,fastMode:null})}>Use harness defaults</button>{/if}
    {/if}
  </div>{/if}
</div>
<style>
  .model-picker{position:relative;min-width:0}.model-trigger{display:flex;align-items:center;gap:6px;color:var(--muted);padding:5px 7px;border-radius:6px;max-width:220px;font-size:calc(11px * var(--interface-font-ratio, 1))}.model-trigger:hover{background:var(--soft);color:var(--ink)}.model-trigger span{overflow:hidden;text-overflow:ellipsis;white-space:nowrap}.model-trigger small{opacity:.5;font-weight:400;white-space:nowrap;font-size:inherit;text-transform:capitalize}.model-trigger :global(svg){flex:none}.model-trigger :global(.fast-icon){margin-left:-5px;color:var(--accent-ink)}
  .model-menu{position:fixed;inset:auto;margin:0;padding:12px;width:330px;max-height:min(520px,80vh);overflow:auto;border:1px solid var(--line);border-radius:12px;background:var(--panel);color:var(--ink);box-shadow:0 12px 40px #0004;font-family:inherit;font-size:calc(12px * var(--interface-font-ratio, 1))}.model-menu header{display:flex;align-items:center;gap:4px;margin-bottom:10px}.model-menu header strong{flex:1;font-weight:500}.model-menu header button{display:grid;place-items:center;width:25px;height:25px;border-radius:5px;color:var(--muted)}.model-menu button:hover{background:var(--soft)}.model-menu input:not([type]){width:100%;padding:7px;margin-bottom:8px}.model-list{max-height:245px;overflow:auto;overscroll-behavior:contain}.model-list button{display:flex;align-items:center;gap:10px;width:100%;padding:9px;border-radius:7px;text-align:left}.model-list button>span{display:grid;gap:4px;flex:1}.model-list b{font-size:calc(12px * var(--interface-font-ratio, 1));font-weight:500}.model-list small,.effort small,.fast-row small{display:block;color:var(--muted);font-size:calc(11px * var(--interface-font-ratio, 1));line-height:1.4}.model-list .chosen{background:var(--soft)}.model-list :global(svg){color:var(--accent-ink);flex-shrink:0}.explanation,.error{font-size:calc(11px * var(--interface-font-ratio, 1));line-height:1.5;margin:5px 0 10px;color:var(--muted)}.error{color:var(--danger,#c44c79)}.effort{border-top:1px solid var(--line);margin-top:10px;padding-top:12px}.effort label{display:flex;justify-content:space-between;gap:8px}.effort label b{text-transform:capitalize;font-weight:500}.effort input{width:100%;margin:12px 0 6px;accent-color:var(--accent)}.fast-row{display:flex;align-items:center;gap:10px;margin-top:14px}.fast-row>span{flex:1}.fast-row b{display:block;font-weight:500;margin-bottom:4px}.fast-row input{appearance:none;flex-shrink:0;width:30px;height:18px;padding:0;border:0;border-radius:12px;background:var(--muted);position:relative;cursor:pointer}.fast-row input:checked{background:var(--accent)}.fast-row input:before{content:"";position:absolute;width:14px;height:14px;left:2px;top:2px;border-radius:50%;background:white;transition:transform .12s}.fast-row input:checked:before{transform:translateX(12px)}.reset{margin-top:12px;color:var(--muted);font-size:calc(11px * var(--interface-font-ratio, 1));text-align:left;padding:5px;border-radius:5px}
  @container (width < 520px){.model-trigger{width:28px;height:28px;padding:0;justify-content:center}.model-trigger span,.model-trigger small,.model-trigger :global(svg:last-child),.model-trigger :global(.fast-icon){display:none}}
</style>
