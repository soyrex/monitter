<script lang="ts">
  export type ActivityAnimation = 'sparkles' | 'grid' | 'matrix';

  let {
    active = false,
    mirror = false,
    contained = false,
    compact = false,
    pane = false,
    animation = 'sparkles',
  }: {
    active?: boolean;
    mirror?: boolean;
    contained?: boolean;
    compact?: boolean;
    pane?: boolean;
    animation?: ActivityAnimation;
  } = $props();

  const sparkles = [
    { x: 6, y: 40, size: '11px', delay: 0 },
    { x: 92, y: 90, size: '9px', delay: .5 },
    { x: 18, y: 20, size: '7px', delay: 1.4 },
    { x: 78, y: 55, size: '13px', delay: .9 },
    { x: 40, y: 130, size: '8px', delay: 2.1 },
    { x: 64, y: 15, size: '10px', delay: 1.7 },
    { x: 30, y: 70, size: '9px', delay: 2.6 },
    { x: 88, y: 160, size: '7px', delay: 3.2 },
    { x: 10, y: 110, size: '10px', delay: 3.6 },
    { x: 55, y: 25, size: '8px', delay: 1.1 },
    { x: 72, y: 185, size: '11px', delay: 2.9 },
    { x: 25, y: 45, size: '7px', delay: 4.1 },
    { x: 48, y: 150, size: '9px', delay: .3 },
    { x: 82, y: 205, size: '8px', delay: 1.9 },
    { x: 15, y: 95, size: '12px', delay: 3.9 },
    { x: 60, y: 60, size: '7px', delay: 2.4 },
    { x: 95, y: 30, size: '9px', delay: .8 },
    { x: 35, y: 175, size: '10px', delay: 4.5 },
    { x: 5, y: 215, size: '8px', delay: 3.1 },
    { x: 68, y: 120, size: '7px', delay: 1.6 },
    { x: 45, y: 195, size: '9px', delay: 4.8 },
  ];
  const compactSparkles = [
    { x: 8, y: 24, size: '7px', delay: 0 },
    { x: 31, y: 66, size: '6px', delay: .8 },
    { x: 56, y: 18, size: '8px', delay: 1.5 },
    { x: 79, y: 72, size: '6px', delay: 2.2 },
    { x: 94, y: 29, size: '7px', delay: 2.9 },
  ];
  const matrixColumns = Array.from({ length: 15 }, (_, column) => ({
    x: 3 + column * 6.75,
    delay: -((column * .41) % 3.6),
    duration: 3.2 + (column % 5) * .34,
    glyphs: Array.from({ length: 7 }, (_, row) => ({ column, row })),
  }));
  const matrixAlphabet = 'ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789アイウエオカキクケコサシスセソ'.split('');
  let matrixFrame = $state(0);

  function matrixGlyph(column: number, row: number): string {
    return matrixAlphabet[(column * 11 + row * 7 + matrixFrame * (row + 3)) % matrixAlphabet.length];
  }

  $effect(() => {
    if (!active || animation !== 'matrix' || typeof window === 'undefined') return;
    const timer = window.setInterval(() => {
      if (document.documentElement.dataset.motion !== 'off') matrixFrame += 1;
    }, 170);
    return () => window.clearInterval(timer);
  });
</script>

