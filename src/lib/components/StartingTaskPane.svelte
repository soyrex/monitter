<script lang="ts">
  import type { Snippet } from 'svelte';
  import { LoaderCircle } from '@lucide/svelte';
  import type { Attachment } from '$lib/types';
  import AnimatedTitle from '$lib/components/AnimatedTitle.svelte';
  import AttachmentList from '$lib/components/AttachmentList.svelte';
  import Markdown from '$lib/components/Markdown.svelte';
  import MessageMeta from '$lib/components/MessageMeta.svelte';
  import MessagePane from '$lib/components/MessagePane.svelte';

  type StartingMessage={id:string;createdAt:number;displayText:string;attachments:Attachment[]};
  let {title,agentName,messages,composer=$bindable(),showHeader=true,oncomposer,onkeydown,avatar,expand}:{
    title:string;agentName:string;messages:StartingMessage[];composer:string;showHeader?:boolean;
    oncomposer:(value:string)=>void;onkeydown:(event:KeyboardEvent)=>void;avatar:Snippet;expand:Snippet;
  }=$props();
</script>

<section class="starting-task-layout" aria-label="Starting chat">
  {#if showHeader}<header>
    <div class="identity">{@render avatar()}<h1>{title || 'New chat'}</h1></div>
    <div class="actions">{@render expand()}</div>
  </header>{/if}
  <section class="conversation">
    <MessagePane resetKey={`draft-starting:${messages.map(message=>message.id).join(':')}`}>
      {#each messages as message (message.id)}
        <article class="message user optimistic-message" data-delivery-status="sending">
          <MessageMeta name="You" createdAt={message.createdAt}><AnimatedTitle text="Sending" active={true} activeTooltip="Sending your message…"/></MessageMeta>
          <Markdown text={message.displayText} preserveLineBreaks/><AttachmentList attachments={message.attachments}/>
        </article>
      {/each}
    </MessagePane>
    <div class="composer">
      <textarea bind:value={composer} aria-label="Task message" placeholder={`Message ${agentName}…`} oninput={(event)=>oncomposer(event.currentTarget.value)} {onkeydown}></textarea>
      <footer><button class="primary" aria-label="Starting task" title="Starting…" disabled><LoaderCircle class="spin" size={15}/></button></footer>
    </div>
  </section>
</section>

<style>
  .starting-task-layout{display:grid;grid-template-rows:auto minmax(0,1fr);width:100%;height:100%;min-width:0;min-height:0;background:var(--paper);color:var(--ink)}
  header{display:flex;align-items:center;justify-content:space-between;gap:10px;padding:8px 15px;border-bottom:1px solid var(--line);background:var(--paper)}
  .identity{display:flex;align-items:center;gap:10px;min-width:0}.identity h1{min-width:0;margin:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;font-size:calc(13.2px * var(--interface-font-ratio,1))}.actions{display:flex;flex:none;align-items:center}
  .conversation{--chat-content-max-width:900px;display:flex;min-width:0;min-height:0;flex-direction:column}.message{max-width:100%;margin:0 0 24px}.optimistic-message{border:1px solid color-mix(in srgb,var(--accent) 35%,var(--line))}
  .composer{width:min(var(--chat-content-max-width),calc(100% - 40px));margin:0 auto 12px;padding:9px;border:1px solid var(--line);border-radius:10px;background:var(--panel)}textarea{display:block;width:100%;min-height:44px;padding:4px;border:0;outline:0;resize:none;color:var(--ink);background:transparent;font:calc(13px * var(--chat-font-ratio,1))/var(--chat-line-height,1.65) var(--chat-font,sans-serif)}footer{display:flex;justify-content:flex-end}.primary{display:grid;place-items:center;width:32px;height:32px;border:0;border-radius:50%;color:var(--on-accent);background:var(--accent)}
  :global(.spin){animation:spin .8s linear infinite}@keyframes spin{to{transform:rotate(360deg)}}@media(prefers-reduced-motion:reduce){:global(.spin){animation:none}}
</style>
