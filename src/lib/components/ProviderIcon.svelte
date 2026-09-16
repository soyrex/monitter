<script lang="ts">
  import type { Provider } from '$lib/types';
  import codex from '@lobehub/icons-static-svg/icons/codex-color.svg?raw';
  import claude from '@lobehub/icons-static-svg/icons/claude-color.svg?raw';
  import minimax from '@lobehub/icons-static-svg/icons/minimax-color.svg?raw';
  import opencode from '@lobehub/icons-static-svg/icons/opencode.svg?raw';
  import hermes from '@lobehub/icons-static-svg/icons/hermesagent.svg?raw';
  import { Bot } from '@lucide/svelte';

  export type ProviderIconId = Provider | 'minimax' | 'opencode-go';
  type IconProps = { provider: ProviderIconId; size?: number; class?: string; title?: string };

  const icons: Partial<Record<ProviderIconId, string>> = { codex, claude, minimax, opencode, hermes, 'opencode-go': opencode };
  let { provider, size = 16, class: className = '', title }: IconProps = $props();
  const markup = $derived(icons[provider] ?? '');
</script>

<span
  class={`provider-icon ${className}`.trim()}
  style={`--provider-icon-size:${size}px`}
  role={title ? 'img' : undefined}
  aria-label={title}
  aria-hidden={title ? undefined : 'true'}
  title={title}
>
  {#if markup}
    {@html markup}
  {:else}
    <Bot size={size} strokeWidth={1.8} aria-hidden="true" />
  {/if}
</span>

<style>
  .provider-icon { display:inline-grid; width:var(--provider-icon-size); height:var(--provider-icon-size); flex:none; place-items:center; color:var(--ink); line-height:0; }
  .provider-icon :global(svg) { display:block; width:100%; height:100%; }
</style>
