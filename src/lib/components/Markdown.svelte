<script lang="ts">
  import { onMount } from "svelte";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { marked } from "marked";
  import DOMPurify from "dompurify";
  let { text }: { text: string } = $props();
  let container = $state<HTMLDivElement>();
  let linkError = $state("");
  const html = $derived(
    DOMPurify.sanitize(marked.parse(text, { async: false }) as string),
  );

  onMount(() => {
    async function openExternal(event: MouseEvent) {
      const target = event.target as Element | null;
      const anchor = target?.closest<HTMLAnchorElement>("a");
      if (!anchor || !container?.contains(anchor)) return;
      event.preventDefault();
      const href = anchor.getAttribute("href")?.trim() ?? "";
      if (!/^(https?:|mailto:)/i.test(href)) {
        linkError = "This link type is not supported in Monitter.";
        return;
      }
      try {
        linkError = "";
        if (Boolean((window as any).__TAURI_INTERNALS__)) await openUrl(href);
        else window.open(href, "_blank", "noopener,noreferrer");
      } catch (reason) {
        linkError = reason instanceof Error ? reason.message : String(reason);
      }
    }
    container?.addEventListener("click", openExternal);
    return () => container?.removeEventListener("click", openExternal);
  });
</script>

<div bind:this={container} class="markdown">{@html html}</div>
{#if linkError}<p class="link-error" role="alert">{linkError}</p>{/if}

<style>
  .markdown :global(p) {
    margin: 0 0 0.7em;
  }
  .markdown :global(p:last-child) {
    margin-bottom: 0;
  }
  .markdown :global(pre) {
    overflow: auto;
    padding: 10px 12px;
    border-radius: 7px;
    background: var(--code);
    font: calc(12px * var(--chat-font-ratio, 1))/var(--chat-line-height, 1.65) var(--mono);
  }
  .markdown :global(code) {
    font: calc(12px * var(--chat-font-ratio, 1)) var(--mono);
  }
  .markdown :global(:not(pre) > code) {
    padding: 1px 4px;
    border-radius: 3px;
    background: var(--code);
  }
  .markdown :global(ul),
  .markdown :global(ol) {
    margin: 0.4em 0;
    padding-left: 1.25em;
  }
  .markdown :global(a) {
    color: var(--accent-ink);
  }
  .markdown :global(blockquote) {
    margin: 0.5em 0;
    padding-left: 10px;
    border-left: 2px solid var(--accent);
    color: var(--muted);
  }
  .link-error {
    margin: 7px 0 0;
    color: #b84c44;
    font-size: calc(11.5px * var(--chat-font-ratio, 1));
  }
</style>
