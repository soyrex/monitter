<script lang="ts">
  import { rangeFill } from '$lib/range-fill';
  import { Bot, Users, Check, ChevronDown, LoaderCircle, MessageSquare, Palette, ShieldCheck, Smartphone, Type, UserRound, Blocks } from "@lucide/svelte";
  import type { Agent, ApprovalRule, Host, Settings } from "$lib/types";
  import { getBridge } from '$lib/bridge';
  import { surfaceTint, setSurfaceTint, DEFAULT_SURFACE_TINT } from '$lib/surface-tint';
  import { borderOpacity, setBorderOpacity, DEFAULT_BORDER_OPACITY } from '$lib/border-opacity';
  import { motionPreference, motionView, setMotionPreference, type MotionPreference } from '$lib/motion';
  import { setTerminalTheme, terminalTheme, terminalThemes, type TerminalThemeId } from '$lib/terminal-theme';
  import { appTheme, appThemePreset, appThemes, resetAppTheme, setAppTheme, setAppThemeAccent, type AppThemeId } from '$lib/app-theme';
  import { isLanBrowser } from '$lib/lan';
  import { remoteControlOpen, setRemoteControlTarget } from '$lib/workspace-panels';
  import LanSettings from './LanSettings.svelte';
  import SavedApprovalRules from './SavedApprovalRules.svelte';
  import ExtensionsSettings from './ExtensionsSettings.svelte';

  type Category = "profile" | "appearance" | "typography" | "behaviour" | "conversation" | "approvals" | "agents" | "directory" | "lan" | "remote" | "extensions";
  type FontKey = "interfaceFont" | "chatFont" | "terminalFont";
  type FontSizeKey = "interfaceFontSize" | "chatFontSize" | "terminalFontSize";
  type LineHeightKey = "chatLineHeight" | "terminalLineHeight";

  let {
    settings,
    approvalRules = [],
    agents = [],
    hosts = [],
    revokingRuleId = null,
    onrevokeRule,
    agentEditor,
    agentDirectory,
    headerActions,
    onsave,
    interfaceScale,
    interfaceScaleViewer,
    onscale,
    category = $bindable<string>("appearance"),
    visible = true,
    active = true,
  }: {
    settings: Settings;
    approvalRules?: ApprovalRule[];
    agents?: Agent[];
    hosts?: Host[];
    revokingRuleId?: string | null;
    onrevokeRule?: (rule: ApprovalRule) => void;
    headerActions?: import("svelte").Snippet;
    agentDirectory?: import("svelte").Snippet;
    agentEditor?: import("svelte").Snippet;
    onsave: (patch: Partial<Settings>) => Promise<void>;
    interfaceScale?: number;
    interfaceScaleViewer?: 'desktop' | 'mobile';
    onscale?: (scale: number) => void | Promise<void>;
    category?: string;
    visible?: boolean;
    active?: boolean;
  } = $props();

  const accents = ["#00a8f0", "#3978d4", "#8755c7", "#c44c79", "#c27524"];
  const remoteControlAvailable = !isLanBrowser() && getBridge().available;
  const categories: { id: Category; label: string; detail: string; icon: typeof Palette }[] = [
    { id: "profile", label: "Profile", detail: "Your name for new chats", icon: UserRound },
    { id: "lan", label: "LAN access", detail: "Open Monitter in a browser", icon: ShieldCheck },
    ...(remoteControlAvailable ? [{ id: "remote" as const, label: "Remote control", detail: "Pair this desktop with your phone", icon: Smartphone }] : []),
    { id: "directory", label: "Agent directory", detail: "Discover skills and responsibilities", icon: Users },
    { id: "agents", label: "Agents", detail: "Identity, harness and skills", icon: Bot },
    { id: "extensions", label: "MCP & Plugins", detail: "Servers, skills and safety", icon: Blocks },
    { id: "appearance", label: "Appearance", detail: "Theme, accent and panes", icon: Palette },
    { id: "typography", label: "Typography", detail: "Fonts and base sizes", icon: Type },
    { id: "behaviour", label: "Permissions & behaviour", detail: "Focus and busy messages", icon: ShieldCheck },
    { id: "approvals", label: "Approvals", detail: "Saved always-approve rules", icon: ShieldCheck },
    { id: "conversation", label: "Conversation", detail: "Messages and activity", icon: MessageSquare },
  ];
  const fonts: { key: FontKey; sizeKey: FontSizeKey; size: number; label: string; fallback: string }[] = [
    { key: "interfaceFont", sizeKey: "interfaceFontSize", size: 14, label: "Interface font", fallback: "IBM Plex Sans" },
    { key: "chatFont", sizeKey: "chatFontSize", size: 13, label: "Chat font", fallback: "IBM Plex Sans" },
    { key: "terminalFont", sizeKey: "terminalFontSize", size: 14, label: "Terminal font", fallback: "IBM Plex Mono" },
  ];
  const activeCategory = $derived(categories.some((item) => item.id === category) ? category as Category : "appearance");
  const densities = ['tight', 'normal', 'spacious'] as const;
  const density = $derived(settings.interfaceDensity ?? 'normal');
  const displayedInterfaceScale = $derived(interfaceScale ?? settings.interfaceScale ?? 125);
  const selectedTerminalTheme = $derived(terminalThemes.find(option=>option.id === $terminalTheme) ?? terminalThemes[0]);
  const selectedLightTheme = $derived(appThemePreset($appTheme.light));
  const selectedDarkTheme = $derived(appThemePreset($appTheme.dark));
  const advancedAccent = $derived($appTheme.accent ?? selectedLightTheme.light.accent);

  let pending = $state(0);
  let saveError = $state("");
  let userNameError = $state("");
  let savedAt = $state(0);
  let remoteControlHost = $state<HTMLDivElement | null>(null);

  // Register only the visible Remote-control category. The route-root controller
  // remains mounted when this target changes, so remembered phones keep reconnecting.
  $effect(() => {
    if (!visible || activeCategory !== 'remote' || !remoteControlHost) return;
    remoteControlOpen.set(true);
    return setRemoteControlTarget(remoteControlHost, active);
  });

  async function save(patch: Partial<Settings>) {
    pending += 1;
    saveError = "";
    try {
      await onsave(patch);
      savedAt = Date.now();
    } catch (reason) {
      saveError = reason instanceof Error ? reason.message : String(reason);
    } finally {
      pending -= 1;
    }
  }

  async function saveInterfaceScale(value: number) {
    if (!onscale) { await save({ interfaceScale: value }); return; }
    pending += 1;
    saveError = '';
    try { await onscale(value); savedAt = Date.now(); }
    catch (reason) { saveError = reason instanceof Error ? reason.message : String(reason); }
    finally { pending -= 1; }
  }

  function saveNumber(event: Event, key: FontSizeKey, fallback: number) {
    const input = event.currentTarget as HTMLInputElement;
    if (input.checkValidity()) void save({ [key]: Number(input.value || fallback) });
  }

  function saveLineHeight(event: Event, key: LineHeightKey, fallback: number) {
    const input = event.currentTarget as HTMLInputElement;
    if (input.checkValidity()) void save({ [key]: Number(input.value || fallback) });
  }

  function checkedUserName(value: string) {
    const name = value.trim();
    if (/[\u0000-\u001F\u007F-\u009F]/.test(name)) return { name, error: "Your name cannot contain control characters." };
    if (Array.from(name).length > 80) return { name, error: "Your name can be at most 80 characters." };
    return { name, error: "" };
  }

  function editUserName(event: Event) {
    const input = event.currentTarget as HTMLInputElement;
    const checked = checkedUserName(input.value);
    userNameError = checked.error;
    input.setCustomValidity(checked.error);
  }

  function saveUserName(event: Event) {
    const input = event.currentTarget as HTMLInputElement;
    const checked = checkedUserName(input.value);
    userNameError = checked.error;
    input.setCustomValidity(checked.error);
    if (checked.error) return;
    void save({ userName: checked.name });
  }

  function chooseAppTheme(mode: 'light' | 'dark', id: AppThemeId, event: MouseEvent) {
    setAppTheme(mode, id);
    const menu = (event.currentTarget as HTMLElement).closest('details');
    if (menu) menu.open = false;
  }
