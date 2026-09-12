<script lang="ts">
  import {FileText,FileCode,FileArchive,FileImage,FileAudio,FileVideo,X} from '@lucide/svelte';
  import type {Attachment} from '$lib/types';
  import ImageLightbox from './ImageLightbox.svelte';
  let {attachments=[],onremove}:{attachments?:Attachment[];onremove?:(id:string)=>void}=$props();
  let lightbox=$state<{src:string;alt:string;title:string}|null>(null), lightboxOpener=$state<HTMLElement|null>(null);
  const unique=$derived(attachments.filter((item,index,list)=>list.findIndex(other=>(other.sourceId||other.id)===(item.sourceId||item.id))===index));
  const preview=(item:Attachment)=>item.previewDataUrl && /^data:image\/(png|jpeg|webp);base64,[a-zA-Z0-9+/=]+$/.test(item.previewDataUrl)?item.previewDataUrl:null;
  function icon(item:Attachment){return item.mimeType.startsWith('image/')?FileImage:item.mimeType.startsWith('audio/')?FileAudio:item.mimeType.startsWith('video/')?FileVideo:/zip|gzip|tar|rar/.test(item.mimeType)?FileArchive:/json|javascript|xml|html|css/.test(item.mimeType)?FileCode:FileText}
  const size=(bytes:number)=>bytes>=1024*1024?`${(bytes/1024/1024).toFixed(1)} MB`:`${Math.max(1,Math.round(bytes/1024))} KB`;
</script>
{#if unique.length}<div class="attachments" aria-label="Attached files">
  {#each unique as attachment (attachment.id)}{@const Icon=icon(attachment)}
    {@const image=preview(attachment)}
    <div class:message-image={!onremove && !!image} class="attachment" title={`${attachment.name}\n${attachment.path}`}>
      {#if image}<button class="image-preview" aria-label={`View full-size ${attachment.name}`} onclick={event=>{lightboxOpener=event.currentTarget;lightbox={src:image,alt:`Preview of ${attachment.name}`,title:attachment.name}}}><img src={image} alt={`Preview of ${attachment.name}`}/></button>{:else}<span class="file-icon"><Icon size={23}/></span>{/if}
      <span class="file-info"><b>{attachment.name}</b><small>{size(attachment.size)}</small></span>
      {#if onremove}<button class="remove" aria-label={`Remove attachment ${attachment.name}`} onclick={()=>onremove?.(attachment.id)}><X size={13}/></button>{/if}
    </div>
  {/each}
</div>{/if}
<ImageLightbox bind:image={lightbox} returnFocus={lightboxOpener}/>
<style>
  .attachments{display:flex;gap:8px;flex-wrap:wrap;min-width:0;margin:8px 0}.attachment{position:relative;display:flex;align-items:center;gap:8px;max-width:100%;min-width:0;padding:6px;border:1px solid var(--line);border-radius:8px;background:var(--panel)}
  .image-preview{display:block;max-width:100%;padding:0;border:0;border-radius:5px;background:var(--soft);cursor:zoom-in}.image-preview:focus-visible{outline:2px solid var(--accent);outline-offset:2px}.attachment img{display:block;max-width:56px;max-height:46px;width:auto;height:auto;border-radius:4px;object-fit:contain}.file-icon{display:grid;place-items:center;width:34px;height:42px;color:var(--muted)}.file-info{display:grid;gap:3px;min-width:0}.file-info b{font-size:calc(11px * var(--interface-font-ratio, 1));max-width:180px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;font-weight:500}.file-info small{color:var(--muted);font:calc(9px * var(--interface-font-ratio, 1)) var(--mono)}.remove{display:grid;place-items:center;width:22px;height:24px;padding:0;border-radius:5px;color:var(--muted)}.remove:hover{background:var(--soft);color:var(--ink)}
  .attachment.message-image{display:grid;grid-template-columns:minmax(0,1fr);width:min(300px,100%);padding:8px}.attachment.message-image .image-preview{max-width:300px;width:auto}.attachment.message-image img{display:block;max-width:min(300px,100%);max-height:none;width:auto;height:auto;border-radius:5px;object-fit:contain}.attachment.message-image .file-info{grid-template-columns:minmax(0,1fr) auto;align-items:baseline}.attachment.message-image .file-info small{grid-column:2}.attachment.message-image .file-info b{max-width:none}
</style>
