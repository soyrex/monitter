<script lang="ts">
  import { appTheme, appThemePreset, mixThemeColour } from '$lib/app-theme';
  import { surfaceTint } from '$lib/surface-tint';
  import AnimatedTitle from './AnimatedTitle.svelte';

  let { error = '', onretry }: { error?: string; onretry?: () => void } = $props();

  type AppearanceMode = 'light' | 'dark' | 'system';

  function storedAppearanceMode(): AppearanceMode {
    if (typeof window === 'undefined') return 'system';
    try {
      const saved = window.localStorage.getItem('monitter.appearance.mode.v1');
      return saved === 'light' || saved === 'dark' || saved === 'system' ? saved : 'system';
    } catch {
      return 'system';
    }
  }

  const mode = storedAppearanceMode();
  const light = $derived(appThemePreset($appTheme.light).light);
  const dark = $derived(appThemePreset($appTheme.dark).dark);
  const lightAccent = $derived($appTheme.accent ?? light.accent);
  const darkAccent = $derived($appTheme.accent ?? dark.accent);
  const themeStyle = $derived([
    `--loading-light-paper:${mixThemeColour(light.paper, lightAccent, ($surfaceTint / 100) * 0.5)}`,
    `--loading-light-ink:${light.ink}`,
    `--loading-light-muted:${light.muted}`,
    `--loading-light-accent:${lightAccent}`,
    `--loading-dark-paper:${mixThemeColour(dark.paper, darkAccent, $surfaceTint / 100)}`,
    `--loading-dark-ink:${dark.ink}`,
    `--loading-dark-muted:${dark.muted}`,
    `--loading-dark-accent:${darkAccent}`,
  ].join(';'));
</script>

<section
  class="workspace-loading-screen"
  data-mode={mode}
  style={themeStyle}
  aria-label="Preparing workspace"
  aria-busy="true"
  data-tauri-drag-region
>
  <div class="loading-lockup" data-tauri-drag-region>
    <img src="/monitter-wordmark.webp" alt="Monitter" draggable="false" />
    <p role="status"><AnimatedTitle text="Preparing workspace…" active activeTooltip="Preparing workspace" /></p>
    {#if error}
      <div class="loading-error" role="alert">
        <span>{error}</span>
        {#if onretry}<button type="button" onclick={onretry}>Try again</button>{/if}
      </div>
    {/if}
  </div>
</section>

<style>
  .workspace-loading-screen {
    --loading-paper: var(--loading-light-paper);
    --loading-ink: var(--loading-light-ink);
    --loading-muted: var(--loading-light-muted);
    --loading-accent: var(--loading-light-accent);
    position: fixed;
    z-index: 10000;
    inset: 0;
    display: grid;
    place-items: center;
    min-width: 0;
    min-height: 0;
    padding: max(32px, env(safe-area-inset-top)) 32px max(32px, env(safe-area-inset-bottom));
    overflow: hidden;
    color: var(--loading-ink);
    background: var(--loading-paper);
  }
  .workspace-loading-screen[data-mode='dark'] {
    --loading-paper: var(--loading-dark-paper);
    --loading-ink: var(--loading-dark-ink);
    --loading-muted: var(--loading-dark-muted);
    --loading-accent: var(--loading-dark-accent);
  }
  .loading-lockup {
    display: grid;
    justify-items: center;
    gap: 22px;
    width: min(360px, 82vw);
    text-align: center;
  }
  img {
    display: block;
    width: min(280px, 72vw);
    height: auto;
    user-select: none;
    filter: drop-shadow(0 10px 24px color-mix(in srgb, var(--loading-ink) 12%, transparent));
  }
  p {
    margin: 0;
    color: var(--loading-muted);
    font-size: 14px;
    font-weight: 600;
    letter-spacing: 0.01em;
  }
  p :global(.animated-title) {
    --accent: var(--loading-accent);
    --accent-ink: var(--loading-accent);
  }
  .loading-error {
    display: grid;
    justify-items: center;
    gap: 10px;
    max-width: 100%;
    color: var(--loading-muted);
    font-size: 12px;
    line-height: 1.45;
  }
  .loading-error span {
    max-width: 100%;
    overflow-wrap: anywhere;
  }
  .loading-error button {
    padding: 7px 12px;
    border: 1px solid color-mix(in srgb, var(--loading-accent) 50%, transparent);
    border-radius: 7px;
    color: var(--loading-ink);
    background: color-mix(in srgb, var(--loading-accent) 10%, var(--loading-paper));
    cursor: pointer;
  }
  @media (prefers-color-scheme: dark) {
    .workspace-loading-screen[data-mode='system'] {
      --loading-paper: var(--loading-dark-paper);
      --loading-ink: var(--loading-dark-ink);
      --loading-muted: var(--loading-dark-muted);
      --loading-accent: var(--loading-dark-accent);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    img { filter: none; }
  }
</style>
