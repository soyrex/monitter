<script lang="ts">
  import { onMount, type Snippet } from 'svelte';
  import { ArrowDown } from '@lucide/svelte';

  let { resetKey, children, header }: { resetKey: string; children: Snippet; header?: Snippet } = $props();
  let viewport = $state<HTMLDivElement>();
  let content = $state<HTMLDivElement>();
  let heading = $state<HTMLDivElement>();
  let showJump = $state(false);
  let followingLatest = true;
  let lastViewportHeight = 0;
  let lastScrollTop = 0;
  let lastScrollHeight = 0;
  const bottomThreshold = 48;

  function atLatest() {
    return !viewport || viewport.scrollHeight - viewport.clientHeight - viewport.scrollTop <= bottomThreshold;
  }

  function wasFollowingBeforeLayout() {
    return followingLatest || lastScrollHeight - lastViewportHeight - lastScrollTop <= bottomThreshold;
  }

  function rememberMetrics() {
    if (!viewport) return;
    lastViewportHeight = viewport.clientHeight;
    lastScrollHeight = viewport.scrollHeight;
    lastScrollTop = viewport.scrollTop;
  }

  function jumpToLatest() {
    if (!viewport) return;
    followingLatest = true;
    viewport.scrollTop = viewport.scrollHeight;
    showJump = false;
  }

  function handleScroll() {
    // A browser may emit scroll before or after ResizeObserver when a pane's
    // height changes. That movement is layout, not an explicit reader choice.
    if (viewport && viewport.clientHeight !== lastViewportHeight) {
      followingLatest = wasFollowingBeforeLayout();
      showJump = !followingLatest;
      rememberMetrics();
      return;
    }
    followingLatest = atLatest();
    showJump = !followingLatest;
    rememberMetrics();
  }

  // A conversation activation or successful send requests a jump after Svelte
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
      followingLatest = wasFollowingBeforeLayout();
      if (followingLatest) jumpToLatest();
      else showJump = true;
      rememberMetrics();
    });
    if (viewport) observer.observe(viewport);
    if (content) observer.observe(content);
    if (heading) observer.observe(heading);
    return () => observer.disconnect();
  });
</script>

<div class="message-pane">
  <!-- svelte-ignore a11y_no_noninteractive_tabindex (the scroll pane must support keyboard scrolling) -->
  <div class="messages" bind:this={viewport} onscroll={handleScroll} role="region" aria-label="Messages" tabindex="0">
    {#if header}<div class="message-header" bind:this={heading}>{@render header()}</div>{/if}
    <div class="message-content" bind:this={content}>{@render children()}</div>
  </div>
  {#if showJump}
    <button class="jump-latest" aria-label="Jump to latest message" title="Jump to latest message" onclick={() => {
      jumpToLatest();
      viewport?.focus({ preventScroll: true });
    }}><ArrowDown size={18} /></button>
  {/if}
</div>

<style>
  .message-pane { position: relative; display: flex; flex: 1; min-width: 0; min-height: 0; }
  .messages { flex: 1; min-width: 0; min-height: 0; overflow: auto; overscroll-behavior: contain; }
  .message-header { position: sticky; top: 0; z-index: 2; max-height: 40%; overflow: auto; scrollbar-width: thin; }
  .message-content { display: flow-root; padding: 25px clamp(25px, 4vw, 50px); }
  .jump-latest { position: absolute; left: 50%; transform: translateX(-50%); bottom: 14px; z-index: 2; display: grid; place-items: center; width: 36px; height: 36px; border: 1px solid var(--line); border-radius: 50%; color: var(--ink); background: var(--panel); box-shadow: 0 3px 12px #0002; cursor: pointer; }
  .jump-latest:hover { color: var(--accent-ink); border-color: var(--accent); background: var(--soft); }
  .jump-latest:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
  @media (max-width: 640px) { .message-content { padding: 14px; } .jump-latest { bottom: 10px; } }
</style>
