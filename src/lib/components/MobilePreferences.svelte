<script lang="ts">
  import { tick, onDestroy } from 'svelte';
  import Search from '@lucide/svelte/icons/search';
  import X from '@lucide/svelte/icons/x';
  import ArrowLeft from '@lucide/svelte/icons/arrow-left';
  import ChevronRight from '@lucide/svelte/icons/chevron-right';
  import Palette from '@lucide/svelte/icons/palette';
  import PanelsTopLeft from '@lucide/svelte/icons/panels-top-left';
  import MessageCircle from '@lucide/svelte/icons/message-circle';
  import type { Settings, SidebarView } from '$lib/types';
  import { interfaceScaleStore, setViewerInterfaceScale } from '$lib/interface-scale';
  import { SETTINGS_CATEGORIES, SETTINGS_CATEGORY_LABELS, SETTINGS_GROUPS, normalizeSettingsCategory, type SettingsCategoryId } from '$lib/settings-navigation';
  import { searchSettings, type SettingsSearchEntry } from '$lib/settings-search';

  let { settings, workspaceView }: { settings: Settings; workspaceView: SidebarView } = $props();
  const mobileScale = interfaceScaleStore('mobile');
  const available = SETTINGS_CATEGORIES.filter(item => ['appearance', 'interface', 'conversation'].includes(item.id));
  const icons = { appearance: Palette, interface: PanelsTopLeft, conversation: MessageCircle };
  let selected = $state<SettingsCategoryId | null>(null);
  // The companion can only search settings it can actually display. Values from
  // the paired desktop never enter this public, static index.
  const searchEntries: SettingsSearchEntry[] = [
    { id: 'mobile-scale', category: 'interface', label: 'Mobile interface scale', description: 'Resize text and controls on this device.', keywords: ['size', 'zoom', 'layout', 'panes'], target: 'mobile-scale-title' },
    { id: 'density', category: 'interface', label: 'Interface density', description: 'Review your desktop spacing preference.', keywords: ['layout', 'panes', 'compact', 'spacious'], target: 'mobile-density-title' },
    { id: 'theme', category: 'appearance', label: 'Appearance mode', description: 'Review the theme from your desktop.', keywords: ['light', 'dark', 'system', 'theme'], target: 'mobile-theme-title' },
    { id: 'accent', category: 'appearance', label: 'Accent colour', description: 'Review the accent from your desktop.', keywords: ['color', 'theme'], target: 'mobile-theme-title' },
    { id: 'sending', category: 'conversation', label: 'Sending messages', description: 'Review Enter and Cmd/Ctrl+Enter behavior.', keywords: ['keyboard', 'send', 'shortcut'], target: 'mobile-sending-title' },
    { id: 'workspace-view', category: 'conversation', label: 'Workspace view', description: 'Review the view saved on this device.', keywords: ['chats', 'activity', 'standard'], target: 'mobile-view-title' },
  ];
  let searchQuery = $state('');
  let activeResult = $state(0);
  let searchInput = $state<HTMLInputElement>();
  let resultsElement = $state<HTMLDivElement>();
  let preferencesElement = $state<HTMLDivElement>();
  let highlightTarget = $state('');
  let highlightTimer: ReturnType<typeof setTimeout> | undefined;
  const searchResults = $derived(searchSettings(searchQuery, searchEntries));
  const searching = $derived(searchQuery.trim().length > 0);
  onDestroy(() => clearTimeout(highlightTimer));
  function clearSearch() { searchQuery = ''; activeResult = 0; searchInput?.focus(); }
  function openCategory(value: string) { selected = normalizeSettingsCategory(value); highlightTarget = ''; }
  async function openResult(result: SettingsSearchEntry) {
    selected = normalizeSettingsCategory(result.category);
    searchQuery = '';
    activeResult = 0;
    highlightTarget = result.target ?? '';
    clearTimeout(highlightTimer);
    await tick();
    const heading = preferencesElement?.querySelector<HTMLElement>(`#${result.target}`);
    heading?.scrollIntoView({ block: 'nearest', behavior: 'instant' });
    heading?.focus({ preventScroll: true });
    highlightTimer = setTimeout(() => highlightTarget = '', 1800);
  }
  async function searchKeydown(event: KeyboardEvent) {
    if (event.isComposing) return;
    if (event.key === 'Escape' && searching) { event.preventDefault(); event.stopPropagation(); clearSearch(); }
    else if (searching && (event.key === 'ArrowDown' || event.key === 'ArrowUp') && searchResults.length) {
      event.preventDefault(); event.stopPropagation();
      activeResult = (activeResult + (event.key === 'ArrowDown' ? 1 : -1) + searchResults.length) % searchResults.length;
      await tick();
      resultsElement?.querySelectorAll('[role="option"]')[activeResult]?.scrollIntoView({ block: 'nearest' });
    } else if (searching && event.key === 'Enter' && searchResults[activeResult]) {
      event.preventDefault(); event.stopPropagation(); void openResult(searchResults[activeResult]);
    }
  }
