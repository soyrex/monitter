<script lang="ts">
  import { onMount, type Component } from 'svelte';

  type ComposerProps = {
    value: string;
    placeholder: string;
    oninput: (value: string) => void;
    onkeydown: (event: KeyboardEvent) => void;
  };

  let { value = $bindable(''), placeholder, oninput, onkeydown }: ComposerProps = $props();
  let Composer = $state<Component<ComposerProps> | null>(null);
  let loadError = $state('');
  let loadingEditor = $state<HTMLTextAreaElement>();

  onMount(() => {
    // Keep an editable, focused control on screen while TipTap loads. Its value
    // remains bound to the parent, so text entered during the import is handed
    // to the rich editor when it mounts.
    loadingEditor?.focus();
    void import('./RichMarkdownComposer.svelte')
      .then((module) => {
        Composer = module.default;
      })
      .catch((error: unknown) => {
        loadError = error instanceof Error ? error.message : String(error);
      });
  });
</script>

{#if Composer}
  <Composer bind:value {placeholder} {oninput} {onkeydown} />
{:else}
  <div class="loading-editor">
    {#if loadError}
      <div class="editor-error" role="alert">Rich editor failed to load: {loadError}</div>
    {:else}
      <div class="editor-loading" role="status" aria-live="polite">Loading rich editor…</div>
    {/if}
    <textarea
      bind:this={loadingEditor}
      bind:value
      aria-label="Markdown message"
      aria-multiline="true"
      placeholder={loadError ? 'Continue typing in plain text' : placeholder}
      oninput={(event) => oninput(event.currentTarget.value)}
      onkeydown={onkeydown}
    ></textarea>
  </div>
{/if}

<style>
  .loading-editor { display:flex; flex-direction:column; min-height:100%; }
  .loading-editor textarea { flex:1; width:100%; min-height:100%; padding:0; border:0; outline:none; resize:none; background:transparent; color:var(--ink); font:var(--chat-font-weight,400) var(--chat-font-size,13px)/var(--chat-line-height,1.65) var(--chat-font,"IBM Plex Sans",system-ui,sans-serif); }
  .loading-editor textarea::placeholder { color:var(--muted); }
  .editor-loading, .editor-error { padding:4px 0; color:var(--muted); font-size:12px; }
  .editor-error { color:var(--danger, #d55); }
</style>
