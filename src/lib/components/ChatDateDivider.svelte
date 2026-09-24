<script lang="ts">
  import { chatDateClock, chatDateLabel, chatDateTimeAttribute } from '$lib/chat-dates';

  let { at }: { at: number } = $props();
  const label = $derived(chatDateLabel(at, $chatDateClock));
</script>

{#if label}
  <div class="chat-date-divider" role="separator" aria-label={label}>
    <span aria-hidden="true"></span>
    <time datetime={chatDateTimeAttribute(at)}>{label}</time>
    <span aria-hidden="true"></span>
  </div>
{/if}

<style>
  .chat-date-divider { display:flex; align-items:center; gap:10px; margin:5px 0 18px; color:var(--muted); font:calc(9px * var(--interface-font-ratio,1)) var(--mono); text-transform:uppercase; letter-spacing:.045em; white-space:nowrap; }
  .chat-date-divider > span { height:1px; flex:1; background:var(--line); }
  .chat-date-divider time { flex:none; }
</style>
