<script lang="ts">
  import { messageArrival } from '$lib/navigation-motion';
  import { onMount, type Snippet } from 'svelte';
  import { ArrowDown } from '@lucide/svelte';

  let { resetKey, children, header, stickyRequest = false, active = true }: { resetKey: string; children: Snippet; header?: Snippet; stickyRequest?: boolean; active?: boolean } = $props();
  let viewport = $state<HTMLDivElement>();
  let content = $state<HTMLDivElement>();
  let heading = $state<HTMLDivElement>();
  let showJump = $state(false);
  let followingLatest = true;
  let lastViewportHeight = 0;
  let lastScrollHeight = 0;
  let followFrame: number | undefined;
  const bottomThreshold = 50;
  let documentVisible = $state(true);

  function distanceFromLatest() {
    return viewport ? Math.max(0, viewport.scrollHeight - viewport.clientHeight - viewport.scrollTop) : 0;
  }

  function nearLatest() {
    return distanceFromLatest() < bottomThreshold;
  }

  function atAbsoluteLatest() {
    return distanceFromLatest() <= 1;
  }

  function rememberMetrics() {
    if (!viewport || !active || !documentVisible) return;
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
    if (followFrame !== undefined || !active || !documentVisible) return;
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
      else if (atAbsoluteLatest()) { followingLatest = true; showJump = false; }
      else showJump = true;
      rememberMetrics();
      return;
    }
    followingLatest = nearLatest();
    showJump = !followingLatest;
    rememberMetrics();
  }

  // A conversation activation or explicit send requests a jump after Svelte
  // has rendered that conversation. Background updates do not change this key.
  $effect(() => {
    resetKey;
    if (active && documentVisible) jumpToLatest();
  });

  onMount(() => {
    const visibilityChanged = () => {
      documentVisible = document.visibilityState !== 'hidden';
      if (documentVisible && active) { rememberMetrics(); jumpToLatest(); }
    };
    documentVisible = document.visibilityState !== 'hidden';
    document.addEventListener('visibilitychange', visibilityChanged);
    return () => document.removeEventListener('visibilitychange', visibilityChanged);
  });

  $effect(() => {
    if (!viewport || !active || !documentVisible) return;
    rememberMetrics();
    const observer = new ResizeObserver(() => {
      // Covers streamed content, expanded tools, images/fonts and pane resizing.
      // Readers who scrolled up keep their place as new content arrives.
      if (followingLatest) followLayout();
      else if (atAbsoluteLatest()) { followingLatest = true; showJump = false; }
      else showJump = true;
      rememberMetrics();
    });
    if (viewport) observer.observe(viewport);
    if (content) observer.observe(content);
    if (heading) observer.observe(heading);
    const liveTextObserver = new MutationObserver(() => {
      // Elapsed timers and streamed text can update an existing text node
      // without adding an element. Correct the followed position in the same
      // microtask so that update cannot flash a small bottom gap for one frame.
      if (followingLatest) followLayout();
    });
    if (content) liveTextObserver.observe(content, { characterData: true, subtree: true });
    return () => {
      observer.disconnect();
      liveTextObserver.disconnect();
      if (followFrame !== undefined) cancelAnimationFrame(followFrame);
      followFrame = undefined;
    };
  });
</script>

