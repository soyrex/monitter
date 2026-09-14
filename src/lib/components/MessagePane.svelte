<script lang="ts">
  import { messageArrival } from '$lib/navigation-motion';
  import { onMount, type Snippet } from 'svelte';
  import { ArrowDown } from '@lucide/svelte';

  let { resetKey, children, header }: { resetKey: string; children: Snippet; header?: Snippet } = $props();
  let viewport = $state<HTMLDivElement>();
  let content = $state<HTMLDivElement>();
  let heading = $state<HTMLDivElement>();
  let showJump = $state(false);
  let followingLatest = true;
  let lastViewportHeight = 0;
  let lastScrollHeight = 0;
  let followFrame: number | undefined;
  const bottomThreshold = 48;

  function atLatest() {
    return !viewport || viewport.scrollHeight - viewport.clientHeight - viewport.scrollTop <= bottomThreshold;
  }

  function rememberMetrics() {
    if (!viewport) return;
    lastViewportHeight = viewport.clientHeight;
    lastScrollHeight = viewport.scrollHeight;
  }

  function pinToLatest() {
    if (!viewport) return;
    viewport.scrollTop = viewport.scrollHeight;
    showJump = false;
    // Keep programmatic scroll events from being mistaken for reader intent.
    rememberMetrics();
  }

  function followLayout() {
    pinToLatest();
    // Svelte children, font/image layout and composer resizing can settle after
    // the first measurement. Coalesce a final correction without smooth-scroll
    // animations that would constantly lag behind streamed replies.
    if (followFrame !== undefined) return;
    followFrame = requestAnimationFrame(() => {
      followFrame = undefined;
      if (followingLatest) pinToLatest();
    });
  }

  function jumpToLatest() {
    followingLatest = true;
    followLayout();
  }

  function handleScroll() {
    // Scroll anchoring/clamping can emit before ResizeObserver for CONTENT
    // growth as well as viewport resizing. Preserve the pre-layout follow state
    // instead of measuring the new, larger bottom gap as a reader scrolling up.
    if (viewport && (viewport.clientHeight !== lastViewportHeight || viewport.scrollHeight !== lastScrollHeight)) {
      if (followingLatest) followLayout();
      else showJump = !atLatest();
      rememberMetrics();
      return;
    }
    followingLatest = atLatest();
    showJump = !followingLatest;
    rememberMetrics();
  }

  // A conversation activation or explicit send requests a jump after Svelte
  // has rendered that conversation. Background updates do not change this key.
  $effect(() => {
    resetKey;
    jumpToLatest();
  });

  onMount(() => {
    rememberMetrics();
    const observer = new ResizeObserver(() => {
      // Covers streamed content, expanded tools, images/fonts and pane resizing.
      // Readers who scrolled up keep their place as new content arrives.
      if (followingLatest) followLayout();
      else showJump = !atLatest();
      rememberMetrics();
    });
    if (viewport) observer.observe(viewport);
    if (content) observer.observe(content);
    if (heading) observer.observe(heading);
    return () => {
      observer.disconnect();
      if (followFrame !== undefined) cancelAnimationFrame(followFrame);
    };
  });
</script>

<div class="message-pane">
  <!-- svelte-ignore a11y_no_noninteractive_tabindex (the scroll pane must support keyboard scrolling) -->
  <div class="messages" bind:this={viewport} onscroll={handleScroll} role="region" aria-label="Messages" tabindex="0">
    {#if header}<div class="message-header" bind:this={heading}>{@render header()}</div>{/if}
    <div class="message-content" use:messageArrival bind:this={content}>{@render children()}</div>
  </div>
  {#if showJump}
    <button class="jump-latest" aria-label="Jump to latest message" title="Jump to latest message" onclick={() => {
      jumpToLatest();
      viewport?.focus({ preventScroll: true });
    }}><ArrowDown size={18} /></button>
  {/if}
</div>

<style>
  .message-content { font-size: var(--chat-font-size, 13px); font-family: var(--chat-font, "IBM Plex Sans", system-ui, sans-serif); line-height: var(--chat-line-height, 1.65); }
  .message-pane { position: relative; display: flex; flex: 1; min-width: 0; min-height: 0; }
  .messages { --scroll-fade:20px; -webkit-mask-image:linear-gradient(to bottom,transparent 0,#000 var(--scroll-fade),#000 calc(100% - var(--scroll-fade)),transparent 100%); mask-image:linear-gradient(to bottom,transparent 0,#000 var(--scroll-fade),#000 calc(100% - var(--scroll-fade)),transparent 100%); flex: 1; min-width: 0; min-height: 0; overflow: auto; overscroll-behavior: contain; }
  .message-header { position: sticky; top: 0; z-index: 2; max-height: 40%; overflow: auto; scrollbar-width: thin; }
  /* Paint behind the translucent header so content fades before its top edge. */
  .message-header::before { content: ""; position: absolute; inset: 0; z-index: -1; pointer-events: none; background: linear-gradient(to bottom, var(--paper) 0%, var(--paper) 20%, transparent 100%); }
  .message-content { display: flow-root; width: min(var(--chat-content-max-width, 900px), calc(100% - 2 * var(--chat-side-padding, clamp(25px, 4vw, 50px)))); margin-inline: auto; padding: 25px 0 calc(var(--scroll-fade) + 8px); }
  .jump-latest { position: absolute; left: 50%; transform: translateX(-50%); bottom: 14px; z-index: 2; display: grid; place-items: center; width: 36px; height: 36px; border: 1px solid var(--line); border-radius: 50%; color: var(--ink); background: var(--panel); box-shadow: 0 3px 12px #0002; cursor: pointer; }
  .jump-latest:hover { color: var(--accent-ink); border-color: var(--accent); background: var(--soft); }
  .jump-latest:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
  @media (max-width: 640px) { .message-content { padding-top: 14px; padding-bottom: calc(var(--scroll-fade) + 8px); } .jump-latest { bottom: 10px; } }

</style>
