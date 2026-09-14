<script lang="ts">
  import { ChevronDown } from '@lucide/svelte';
  import { onMount, tick } from 'svelte';

  let { text }: { text: string } = $props();
  let body = $state<HTMLParagraphElement>();
  let expanded = $state(false);
  let overflowing = $state(false);

  function measure() {
    if (!body || expanded) return;
    overflowing = body.scrollHeight > body.clientHeight + 1;
  }

  function toggle() {
    expanded = !expanded;
    if (!expanded) void tick().then(measure);
  }

  $effect(() => {
    text;
    expanded = false;
    void tick().then(measure);
  });

  onMount(() => {
    const observer = new ResizeObserver(measure);
    if (body) observer.observe(body);
    measure();
    return () => observer.disconnect();
  });
</script>

<aside class="request" class:expanded aria-label="Current user request">
  <div class="request-heading">Your request</div>
  <div class="request-content">
    <p bind:this={body}>{text}</p>
    {#if overflowing || expanded}
      <button type="button" aria-expanded={expanded} aria-label={expanded ? 'Collapse current request' : 'Expand current request'} onclick={toggle}>
        <span>{expanded ? 'Collapse' : 'Expand'}</span><ChevronDown size={14} aria-hidden="true" />
      </button>
    {/if}
  </div>
</aside>

<style>
  .request { width:100%; padding:9px 11px 10px 13px; border:1px solid color-mix(in srgb,var(--accent) 22%,var(--line)); border-radius:10px 10px 3px 10px; color:var(--ink); background:color-mix(in srgb,var(--paper) 94%,var(--accent) 6%); box-shadow:0 8px 24px #0002,0 2px 7px #00000012; }
  .request-heading { margin-bottom:2px; color:var(--muted); font:600 calc(9px * var(--interface-font-ratio,1))/1.35 var(--mono); letter-spacing:.05em; text-transform:uppercase; }
  .request-content { display:flex; align-items:flex-end; gap:9px; min-width:0; }
  p { display:-webkit-box; min-width:0; flex:1; margin:0; overflow:hidden; overflow-wrap:anywhere; white-space:pre-wrap; -webkit-box-orient:vertical; -webkit-line-clamp:2; line-clamp:2; font:var(--chat-font-size,13px)/var(--chat-line-height,1.65) var(--chat-font,"IBM Plex Sans",system-ui,sans-serif); }
  .expanded p { display:block; max-height:min(42vh,420px); overflow:auto; scrollbar-width:thin; -webkit-line-clamp:unset; line-clamp:unset; }
  button { display:inline-flex; flex:none; align-items:center; gap:3px; min-height:25px; margin:-2px -3px -3px 0; padding:2px 5px; border:0; border-radius:5px; color:var(--accent-ink); background:transparent; font:600 calc(10px * var(--interface-font-ratio,1))/1 var(--interface-font,"IBM Plex Sans",sans-serif); cursor:pointer; }
  button:hover { background:var(--soft); }
  button:focus-visible { outline:2px solid var(--accent); outline-offset:1px; }
  button :global(svg) { transition:transform .14s ease; }
  .expanded button :global(svg) { transform:rotate(180deg); }
  @media (prefers-reduced-motion:reduce) { button :global(svg) { transition:none; } }
</style>
