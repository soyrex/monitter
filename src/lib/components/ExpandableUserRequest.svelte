<script lang="ts">
  import { ChevronDown } from '@lucide/svelte';
  import { onMount, tick } from 'svelte';
  import Markdown from './Markdown.svelte';

  let { text }: { text: string } = $props();
  let body = $state<HTMLDivElement>();
  let expanded = $state(false);
  let overflowing = $state(false);

  function measure() {
    const markdown = body?.querySelector<HTMLElement>('.markdown');
    if (!markdown || expanded) return;
    overflowing = markdown.scrollHeight > markdown.clientHeight + 1;
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

<div class="request-text" class:expanded bind:this={body}>
  <Markdown {text} />
  {#if overflowing || expanded}
    <button type="button" aria-expanded={expanded} aria-label={expanded ? 'Collapse current request' : 'Expand current request'} onclick={toggle}>
      <span>{expanded ? 'Collapse' : 'Expand'}</span><ChevronDown size={14} aria-hidden="true" />
    </button>
  {/if}
</div>

<style>
  .request-text :global(.markdown) { display:-webkit-box; overflow:hidden; overflow-wrap:anywhere; -webkit-box-orient:vertical; -webkit-line-clamp:2; line-clamp:2; }
  .request-text.expanded :global(.markdown) { display:block; max-height:min(42vh,420px); overflow:auto; scrollbar-width:thin; -webkit-line-clamp:unset; line-clamp:unset; }
  button { display:inline-flex; align-items:center; gap:3px; min-height:25px; margin:5px -3px -3px auto; padding:2px 5px; border:0; border-radius:5px; color:var(--accent-ink); background:transparent; font:600 calc(10px * var(--interface-font-ratio,1))/1 var(--interface-font,"IBM Plex Sans",sans-serif); cursor:pointer; }
  button:hover { background:var(--panel); }
  button:focus-visible { outline:2px solid var(--accent); outline-offset:1px; }
  button :global(svg) { transition:transform .14s ease; }
  .expanded button :global(svg) { transform:rotate(180deg); }
  @media (max-height:500px) {
    .request-text :global(.markdown) { display:block; overflow:visible; -webkit-line-clamp:unset; line-clamp:unset; }
    button { display:none; }
  }
  @media (prefers-reduced-motion:reduce) { button :global(svg) { transition:none; } }
</style>
