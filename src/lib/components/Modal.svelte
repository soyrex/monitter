<script lang="ts">
  import { tick } from "svelte";
  import { X } from "@lucide/svelte";
  let dialog = $state<HTMLDivElement>();
  let previouslyFocused: HTMLElement | null = null;
  let {
    title,
    open = false,
    onclose,
    children,
  }: {
    title: string;
    open?: boolean;
    onclose: () => void;
    children: import("svelte").Snippet;
  } = $props();

  $effect(() => {
    if (!open) return;
    previouslyFocused = document.activeElement as HTMLElement | null;
    void tick().then(() => {
      const preferred = dialog?.querySelector<HTMLElement>("[data-autofocus]");
      (preferred ?? dialog)?.focus();
    });
    return () => previouslyFocused?.focus();
  });

  function handleKeydown(event: KeyboardEvent) {
    if (!open) return;
    if (event.key === "Escape") {
      event.preventDefault();
      onclose();
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

{#if open}
  <div
    class="backdrop"
    role="presentation"
    onclick={(event) => event.currentTarget === event.target && onclose()}
  >
    <div
      bind:this={dialog}
      class="modal"
      role="dialog"
      aria-modal="true"
      aria-label={title}
      tabindex="-1"
    >
      <header>
        <h2>{title}</h2>
        <button class="icon" aria-label="Close" onclick={onclose}
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
    background: rgba(32, 27, 20, 0.42);
    backdrop-filter: blur(3px);
  }
  .modal {
    width: min(620px, 100%);
    max-height: calc(100vh - 48px);
    overflow: auto;
    border: 1px solid var(--line);
    border-radius: 13px;
    background: var(--panel);
    box-shadow: 0 22px 70px rgba(40, 31, 18, 0.27);
  }
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
