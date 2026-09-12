<script lang="ts">
  import { tick } from 'svelte';
  import { X } from '@lucide/svelte';

  let { image = $bindable<{ src: string; alt: string; title?: string } | null>(null), returnFocus = null }: {
    image?: { src: string; alt: string; title?: string } | null;
    returnFocus?: HTMLElement | null;
  } = $props();
  let dialog = $state<HTMLDialogElement>();
  let previouslyFocused: HTMLElement | null = null;
  let actualSize = $state(false);

  function restoreFocus() {
    const target = returnFocus ?? previouslyFocused;
    void tick().then(() => requestAnimationFrame(() => requestAnimationFrame(() => target?.focus())));
  }
  function close() {
    image = null;
    // Native dialog restoration can choose the document body when the dialog
    // is removed by Svelte, so restore the image trigger after that update.
    restoreFocus();
  }
  $effect(() => {
    if (!image) { if (dialog?.open) dialog.close(); return; }
    // Binding/showing the dialog can re-run this effect; retain the original
    // trigger instead of replacing it with one of the dialog controls.
    if (!dialog?.contains(document.activeElement)) previouslyFocused = document.activeElement as HTMLElement | null;
    actualSize = false;
    void tick().then(() => {
      if (!image || !dialog) return;
      if (!dialog.open) dialog.showModal();
      dialog.querySelector<HTMLElement>('[data-autofocus]')?.focus();
    });
    return restoreFocus;
  });
</script>

{#if image}
  <dialog bind:this={dialog} class="image-lightbox" aria-label={image.title || image.alt || 'Image preview'} oncancel={event=>{event.preventDefault();close();}} onclose={()=>image&&close()} onclick={event=>event.target===dialog&&close()}>
    <header><span>{image.title || image.alt || 'Image preview'}</span><div><button data-autofocus aria-label={actualSize ? 'Fit image to window' : 'View actual size'} onclick={()=>actualSize=!actualSize}>{actualSize ? 'Fit image' : 'Actual size'}</button><button aria-label="Close image preview" onclick={close}><X size={18}/></button></div></header>
    <div class="image-frame"><img class:actual-size={actualSize} src={image.src} alt={image.alt}/></div>
  </dialog>
{/if}

<style>
  .image-lightbox{grid-template-rows:auto minmax(0,1fr);width:min(1200px,calc(100vw - 48px));height:min(900px,calc(100vh - 48px));max-width:calc(100vw - 48px);max-height:calc(100vh - 48px);padding:0;border:1px solid var(--line);border-radius:12px;background:var(--panel);color:var(--ink);box-shadow:0 22px 70px #0008;overflow:hidden}.image-lightbox[open]{display:grid}.image-lightbox::backdrop{background:#0009;backdrop-filter:blur(3px)}
  header{display:flex;align-items:center;justify-content:space-between;gap:12px;padding:12px 14px;border-bottom:1px solid var(--line);font:500 calc(12px * var(--interface-font-ratio,1)) var(--interface-font,sans-serif)}header>div{display:flex;gap:7px}header span{min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}button{display:grid;place-items:center;flex:none;min-height:30px;padding:0 9px;border:0;border-radius:6px;background:var(--soft);color:var(--ink);cursor:pointer}button:focus-visible{outline:2px solid var(--accent);outline-offset:2px}.image-frame{display:grid;place-items:center;min-height:0;padding:14px;overflow:auto}.image-frame img{display:block;max-width:100%;max-height:100%;width:auto;height:auto;object-fit:contain}.image-frame img.actual-size{align-self:start;justify-self:start;max-width:none;max-height:none;object-fit:initial}
</style>
