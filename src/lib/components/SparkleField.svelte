<script lang="ts">
  let { active = false }: { active?: boolean } = $props();
  const sparkles = [
    { x: 6, y: 78, size: '11px', delay: 0 },
    { x: 92, y: 62, size: '9px', delay: .5 },
    { x: 18, y: 90, size: '7px', delay: 1.4 },
    { x: 78, y: 84, size: '13px', delay: .9 },
    { x: 40, y: 70, size: '8px', delay: 2.1 },
    { x: 64, y: 94, size: '10px', delay: 1.7 },
    { x: 30, y: 88, size: '9px', delay: 2.6 },
    { x: 88, y: 80, size: '7px', delay: 3.2 },
    { x: 10, y: 60, size: '10px', delay: 3.6 },
    { x: 55, y: 92, size: '8px', delay: 1.1 },
    { x: 72, y: 68, size: '11px', delay: 2.9 },
    { x: 25, y: 96, size: '7px', delay: 4.1 },
    { x: 48, y: 58, size: '9px', delay: .3 },
    { x: 82, y: 96, size: '8px', delay: 1.9 },
    { x: 15, y: 72, size: '12px', delay: 3.9 },
    { x: 60, y: 82, size: '7px', delay: 2.4 },
    { x: 95, y: 90, size: '9px', delay: .8 },
    { x: 35, y: 76, size: '10px', delay: 4.5 },
  ];
</script>

{#if active}<div class="sparkle-field" aria-hidden="true">
  <div class="sparkle-glow"></div>
  {#each sparkles as sparkle}<span class="sparkle" style:left={`${sparkle.x}%`} style:top={`${sparkle.y}%`} style:font-size={sparkle.size} style:animation-delay={`${sparkle.delay}s`}>✦</span>{/each}
</div>{/if}

<style>
  .sparkle-field { position:absolute; inset:0; overflow:hidden; pointer-events:none; z-index:0; }
  .sparkle-glow { position:absolute; inset:auto 0 0 0; height:40px; background:linear-gradient(to top, color-mix(in srgb, var(--accent) 20%, transparent), transparent); }
  .sparkle { position:absolute; line-height:1; color:var(--accent-ink); text-shadow:0 0 8px var(--accent); opacity:0; animation:sparkle-fade-up 3.4s ease-in infinite; }
  @keyframes sparkle-fade-up {
    0% { opacity:0; transform:translateY(14px) scale(.5); }
    15% { opacity:1; transform:translateY(0) scale(1); }
    45% { opacity:.85; transform:translateY(-4px) scale(1); }
    70%, 100% { opacity:0; transform:translateY(-18px) scale(.6); }
  }
  @media (prefers-reduced-motion:reduce) { .sparkle { animation:none; opacity:.55; transform:none; } }
</style>
