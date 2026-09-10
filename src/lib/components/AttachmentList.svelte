<script lang="ts">
  import {FileText,FileCode,FileArchive,FileImage,FileAudio,FileVideo,X} from '@lucide/svelte';
  import type {Attachment} from '$lib/types';
  let {attachments=[],onremove}:{attachments?:Attachment[];onremove?:(id:string)=>void}=$props();
  const unique=$derived(attachments.filter((item,index,list)=>list.findIndex(other=>(other.sourceId||other.id)===(item.sourceId||item.id))===index));
  const preview=(item:Attachment)=>item.previewDataUrl && /^data:image\/(png|jpeg|webp);base64,[a-zA-Z0-9+/=]+$/.test(item.previewDataUrl)?item.previewDataUrl:null;
  function icon(item:Attachment){return item.mimeType.startsWith('image/')?FileImage:item.mimeType.startsWith('audio/')?FileAudio:item.mimeType.startsWith('video/')?FileVideo:/zip|gzip|tar|rar/.test(item.mimeType)?FileArchive:/json|javascript|xml|html|css/.test(item.mimeType)?FileCode:FileText}
  const size=(bytes:number)=>bytes>=1024*1024?`${(bytes/1024/1024).toFixed(1)} MB`:`${Math.max(1,Math.round(bytes/1024))} KB`;
</script>
{#if unique.length}<div class="attachments" aria-label="Attached files">
  {#each unique as attachment (attachment.id)}{@const Icon=icon(attachment)}
    <div class="attachment" title={`${attachment.name}\n${attachment.path}`}>
      {#if preview(attachment)}<img src={preview(attachment)!} alt={`Preview of ${attachment.name}`}/>{:else}<span class="file-icon"><Icon size={23}/></span>{/if}
      <span class="file-info"><b>{attachment.name}</b><small>{size(attachment.size)}</small></span>
      {#if onremove}<button class="remove" aria-label={`Remove attachment ${attachment.name}`} onclick={()=>onremove?.(attachment.id)}><X size={13}/></button>{/if}
    </div>
  {/each}
</div>{/if}
<style>
  .attachments{display:flex;gap:8px;flex-wrap:wrap;min-width:0;margin:8px 0}.attachment{position:relative;display:flex;align-items:center;gap:8px;max-width:100%;min-width:0;padding:6px;border:1px solid var(--line);border-radius:8px;background:var(--panel)}
  .attachment img{width:56px;height:46px;object-fit:contain;border-radius:4px;background:var(--soft)}.file-icon{display:grid;place-items:center;width:34px;height:42px;color:var(--muted)}.file-info{display:grid;gap:3px;min-width:0}.file-info b{font-size:11px;max-width:180px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;font-weight:500}.file-info small{color:var(--muted);font:9px var(--mono)}.remove{display:grid;place-items:center;width:22px;height:24px;padding:0;border-radius:5px;color:var(--muted)}.remove:hover{background:var(--soft);color:var(--ink)}
</style>
