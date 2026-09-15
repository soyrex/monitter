<script lang="ts">
  import { ChevronDown } from '@lucide/svelte';
  import { onMount, tick } from 'svelte';
  import Markdown from './Markdown.svelte';

  let { text }: { text: string } = $props();
  let body = $state<HTMLDivElement>();
  let expanded = $state(false);
  let overflowing = $state(false);
  let pinned = $state(false);

  function updatePinned() {
    const message = body?.closest<HTMLElement>('.sticky-user-request');
    const viewport = body?.closest<HTMLElement>('.messages');
    const next = Boolean(message && viewport && window.matchMedia('(min-height: 501px)').matches
      && message.getBoundingClientRect().top <= viewport.getBoundingClientRect().top + 10);
    if (next === pinned) return;
    pinned = next;
    expanded = false;
    overflowing = false;
    if (pinned) void tick().then(measure);
  }

  function measure() {
    const markdown = body?.querySelector<HTMLElement>('.markdown');
    if (!pinned || !markdown || expanded) return;
    overflowing = markdown.scrollHeight > markdown.clientHeight + 1;
  }

  function toggle() {
    expanded = !expanded;
    if (!expanded) void tick().then(measure);
  }

  $effect(() => {
    text;
    expanded = false;
    overflowing = false;
    void tick().then(() => { updatePinned(); measure(); });
  });

  onMount(() => {
    const viewport = body?.closest<HTMLElement>('.messages');
    const observer = new ResizeObserver(() => { updatePinned(); measure(); });
    if (body) observer.observe(body);
    const onViewportChange = () => updatePinned();
    viewport?.addEventListener('scroll', onViewportChange, { passive: true });
    window.addEventListener('resize', onViewportChange, { passive: true });
    updatePinned();
    measure();
    return () => {
      observer.disconnect();
      viewport?.removeEventListener('scroll', onViewportChange);
      window.removeEventListener('resize', onViewportChange);
    };
  });
</script>

<div class="request-text" class:pinned class:expanded bind:this={body}>
  <Markdown {text} />
  {#if pinned && (overflowing || expanded)}
    <button type="button" aria-expanded={expanded} aria-label={expanded ? 'Collapse current request' : 'Expand current request'} onclick={toggle}>
      <span>{expanded ? 'Collapse' : 'Expand'}</span><ChevronDown size={14} aria-hidden="true" />
    </button>
  {/if}
</div>

<style>
  .request-text :global(.markdown) { overflow-wrap:anywhere; }
  .request-text.pinned :global(.markdown) { display:-webkit-box; overflow:hidden; -webkit-box-orient:vertical; -webkit-line-clamp:2; line-clamp:2; }
  .request-text.pinned.expanded :global(.markdown) { display:block; max-height:min(42vh,420px); overflow:auto; scrollbar-width:thin; -webkit-line-clamp:unset; line-clamp:unset; }
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
