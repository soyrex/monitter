<script lang="ts">
  import { onDestroy, tick, untrack } from "svelte";
  import { X } from "@lucide/svelte";
  import { animateMotion } from "$lib/motion";
  import { activeModal } from "$lib/active-modal";
  let {
    title,
    open = false,
    wide = false,
    onclose,
    children,
  }: {
    title: string;
    open?: boolean;
    wide?: boolean;
    onclose: () => void;
    children: import("svelte").Snippet;
  } = $props();
  let dialog = $state<HTMLDivElement>();
  let previouslyFocused: HTMLElement | null = null;
  let visible = $state(false);
  let closing = $state(false);
  let closeRequested = false;
  let lifecycle = 0;
  let animations: Animation[] = [];
  let focusedWhenClosing = false;

  function cancelAnimations() {
    for (const animation of animations) animation.cancel();
    animations = [];
  }

  function requestClose() {
    if (!open || closeRequested) return;
    closeRequested = true;
    onclose();
    void tick().then(() => {
      if (open) closeRequested = false;
    });
  }

  function finishClose(token: number) {
    if (token !== lifecycle || open) return;
    const activeElement = document.activeElement;
    const shouldRestoreFocus = focusedWhenClosing && (
      !activeElement || activeElement === document.body || activeElement === document.documentElement || dialog?.contains(activeElement)
    );
    visible = false;
    closing = false;
    focusedWhenClosing = false;
    const focusTarget = previouslyFocused;
    previouslyFocused = null;
    if (shouldRestoreFocus) {
      void tick().then(() => {
        const activeElement = document.activeElement;
        const focusStillUnclaimed = !activeElement || activeElement === document.body || activeElement === document.documentElement || dialog?.contains(activeElement);
        if (token === lifecycle && !open && focusStillUnclaimed) focusTarget?.focus();
      });
    }
  }

  function enter() {
    const token = ++lifecycle;
    cancelAnimations();
    closing = false;
    closeRequested = false;
    if (!visible) visible = true;
    if (!previouslyFocused) previouslyFocused = document.activeElement as HTMLElement | null;
    void tick().then(() => {
      if (token !== lifecycle || !open || !dialog) return;
      const preferred = dialog.querySelector<HTMLElement>("[data-autofocus]");
      (preferred ?? dialog).focus();
      const backdrop = dialog.parentElement;
      animations = [
        ...(backdrop ? [animateMotion(backdrop, [{ opacity: 0 }, { opacity: 1 }], { duration: 170, easing: "ease-out", fill: "both" })] : []),
        animateMotion(dialog, [{ opacity: 0, transform: "translateY(6px)" }, { opacity: 1, transform: "translateY(0)" }], { duration: 170, easing: "cubic-bezier(.2,.8,.2,1)", fill: "both" }),
      ].filter((animation): animation is Animation => animation !== null);
    });
  }

  function exit() {
    if (!visible) return;
    const token = ++lifecycle;
    cancelAnimations();
    focusedWhenClosing = !!dialog?.contains(document.activeElement);
    closing = true;
    if (!dialog) {
      finishClose(token);
      return;
    }
    const backdrop = dialog.parentElement;
    animations = [
      ...(backdrop ? [animateMotion(backdrop, [{ opacity: 1 }, { opacity: 0 }], { duration: 110, easing: "ease-in", fill: "both" })] : []),
      animateMotion(dialog, [{ opacity: 1, transform: "translateY(0)" }, { opacity: 0, transform: "translateY(6px)" }], { duration: 110, easing: "ease-in", fill: "both" }),
    ].filter((animation): animation is Animation => animation !== null);
    if (!animations.length) {
      finishClose(token);
      return;
    }
    void Promise.all(animations.map((animation) => animation.finished.catch(() => undefined))).then(() => finishClose(token));
  }

  $effect(() => {
    if (open) untrack(enter);
    else untrack(exit);
  });

  onDestroy(() => {
    ++lifecycle;
    const restore = dialog?.contains(document.activeElement);
    cancelAnimations();
    if (restore) previouslyFocused?.focus();
  });

  function handleKeydown(event: KeyboardEvent) {
    if (!visible || closing || !dialog?.contains(event.target as Node)) return;
    if (event.key === "Escape") {
      event.preventDefault();
      requestClose();
      return;
    }
    if (event.key !== "Tab" || !dialog) return;
    const focusable = [
      ...dialog.querySelectorAll<HTMLElement>(
        'button:not([disabled]), input:not([disabled]), textarea:not([disabled]), select:not([disabled]), [href], [tabindex]:not([tabindex="-1"])',
      ),
    ];
    if (!focusable.length) {
      event.preventDefault();
      dialog.focus();
      return;
    }
    const first = focusable[0],
      last = focusable.at(-1)!;
    if (
      event.shiftKey &&
      (document.activeElement === first || document.activeElement === dialog)
    ) {
      event.preventDefault();
      last.focus();
    } else if (
      !event.shiftKey &&
      (document.activeElement === last || document.activeElement === dialog)
    ) {
      event.preventDefault();
      first.focus();
    }
  }
</script>

<svelte:window onkeydown={handleKeydown} />

{#if visible}
  <div
    class="backdrop"
    class:closing
    role="presentation"
    onclick={(event) => event.currentTarget === event.target && requestClose()}
  >
    <div
      bind:this={dialog} use:activeModal
      class="modal"
      class:wide
      data-motion-closing={closing ? 'true' : undefined}
      inert={closing}
      role="dialog"
      aria-modal="true"
      aria-label={title}
      tabindex="-1"
    >
      <header>
        <h2>{title}</h2>
        <button class="icon" aria-label="Close" onclick={requestClose}
          ><X size={18} /></button
        >
      </header>
      <div class="body">{@render children()}</div>
    </div>
  </div>
{/if}

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 30;
    display: grid;
    place-items: center;
    padding: 24px;
    background: rgba(0, 0, 0, 0.3);
    backdrop-filter: blur(3px);
  }
  .modal {
    width: min(620px, 100%);
    max-height: calc(100vh - 48px);
    overflow: auto;
    border: 1px solid var(--line);
    border-radius: 13px;
    background: var(--panel);
    box-shadow: 0 12px 40px rgba(0, 0, 0, 0.8) !important;
  }
  .modal.wide { width: min(860px, 100%); }
  header {
    position: sticky;
    top: 0;
    z-index: 1;
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 17px 20px;
    border-bottom: 1px solid var(--line);
    background: var(--panel);
  }
  h2 {
    margin: 0;
    font-size: calc(16px * var(--interface-font-ratio, 1));
    font-weight: 600;
  }
  .body {
    padding: 20px;
  }
  .icon {
    display: grid;
    place-items: center;
    width: 30px;
    height: 30px;
    padding: 0;
  }
</style>
