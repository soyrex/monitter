<script lang="ts">
  import type { Provider } from '$lib/types';
  import codex from '@lobehub/icons-static-svg/icons/codex-color.svg?raw';
  import claude from '@lobehub/icons-static-svg/icons/claude-color.svg?raw';
  import minimax from '@lobehub/icons-static-svg/icons/minimax-color.svg?raw';
  import opencode from '@lobehub/icons-static-svg/icons/opencode.svg?raw';
  import hermes from '@lobehub/icons-static-svg/icons/hermesagent.svg?raw';
  import openai from '@lobehub/icons-static-svg/icons/openai.svg?raw';
  import deepseek from '@lobehub/icons-static-svg/icons/deepseek-color.svg?raw';
  import gemini from '@lobehub/icons-static-svg/icons/gemini-color.svg?raw';
  import glm from '@lobehub/icons-static-svg/icons/glmv-color.svg?raw';
  import grok from '@lobehub/icons-static-svg/icons/grok.svg?raw';
  import kimi from '@lobehub/icons-static-svg/icons/kimi-color.svg?raw';
  import mistral from '@lobehub/icons-static-svg/icons/mistral-color.svg?raw';
  import qwen from '@lobehub/icons-static-svg/icons/qwen-color.svg?raw';
  import { Bot } from '@lucide/svelte';

  export type ProviderIconId = Provider | 'minimax' | 'opencode-go';
  type IconProps = { provider: ProviderIconId; model?: string; size?: number; class?: string; title?: string };

  const icons: Partial<Record<ProviderIconId, string>> = { codex, claude, minimax, opencode, hermes, 'opencode-go': opencode };
  let { provider, model, size = 16, class: className = '', title }: IconProps = $props();
  function modelIcon(value: string | undefined): string | null {
    const name = value?.toLowerCase() ?? '';
    if (!name) return null;
    if (/(?:^|[\W_])claude(?:$|[\W_])/.test(name)) return claude;
    if (/(?:^|[\W_])(?:gpt|openai)(?:$|[\W_])/.test(name) || /(?:^|\/)o[134](?:$|[.\-/])/.test(name)) return openai;
    if (/(?:^|[\W_])(?:minimax|mmx)(?:$|[\W_])/.test(name)) return minimax;
    if (/(?:^|[\W_])(?:kimi|moonshot)(?:$|[\W_])/.test(name)) return kimi;
    if (/(?:^|[\W_])deepseek(?:$|[\W_])/.test(name)) return deepseek;
    if (/(?:^|[\W_])qwen(?:$|[\W_])/.test(name)) return qwen;
    if (/(?:^|[\W_])(?:glm|zhipu)(?:$|[\W_])/.test(name)) return glm;
    if (/(?:^|[\W_])gemini(?:$|[\W_])/.test(name)) return gemini;
    if (/(?:^|[\W_])grok(?:$|[\W_])/.test(name)) return grok;
    if (/(?:^|[\W_])mistral(?:$|[\W_])/.test(name)) return mistral;
    return null;
  }
  const markup = $derived(modelIcon(model) ?? icons[provider] ?? '');
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
