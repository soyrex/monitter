<script lang="ts">
  import { mentionParts, type MentionAgent } from '$lib/mentions';
  let { value = $bindable(''), agents, oninput, onkeydown }: { value?: string; agents: MentionAgent[]; oninput: (value:string)=>void; onkeydown:(event:KeyboardEvent)=>void } = $props();
  let scrollTop=$state(0), scrollLeft=$state(0), width=$state(0);
  const parts=$derived(mentionParts(value,agents));
</script>
<div class="mention-composer">
  <div class="mention-overlay" aria-hidden="true" style:width={width ? `${width}px` : undefined}><div class="mention-text" style:transform={`translate(${-scrollLeft}px, ${-scrollTop}px)`}>{#each parts as part}{#if part.agentId}<span class="mention-pill" data-agent-id={part.agentId}>{part.text}</span>{:else}{part.text}{/if}{/each}{value.endsWith('\n') ? '\n' : ''}</div></div>
  <textarea bind:clientWidth={width} aria-label="Channel message" placeholder="Message this channel… Use @ to mention an agent" bind:value oninput={event=>oninput(event.currentTarget.value)} {onkeydown} onscroll={event=>{scrollTop=event.currentTarget.scrollTop;scrollLeft=event.currentTarget.scrollLeft;}}></textarea>
</div>
<style>
  .mention-composer { position:relative; min-width:0; }
  .mention-overlay { position:absolute; inset:0; overflow:hidden; pointer-events:none; }
  .mention-text, textarea { margin:0; padding:2px; box-sizing:border-box; font-family:var(--chat-font,"IBM Plex Sans",system-ui,sans-serif); font-size:var(--chat-font-size,13px); line-height:1.5; letter-spacing:normal; white-space:pre-wrap; overflow-wrap:break-word; tab-size:8; }
  .mention-text { color:var(--ink); }
  textarea { position:relative; display:block; width:100%; min-height:52px; max-height:25vh; resize:vertical; border:0; outline:0; background:transparent; color:transparent; caret-color:var(--ink); }
  textarea::placeholder { color:var(--muted); }
  .mention-pill { color:var(--accent-ink); background:color-mix(in srgb,var(--accent) 16%,transparent); border-radius:4px; box-shadow:inset 0 0 0 1px color-mix(in srgb,var(--accent) 40%,transparent); }
</style>
