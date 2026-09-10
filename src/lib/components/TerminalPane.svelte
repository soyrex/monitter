<script lang="ts">
  import { terminalErrors, terminalSessions, mountTerminal, setTerminalActive, unmountTerminal } from '$lib/terminal-runtime';
  let { sessionId, active = false }: { sessionId: string; active?: boolean } = $props();
  let host = $state<HTMLElement>();
  $effect(() => { const id=sessionId, element=host; if (element) void mountTerminal(id, element); return () => { if (element) unmountTerminal(id, element); }; });
  $effect(() => { setTerminalActive(sessionId, active); });
  const session = $derived($terminalSessions[sessionId]); const error = $derived($terminalErrors[sessionId]);
</script>
<section class="terminal-pane" aria-label={session?.title ?? 'Terminal'} class:inactive={!active}>
  <div class="terminal-screen" bind:this={host}></div>
  {#if !session}<p class="terminal-state">Terminal session is unavailable.</p>
  {:else if error}<p class="terminal-state error">Terminal error: {error}</p>
  {:else if session.status === 'exited'}<p class="terminal-state">Exited {session.exitCode === null ? '' : 'with status ' + session.exitCode}.</p>{/if}
</section>
<style>
  .terminal-pane{position:relative;min-height:0;height:100%;padding:1em;box-sizing:border-box;overflow:hidden;background:var(--terminal-background,#090b0d)}.terminal-screen{height:100%;min-height:0;overflow:hidden;box-sizing:border-box}.terminal-state{position:absolute;right:10px;bottom:8px;margin:0;padding:3px 6px;border-radius:4px;background:var(--soft);color:var(--muted);font-size:calc(11px * var(--interface-font-ratio, 1));pointer-events:none}.terminal-state.error{color:#d66}
</style>
