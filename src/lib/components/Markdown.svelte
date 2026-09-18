<script lang="ts">
  import { getContext, onMount, tick } from "svelte";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { marked } from "marked";
  import DOMPurify from "dompurify";
  import ImageLightbox from './ImageLightbox.svelte';
  let { text, taskId, basePath }: { text: string; taskId?: string; basePath?: string } = $props();
  const contextTaskId = getContext<string | undefined>('monitter-markdown-task-id');
  const openMarkdown = getContext<((taskId: string, href: string, basePath?: string) => Promise<void>) | undefined>('monitter-open-markdown');
  let container = $state<HTMLDivElement>();
  let linkError = $state("");
  let lightbox = $state<{ src: string; alt: string; title?: string } | null>(null);
  let lightboxOpener = $state<HTMLElement | null>(null);
  // Preserve local Markdown file URLs for the scoped native reader. Other URI
  // schemes retain DOMPurify's normal allowlist; the click handler owns opening.
  const allowedMarkdownUris = /^(?:(?:(?:f|ht)tps?|mailto|tel|callto|sms|cid|xmpp|matrix):|file:\/\/\/[^?#]*\.(?:md|markdown)(?:[?#]|$)|[^a-z]|[a-z+.\-]+(?:[^a-z+.\-:]|$))/i;
  // Keep expensive parsing tied to the primitive content, not a replaced message object.
  const sourceText = $derived(text);
  const html = $derived(
    DOMPurify.sanitize(marked.parse(sourceText, { async: false }) as string, { ALLOWED_URI_REGEXP: allowedMarkdownUris }),
  );
  function makeImagesInteractive() {
    for (const image of container?.querySelectorAll<HTMLImageElement>('img') ?? []) {
      image.tabIndex = 0;
      image.setAttribute('role', 'button');
      image.setAttribute('aria-label', `View full-size ${image.alt || 'image'}`);
    }
  }
  $effect(() => { html; container; void tick().then(makeImagesInteractive); });

  onMount(() => {
    function openImage(target: EventTarget | null) {
      const image = target instanceof Element ? target.closest<HTMLImageElement>('img') : null;
      if (!image || !container?.contains(image)) return false;
      const src = image.currentSrc || image.getAttribute('src') || '';
      if (!/^(?:data:image\/(?:png|jpeg|webp);base64,|https?:)/i.test(src)) return false;
      lightboxOpener = image;
      lightbox = { src, alt: image.alt || 'Image preview', title: image.alt || undefined };
      return true;
    }
    async function openExternal(event: MouseEvent) {
      if (openImage(event.target)) { event.preventDefault(); return; }
      const target = event.target as Element | null;
      const anchor = target?.closest<HTMLAnchorElement>("a");
      if (!anchor || !container?.contains(anchor)) return;
      event.preventDefault();
      const href = anchor.getAttribute("href")?.trim() ?? "";
      if (/^(?:file:\/\/|\/|\.{0,2}\/|[^:/?#]+(?:\/[^:/?#]+)*\.(?:md|markdown)(?:[?#].*)?$)/i.test(href) && /\.(?:md|markdown)$/i.test(href.split(/[?#]/)[0])) {
        if (!openMarkdown || !(taskId ?? contextTaskId)) { linkError = 'Markdown files can only be opened from a local task.'; return; }
        try { linkError = ''; await openMarkdown(taskId ?? contextTaskId!, href, basePath); }
        catch (reason) { linkError = reason instanceof Error ? reason.message : String(reason); }
        return;
      }
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
    function openImageFromKeyboard(event: KeyboardEvent) {
      if (event.key !== 'Enter' && event.key !== ' ') return;
      if (openImage(event.target)) event.preventDefault();
    }
    container?.addEventListener("click", openExternal);
    container?.addEventListener("keydown", openImageFromKeyboard);
    return () => { container?.removeEventListener("click", openExternal); container?.removeEventListener("keydown", openImageFromKeyboard); };
  });
</script>

<div bind:this={container} class="markdown">{@html html}</div>
{#if linkError}<p class="link-error" role="alert">{linkError}</p>{/if}
<ImageLightbox bind:image={lightbox} returnFocus={lightboxOpener}/>

<style>
  .markdown { min-width:0; max-width:100%; overflow:hidden; overflow-wrap:anywhere; }
  .markdown :global(p) {
    margin: 0 0 0.7em;
  }
  .markdown :global(p:last-child) {
    margin-bottom: 0;
  }
  .markdown :global(pre) {
    max-width:100%;
    overflow:hidden;
    padding: 10px 12px;
    border-radius: 7px;
    background: var(--code);
    white-space:pre;
    text-overflow:ellipsis;
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
  .markdown :global(img) {
    display: block;
    max-width: min(300px, 100%);
    width: auto;
    height: auto;
    cursor: zoom-in;
  }
  .markdown :global(img[role="button"]) { outline: none; }
  .markdown :global(img[role="button"]:focus-visible) { outline: 2px solid var(--accent); outline-offset: 2px; }
  .link-error {
    margin: 7px 0 0;
    color: #b84c44;
    font-size: calc(11.5px * var(--chat-font-ratio, 1));
  }
</style>
