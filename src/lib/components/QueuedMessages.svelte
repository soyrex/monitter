<script lang="ts">
  import { Clock3, LoaderCircle, X, Paperclip, Pencil } from '@lucide/svelte';
  import type { QueuedMessage, Agent, Task } from '$lib/types';
  let {messages,agents,tasks,busy=false,onremove,onedit}: {messages:QueuedMessage[];agents:Agent[];tasks:Task[];busy?:boolean;onremove:(id:string)=>void;onedit:(id:string,text:string)=>Promise<boolean>}=$props();
  let editingId = $state<string|null>(null), draft = $state(''), saving = $state(false);
  const edited = $derived(messages.find(message=>message.id===editingId));
  const editable = $derived(edited && edited.status!=='sending' && edited.origin!=='channel-agent-mention');
  function focusEditor(node:HTMLTextAreaElement) { node.focus(); }
  async function save() {
    if (!editingId || !editable || saving) return;
    saving=true;
    try { if(await onedit(editingId,draft))editingId=null; } finally {saving=false;}
  }
</script>
{#if messages.length || editingId}<section class="message-queue" class:editing={editingId!==null} aria-label="Queued messages">
  {#if editingId}<div class="queue-editor">
    <label>Edit queued message
    <textarea aria-label="Edit queued message" bind:value={draft} disabled={saving} use:focusEditor></textarea></label>
    {#if !editable}<p class="edit-warning" role="status">This message is no longer waiting to be sent. Your edit has not been sent.</p>{/if}
    <div class="edit-actions"><button disabled={saving} onclick={()=>editingId=null}>Cancel</button><button disabled={busy||saving||!editable||(!draft.trim()&&!edited?.attachmentIds.length)} onclick={save}>{saving?'Saving…':'Save'}</button></div>
  </div>{/if}
  {#each messages as message (message.id)}{@const agent=agents.find(agent=>agent.id===tasks.find(task=>task.id===message.taskId)?.agentId)}{@const sender=agents.find(agent=>agent.id===message.senderAgentId)}
    <article class:error={message.status==='error'}>
      <div class="queue-heading">{#if message.status==='sending'}<LoaderCircle size={13}/>{:else}<Clock3 size={13}/>{/if}<strong>{message.status==='error'?'Needs attention':message.status==='sending'?'Sending':'Queued'}{sender?` · ${sender.name} → ${agent?.name??'agent'}`:agent?` · ${agent.name}`:''}</strong><button aria-label={`Edit queued message for ${agent?.name??'agent'}`} title={message.origin==='channel-agent-mention'?'Agent requests can be removed, but not rewritten':'Edit queued message'} disabled={busy||message.status==='sending'||editingId!==null||message.origin==='channel-agent-mention'} onclick={()=>{editingId=message.id;draft=message.text}}><Pencil size={13}/></button><button aria-label={`Remove queued message for ${agent?.name??'agent'}`} title="Remove queued message" disabled={busy||message.status==='sending'} onclick={()=>onremove(message.id)}><X size={13}/></button></div>
      <p>{message.text || 'Attachments only'}</p>
      {#if message.attachmentIds.length}<small><Paperclip size={11}/>{message.attachmentIds.length} attachment{message.attachmentIds.length===1?'':'s'}</small>{/if}
      {#if message.error}<p class="queue-error">{message.error}</p>{/if}
    </article>
  {/each}
</section>{/if}
<style>
  .message-queue { flex:none;max-height:150px;overflow:auto;width:min(var(--chat-content-max-width,900px),calc(100% - 2 * var(--chat-side-padding,clamp(25px,4vw,50px))));margin:0 auto 8px;border-left:2px solid var(--line);padding-left:10px; }
  .message-queue.editing { max-height:300px; }
  .queue-editor { display:grid; gap:6px; padding:8px 0; font-size:12px; }
  textarea { box-sizing:border-box; width:100%; min-height:70px; resize:vertical; border:1px solid var(--line); border-radius:6px; padding:8px; background:var(--panel); color:var(--ink); font:inherit; }
  .edit-actions { display:flex; justify-content:flex-end; gap:8px; }
  .edit-actions button { width:auto; padding:0 10px; }
  .edit-warning { display:block; color:var(--muted); }
  article { padding:6px 0;color:var(--muted);font-size:calc(11px * var(--interface-font-ratio,1)); }
  .queue-heading { display:flex;align-items:center;gap:6px; } strong { flex:1;font-weight:500; } button { display:grid;place-items:center;width:24px;height:24px;border-radius:4px; } button:hover { background:var(--soft); }
  p { margin:2px 0;white-space:pre-wrap;overflow-wrap:anywhere;display:-webkit-box;-webkit-line-clamp:2;line-clamp:2;-webkit-box-orient:vertical;overflow:hidden; }
  small { display:flex;align-items:center;gap:4px; } .error,.queue-error { color:#bd655b; } .queue-error { display:block; }
</style>
