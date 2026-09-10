<script lang="ts">
  import { UserMinus, Plus, Settings2, X, Square } from '@lucide/svelte';
  import type { Channel, Agent, Host, Task } from '$lib/types';
  let {channel,agents,hosts,tasks,busy=false,onmembership,onadmin,onclose,onconversation,onstopconversation}: {channel:Channel;agents:Agent[];hosts:Host[];tasks:Task[];busy?:boolean;onmembership:(id:string,member:boolean)=>void;onadmin:()=>void;onclose:()=>void;onconversation:(enabled:boolean,turnLimit:number)=>void;onstopconversation:()=>void}=$props();
  let inviteId=$state('');
  const members=$derived(agents.filter(agent=>channel.agentIds.includes(agent.id)));
  const available=$derived(agents.filter(agent=>!channel.agentIds.includes(agent.id)));
</script>
<header><strong>Members <span>{members.length}</span></strong><button aria-label="Hide channel members" title="Hide members" onclick={onclose}><X size={16}/></button></header>
<div class="member-content">
  {#if channel.description}<p class="topic">{channel.description}</p>{/if}
  {#each members as agent (agent.id)}
    <div class="member">
      <span class="member-avatar" style:background={agent.color}>{#if agent.avatar && /^data:image\/(png|jpeg|webp);base64,/i.test(agent.avatar)}<img src={agent.avatar} alt=""/>{:else}{agent.name.slice(0,1).toUpperCase()}{/if}</span>
      <div class="member-copy"><b>{agent.name}</b><small>{tasks.some(task=>task.channelId===channel.id&&task.agentId===agent.id&&task.status==='running')?'Working · ':''}{hosts.find(host=>host.id===agent.hostId)?.name??'Unknown host'}</small></div>
      <button disabled={busy} aria-label={`Remove ${agent.name} from channel`} title={`Remove ${agent.name}`} onclick={()=>onmembership(agent.id,false)}><UserMinus size={15}/></button>
    </div>
  {:else}<p class="hint">No agents in this channel yet.</p>{/each}
  <div class="invite"><label for={`invite-${channel.id}`}>Invite agent</label><div><select id={`invite-${channel.id}`} aria-label="Agent to invite" bind:value={inviteId} disabled={busy||!available.length}><option value="">Choose an agent…</option>{#each available as agent}<option value={agent.id}>{agent.name}</option>{/each}</select><button aria-label="Invite selected agent" title="Invite agent" disabled={busy||!available.some(agent=>agent.id===inviteId)} onclick={()=>{onmembership(inviteId,true);inviteId='';}}><Plus size={16}/></button></div></div>
  <section class="agent-conversation" aria-label="Agent conversation">
    <label class="conversation-toggle"><span>Agent conversation</span><input type="checkbox" role="switch" aria-label="Agent conversation" checked={channel.agentConversationEnabled===true} disabled={busy} onchange={event=>onconversation(event.currentTarget.checked,channel.agentConversationTurnLimit??6)}/></label>
    <p class="hint">Agents can @mention another member to ask a question or request a reply here.</p>
    {#if channel.agentConversationEnabled}
      <label class="turn-limit">Automatic reply limit<input type="number" min="1" max="20" aria-label="Automatic reply limit" value={channel.agentConversationTurnLimit??6} disabled={busy} onchange={event=>{const value=Number(event.currentTarget.value);if(Number.isInteger(value)&&value>=1&&value<=20)onconversation(true,value);else event.currentTarget.value=String(channel.agentConversationTurnLimit??6);}}/></label>
      <p class="conversation-progress" role="status">{channel.agentConversationTurnsUsed??0} of {channel.agentConversationTurnLimit??6} automatic replies{channel.agentConversationPaused?' · Paused':''}</p>
      <p class="hint">Your next message starts a new round.</p>
      <button class="stop-conversation" disabled={busy||channel.agentConversationPaused} onclick={onstopconversation}><Square size={13}/>Stop agent conversation</button>
    {/if}
  </section>
  <p class="hint admin-note">You administer this channel.</p>
  <button class="admin" disabled={busy} onclick={onadmin}><Settings2 size={15}/>Channel administration</button>
  <details class="command-help"><summary>Channel commands</summary><p><code>/members</code> List members<br/><code>/invite @name</code> Add an agent<br/><code>/kick @name</code> Remove an agent<br/><code>/topic text</code> Change topic<br/><code>/admin</code> Channel settings</p></details>
</div>
<style>
  header { display:flex;align-items:center;justify-content:space-between;gap:8px;padding:14px;border-bottom:1px solid var(--line);flex:none; }
  header strong { font-size:calc(13px * var(--interface-font-ratio,1)); } header span,small,.hint { color:var(--muted); }
  button { display:inline-flex;align-items:center;justify-content:center;gap:7px;min-width:28px;min-height:28px;border-radius:5px;color:var(--muted); } button:hover { background:var(--soft);color:var(--ink); }
  .member-content { flex:1;min-height:0;overflow:auto;padding:12px; }
  .member { display:flex;align-items:center;gap:9px;padding:9px 0; }
  .member-avatar { width:26px;height:26px;border-radius:6px;flex:none;display:grid;place-items:center;color:white;overflow:hidden; } img { width:100%;height:100%;object-fit:cover; }
  .member-copy { flex:1;min-width:0; } b,small { display:block;overflow-wrap:anywhere; } b { font-size:calc(12px * var(--interface-font-ratio,1)); } small { font-size:calc(10px * var(--interface-font-ratio,1));margin-top:3px; }
  .agent-conversation { margin:16px 0;padding:12px 0;border-top:1px solid var(--line);border-bottom:1px solid var(--line);font-size:calc(11px * var(--interface-font-ratio,1)); }
  .conversation-toggle,.turn-limit { display:flex;align-items:center;justify-content:space-between;gap:10px; }
  .conversation-toggle input { appearance:none;position:relative;flex:none;width:30px;height:17px;border:1px solid var(--line);border-radius:20px;background:var(--soft); }
  .conversation-toggle input::after { content:'';position:absolute;left:2px;top:2px;width:11px;height:11px;border-radius:50%;background:var(--muted);transition:transform .15s; }
  .conversation-toggle input:checked { background:var(--accent); }
  .conversation-toggle input:checked::after { transform:translateX(13px);background:var(--on-accent,#fff); }
  .turn-limit input { width:48px;background:var(--panel);color:var(--ink);border:1px solid var(--line);border-radius:4px;padding:5px; }
  .conversation-progress { color:var(--muted); }.stop-conversation { width:100%;justify-content:flex-start; }
  .topic { margin:0 0 10px;color:var(--muted);overflow-wrap:anywhere; }
  .invite { margin:18px 0 10px; } label { display:block;font-size:calc(11px * var(--interface-font-ratio,1));margin-bottom:6px; } .invite>div { display:flex;gap:6px; } select { width:100%;min-width:0;background:var(--panel);color:var(--ink);border:1px solid var(--line);border-radius:5px;padding:6px; } .admin { width:100%;justify-content:flex-start; }
  .admin-note { font-size:calc(11px * var(--interface-font-ratio,1)); }
  .command-help { margin-top:18px;color:var(--muted);font-size:calc(11px * var(--interface-font-ratio,1));line-height:1.8; } summary { cursor:pointer; } code { font-family:var(--mono); }
</style>
