<script lang="ts">
  import { onDestroy } from 'svelte';
  import { Eye } from '@lucide/svelte';
  import { floating } from '$lib/floating';

  let { names }: { names: string[] } = $props();
  const observers = $derived([...new Set(names.map(name => name.trim()).filter(Boolean))]);
  const tooltipId = $props.id();
  let anchor = $state<HTMLButtonElement>();
  let hovered = $state(false), focused = $state(false), dismissed = $state(false);
  let leaveTimer: ReturnType<typeof setTimeout> | undefined;
  const visible = $derived(observers.length > 0 && !dismissed && (hovered || focused));
  function enter() {
    clearTimeout(leaveTimer);
    hovered = true;
    dismissed = false;
  }
  function leave() {
    clearTimeout(leaveTimer);
    leaveTimer = setTimeout(() => hovered = false, 120);
  }
  $effect(() => {
    if (!observers.length) {
      clearTimeout(leaveTimer);
      hovered = false;
      focused = false;
      dismissed = false;
    }
  });
  onDestroy(() => clearTimeout(leaveTimer));
</script>

{#if observers.length}
  <span class="observer-indicator">
    <button bind:this={anchor} type="button" class="observer-eye" aria-label={`Observers: ${observers.join(', ')}`} aria-describedby={visible ? tooltipId : undefined}
      onpointerenter={enter} onpointerleave={leave}
      onfocus={()=>{focused=true;dismissed=false;}} onblur={()=>focused=false}
      onclick={()=>dismissed=false} onkeydown={event=>{if(event.key==='Escape'){dismissed=true;event.stopPropagation();}}}>
      <Eye size={15} aria-hidden="true"/>
      <span class="sparkle first" aria-hidden="true">✦</span>
      <span class="sparkle second" aria-hidden="true">✦</span>
      <span class="sparkle third" aria-hidden="true">✦</span>
    </button>
    {#if visible && anchor}
      <div id={tooltipId} role="tooltip" class="observer-tooltip" use:floating={{anchor,focus:false}} onpointerenter={enter} onpointerleave={leave}>
        <strong>Watching this chat</strong>
        <ul>{#each observers as name (name)}<li>{name}</li>{/each}</ul>
      </div>
    {/if}
  </span>
{/if}

<style>
  .observer-indicator{display:inline-flex;align-items:center;flex:none;vertical-align:middle}
  .observer-eye{position:relative;display:inline-grid;place-items:center;width:25px;height:25px;padding:0;border:0;border-radius:6px;background:transparent;color:var(--accent-ink,var(--accent));cursor:help}
  .observer-eye:hover,.observer-eye:focus-visible{background:color-mix(in srgb,var(--accent) 12%,transparent)}
  .observer-eye:focus-visible{outline:2px solid var(--accent);outline-offset:2px}
  .sparkle{position:absolute;line-height:1;pointer-events:none;text-shadow:0 0 5px var(--accent);animation:observer-twinkle 1.8s ease-in-out infinite}
  .first{top:0;right:0;font-size:9px}
  .second{bottom:0;left:0;font-size:7px;animation-delay:-.6s}
  .third{top:0;left:2px;font-size:5px;animation-delay:-1.2s}
  .observer-tooltip{position:fixed;inset:auto;margin:0;padding:10px 12px;min-width:145px;max-width:260px;border:1px solid var(--line);border-radius:8px;background:var(--panel);color:var(--ink);box-shadow:0 8px 24px #0004;font:12px/1.5 var(--interface-font,sans-serif);overflow-wrap:anywhere}
  .observer-tooltip strong{font-weight:500;color:var(--muted);font-size:11px}
  .observer-tooltip ul{list-style:none;margin:5px 0 0;padding:0}
  .observer-tooltip li+li{margin-top:3px}
  @keyframes observer-twinkle{0%,100%{opacity:.25;transform:scale(.65) rotate(-12deg)}50%{opacity:1;transform:scale(1) rotate(12deg)}}
  @media(prefers-reduced-motion:reduce){.sparkle{animation:none;opacity:.75}}
</style>
