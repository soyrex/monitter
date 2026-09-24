<script lang="ts">
  import { onMount } from 'svelte';
  import { Editor } from '@tiptap/core';
  import { StarterKit } from '@tiptap/starter-kit';
  import { Markdown } from '@tiptap/markdown';

  let { value = $bindable(''), placeholder, oninput, onkeydown }: {
    value: string;
    placeholder: string;
    oninput: (value: string) => void;
    onkeydown: (event: KeyboardEvent) => void;
  } = $props();
  let element: HTMLDivElement;
  let editor: Editor | null = null;
  let written = '';

  onMount(() => {
    written = value;
    editor = new Editor({
      element,
      extensions: [StarterKit.configure({ link: { openOnClick: false } }), Markdown],
      content: value,
      contentType: 'markdown',
      editorProps: {
        attributes: { role: 'textbox', 'aria-label': 'Markdown message', 'aria-multiline': 'true' },
        handleKeyDown: (_view, event) => {
          onkeydown(event);
          return event.defaultPrevented;
        },
      },
      onUpdate: ({ editor }) => {
        written = editor.getMarkdown();
        value = written;
        oninput(written);
      },
    });
    queueMicrotask(() => editor?.commands.focus('end'));
    return () => { editor?.destroy(); editor = null; };
  });

  $effect(() => {
    const next = value;
    if (editor && next !== written) {
      written = next;
      editor.commands.setContent(next, { contentType: 'markdown', emitUpdate: false });
    }
  });
</script>

<div class="rich-editor">
  {#if !value}<span class="placeholder" aria-hidden="true">{placeholder}</span>{/if}
  <div bind:this={element}></div>
</div>

<style>
  .rich-editor { position:relative; min-height:100%; color:var(--ink); font:var(--chat-font-weight,400) var(--chat-font-size,13px)/var(--chat-line-height,1.65) var(--chat-font,"IBM Plex Sans",system-ui,sans-serif); }
  .placeholder { position:absolute; top:0; left:0; color:var(--muted); pointer-events:none; }
  .rich-editor :global(.tiptap) { min-height:100%; outline:none; overflow-wrap:anywhere; white-space:pre-wrap; }
  .rich-editor :global(.tiptap > :first-child) { margin-top:0; }
  .rich-editor :global(.tiptap p) { margin:.25em 0; }
  .rich-editor :global(.tiptap h1), .rich-editor :global(.tiptap h2), .rich-editor :global(.tiptap h3) { margin:.55em 0 .25em; line-height:1.25; font-weight:650; }
  .rich-editor :global(.tiptap h1) { font-size:1.65em; }
  .rich-editor :global(.tiptap h2) { font-size:1.35em; }
  .rich-editor :global(.tiptap h3) { font-size:1.15em; }
  .rich-editor :global(.tiptap ul), .rich-editor :global(.tiptap ol) { margin:.35em 0; padding-left:1.6em; }
  .rich-editor :global(.tiptap li p) { margin:0; }
  .rich-editor :global(.tiptap blockquote) { margin:.6em 0; padding-left:1em; border-left:2px solid var(--accent); color:var(--muted); }
  .rich-editor :global(.tiptap code) { padding:.1em .25em; border-radius:4px; background:var(--soft); font-family:var(--mono,monospace); }
  .rich-editor :global(.tiptap pre) { padding:12px; border:1px solid var(--line); border-radius:8px; background:var(--soft); overflow-x:auto; }
  .rich-editor :global(.tiptap pre code) { padding:0; background:none; }
  .rich-editor :global(.tiptap a) { color:var(--accent-ink); text-decoration:underline; }
  .rich-editor :global(.tiptap hr) { border:0; border-top:1px solid var(--line); margin:1em 0; }
</style>