{#if active}<div class="sparkle-field" class:mirror class:contained class:compact class:pane data-effect={animation} aria-hidden="true">
  <div class="sparkle-glow"></div>
  {#if animation === 'sparkles'}
    {#each compact ? compactSparkles : sparkles as sparkle}<span class="sparkle" style:left={`${sparkle.x}%`} style:bottom={compact ? `${sparkle.y}%` : `${sparkle.y}px`} style:font-size={sparkle.size} style:animation-delay={`${sparkle.delay}s`}>✦</span>{/each}
  {:else if animation === 'grid'}
    <div class="pulse-grid"></div>
  {:else}
    <div class="matrix-stream">
      {#each matrixColumns as column}
        <span class="matrix-column" style:left={`${column.x}%`} style:--delay={`${column.delay}s`} style:--duration={`${column.duration}s`}>
          {#each column.glyphs as glyph}<i>{matrixGlyph(glyph.column, glyph.row)}</i>{/each}
        </span>
      {/each}
    </div>
  {/if}
</div>{/if}

<style>
  .sparkle-field { position:absolute; left:0; right:0; top:-80px; bottom:0; overflow:hidden; pointer-events:none; z-index:0; opacity:.5; }
  .sparkle-field.contained { inset:0; }
  .sparkle-field.compact { opacity:.65; }
  .sparkle-field.compact .sparkle-glow { background:linear-gradient(90deg,transparent,color-mix(in srgb,var(--accent) 12%,transparent),transparent); }
  .sparkle-field.pane { top:auto; height:40%; }
  .sparkle-field.mirror { transform:scaleY(-1); transform-origin:center; }
  :global([data-theme="dark"]) .sparkle-field { opacity:1; }
  @media (prefers-color-scheme:dark) { :global([data-theme="system"]) .sparkle-field { opacity:1; } }
  .sparkle-glow { position:absolute; inset:0; background:linear-gradient(to top, color-mix(in srgb, var(--accent) 20%, transparent), transparent); animation:sparkle-glow-enter .85s cubic-bezier(.2,.6,.2,1) both; }
  @keyframes sparkle-glow-enter { from { opacity:0; } to { opacity:1; } }

  .sparkle { position:absolute; line-height:1; color:var(--accent-ink); text-shadow:0 0 8px var(--accent); opacity:0; animation:sparkle-fade-up 3.4s ease-in infinite; }
  @keyframes sparkle-fade-up {
    0% { opacity:0; transform:translateY(14px) scale(.5); }
    15% { opacity:1; transform:translateY(0) scale(1); }
    45% { opacity:.85; transform:translateY(-4px) scale(1); }
    70%, 100% { opacity:0; transform:translateY(-18px) scale(.6); }
  }

  .pulse-grid { position:absolute; inset:0; opacity:.7; background-image:linear-gradient(color-mix(in srgb, var(--accent-ink) 43%, transparent) 1px, transparent 1px),linear-gradient(90deg, color-mix(in srgb, var(--accent-ink) 43%, transparent) 1px, transparent 1px); background-position:center bottom; background-size:18px 18px; mask-image:linear-gradient(to top, #000 0%, #000 43%, transparent 100%); animation:grid-pulse 2.8s ease-in-out infinite; }
  @keyframes grid-pulse {
    0%, 100% { opacity:.28; transform:scale(1); filter:drop-shadow(0 0 0 transparent); }
    50% { opacity:.82; transform:scale(1.035); filter:drop-shadow(0 0 5px var(--accent)); }
  }

  .matrix-stream { position:absolute; inset:0; mask-image:linear-gradient(to top, #000 0%, #000 56%, transparent 100%); font:600 10px/1.25 var(--mono, ui-monospace, monospace); color:var(--accent-ink); text-shadow:0 0 7px var(--accent); }
  .matrix-column { position:absolute; top:-9em; display:grid; justify-items:center; gap:.12em; opacity:0; animation:matrix-fall var(--duration) linear var(--delay) infinite; }
  .matrix-column i { display:block; min-width:1ch; font-style:normal; }
  .matrix-column i:first-child { color:var(--ink); text-shadow:0 0 9px var(--accent); }
  @keyframes matrix-fall {
    0% { opacity:0; transform:translateY(-10%); }
    8% { opacity:.88; }
    82% { opacity:.55; }
    100% { opacity:0; transform:translateY(270%); }
  }

  :global([data-motion="off"]) .sparkle,
  :global([data-motion="off"]) .pulse-grid,
  :global([data-motion="off"]) .matrix-column { animation:none; }
  :global([data-motion="off"]) .sparkle-glow { animation:none; }
  :global([data-motion="off"]) .sparkle { opacity:.55; transform:none; }
  :global([data-motion="off"]) .pulse-grid { opacity:.32; transform:none; filter:none; }
  :global([data-motion="off"]) .matrix-column { opacity:.38; transform:translateY(120%); }
  @media (prefers-reduced-motion:reduce) {
    .sparkle-glow { animation:none; }
    .sparkle,.pulse-grid,.matrix-column { animation:none; }
    .sparkle { opacity:.55; transform:none; }
    .pulse-grid { opacity:.32; transform:none; filter:none; }
    .matrix-column { opacity:.38; transform:translateY(120%); }
  }
</style>
