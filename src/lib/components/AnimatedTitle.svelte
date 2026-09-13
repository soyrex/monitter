<script lang="ts">
  let { text, active = false, activeTooltip = 'Generating a title…' }: { text: string; active?: boolean; activeTooltip?: string } = $props();
  const sparkles = [
    { x: 20, y: -15, size: '.65em', delay: 0 },
    { x: 88, y: 80, size: '.65em', delay: .65 },
    { x: 5, y: 55, size: 'calc(.52em + .2px)', delay: .2 },
    { x: 36, y: 85, size: 'calc(.455em + .3px)', delay: .9 },
    { x: 55, y: 0, size: 'calc(.39em + .4px)', delay: .4 },
    { x: 72, y: 65, size: 'calc(.325em + .5px)', delay: 1.1 },
    { x: 96, y: 15, size: 'calc(.26em + .6px)', delay: .55 },
    { x: 44, y: 35, size: 'calc(.195em + .7px)', delay: 1.25 },
    { x: 12, y: 90, size: 'calc(.13em + .8px)', delay: .75 },
    { x: 64, y: 90, size: '1px', delay: .3 },
  ];
</script>
<span class="animated-title" class:naming={active} aria-busy={active} title={active ? activeTooltip : text}>{text}{#if active}{#each sparkles as sparkle}<span class="title-sparkle" aria-hidden="true" style:left={`${sparkle.x}%`} style:top={`${sparkle.y}%`} style:font-size={sparkle.size} style:animation-delay={`${sparkle.delay}s`}>✦</span>{/each}{/if}</span>
<style>
  .animated-title { position:relative; }
  .naming { color:var(--accent-ink); animation:title-glow 1.6s ease-in-out infinite; }
  .title-sparkle { position:absolute; pointer-events:none; line-height:1; color:var(--accent-ink); text-shadow:0 0 8px var(--accent); animation:title-twinkle 1.3s ease-in-out infinite; }
  @keyframes title-glow { 50% { opacity:.6; } }
  @keyframes title-twinkle { 0%,100% { opacity:.15; transform:scale(.45) rotate(-15deg); } 50% { opacity:1; transform:scale(1) rotate(15deg); } }
  @media(prefers-reduced-motion:reduce) { .naming,.title-sparkle { animation:none; } .title-sparkle { opacity:.5; } }
</style>
