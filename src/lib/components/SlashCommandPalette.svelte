<script lang="ts">
  import { Command, LoaderCircle } from '@lucide/svelte';
  import ProviderIcon from './ProviderIcon.svelte';
  import type { SlashCommand } from '$lib/types';

  export type SlashPaletteItem = SlashCommand & { id: string; label: string; detail: string };

  let {
    open,
    items,
    activeIndex = 0,
    loading = false,
    error = '',
    disabled = false,
    onselect,
  }: {
    open: boolean;
    items: SlashPaletteItem[];
    activeIndex?: number;
    loading?: boolean;
    error?: string;
    disabled?: boolean;
    onselect: (item?: SlashPaletteItem) => unknown;
  } = $props();

  const sourceLabel = (item: SlashPaletteItem) => item.source === 'monitter'
    ? 'Monitter'
    : item.source === 'codex'
      ? 'Codex'
      : 'ACP provider';
</script>

{#if open}
  <div class="slash-dock" data-open role="menu" aria-label="Available slash commands">
    <div class="slash-header"><span>Commands</span>{#if loading}<LoaderCircle class="spin" size={12}/>{/if}</div>
    <div class="slash-options">
      {#each items as item, index (item.id)}
        <button
          type="button"
          role="menuitem"
          class:active={index === activeIndex}
          aria-label={`${item.label}, ${sourceLabel(item)}. ${item.detail}`}
          {disabled}
          onclick={() => onselect(item)}
        >
          <span class="source-icon" title={sourceLabel(item)}>
            {#if item.source === 'monitter'}<Command size={14} aria-hidden="true"/>
            {:else}<ProviderIcon provider={item.provider} size={14}/>{/if}
          </span>
          <b>{item.label}</b>
          <span class="description">{item.detail}</span>
          <small>{sourceLabel(item)}</small>
        </button>
      {:else}
        {#if !loading}<p>{error || 'No matching commands.'}</p>{/if}
      {/each}
    </div>
    {#if error && items.length}<p class="slash-error" role="status">{error}</p>{/if}
  </div>
{/if}

<style>
  .slash-dock {
    position: relative;
    z-index: 0;
    width: min(calc(var(--chat-content-max-width, 900px) - 10px), calc(100% - (2 * var(--density-composer-margin-inline, 12px)) - 10px));
    max-height: min(270px, 42vh);
    box-sizing: border-box;
    margin: 0 auto -10px;
    overflow: hidden;
    border: 1px solid var(--line);
    border-bottom-left-radius: 0;
    border-bottom-right-radius: 0;
    border-radius: 10px 10px 0 0;
    color: var(--ink);
    background: color-mix(in srgb, var(--panel) 94%, transparent);
    box-shadow: 0 -12px 30px #0002;
    animation: slash-rise 150ms cubic-bezier(.2,.75,.25,1) both;
  }
  .slash-header { display:flex; align-items:center; gap:7px; padding:9px 11px 7px; color:var(--muted); font:calc(10px * var(--interface-font-ratio,1)) var(--mono); text-transform:uppercase; letter-spacing:.06em; }
  .slash-header :global(svg) { margin-left:auto; }
  .slash-options { max-height:min(225px,34vh); overflow-y:auto; overscroll-behavior:contain; padding-bottom:10px; }
  button { width:100%; min-height:42px; display:grid; grid-template-columns:18px minmax(76px,auto) minmax(0,1fr) auto; align-items:center; gap:9px; padding:7px 11px; border-radius:0; text-align:left; }
  button:hover, button.active { background:color-mix(in srgb,var(--accent) 13%,transparent); }
  .source-icon { display:grid; width:18px; height:18px; place-items:center; color:var(--ink); }
  b { font:calc(12px * var(--interface-font-ratio,1)) var(--mono); font-weight:600; white-space:nowrap; }
  .description { min-width:0; overflow:hidden; color:var(--muted); font-size:calc(12px * var(--interface-font-ratio,1)); text-overflow:ellipsis; white-space:nowrap; }
  small { color:var(--muted); font:calc(9px * var(--interface-font-ratio,1)) var(--mono); text-transform:uppercase; letter-spacing:.04em; white-space:nowrap; }
  p { margin:0; padding:9px 11px 18px; color:var(--muted); font-size:calc(12px * var(--interface-font-ratio,1)); }
  .slash-error { padding-top:5px; color:var(--danger,#bd655b); }
  :global(.spin) { animation:spin .8s linear infinite; }
  @keyframes spin { to { transform:rotate(360deg); } }
  @keyframes slash-rise { from { opacity:0; transform:translateY(12px); } to { opacity:1; transform:translateY(0); } }
  @media (prefers-reduced-motion:reduce) { .slash-dock,:global(.spin) { animation:none; } }
  @container (width < 560px) { button { grid-template-columns:18px minmax(70px,auto) minmax(0,1fr); } button small { display:none; } }
</style>