<div class="message-pane" class:has-sticky-request={stickyRequest}>
  <!-- svelte-ignore a11y_no_noninteractive_tabindex (the scroll pane must support keyboard scrolling) -->
  <div class="messages" bind:this={viewport} onscroll={handleScroll} role="region" aria-label="Messages" tabindex="0">
    {#if header}<div class="message-header" bind:this={heading}>{@render header()}</div>{/if}
    <div class="message-content" use:messageArrival bind:this={content}>{@render children()}</div>
  </div>
  <button class="jump-latest" class:visible={showJump} aria-label="Jump to latest message" title="Jump to latest message" aria-hidden={!showJump} tabindex={showJump ? 0 : -1} disabled={!showJump} onclick={() => {
      jumpToLatest();
      viewport?.focus({ preventScroll: true });
    }}><ArrowDown size={18} /></button>
</div>

<style>
  .message-content { font-size: var(--chat-font-size, 13px); font-family: var(--chat-font, "IBM Plex Sans", system-ui, sans-serif); line-height: var(--chat-line-height, 1.65); }
  .message-pane { position: relative; display: flex; flex: 1; min-width: 0; min-height: 0; }
  .messages { --scroll-fade:20px; -webkit-mask-image:linear-gradient(to bottom,transparent 0,#000 var(--scroll-fade),#000 calc(100% - var(--scroll-fade)),transparent 100%); mask-image:linear-gradient(to bottom,transparent 0,#000 var(--scroll-fade),#000 calc(100% - var(--scroll-fade)),transparent 100%); flex: 1; min-width: 0; min-height: 0; overflow: auto; overscroll-behavior: contain; }
  .message-header { position: sticky; top: 0; z-index: 2; max-height: 40%; overflow: auto; scrollbar-width: thin; }
  /* Paint behind the translucent header so content fades before its top edge. */
  .message-header::before { content: ""; position: absolute; inset: 0; z-index: -1; pointer-events: none; background: linear-gradient(to bottom, var(--paper) 0%, var(--paper) 20%, transparent 100%); }
  .message-content { display: flow-root; width: min(var(--chat-content-max-width, 900px), calc(100% - 2 * var(--chat-side-padding, clamp(25px, 4vw, 50px)))); margin-inline: auto; padding: 25px 0 calc(var(--scroll-fade) + 8px); }
  .jump-latest { position:absolute; left:50%; bottom:14px; z-index:2; display:grid; place-items:center; width:36px; height:36px; border:1px solid var(--line); border-radius:50%; color:var(--ink); background:var(--panel); box-shadow:0 0 30px black; opacity:0; transform:translate(-50%,8px); visibility:hidden; pointer-events:none; cursor:pointer; transition:opacity .18s ease,transform .18s ease,visibility .18s step-end; }
  .jump-latest.visible { opacity:1; transform:translate(-50%,0); visibility:visible; pointer-events:auto; transition:opacity .18s ease,transform .18s ease; }
  .message-pane:has(:global(.reasoning-pending .animated-title[aria-busy="true"])) .jump-latest.visible { color:var(--accent-ink); border-color:var(--accent); box-shadow:0 0 0 1px color-mix(in srgb,var(--accent) 20%,transparent),0 0 14px color-mix(in srgb,var(--accent) 28%,transparent),0 3px 12px #0002; animation:jump-attention-glow 1.6s ease-in-out infinite; }
  .message-pane:has(:global(.reasoning-pending .animated-title[aria-busy="true"])) .jump-latest.visible::before,
  .message-pane:has(:global(.reasoning-pending .animated-title[aria-busy="true"])) .jump-latest.visible::after { content:"✦"; position:absolute; pointer-events:none; line-height:1; color:var(--accent-ink); text-shadow:0 0 8px var(--accent); animation:jump-attention-twinkle 1.3s ease-in-out infinite; }
  .message-pane:has(:global(.reasoning-pending .animated-title[aria-busy="true"])) .jump-latest.visible::before { top:-5px; right:-1px; font-size:8px; }
  .message-pane:has(:global(.reasoning-pending .animated-title[aria-busy="true"])) .jump-latest.visible::after { bottom:-4px; left:1px; font-size:6px; animation-delay:.65s; }
  .jump-latest:hover { color: var(--accent-ink); border-color: var(--accent); background: var(--soft); }
  .jump-latest:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
  @keyframes jump-attention-glow { 50% { box-shadow:0 0 0 2px color-mix(in srgb,var(--accent) 26%,transparent),0 0 20px color-mix(in srgb,var(--accent) 40%,transparent),0 3px 12px #0002; } }
  @keyframes jump-attention-twinkle { 0%,100% { opacity:.15; transform:scale(.45) rotate(-15deg); } 50% { opacity:1; transform:scale(1) rotate(15deg); } }
  @media (max-width: 640px) { .message-content { padding-top: 14px; padding-bottom: calc(var(--scroll-fade) + 8px); } .jump-latest { bottom: 10px; } }
  @media (min-height:501px) {
    .has-sticky-request .messages { -webkit-mask-image:linear-gradient(to bottom,#000 0,#000 calc(100% - var(--scroll-fade)),transparent 100%); mask-image:linear-gradient(to bottom,#000 0,#000 calc(100% - var(--scroll-fade)),transparent 100%); }
    :global(.message.sticky-user-request) { position:sticky; z-index:4; top:9px; isolation:isolate; box-shadow:0 8px 24px #0002,0 2px 7px #00000012; }
    :global(.message.sticky-user-request)::before { content:""; position:absolute; z-index:-1; top:-9px; right:0; bottom:-20px; left:0; pointer-events:none; background:linear-gradient(to bottom,var(--paper) 0,var(--paper) calc(100% - 20px),transparent 100%); }
  }
  @media (prefers-reduced-motion:reduce) {
    .jump-latest { transition:none; }
    .message-pane:has(:global(.reasoning-pending .animated-title[aria-busy="true"])) .jump-latest.visible { animation:none; }
    .message-pane:has(:global(.reasoning-pending .animated-title[aria-busy="true"])) .jump-latest.visible::before,
    .message-pane:has(:global(.reasoning-pending .animated-title[aria-busy="true"])) .jump-latest.visible::after { animation:none; opacity:.5; }
  }

</style>
