<script lang="ts">
  import ArrowLeft from '@lucide/svelte/icons/arrow-left';
  import ChevronRight from '@lucide/svelte/icons/chevron-right';
  import Palette from '@lucide/svelte/icons/palette';
  import PanelsTopLeft from '@lucide/svelte/icons/panels-top-left';
  import MessageCircle from '@lucide/svelte/icons/message-circle';
  import type { Settings, SidebarView } from '$lib/types';
  import { interfaceScaleStore, setViewerInterfaceScale } from '$lib/interface-scale';
  import { SETTINGS_CATEGORIES, SETTINGS_CATEGORY_LABELS, SETTINGS_GROUPS, normalizeSettingsCategory, type SettingsCategoryId } from '$lib/settings-navigation';

  let { settings, workspaceView }: { settings: Settings; workspaceView: SidebarView } = $props();
  const mobileScale = interfaceScaleStore('mobile');
  const available = SETTINGS_CATEGORIES.filter(item => ['appearance', 'interface', 'conversation'].includes(item.id));
  const icons = { appearance: Palette, interface: PanelsTopLeft, conversation: MessageCircle };
  let selected = $state<SettingsCategoryId | null>(null);
  function openCategory(value: string) { selected = normalizeSettingsCategory(value); }
</script>

<div class="preferences" class:show-detail={selected !== null}>
  <nav class="categories" aria-label="Preference categories">
    <h1>Preferences</h1>
    <p class="intro">Personalise this viewer and review your desktop preferences.</p>
    {#each SETTINGS_GROUPS as group}
      {@const items = available.filter(item => item.group === group)}
      {#if items.length}
        <h2>{group}</h2>
        {#each items as item}
          {@const Icon = icons[item.id as keyof typeof icons]}
          <button class:active={selected === item.id} aria-current={selected === item.id ? 'page' : undefined} onclick={() => openCategory(item.id)}>
            <Icon size={20} aria-hidden="true" />
            <span><strong>{item.label}</strong><small>{item.id === 'interface' ? 'Mobile scale and desktop density' : item.id === 'conversation' ? 'Sending messages and workspace view' : 'Desktop theme and accent'}</small></span>
            <ChevronRight size={18} aria-hidden="true" />
          </button>
        {/each}
      {/if}
    {/each}
  </nav>
  {#if selected}
    <section class="detail" aria-label={SETTINGS_CATEGORY_LABELS[selected]}>
      <button class="back" onclick={() => selected = null}><ArrowLeft size={18} aria-hidden="true" />All preferences</button>
      <h1>{SETTINGS_CATEGORY_LABELS[selected]}</h1>
      {#if selected === 'interface'}
        <p class="intro">Scale saves on this device. Density comes from your desktop.</p>
        <section class="card" aria-labelledby="mobile-scale-title">
          <h2 id="mobile-scale-title">Interface scale</h2>
          <p>Resize text and controls for this mobile viewer.</p>
          <label for="mobile-preferences-scale">Mobile interface scale <output for="mobile-preferences-scale">{$mobileScale}%</output></label>
          <input id="mobile-preferences-scale" type="range" aria-label="Mobile interface scale" min="80" max="200" step="5" value={$mobileScale} oninput={event => setViewerInterfaceScale('mobile', event.currentTarget.value)} />
          <div class="range-footer"><span>80%</span><button onclick={() => setViewerInterfaceScale('mobile', 125)}>Reset · 125%</button><span>200%</span></div>
          <small>Saved on this device/browser for mobile viewers, independent of the connected desktop.</small>
        </section>
        <section class="card"><h2>Interface density</h2><p class="readonly">Read-only · change on desktop</p><dl><div><dt>Density</dt><dd>{settings.interfaceDensity ?? 'normal'}</dd></div></dl></section>
      {:else if selected === 'appearance'}
        <p class="intro">These appearance preferences come from your connected desktop.</p>
        <section class="card"><h2>Theme & colour</h2><p class="readonly">Read-only · change on desktop</p><dl><div><dt>Appearance mode</dt><dd>{settings.theme || 'system'}</dd></div><div><dt>Accent</dt><dd>{settings.accent || 'Default'}</dd></div></dl></section>
      {:else if selected === 'conversation'}
        <p class="intro">Review how this workspace sends messages and organises chats.</p>
        <section class="card"><h2>Sending messages</h2><p class="readonly">Read-only · change on desktop</p><p class="value">{settings.sendWithEnter ? 'Enter sends messages.' : 'Cmd/Ctrl+Enter sends messages.'}</p></section>
        <section class="card"><h2>Workspace view</h2><dl><div><dt>Current view</dt><dd>{workspaceView}</dd></div></dl><small>Stored on this device. Change it from the Workspace view buttons.</small></section>
      {/if}
    </section>
  {/if}
</div>

<style>
  .preferences{width:100%;min-width:0;max-width:960px;margin:auto;display:grid;gap:24px}.preferences *{box-sizing:border-box}h1{font-size:24px;line-height:1.2;margin:0;color:var(--ink)}h2{font-size:14px;line-height:1.4;margin:0;color:var(--ink)}p{font-size:13px;line-height:1.5;color:var(--soft);margin:0}.intro{margin:8px 0 20px}.categories>h2{font-size:11px;text-transform:uppercase;letter-spacing:.08em;color:var(--muted);margin:20px 0 8px}button{font:inherit;color:inherit;cursor:pointer;min-height:44px;border:0;background:transparent}.categories>button{width:100%;min-width:0;display:flex;align-items:center;gap:12px;padding:12px;border-radius:12px;text-align:left;margin:4px 0;background:var(--panel);border:1px solid var(--line)}.categories>button>span{flex:1;min-width:0}.categories :global(svg){flex:none;color:var(--muted)}strong{font-size:14px;font-weight:600}small{display:block;font-size:12px;line-height:1.5;color:var(--muted);overflow-wrap:anywhere}.categories small{margin-top:3px}.categories>button.active{border-color:var(--accent)}.detail{min-width:0}.back{display:flex;align-items:center;gap:8px;padding:0 0 12px;color:var(--accent);margin-bottom:12px}.card{padding:18px;background:var(--panel);border:1px solid var(--line);border-radius:14px;margin:14px 0;min-width:0}.card>h2+p{margin-top:6px}.readonly{color:var(--muted);font-size:12px}.card label{display:flex;justify-content:space-between;flex-wrap:wrap;gap:8px;font-size:13px;margin-top:20px;color:var(--ink)}output{font-weight:650}input[type=range]{display:block;min-height:44px;width:100%;margin:4px 0;padding:0;background:transparent;accent-color:var(--accent)}.range-footer{display:flex;align-items:center;justify-content:space-between;gap:8px;font-size:11px;color:var(--muted);margin-bottom:8px}.range-footer button{padding:0 8px;color:var(--accent);font-size:12px}dl{margin:16px 0 0;font-size:13px}dl>div{display:flex;justify-content:space-between;gap:16px;flex-wrap:wrap;margin:12px 0}dt{color:var(--muted)}dd{margin:0;color:var(--ink);overflow-wrap:anywhere;text-transform:capitalize}.value{margin-top:16px;color:var(--ink)}button:focus-visible,input:focus-visible{outline:2px solid var(--accent);outline-offset:3px}.show-detail .categories{display:none}@media(min-width:720px){.preferences.show-detail{grid-template-columns:240px minmax(0,1fr)}.show-detail .categories{display:block}.back{display:none}}@media(prefers-reduced-motion:reduce){*{scroll-behavior:auto}}
</style>
