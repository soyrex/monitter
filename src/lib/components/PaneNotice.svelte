<script lang="ts">
  import {Check, CircleAlert, X} from '@lucide/svelte';
  let {message,blocking=false,ondismiss}: {message:string;blocking?:boolean;ondismiss:()=>void}=$props();
  let hovered=$state(false),focused=$state(false),fading=$state(false);
  $effect(()=>{
    const current=message;
    fading=false;
    if(blocking||hovered||focused||!current)return;
    let removal:ReturnType<typeof setTimeout>|undefined;
    const timer=setTimeout(()=>{fading=true;removal=setTimeout(()=>{if(message===current)ondismiss();},250);},4000);
    return()=>{clearTimeout(timer);clearTimeout(removal);};
  });
</script>
<div class="alert" class:error={blocking} class:notice={!blocking} class:fading role={blocking?'alert':'status'} onpointerenter={()=>hovered=true} onpointerleave={()=>hovered=false} onfocusin={()=>focused=true} onfocusout={event=>{if(!event.currentTarget.contains(event.relatedTarget as Node|null))focused=false;}}>
  <span class="icon-slot" aria-hidden="true">{#if blocking}<CircleAlert size={16}/>{:else}<Check size={16}/>{/if}</span>
  <span class="notice-text">{message}</span>
  <button class="close-slot" aria-label={blocking?'Dismiss error':'Dismiss notice'} onclick={ondismiss}><X size={15}/></button>
</div>
<style>
  .alert { position:absolute;z-index:50;top:calc(var(--pane-tabbar-height,52px) + 12px);left:50%;transform:translateX(-50%);display:grid;grid-template-columns:28px minmax(0,1fr) 28px;align-items:center;gap:8px;width:max-content;max-width:calc(100% - 40px);box-sizing:border-box;padding:7px 8px;border:1px solid var(--line);border-radius:7px;background:var(--panel);color:var(--accent-ink);box-shadow:0 3px 8px #0002,0 10px 28px #0003;font-size:calc(12px * var(--interface-font-ratio,1));transition:opacity 250ms ease,translate 250ms ease; }
  .icon-slot,.close-slot { width:28px;height:28px;display:grid;place-items:center;padding:0;box-sizing:border-box; }
  .notice-text { min-width:0;overflow-wrap:anywhere; }
  .close-slot { border:0;background:transparent;color:var(--muted);border-radius:5px;opacity:0;transition:opacity 120ms;cursor:pointer; }
  .alert:focus-within .close-slot { opacity:1; }
  @media (hover:hover) and (pointer:fine) { .alert:hover .close-slot { opacity:1; } }
  @media (hover:none), (pointer:coarse) { .close-slot { opacity:1; } }
  .close-slot:hover { background:var(--soft); }
  .error { border-color:color-mix(in srgb,#bd4c43 45%,var(--line));color:#b84c44; }
  .fading { opacity:0;translate:0 -4px;pointer-events:none; }
  @media(hover:none) { .close-slot { opacity:1; } }
  @media(prefers-reduced-motion:reduce) { .alert,.close-slot { transition:none; } }
</style>
