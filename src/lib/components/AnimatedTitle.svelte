<script lang="ts">
  let { text, active = false }: { text: string; active?: boolean } = $props();
</script>
<span class="animated-title" class:naming={active} aria-busy={active} title={active ? 'Generating a title…' : text}>{text}</span>
<style>
  .animated-title { position:relative; }
  .naming { color:var(--accent-ink); animation:title-glow 1.6s ease-in-out infinite; }
  .naming::before,.naming::after { content:'✦'; position:absolute; pointer-events:none; font-size:.65em; color:var(--accent-ink); text-shadow:0 0 8px var(--accent); animation:title-twinkle 1.3s ease-in-out infinite; }
  .naming::before { left:20%; top:-.35em; }
  .naming::after { right:12%; bottom:-.2em; animation-delay:.65s; }
  @keyframes title-glow { 50% { opacity:.6; } }
  @keyframes title-twinkle { 0%,100% { opacity:.15; transform:scale(.45) rotate(-15deg); } 50% { opacity:1; transform:scale(1) rotate(15deg); } }
  @media(prefers-reduced-motion:reduce) { .naming,.naming::before,.naming::after { animation:none; } .naming::before,.naming::after { opacity:.5; } }
</style>