</script>

<div bind:this={preferencesElement} class="preferences" class:show-detail={selected !== null}>
  <div class="preferences-search">
    <div class="search-field">
      <Search size={18} aria-hidden="true" />
      <input bind:this={searchInput} type="search" role="combobox" aria-label="Search preferences" placeholder="Search preferences" autocomplete="off" autocapitalize="none" spellcheck="false" aria-autocomplete="list" aria-expanded={searching} aria-controls={searching ? 'mobile-preferences-search-results' : undefined} aria-activedescendant={searching && searchResults[activeResult] ? `mobile-preference-result-${searchResults[activeResult].id}` : undefined} value={searchQuery} oninput={event => { searchQuery = event.currentTarget.value; activeResult = 0; }} onkeydown={searchKeydown} />
      {#if searchQuery}<button type="button" aria-label="Clear preferences search" onclick={clearSearch}><X size={18} aria-hidden="true" /></button>{/if}
    </div>
    {#if searching}
      <p class="search-count" role="status">{searchResults.length ? `${searchResults.length} matching ${searchResults.length === 1 ? 'setting' : 'settings'}` : 'No matching settings. Try another word.'}</p>
      <div bind:this={resultsElement} id="mobile-preferences-search-results" class="search-results" role="listbox" aria-label="Matching preferences">
        {#each searchResults as result, index}
          <button id={`mobile-preference-result-${result.id}`} type="button" role="option" aria-selected={index === activeResult} class:active={index === activeResult} onclick={() => openResult(result)}>
            <strong>{result.label}</strong><small>{SETTINGS_CATEGORY_LABELS[result.category as SettingsCategoryId]} · {result.description}</small>
          </button>
        {/each}
      </div>
    {/if}
  </div>
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
          <h2 id="mobile-scale-title" tabindex="-1" class:search-highlight={highlightTarget === 'mobile-scale-title'}>Interface scale</h2>
          <p>Resize text and controls for this mobile viewer.</p>
          <label for="mobile-preferences-scale">Mobile interface scale <output for="mobile-preferences-scale">{$mobileScale}%</output></label>
          <input id="mobile-preferences-scale" type="range" aria-label="Mobile interface scale" min="80" max="200" step="5" value={$mobileScale} oninput={event => setViewerInterfaceScale('mobile', event.currentTarget.value)} />
          <div class="range-footer"><span>80%</span><button onclick={() => setViewerInterfaceScale('mobile', 125)}>Reset · 125%</button><span>200%</span></div>
          <small>Saved on this device/browser for mobile viewers, independent of the connected desktop.</small>
        </section>
        <section class="card"><h2 id="mobile-density-title" tabindex="-1" class:search-highlight={highlightTarget === 'mobile-density-title'}>Interface density</h2><p class="readonly">Read-only · change on desktop</p><dl><div><dt>Density</dt><dd>{settings.interfaceDensity ?? 'normal'}</dd></div></dl></section>
      {:else if selected === 'appearance'}
        <p class="intro">These appearance preferences come from your connected desktop.</p>
        <section class="card"><h2 id="mobile-theme-title" tabindex="-1" class:search-highlight={highlightTarget === 'mobile-theme-title'}>Theme & colour</h2><p class="readonly">Read-only · change on desktop</p><dl><div><dt>Appearance mode</dt><dd>{settings.theme || 'system'}</dd></div><div><dt>Accent</dt><dd>{settings.accent || 'Default'}</dd></div></dl></section>
      {:else if selected === 'conversation'}
        <p class="intro">Review how this workspace sends messages and organises chats.</p>
        <section class="card"><h2 id="mobile-sending-title" tabindex="-1" class:search-highlight={highlightTarget === 'mobile-sending-title'}>Sending messages</h2><p class="readonly">Read-only · change on desktop</p><p class="value">{settings.sendWithEnter ? 'Enter sends messages.' : 'Cmd/Ctrl+Enter sends messages.'}</p></section>
        <section class="card"><h2 id="mobile-view-title" tabindex="-1" class:search-highlight={highlightTarget === 'mobile-view-title'}>Workspace view</h2><dl><div><dt>Current view</dt><dd>{workspaceView}</dd></div></dl><small>Stored on this device. Change it from the Workspace view buttons.</small></section>
      {/if}
    </section>
  {/if}
</div>

<style>
  .preferences-search{grid-column:1 / -1;min-width:0}.search-field{display:flex;align-items:center;gap:8px;padding:0 12px;min-height:44px;border:1px solid var(--line);border-radius:10px;background:var(--panel);color:var(--muted)}.search-field:focus-within{outline:2px solid var(--accent);outline-offset:2px}.search-field :global(svg){flex:none}.search-field input{flex:1;min-width:0;width:100%;min-height:44px;border:0;background:transparent;outline:none;color:var(--ink);font:inherit;font-size:14px}.search-field input:focus-visible{outline:none}.search-field input::-webkit-search-cancel-button{display:none}.search-field button{display:flex;align-items:center;justify-content:center;width:44px;flex:none;padding:0}.search-count{margin:12px 0 6px}.search-results{display:grid;gap:4px;max-height:50vh;overflow:auto}.search-results button{display:grid;gap:4px;text-align:left;padding:12px;border-radius:8px;overflow-wrap:anywhere}.search-results button.active,.search-results button:hover{background:color-mix(in srgb,var(--accent) 12%,var(--panel))}.search-highlight{outline:2px solid var(--accent);outline-offset:6px;border-radius:2px}
  .preferences{width:100%;min-width:0;max-width:960px;margin:auto;display:grid;gap:24px}.preferences *{box-sizing:border-box}h1{font-size:24px;line-height:1.2;margin:0;color:var(--ink)}h2{font-size:14px;line-height:1.4;margin:0;color:var(--ink)}p{font-size:13px;line-height:1.5;color:var(--soft);margin:0}.intro{margin:8px 0 20px}.categories>h2{font-size:11px;text-transform:uppercase;letter-spacing:.08em;color:var(--muted);margin:20px 0 8px}button{font:inherit;color:inherit;cursor:pointer;min-height:44px;border:0;background:transparent}.categories>button{width:100%;min-width:0;display:flex;align-items:center;gap:12px;padding:12px;border-radius:12px;text-align:left;margin:4px 0;background:var(--panel);border:1px solid var(--line)}.categories>button>span{flex:1;min-width:0}.categories :global(svg){flex:none;color:var(--muted)}strong{font-size:14px;font-weight:600}small{display:block;font-size:12px;line-height:1.5;color:var(--muted);overflow-wrap:anywhere}.categories small{margin-top:3px}.categories>button.active{border-color:var(--accent)}.detail{min-width:0}.back{display:flex;align-items:center;gap:8px;padding:0 0 12px;color:var(--accent);margin-bottom:12px}.card{padding:18px;background:var(--panel);border:1px solid var(--line);border-radius:14px;margin:14px 0;min-width:0}.card>h2+p{margin-top:6px}.readonly{color:var(--muted);font-size:12px}.card label{display:flex;justify-content:space-between;flex-wrap:wrap;gap:8px;font-size:13px;margin-top:20px;color:var(--ink)}output{font-weight:650}input[type=range]{display:block;min-height:44px;width:100%;margin:4px 0;padding:0;background:transparent;accent-color:var(--accent)}.range-footer{display:flex;align-items:center;justify-content:space-between;gap:8px;font-size:11px;color:var(--muted);margin-bottom:8px}.range-footer button{padding:0 8px;color:var(--accent);font-size:12px}dl{margin:16px 0 0;font-size:13px}dl>div{display:flex;justify-content:space-between;gap:16px;flex-wrap:wrap;margin:12px 0}dt{color:var(--muted)}dd{margin:0;color:var(--ink);overflow-wrap:anywhere;text-transform:capitalize}.value{margin-top:16px;color:var(--ink)}button:focus-visible,input:focus-visible{outline:2px solid var(--accent);outline-offset:3px}.show-detail .categories{display:none}@media(min-width:720px){.preferences.show-detail{grid-template-columns:240px minmax(0,1fr)}.show-detail .categories{display:block}.back{display:none}}@media(prefers-reduced-motion:reduce){*{scroll-behavior:auto}}
</style>