</script>

<section class="settings-pane" aria-label="Settings">
  <nav class="settings-nav" aria-label="Settings categories">
    <div class="nav-heading">Settings</div>
    {#each categories as item}
      <button
        type="button"
        class:active={activeCategory === item.id}
        aria-current={activeCategory === item.id ? "page" : undefined}
        aria-label={item.label}
        title={item.label}
        onclick={() => (category = item.id)}
      >
        <item.icon size={16} />
        <span><strong>{item.label}</strong><small>{item.detail}</small></span>
      </button>
    {/each}
  </nav>

  <div class="settings-content">
    <header class="settings-header">
      <div>
        <p>Preferences</p>
        <h1>{categories.find((item) => item.id === activeCategory)?.label}</h1>
      </div>
      <div class="save-state" aria-live="polite">
        {#if activeCategory === "lan"}<span>Local network</span>
        {:else if activeCategory === "remote"}<span>Desktop pairing</span>
        {:else if activeCategory === "directory"}<span>Browse available agents</span>
      {:else if activeCategory === "extensions"}<span>Applies on next harness launch</span>
      {:else if activeCategory === "agents"}Save changes with Save agent
        {:else if activeCategory === "approvals"}<span>Rules update immediately</span>
        {:else if pending > 0}<LoaderCircle class="spin" size={14} /> Saving…
        {:else if saveError}<span class="save-error">Could not save</span>
        {:else if savedAt}<Check size={14} /> Saved{:else}Changes save automatically{/if}
      </div>
      {#if headerActions}{@render headerActions()}{/if}
    </header>

    {#if saveError}
      <p class="error" role="alert">{saveError}</p>
    {/if}

    <div class="settings-category" use:motionView={{ key: activeCategory, y: 4, duration: 150, opacity: .9, enabled: visible && active }}>
    {#if activeCategory === "lan"}<LanSettings />
    {:else if activeCategory === "remote"}
      <div class="section-stack">
        <section class="setting-card" aria-labelledby="remote-control-heading">
          <div class="card-heading"><h2 id="remote-control-heading">Pair a phone</h2><p>Create a one-time pairing code from this desktop. The listener and remembered phones remain active while you use another Settings category or close Settings.</p></div>
          <div class="remote-control-host" bind:this={remoteControlHost}></div>
        </section>
      </div>
    {:else if activeCategory === "directory"}
      <div class="section-stack">{#if agentDirectory}{@render agentDirectory()}{/if}</div>
    {:else if activeCategory === "agents"}
      <div class="section-stack">{#if agentEditor}{@render agentEditor()}{/if}</div>
    {:else if activeCategory === "extensions"}
      <ExtensionsSettings {agents}/>
    {:else if activeCategory === "approvals"}
      <div class="section-stack" data-approval-rules-settings>
        <section class="setting-card" aria-labelledby="saved-approvals-heading">
          <div class="card-heading"><h2 id="saved-approvals-heading">Saved approvals</h2><p>These rules only cover the exact agent, host, folder and launch identity that created them. Revoking a rule affects future requests only; work already approved keeps running.</p></div>
          <SavedApprovalRules rules={approvalRules} {agents} {hosts} onrevoke={onrevokeRule} revokingId={revokingRuleId}/>
        </section>
      </div>
    {:else if activeCategory === "profile"}
      <div class="section-stack">
        <section class="setting-card" aria-labelledby="your-name-heading">
          <div class="card-heading"><h2 id="your-name-heading">Your name</h2><p>Used by agents in new chats.</p></div>
          <label class="profile-name">Your name<input aria-label="Your name" autocomplete="name" value={settings.userName ?? ""} aria-invalid={userNameError ? "true" : undefined} oninput={editUserName} onchange={saveUserName} /></label>
          {#if userNameError}<p class="error" role="alert">{userNameError}</p>{/if}
        </section>
      </div>
    {:else if activeCategory === "appearance"}
      <div class="section-stack">
        <section class="setting-card" aria-labelledby="theme-heading">
          <div class="card-heading"><h2 id="theme-heading">Appearance mode</h2><p>Follow the system, or keep one of your two palettes active.</p></div>
          <div class="segmented" aria-label="Theme">
            {#each ["system", "light", "dark"] as theme}
              <button type="button" class:chosen={settings.theme === theme} aria-pressed={settings.theme === theme}
                onclick={() => void save({ theme: theme as Settings["theme"] })}>{theme}</button>
            {/each}
          </div>
        </section>

        <section class="setting-card" aria-labelledby="app-theme-heading">
          <div class="card-heading"><h2 id="app-theme-heading">App theme</h2><p>Choose the palette Monitter uses in light and dark mode. System appearance switches between them automatically.</p></div>
          <div class="app-theme-selectors">
            {#each ['light', 'dark'] as mode}
              {@const selected = mode === 'light' ? selectedLightTheme : selectedDarkTheme}
              {@const palette = selected[mode as 'light' | 'dark']}
              <label class="app-theme-label">
                <span>{mode === 'light' ? 'Light theme' : 'Dark theme'}</span>
                <details class="rich-theme-select">
                  <summary aria-label={`${mode === 'light' ? 'Light' : 'Dark'} theme: ${selected.label}`} style={`--theme-paper:${palette.paper};--theme-panel:${palette.panel};--theme-accent:${palette.accent};--theme-ink:${palette.ink}`}>
                    <span class="theme-preview" aria-hidden="true"><i></i><i></i><i></i></span>
                    <strong>{selected.label}</strong>
                    <ChevronDown size={15} aria-hidden="true" />
                  </summary>
                  <div class="theme-options" role="listbox" aria-label={`${mode === 'light' ? 'Light' : 'Dark'} theme choices`}>
                    {#each appThemes as option}
                      {@const optionPalette = option[mode as 'light' | 'dark']}
                      <button type="button" role="option" aria-selected={$appTheme[mode as 'light' | 'dark'] === option.id} class:chosen={$appTheme[mode as 'light' | 'dark'] === option.id} style={`--theme-paper:${optionPalette.paper};--theme-panel:${optionPalette.panel};--theme-accent:${optionPalette.accent};--theme-ink:${optionPalette.ink}`} onclick={(event)=>chooseAppTheme(mode as 'light' | 'dark', option.id, event)}>
                        <span class="theme-preview" aria-hidden="true"><i></i><i></i><i></i></span><strong>{option.label}</strong>{#if $appTheme[mode as 'light' | 'dark'] === option.id}<Check size={14} aria-hidden="true" />{/if}
                      </button>
                    {/each}
                  </div>
                </details>
              </label>
            {/each}
          </div>
        </section>

        <details class="advanced-theme">
          <summary><span><strong>Advanced theme controls</strong><small>Fine-tune both light and dark palettes together.</small></span><ChevronDown size={16} aria-hidden="true" /></summary>
          <div class="advanced-theme-content">
            <section class="setting-card" aria-labelledby="accent-heading">
              <div class="card-heading"><h2 id="accent-heading">Accent colour</h2><p>Override the selected themes with one accent in both modes.</p></div>
              <div class="swatches">
                {#each accents as accent}
                  <button type="button" aria-label={accent} class:chosen={$appTheme.accent === accent}
                    style={`--swatch:${accent}`} onclick={() => setAppThemeAccent(accent)}></button>
                {/each}
                <label class="colour-picker"><span>Custom accent colour</span><input aria-label="Custom accent colour" type="color" value={advancedAccent} onchange={(event) => setAppThemeAccent(event.currentTarget.value)} /></label>
              </div>
              <div class="theme-default-row"><span>{#if $appTheme.accent}Custom accent applies to both themes.{:else}Each theme is using its own accent.{/if}</span><button type="button" disabled={!$appTheme.accent} onclick={()=>setAppThemeAccent(null)}>Use theme accents</button></div>
              <label class="range-setting">Tint intensity <strong>{$surfaceTint}%</strong>
                <input class="range" type="range" use:rangeFill={$surfaceTint} aria-label="Tint intensity" aria-valuetext={`${$surfaceTint}%`} min="0" max="50" step="1" value={$surfaceTint} oninput={event=>setSurfaceTint(Number(event.currentTarget.value))} />
              </label>
              <div class="range-footer"><span>0%</span><button type="button" onclick={()=>setSurfaceTint(DEFAULT_SURFACE_TINT)}>Reset to default · {DEFAULT_SURFACE_TINT}%</button><span>50%</span></div>
              <p class="hint">Light mode uses half this intensity. Saved on this device/browser.</p>
            </section>

            <section class="setting-card" aria-labelledby="borders-heading">
              <div class="card-heading"><h2 id="borders-heading">Borders & dividers</h2><p>Adjust structural lines in both themes.</p></div>
              <label class="range-setting">Line opacity <strong>{$borderOpacity}%</strong>
                <input class="range" type="range" use:rangeFill={$borderOpacity} aria-label="Border and divider opacity" aria-valuetext={`${$borderOpacity}%`} min="0" max="100" step="1" value={$borderOpacity} oninput={event=>setBorderOpacity(Number(event.currentTarget.value))} />
              </label>
              <div class="range-footer"><span>0%</span><button type="button" onclick={()=>setBorderOpacity(DEFAULT_BORDER_OPACITY)}>Reset to default · {DEFAULT_BORDER_OPACITY}%</button><span>100%</span></div>
              <p class="hint">Saved on this device/browser.</p>
            </section>

            <button type="button" class="reset-theme" onclick={resetAppTheme}>Reset both themes and accent</button>
          </div>
        </details>

        <section class="setting-card" aria-labelledby="terminal-theme-heading">
          <div class="card-heading"><h2 id="terminal-theme-heading">Terminal colour theme</h2><p>Choose a familiar ANSI palette for every terminal. Open terminals update immediately.</p></div>
          <label class="terminal-theme-picker">Colour theme<select aria-label="Terminal colour theme" value={$terminalTheme} onchange={event=>setTerminalTheme(event.currentTarget.value as TerminalThemeId)}>{#each terminalThemes as option}<option value={option.id}>{option.label}</option>{/each}</select></label>
          <div class="terminal-palette" aria-hidden="true" title={`${selectedTerminalTheme.label} ANSI colours`}>
            {#each [selectedTerminalTheme.palette.black,selectedTerminalTheme.palette.red,selectedTerminalTheme.palette.green,selectedTerminalTheme.palette.yellow,selectedTerminalTheme.palette.blue,selectedTerminalTheme.palette.magenta,selectedTerminalTheme.palette.cyan,selectedTerminalTheme.palette.white,selectedTerminalTheme.palette.brightBlack,selectedTerminalTheme.palette.brightRed,selectedTerminalTheme.palette.brightGreen,selectedTerminalTheme.palette.brightYellow,selectedTerminalTheme.palette.brightBlue,selectedTerminalTheme.palette.brightMagenta,selectedTerminalTheme.palette.brightCyan,selectedTerminalTheme.palette.brightWhite] as colour}<span style={`--terminal-swatch:${colour}`}></span>{/each}
          </div>
          <p class="hint">Saved on this device/browser.</p>
        </section>

        <section class="setting-card" aria-labelledby="motion-heading">
          <div class="card-heading"><h2 id="motion-heading">Motion</h2><p>Keep interface movement comfortable. Your operating-system reduced-motion setting always takes priority.</p></div>
          <div class="segmented" aria-label="Motion">
            {#each ['system', 'subtle', 'off'] as preference}
              <button type="button" class:chosen={$motionPreference === preference} aria-pressed={$motionPreference === preference}
                onclick={() => setMotionPreference(preference as MotionPreference)}>{preference[0].toUpperCase() + preference.slice(1)}</button>
            {/each}
          </div>
          <p class="hint">Saved on this device/browser.</p>
        </section>

        <section class="setting-card" aria-labelledby="scale-heading">
          <div class="card-heading inline-heading"><div><h2 id="scale-heading">Interface scale</h2><p>Resize text and controls for this {interfaceScale === undefined ? 'connection' : 'viewer type'}.</p></div><strong>{displayedInterfaceScale}%</strong></div>
          <input class="range" type="range" use:rangeFill={displayedInterfaceScale} aria-label="Interface scale" min="80" max="200" step="5" value={displayedInterfaceScale} onchange={(event) => void saveInterfaceScale(Number(event.currentTarget.value))} />
          <div class="range-footer"><span>80%</span><button type="button" onclick={() => void saveInterfaceScale(125)}>Reset to default · 125%</button><span>200%</span></div>
          {#if interfaceScale !== undefined}<p class="hint">Saved on this device/browser for {interfaceScaleViewer ?? 'desktop'} viewers, independent of the connected workspace.</p>{/if}
        </section>

        <section class="setting-card" aria-labelledby="density-heading">
          <div class="card-heading"><h2 id="density-heading">Interface density</h2><p>Adjust the height and spacing of tabs, sidebars and pane controls.</p></div>
          <input class="range density-range" type="range" use:rangeFill={densities.indexOf(density)} aria-label="Interface density" aria-valuetext={density} min="0" max="2" step="1" value={densities.indexOf(density)} onchange={(event) => void save({ interfaceDensity: densities[Number(event.currentTarget.value)] ?? 'normal' })} />
          <div class="density-labels" aria-hidden="true">
            {#each densities as option}<button type="button" class:chosen={density === option} onclick={() => void save({ interfaceDensity: option })}>{option}</button>{/each}
          </div>
        </section>

        <section class="setting-card" aria-labelledby="tabs-heading">
          <div class="card-heading"><h2 id="tabs-heading">Tabs</h2><p>Choose a classic rounded strip or a modern full-height straight-edge strip.</p></div>
          <div class="segmented" aria-label="Tab style">
            {#each ['classic', 'modern'] as tabStyle}
              <button type="button" class:chosen={(settings.tabStyle ?? 'classic') === tabStyle} aria-pressed={(settings.tabStyle ?? 'classic') === tabStyle} onclick={() => void save({ tabStyle: tabStyle as NonNullable<Settings['tabStyle']> })}>{tabStyle}</button>
            {/each}
          </div>
          <label class="switch-row"><span><strong>Show tab close buttons</strong><small>Tabs can also be closed with your chosen keyboard shortcuts.</small></span><input type="checkbox" role="switch" aria-label="Show tab close buttons" checked={settings.showTabCloseButtons !== false} onchange={event=>void save({showTabCloseButtons:event.currentTarget.checked})}/></label>
        </section>
        <section class="setting-card" aria-labelledby="panes-heading">
          <div class="card-heading"><h2 id="panes-heading">Pane appearance</h2><p>Keep the active workspace easy to identify.</p></div>
          <label class="switch-row"><span><strong>Dim inactive panes</strong><small>Reduce visual weight for panes that are not active.</small></span><input type="checkbox" role="switch" aria-label="Dim inactive panes" checked={settings.dimInactivePanes ?? true} onchange={(event) => void save({ dimInactivePanes: event.currentTarget.checked })} /></label>
          <label class:disabled={settings.dimInactivePanes === false} class="range-setting">Inactive pane opacity <strong>{Math.round((settings.inactivePaneOpacity ?? 0.6) * 100)}%</strong><input class="range" type="range" use:rangeFill={(settings.inactivePaneOpacity ?? 0.6) * 100} aria-label="Inactive pane opacity" min="10" max="90" step="5" disabled={settings.dimInactivePanes === false} value={(settings.inactivePaneOpacity ?? 0.6) * 100} onchange={(event) => void save({ inactivePaneOpacity: Number(event.currentTarget.value) / 100 })} /></label>
        </section>
      </div>
    {:else if activeCategory === "typography"}
      <div class="section-stack">
        <section class="setting-card" aria-labelledby="fonts-heading">
          <div class="card-heading"><h2 id="fonts-heading">Fonts</h2><p>Installed family names work too. Clear a field to restore its default.</p></div>
          <div class="font-grid">
            {#each fonts as font}
              <div class="font-setting">
                <label>{font.label}<input aria-label={font.label} list={font.key === "terminalFont" ? "terminal-font-choices" : "text-font-choices"} placeholder={font.fallback + " (default)"} maxlength="100" value={settings[font.key] ?? ""} onchange={(event) => void save({ [font.key]: event.currentTarget.value.trim() })} /></label>
                <label>{font.label} base size (px)<input type="number" aria-label={font.label + " base size"} min="8" max="32" step="1" value={settings[font.sizeKey] ?? font.size} onchange={(event) => saveNumber(event, font.sizeKey, font.size)} /></label>
              </div>
            {/each}
          </div>
          <datalist id="text-font-choices">{#each ["IBM Plex Sans", "Arial", "Helvetica Neue", "Avenir Next", "Georgia", "Verdana", "IBM Plex Mono", "Menlo"] as name}<option value={name}></option>{/each}</datalist>
          <datalist id="terminal-font-choices">{#each ["IBM Plex Mono", "Menlo", "Monaco", "Courier New", "SF Mono", "JetBrains Mono", "Fira Code"] as name}<option value={name}></option>{/each}</datalist>
          <p class="hint">Base sizes are before interface scaling. Use a monospace font for terminals; unavailable fonts fall back to the default.</p>
        </section>
        <section class="setting-card" aria-labelledby="line-height-heading">
          <div class="card-heading"><h2 id="line-height-heading">Line height</h2><p>Control the vertical space in chat messages and terminal output.</p></div>
          <label class="range-setting">Chat line height <strong>{(settings.chatLineHeight ?? 1.65).toFixed(2)}×</strong><input class="range" type="range" use:rangeFill={settings.chatLineHeight ?? 1.65} aria-label="Chat line height" min="1" max="2.5" step="0.05" value={settings.chatLineHeight ?? 1.65} onchange={(event) => saveLineHeight(event, "chatLineHeight", 1.65)} /></label>
          <label class="range-setting">Terminal line height <strong>{(settings.terminalLineHeight ?? 1).toFixed(2)}×</strong><input class="range" type="range" use:rangeFill={settings.terminalLineHeight ?? 1} aria-label="Terminal line height" min="1" max="2.5" step="0.05" value={settings.terminalLineHeight ?? 1} onchange={(event) => saveLineHeight(event, "terminalLineHeight", 1)} /></label>
          <p class="hint">These values are relative to each surface’s chosen font size. Terminal changes refit open terminal tabs immediately.</p>
        </section>
      </div>
    {:else if activeCategory === "behaviour"}
      <div class="section-stack">
        <section class="setting-card" aria-labelledby="shortcut-heading">
          <div class="card-heading"><h2 id="shortcut-heading">Keyboard shortcuts</h2><p>Choose how workspace commands and pane navigation behave.</p></div>
          <label>Shortcut mode<select aria-label="Shortcut mode" value={settings.shortcutMode ?? 'standard'} onchange={event=>void save({shortcutMode:event.currentTarget.value as 'standard'|'vim'})}><option value="standard">Standard</option><option value="vim">Vim</option></select></label>
          <p class="hint">{#if settings.shortcutMode === 'vim'}Escape then : opens commands. Ctrl/Cmd-W then an arrow or h/j/k/l moves between panes. Use :q to close a tab. Terminal keystrokes stay with the terminal.{:else}Cmd/Ctrl-W closes the current tab. Preferences and the command palette keep their standard shortcuts.{/if}</p>
        </section>
        <section class="setting-card" aria-labelledby="focus-heading">
          <div class="card-heading"><h2 id="focus-heading">Workspace focus</h2><p>Set how split panes respond as you move through them.</p></div>
          <label class="switch-row"><span><strong>Focus follows mouse</strong><small>Activate a pane when the pointer enters it.</small></span><input type="checkbox" role="switch" aria-label="Focus follows mouse" checked={settings.focusFollowsMouse ?? false} onchange={(event) => void save({ focusFollowsMouse: event.currentTarget.checked })} /></label>
        </section>
        <section class="setting-card" aria-labelledby="busy-heading">
          <div class="card-heading"><h2 id="busy-heading">Busy agents</h2><p>Choose what happens when you send another message during a running turn.</p></div>
          <label class="switch-row"><span><strong>Steer busy agents when supported</strong><small>Off queues a message for the next turn. On requests steering where a connected harness supports it; otherwise it queues safely.</small></span><input type="checkbox" role="switch" aria-label="Steer busy agents when supported" checked={settings.busyMessageMode === "steer"} onchange={(event) => void save({ busyMessageMode: event.currentTarget.checked ? "steer" : "queue" })} /></label>
          <p class="hint">Current CLI integrations use the queue; live steering is not connected yet. Queued messages appear above the input box.</p>
        </section>
        <section class="setting-card" aria-labelledby="permissions-heading">
          <div class="card-heading"><h2 id="permissions-heading">Agent permissions</h2><p>Permission modes are configured per agent. Each task keeps the permission mode captured when it was created, so changing an agent affects future tasks.</p></div>
        </section>
      </div>
    {:else}
      <div class="section-stack">
        <section class="setting-card" aria-labelledby="activity-heading">
          <div class="card-heading"><h2 id="activity-heading">Conversation activity</h2><p>Control supporting information shown alongside messages.</p></div>
          <label class="switch-row"><span><strong>Show tool activity</strong><small>Show collapsible blocks for activity supplied by the harness.</small></span><input type="checkbox" role="switch" aria-label="Show tool activity" checked={settings.showToolActivity !== false} onchange={(event) => void save({ showToolActivity: event.currentTarget.checked })} /></label>
          <label class="switch-row"><span><strong>Compress tool calls</strong><small>Combine calls of the same kind across thinking time within each turn. Open the summary to see every call.</small></span><input type="checkbox" role="switch" aria-label="Compress tool calls" checked={settings.compressToolCalls === true} onchange={(event) => void save({ compressToolCalls: event.currentTarget.checked })} /></label>
          <label class="switch-row"><span><strong>Tint my messages</strong><small>Use the accent colour for your message bubbles.</small></span><input type="checkbox" role="switch" aria-label="Tint my messages" checked={settings.tintUserMessages ?? false} onchange={event=>void save({tintUserMessages:event.currentTarget.checked})}/></label>
          <label class="switch-row"><span><strong>Show reasoning summaries</strong><small>Show summaries supplied by the harness.</small></span><input type="checkbox" role="switch" aria-label="Show reasoning summaries" checked={settings.showReasoningSummaries !== false} onchange={(event) => void save({ showReasoningSummaries: event.currentTarget.checked })} /></label>
        </section>
        <section class="setting-card" aria-labelledby="messages-heading">
          <div class="card-heading"><h2 id="messages-heading">Messages</h2><p>Choose the keyboard shortcut that sends a task or channel message.</p></div>
          <label class="switch-row"><span><strong>Enter to send</strong><small>{settings.sendWithEnter ? "Enter sends. Shift+Enter adds a new line." : "Cmd/Ctrl+Enter sends. Enter adds a new line."}</small></span><input type="checkbox" role="switch" aria-label="Enter to send" checked={settings.sendWithEnter ?? false} onchange={(event) => void save({ sendWithEnter: event.currentTarget.checked })} /></label>
        </section>
      </div>
    {/if}
    </div>
  </div>
</section>

<style>
  .settings-pane { display:grid; flex:1; width:100%; min-width:0; grid-template-columns:220px minmax(0, 1fr); height:100%; min-height:0; color:var(--ink); background:var(--paper); font-family:var(--interface-font, "IBM Plex Sans", system-ui, sans-serif); }
  .settings-nav { min-height:0; overflow:auto; padding:20px 12px; border-right:1px solid var(--line); background:var(--paper); }
  .nav-heading { padding:0 9px 11px; color:var(--muted); font:600 calc(10px * var(--interface-font-ratio, 1)) var(--mono, monospace); letter-spacing:.09em; text-transform:uppercase; }
  .settings-nav button { display:flex; width:100%; align-items:flex-start; gap:9px; padding:10px 9px; border:0; border-radius:7px; color:var(--muted); background:transparent; text-align:left; font-family:inherit; cursor:pointer; }
  .settings-nav button:hover { color:var(--ink); background:color-mix(in srgb, var(--accent) 7%, transparent); }
  .settings-nav button.active { color:var(--accent-ink, var(--accent)); background:color-mix(in srgb, var(--accent) 13%, transparent); }
  .settings-nav span { display:grid; gap:2px; min-width:0; }.settings-nav strong { font-size:calc(12px * var(--interface-font-ratio, 1)); font-weight:600; }.settings-nav small { overflow:hidden; color:var(--muted); font-size:calc(10px * var(--interface-font-ratio, 1)); text-overflow:ellipsis; white-space:nowrap; }
  .settings-content { --scroll-fade:20px; -webkit-mask-image:linear-gradient(to bottom,transparent 0,#000 var(--scroll-fade),#000 calc(100% - var(--scroll-fade)),transparent 100%); mask-image:linear-gradient(to bottom,transparent 0,#000 var(--scroll-fade),#000 calc(100% - var(--scroll-fade)),transparent 100%); min-width:0; min-height:0; overflow:auto; padding:clamp(18px, 4cqi, 38px); }
  .settings-header { display:flex; align-items:flex-start; justify-content:space-between; gap:16px; max-width:760px; margin:0 auto 24px; }.settings-header p { margin:0 0 4px; color:var(--muted); font:600 calc(10px * var(--interface-font-ratio, 1)) var(--mono, monospace); letter-spacing:.09em; text-transform:uppercase; }.settings-header h1 { margin:0; font-size:calc(24px * var(--interface-font-ratio, 1)); letter-spacing:-.03em; }.save-state { display:flex; flex:none; align-items:center; gap:5px; margin-top:4px; color:var(--muted); font-size:calc(11px * var(--interface-font-ratio, 1)); white-space:nowrap; }.save-error { color:#b84c44; }.save-state :global(svg.spin) { animation:spin .85s linear infinite; }
  .settings-category { min-width:0; }.section-stack { display:grid; gap:14px; max-width:760px; margin:0 auto; }.setting-card { display:grid; gap:16px; padding:18px; border:1px solid var(--line); border-radius:10px; background:var(--panel); box-shadow:0 1px 2px rgba(0,0,0,.025); }.card-heading h2 { margin:0 0 4px; font-size:calc(14px * var(--interface-font-ratio, 1)); }.card-heading p,.hint { margin:0; color:var(--muted); font-size:calc(11.5px * var(--interface-font-ratio, 1)); line-height:1.5; }.inline-heading { display:flex; align-items:start; justify-content:space-between; gap:12px; }.inline-heading > strong { color:var(--accent-ink, var(--accent)); font:600 calc(13px * var(--interface-font-ratio, 1)) var(--mono, monospace); }
  .segmented { display:flex; padding:3px; border:1px solid var(--line); border-radius:7px; background:var(--soft); }.segmented button { flex:1; padding:7px 8px; border:0; border-radius:4px; color:var(--muted); background:transparent; font:calc(11.5px * var(--interface-font-ratio, 1)) var(--interface-font, sans-serif); text-transform:capitalize; cursor:pointer; }.segmented button.chosen { color:var(--ink); background:var(--panel); box-shadow:0 1px 2px rgba(0,0,0,.08); }
  .app-theme-selectors { display:grid; grid-template-columns:repeat(2,minmax(0,1fr)); gap:12px; }.app-theme-label { display:grid; gap:6px; color:var(--muted); font-size:calc(11px * var(--interface-font-ratio,1)); }.rich-theme-select { position:relative; color:var(--ink); }.rich-theme-select > summary { display:grid; grid-template-columns:44px minmax(0,1fr) auto; align-items:center; gap:9px; min-height:44px; padding:7px; border:1px solid var(--line); border-radius:7px; background:var(--paper); cursor:pointer; list-style:none; }.rich-theme-select > summary::-webkit-details-marker { display:none; }.rich-theme-select > summary:hover,.rich-theme-select[open] > summary { border-color:color-mix(in srgb,var(--accent) 55%,var(--line)); background:var(--soft); }.rich-theme-select > summary strong,.theme-options strong { overflow:hidden; font-size:calc(11.5px * var(--interface-font-ratio,1)); font-weight:600; text-overflow:ellipsis; white-space:nowrap; }.rich-theme-select > summary > :global(svg) { color:var(--muted); transition:transform .15s ease; }.rich-theme-select[open] > summary > :global(svg) { transform:rotate(180deg); }.theme-options { position:absolute; z-index:12; top:calc(100% + 5px); left:0; display:grid; width:100%; max-height:280px; overflow:auto; padding:5px; border:1px solid var(--line); border-radius:8px; background:var(--panel); box-shadow:0 18px 40px rgba(0,0,0,.35); }.theme-options button { display:grid; grid-template-columns:44px minmax(0,1fr) 14px; align-items:center; gap:9px; width:100%; min-width:0; padding:6px; border:0; border-radius:5px; color:var(--muted); background:transparent; text-align:left; cursor:pointer; }.theme-options button:hover,.theme-options button.chosen { color:var(--ink); background:var(--soft); }.theme-options button.chosen > :global(svg) { color:var(--accent-ink); }.theme-preview { display:grid; grid-template-columns:repeat(3,1fr); width:44px; height:28px; overflow:hidden; border:1px solid color-mix(in srgb,var(--theme-ink) 30%,transparent); border-radius:5px; background:var(--theme-paper); }.theme-preview i:first-child { background:var(--theme-paper); }.theme-preview i:nth-child(2) { background:var(--theme-panel); }.theme-preview i:last-child { background:var(--theme-accent); }
  .advanced-theme { overflow:visible; border:1px solid var(--line); border-radius:10px; background:var(--panel); }.advanced-theme > summary { display:flex; align-items:center; justify-content:space-between; gap:12px; padding:16px 18px; cursor:pointer; list-style:none; }.advanced-theme > summary::-webkit-details-marker { display:none; }.advanced-theme > summary span { display:grid; gap:3px; }.advanced-theme > summary strong { font-size:calc(13px * var(--interface-font-ratio,1)); }.advanced-theme > summary small { color:var(--muted); font-size:calc(11px * var(--interface-font-ratio,1)); }.advanced-theme > summary > :global(svg) { flex:none; color:var(--muted); transition:transform .15s ease; }.advanced-theme[open] > summary > :global(svg) { transform:rotate(180deg); }.advanced-theme-content { display:grid; gap:10px; padding:0 10px 10px; }.advanced-theme-content > .setting-card { box-shadow:none; }.theme-default-row { display:flex; align-items:center; justify-content:space-between; gap:12px; color:var(--muted); font-size:calc(10.5px * var(--interface-font-ratio,1)); }.theme-default-row button,.reset-theme { padding:6px 8px; border:1px solid var(--line); border-radius:6px; color:var(--ink); background:var(--paper); font:calc(10.5px * var(--interface-font-ratio,1)) var(--interface-font,sans-serif); cursor:pointer; }.theme-default-row button:disabled { opacity:.45; cursor:default; }.reset-theme { justify-self:end; margin:2px 4px 4px; color:var(--muted); }
  .swatches { display:flex; align-items:center; gap:9px; flex-wrap:wrap; }.swatches > button { width:26px; height:26px; padding:0; border:2px solid transparent; border-radius:50%; background:var(--swatch); cursor:pointer; }.swatches > button.chosen { border-color:var(--ink); outline:2px solid var(--paper); outline-offset:-4px; }.colour-picker { display:flex; align-items:center; gap:7px; margin-left:3px; color:var(--muted); font-size:calc(11px * var(--interface-font-ratio, 1)); }.colour-picker input { width:28px; height:25px; padding:1px; border:1px solid var(--line); border-radius:5px; background:var(--paper); cursor:pointer; }.colour-picker span { position:absolute; width:1px; height:1px; overflow:hidden; clip:rect(0 0 0 0); }
  .range { width:100%; padding:0; accent-color:var(--accent); cursor:pointer; }.range:disabled { cursor:not-allowed; }.range-footer { display:flex; align-items:center; justify-content:space-between; gap:10px; color:var(--muted); font:calc(10px * var(--interface-font-ratio, 1)) var(--mono, monospace); }.range-footer button { padding:0; border:0; color:var(--accent-ink, var(--accent)); background:none; font:calc(11px * var(--interface-font-ratio, 1)) var(--interface-font, sans-serif); cursor:pointer; }
  .density-range { margin-top:2px; }.density-labels { display:grid; grid-template-columns:repeat(3,1fr); gap:8px; }.density-labels button { padding:0; color:var(--muted); text-transform:capitalize; font-size:calc(10.5px * var(--interface-font-ratio, 1)); }.density-labels button:first-child { text-align:left; }.density-labels button:last-child { text-align:right; }.density-labels button.chosen { color:var(--accent-ink); font-weight:600; }
  .terminal-theme-picker { display:grid; gap:6px; color:var(--muted); font-size:calc(11px * var(--interface-font-ratio, 1)); }.terminal-theme-picker select { width:100%; padding:8px 9px; border:1px solid var(--line); border-radius:6px; color:var(--ink); background:var(--paper); font:calc(12px * var(--interface-font-ratio, 1)) var(--interface-font,sans-serif); }
  .terminal-palette { display:grid; grid-template-columns:repeat(8,1fr); height:24px; overflow:hidden; border:1px solid var(--line); border-radius:6px; background:var(--terminal-background); }.terminal-palette span { min-width:0; background:var(--terminal-swatch); }
  .remote-control-host { min-width:0; }
  .profile-name { display:grid; gap:6px; color:var(--muted); font-size:calc(11px * var(--interface-font-ratio, 1)); }.profile-name input { width:100%; max-width:420px; padding:8px 9px; border:1px solid var(--line); border-radius:6px; outline:none; color:var(--ink); background:var(--paper); font:calc(12px * var(--interface-font-ratio, 1)) var(--interface-font, sans-serif); }.profile-name input:focus { border-color:var(--accent); box-shadow:0 0 0 2px color-mix(in srgb, var(--accent) 16%, transparent); }
  .switch-row { display:flex; align-items:center; justify-content:space-between; gap:20px; padding:5px 0; }.switch-row span { display:grid; gap:3px; }.switch-row strong,.range-setting { font-size:calc(12px * var(--interface-font-ratio, 1)); }.switch-row small { color:var(--muted); font-size:calc(11px * var(--interface-font-ratio, 1)); line-height:1.45; }.switch-row input { appearance:none; -webkit-appearance:none; position:relative; flex:none; width:32px; height:18px; margin:0; border:1px solid var(--line); border-radius:999px; background:var(--soft); cursor:pointer; transition:.15s ease; }.switch-row input::after { position:absolute; top:2px; left:2px; width:12px; height:12px; border-radius:50%; background:var(--muted); content:""; transition:.15s ease; }.switch-row input:checked { border-color:var(--accent); background:var(--accent); }.switch-row input:checked::after { left:16px; background:var(--on-accent, #fff); }.switch-row input:focus-visible,.settings-nav button:focus-visible,.segmented button:focus-visible,.swatches button:focus-visible { outline:2px solid var(--accent); outline-offset:2px; }
  .range-setting { display:grid; grid-template-columns:1fr auto; gap:9px; padding-top:8px; }.range-setting input { grid-column:1 / -1; }.range-setting.disabled { opacity:.52; }.font-grid { display:grid; gap:16px; }.font-setting { display:grid; grid-template-columns:minmax(0, 1fr) 180px; gap:12px; padding-bottom:16px; border-bottom:1px solid var(--line); }.font-setting:last-child { padding-bottom:0; border-bottom:0; }.font-setting label { display:grid; gap:6px; color:var(--muted); font-size:calc(11px * var(--interface-font-ratio, 1)); }.font-setting input { width:100%; padding:8px 9px; border:1px solid var(--line); border-radius:6px; outline:none; color:var(--ink); background:var(--paper); font:calc(12px * var(--interface-font-ratio, 1)) var(--interface-font, sans-serif); }.font-setting input:focus { border-color:var(--accent); box-shadow:0 0 0 2px color-mix(in srgb, var(--accent) 16%, transparent); }.error { max-width:760px; margin:0 auto 14px; padding:9px 11px; border:1px solid color-mix(in srgb, #b84c44 40%, var(--line)); border-radius:7px; color:#b84c44; background:color-mix(in srgb, #b84c44 7%, transparent); font-size:calc(11.5px * var(--interface-font-ratio, 1)); }
  @keyframes spin { to { transform:rotate(360deg); } }
  @container (max-width:620px) { .settings-pane { grid-template-columns:150px minmax(0,1fr); }.settings-nav { padding:14px 7px; }.settings-nav button { padding:9px 7px; }.settings-nav small { display:none; }.settings-content { padding:18px 14px; }.settings-header { margin-bottom:18px; }.settings-header h1 { font-size:calc(20px * var(--interface-font-ratio, 1)); }.save-state { font-size:calc(10px * var(--interface-font-ratio, 1)); }.font-setting { grid-template-columns:1fr; gap:10px; } }
  @container (max-width:430px) { .settings-pane { grid-template-columns:52px minmax(0,1fr); }.settings-nav { padding:12px 8px; }.nav-heading,.settings-nav span { display:none; }.settings-nav button { justify-content:center; padding:10px; }.settings-content { padding:16px 12px; }.setting-card { padding:14px; }.settings-header { align-items:center; }.save-state { font-size:0; }.save-state :global(svg) { width:14px; height:14px; }.swatches { gap:7px; }.colour-picker { margin-left:0; }.app-theme-selectors { grid-template-columns:1fr; }.theme-default-row { align-items:flex-start; flex-direction:column; } }
  @media (prefers-reduced-motion: reduce) { .save-state :global(svg.spin) { animation:none; }.switch-row input,.switch-row input::after { transition:none; } }
</style>
