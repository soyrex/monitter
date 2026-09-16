<script lang="ts">
  import { onMount } from 'svelte';
  import { Check, FileUp, Plus, Trash2 } from '@lucide/svelte';
  import { getBridge } from '$lib/bridge';
  import type { Agent, ExtensionConfig, ManagedSkill, McpServerConfig } from '$lib/types';
  import { isLanBrowser } from '$lib/lan';

  let { agents = [] }: { agents?: Agent[] } = $props();
  type Tab = 'servers' | 'skills' | 'plugins';
  const empty = (): ExtensionConfig => ({ mcpServers: [], skills: [] });
  const newServer = (): McpServerConfig => ({ id: crypto.randomUUID(), name: '', enabled: true, agentIds: [], transport: 'stdio', command: '', args: [], env: {}, url: '', headers: {} });
  const newSkill = (): ManagedSkill => ({ id: crypto.randomUUID(), name: '', description: '', enabled: true, agentIds: [], allAgents: false, sourceUrl: undefined, content: '' });
  let tab = $state<Tab>('servers');
  let config = $state<ExtensionConfig>(empty());
  let loading = $state(true);
  let loaded = $state(false);
  let saving = $state(false);
  let error = $state('');
  let notice = $state('');
  let editingServer = $state<McpServerConfig | null>(null);
  let editingSkill = $state<ManagedSkill | null>(null);
  let envText = $state('');
  let headerText = $state('');
  let pasteOpen = $state(false);
  let pastedJson = $state('');
  let importInput = $state<HTMLInputElement>();
  let skillInput = $state<HTMLInputElement>();
  const nativeAvailable = $derived(!isLanBrowser() && getBridge().available);

  onMount(async () => {
    if (!nativeAvailable) { loading = false; return; }
    try { config = await getBridge().getExtensionConfig(); loaded = true; }
    catch (reason) { error = `Could not load extension settings: ${String(reason)}`; }
    finally { loading = false; }
  });

  function clone<T>(value: T): T { return JSON.parse(JSON.stringify(value)); }
  function pairText(values: Record<string, string>) { return Object.entries(values).map(([key, value]) => `${key}=${value}`).join('\n'); }
  function parsePairs(value: string, label: string): Record<string, string> {
    const result: Record<string, string> = {};
    for (const raw of value.split('\n').filter(Boolean)) {
      const index = raw.indexOf('=');
      const key = raw.slice(0, index).trim(); const item = raw.slice(index + 1);
      const validKey = label === 'Headers' ? /^[!#$%&'*+.^_`|~0-9A-Za-z-]+$/.test(key) : /^[A-Za-z_][A-Za-z0-9_]*$/.test(key);
      if (index < 1 || !validKey) throw new Error(`${label} entries must be KEY=value.`);
      result[key] = item;
    }
    return result;
  }
  function selected(ids: string[], id: string) { return ids.includes(id) ? ids.filter(item => item !== id) : [...ids, id]; }
  function agentSummary(ids: string[], allAgents = false) { return allAgents ? 'All agents (including future agents)' : ids.length ? `${ids.length} assigned` : 'No agents'; }
  function serverLocation(server: McpServerConfig) { if (server.transport === 'stdio') return server.command; try { const url = new URL(server.url); return `${url.origin}${url.pathname}`; } catch { return 'HTTP endpoint'; } }
  function startServer(server: McpServerConfig) { editingServer = server; envText = pairText(server.env); headerText = pairText(server.headers); }
  function validateServer(server: McpServerConfig) {
    if (!server.name.trim()) throw new Error('Server name is required.');
    if (server.transport === 'stdio' && !server.command.trim()) throw new Error('A stdio command is required.');
    if (server.transport === 'http' && !/^https?:\/\/.+/.test(server.url)) throw new Error('An HTTP server needs a valid http:// or https:// URL.');
    if (!server.args.every(item => typeof item === 'string')) throw new Error('Arguments must be text.');
    if (!server.agentIds.every(id => agents.some(agent => agent.id === id))) throw new Error('A selected agent is no longer available.');
  }
  function validateSkill(skill: ManagedSkill) {
    if (!skill.name.trim()) throw new Error('Skill name is required.');
    if (!skill.content.trim()) throw new Error('Skill content is required.');
    if (new Blob([skill.content]).size > 131072) throw new Error('Skill files are limited to 128 KiB.');
    if (!skill.agentIds.every(id => agents.some(agent => agent.id === id))) throw new Error('A selected agent is no longer available.');
  }
  async function persist(next: ExtensionConfig): Promise<boolean> {
    saving = true; error = ''; notice = '';
    try { config = await getBridge().saveExtensionConfig(next); notice = 'Saved. MCP and skill changes apply to the next harness launch. Existing resident sessions keep their current configuration until the app restarts.'; return true; }
    catch (reason) { error = `Could not save. Your draft is still open: ${String(reason)}`; return false; }
    finally { saving = false; }
  }
  async function saveServer() {
    if (!editingServer) return;
    try {
      validateServer(editingServer);
      const server = clone(editingServer); server.name = server.name.trim(); server.env = parsePairs(envText, 'Environment'); server.headers = parsePairs(headerText, 'Headers');
      if (await persist({ ...config, mcpServers: config.mcpServers.some(item => item.id === server.id) ? config.mcpServers.map(item => item.id === server.id ? server : item) : [...config.mcpServers, server] })) editingServer = null;
    } catch (reason) { error = String(reason); }
  }
  async function saveSkill() {
    if (!editingSkill) return;
    try {
      validateSkill(editingSkill);
      const skill = clone(editingSkill); skill.name = skill.name.trim();
      if (await persist({ ...config, skills: config.skills.some(item => item.id === skill.id) ? config.skills.map(item => item.id === skill.id ? skill : item) : [...config.skills, skill] })) editingSkill = null;
    } catch (reason) { error = String(reason); }
  }
  function removeServer(id: string) { void persist({ ...config, mcpServers: config.mcpServers.filter(item => item.id !== id) }); }
  function removeSkill(id: string) { void persist({ ...config, skills: config.skills.filter(item => item.id !== id) }); }
  function toggleServer(server: McpServerConfig) { void persist({ ...config, mcpServers: config.mcpServers.map(item => item.id === server.id ? { ...item, enabled: !item.enabled } : item) }); }
  function toggleSkill(skill: ManagedSkill) { void persist({ ...config, skills: config.skills.map(item => item.id === skill.id ? { ...item, enabled: !item.enabled } : item) }); }

  function readImport(value: unknown): McpServerConfig[] {
    if (!value || typeof value !== 'object' || Array.isArray(value)) throw new Error('The import must be a JSON object.');
    const root = value as Record<string, unknown>; const source = root.mcpServers;
    if (!source || typeof source !== 'object' || Array.isArray(source)) throw new Error('Expected mcpServers as an object keyed by server name.');
    if (Object.keys(root).some(key => key !== 'mcpServers')) throw new Error('Only mcpServers is accepted in this import.');
    return Object.entries(source as Record<string, unknown>).map(([name, value]) => {
      if (!value || typeof value !== 'object' || Array.isArray(value)) throw new Error(`Server ${name} must be an object.`);
      const raw = value as Record<string, unknown>; const allowed = ['command', 'args', 'env', 'type', 'url', 'headers'];
      if (Object.keys(raw).some(key => !allowed.includes(key))) throw new Error(`Server ${name} contains unsupported fields.`);
      const transport = raw.type === 'http' || 'url' in raw ? 'http' : 'stdio';
      if (raw.type !== undefined && raw.type !== 'http' && raw.type !== 'stdio') throw new Error(`Server ${name} has an unsupported transport.`);
      const pairs = (key: 'env' | 'headers') => {
        const input = raw[key] ?? {}; if (!input || typeof input !== 'object' || Array.isArray(input) || Object.values(input).some(item => typeof item !== 'string')) throw new Error(`Server ${name} has invalid ${key}.`);
        return input as Record<string, string>;
      };
      const args = raw.args ?? []; if (!Array.isArray(args) || args.some(item => typeof item !== 'string')) throw new Error(`Server ${name} has invalid args.`);
      const server: McpServerConfig = { ...newServer(), name, transport, command: typeof raw.command === 'string' ? raw.command : '', args, env: pairs('env'), url: typeof raw.url === 'string' ? raw.url : '', headers: pairs('headers') };
      validateServer(server); return server;
    });
  }
  async function importJson(event: Event) {
    const file = (event.currentTarget as HTMLInputElement).files?.[0]; if (!file) return;
    try { if (file.size > 524288) throw new Error('JSON imports are limited to 512 KiB.'); const servers = readImport(JSON.parse(await file.text())); await persist({ ...config, mcpServers: [...config.mcpServers, ...servers] }); }
    catch (reason) { error = `Import rejected: ${String(reason)}`; }
    finally { if (importInput) importInput.value = ''; }
  }
  async function importPastedJson() {
    try {
      if (new Blob([pastedJson]).size > 524288) throw new Error('Pasted JSON is limited to 512 KiB.');
      const servers = readImport(JSON.parse(pastedJson));
      if (await persist({ ...config, mcpServers: [...config.mcpServers, ...servers] })) { pastedJson = ''; pasteOpen = false; }
    } catch (reason) { error = `Import rejected: ${String(reason)}`; }
  }
  async function importSkill(event: Event) {
    const file = (event.currentTarget as HTMLInputElement).files?.[0]; if (!file) return;
    try {
      if (!file.name.toLowerCase().endsWith('.md') || file.size > 131072) throw new Error('Choose a Markdown file no larger than 128 KiB.');
      editingSkill = { ...newSkill(), name: file.name.replace(/\.md$/i, ''), content: await file.text() };
    } catch (reason) { error = `Skill import rejected: ${String(reason)}`; }
    finally { if (skillInput) skillInput.value = ''; }
  }
</script>

<div class="section-stack extensions" aria-busy={loading}>
  <section class="setting-card">
    <div class="card-heading"><h2>MCP &amp; Plugins</h2><p>Local configuration for real harnesses. No server is started or restarted here. Changing an MCP server revokes remembered approvals for its affected agents; new tool approvals are still required.</p></div>
    {#if !nativeAvailable}
      <p class="manage-note">Extension configuration is managed on the desktop app. This LAN view cannot read local commands or credentials.</p>
    {:else if loading}<p class="hint">Loading extension settings…</p>
    {:else if !loaded}<p class="local-error" role="alert">{error}</p><button type="button" onclick={() => { loading = true; error = ''; getBridge().getExtensionConfig().then(value => { config = value; loaded = true; }).catch(reason => error = `Could not load extension settings: ${String(reason)}`).finally(() => loading = false); }}>Retry loading</button>
    {:else}
      <div class="extension-tabs" role="tablist" aria-label="MCP and Plugins sections">
        <button role="tab" aria-selected={tab === 'servers'} onclick={() => tab = 'servers'}>MCP servers</button><button role="tab" aria-selected={tab === 'skills'} onclick={() => tab = 'skills'}>Skills</button><button role="tab" aria-selected={tab === 'plugins'} onclick={() => tab = 'plugins'}>Plugins</button>
      </div>
      {#if error}<p class="local-error" role="alert">{error}</p>{/if}{#if notice}<p class="saved" aria-live="polite"><Check size={14}/>{notice}</p>{/if}
      {#if tab === 'servers'}
        <div class="toolbar"><p class="hint">Enabled servers are available only to assigned agents. Leaving assignment blank means no agent can use it. Stored privately on this Mac and not shared with web visitors.</p><span><input bind:this={importInput} class="file-input" type="file" accept="application/json,.json" onchange={importJson}/><button type="button" disabled={saving} onclick={() => importInput?.click()}><FileUp size={14}/>Import JSON</button><button type="button" disabled={saving} onclick={() => pasteOpen = !pasteOpen}>Paste JSON</button><button type="button" class="primary" disabled={saving} onclick={() => startServer(newServer())}><Plus size={14}/>Add server</button></span></div>
        {#if pasteOpen}<div class="paste-import"><label>Paste MCP JSON<textarea bind:value={pastedJson} placeholder="Paste a JSON object containing mcpServers"></textarea></label><span><button type="button" onclick={() => pasteOpen = false}>Cancel</button><button type="button" class="primary" disabled={saving} onclick={importPastedJson}>Import pasted JSON</button></span></div>{/if}
        {#each config.mcpServers as server (server.id)}<article class="extension-row"><label class="switch"><input type="checkbox" disabled={saving} checked={server.enabled} onchange={() => toggleServer(server)}/><span>{server.enabled ? 'Enabled' : 'Disabled'}</span></label><div><strong>{server.name}</strong><small>{serverLocation(server)} · {agentSummary(server.agentIds)}</small></div><button type="button" disabled={saving} onclick={() => startServer(clone(server))}>Edit</button><button type="button" disabled={saving} class="danger" aria-label={`Remove ${server.name}`} onclick={() => removeServer(server.id)}><Trash2 size={15}/></button></article>{:else}<p class="empty">No MCP servers configured.</p>{/each}
      {:else if tab === 'skills'}
        <div class="toolbar"><p class="hint">Skills are Markdown instructions. They are applied on the next harness launch.</p><span><input bind:this={skillInput} class="file-input" type="file" accept="text/markdown,.md" onchange={importSkill}/><button type="button" disabled={saving} onclick={() => skillInput?.click()}><FileUp size={14}/>Import .md</button><button type="button" class="primary" disabled={saving} onclick={() => editingSkill = newSkill()}><Plus size={14}/>Add skill</button></span></div>
        {#each config.skills as skill (skill.id)}<article class="extension-row"><label class="switch"><input type="checkbox" disabled={saving} checked={skill.enabled} onchange={() => toggleSkill(skill)}/><span>{skill.enabled ? 'Enabled' : 'Disabled'}</span></label><div><strong>{skill.name}</strong><small>{skill.description || 'No description'} · {agentSummary(skill.agentIds, skill.allAgents)}{#if skill.sourceUrl} · source: {skill.sourceUrl}{/if}</small></div><button type="button" disabled={saving} onclick={() => editingSkill = clone(skill)}>Edit</button><button type="button" disabled={saving} class="danger" aria-label={`Remove ${skill.name}`} onclick={() => removeSkill(skill.id)}><Trash2 size={15}/></button></article>{:else}<p class="empty">No skills configured.</p>{/each}
      {:else}
        <div class="plugin-note"><strong>Native plugins are not supported</strong><p>Monitter does not install scripts, assets, native packages, or third-party plugins. Add reviewed MCP servers or Markdown skills instead.</p></div>
      {/if}
    {/if}
  </section>
  {#if editingServer}<section class="setting-card editor" aria-labelledby="server-editor"><div class="inline-heading"><strong id="server-editor">{config.mcpServers.some(item => item.id === editingServer?.id) ? 'Edit MCP server' : 'Add MCP server'}</strong><button type="button" onclick={() => editingServer = null}>Cancel</button></div><label>Name<input bind:value={editingServer.name}/></label><label>Transport<select bind:value={editingServer.transport}><option value="stdio">Local stdio command</option><option value="http">HTTP endpoint</option></select></label>{#if editingServer.transport === 'stdio'}<label>Command<input bind:value={editingServer.command} placeholder="npx"/></label><label>Arguments <small>One argument per line. A shell is never used.</small><textarea value={editingServer.args.join('\n')} oninput={event => editingServer && (editingServer.args = event.currentTarget.value.split('\n').filter(Boolean))}></textarea></label><label>Environment values <small>Only enter secrets here when needed. Saved values are never shown in the server list.</small><textarea placeholder="API_TOKEN=value" bind:value={envText}></textarea></label>{:else}<label>URL<input type="url" bind:value={editingServer.url} placeholder="https://example.com/mcp"/></label><label>HTTP headers <small>Only enter secret values here when needed. Saved values are never shown in the server list.</small><textarea placeholder="Authorization=Bearer …" bind:value={headerText}></textarea></label>{/if}<fieldset><legend>Available to</legend>{#each agents as agent}<label class="agent-choice"><input type="checkbox" checked={editingServer.agentIds.includes(agent.id)} onchange={() => editingServer && (editingServer.agentIds = selected(editingServer.agentIds, agent.id))}/>{agent.name}</label>{:else}<p class="hint">No agents exist yet. Leaving this blank means no agent can use it.</p>{/each}</fieldset><button type="button" class="primary" disabled={saving} onclick={saveServer}>{saving ? 'Saving…' : 'Save server'}</button></section>{/if}
  {#if editingSkill}<section class="setting-card editor" aria-labelledby="skill-editor"><div class="inline-heading"><strong id="skill-editor">{config.skills.some(item => item.id === editingSkill?.id) ? 'Edit skill' : 'Add skill'}</strong><button type="button" onclick={() => editingSkill = null}>Cancel</button></div><label>Name<input bind:value={editingSkill.name}/></label><label>Description<input bind:value={editingSkill.description} placeholder="What this skill helps with"/></label>{#if editingSkill.sourceUrl}<p class="hint">Source: {editingSkill.sourceUrl}</p>{/if}<label>Markdown <small>Plain Markdown only, maximum 128 KiB.</small><textarea class="skill-content" bind:value={editingSkill.content}></textarea></label><fieldset><legend>Available to</legend><label class="agent-choice all-agents-choice"><input type="checkbox" checked={editingSkill.allAgents === true} onchange={() => editingSkill && (editingSkill.allAgents = !editingSkill.allAgents)}/>All agents (including future agents)<small>Manual selections are retained if you turn this off.</small></label>{#each agents as agent}<label class="agent-choice"><input type="checkbox" disabled={editingSkill.allAgents === true} checked={editingSkill.agentIds.includes(agent.id)} onchange={() => editingSkill && (editingSkill.agentIds = selected(editingSkill.agentIds, agent.id))}/>{agent.name}</label>{:else}<p class="hint">No agents exist yet. Choose All agents to include agents created later.</p>{/each}</fieldset><button type="button" class="primary" disabled={saving} onclick={saveSkill}>{saving ? 'Saving…' : 'Save skill'}</button></section>{/if}
</div>

<style>
  .extensions { max-width:760px; }.extensions .setting-card { display:grid; gap:16px; padding:18px; border:1px solid var(--line); border-radius:10px; background:var(--panel); }.extensions .card-heading h2 { margin:0 0 4px; font-size:14px; }.extensions .card-heading p,.hint { margin:0; color:var(--muted); font-size:11.5px; line-height:1.5; }.extensions .inline-heading { display:flex; justify-content:space-between; gap:12px; }.extension-tabs { display:flex; gap:4px; padding:3px; border:1px solid var(--line); border-radius:7px; background:var(--soft); }.extension-tabs button,.toolbar button,.extension-row button,.editor button,.paste-import button { min-height:32px; padding:6px 9px; border:1px solid var(--line); border-radius:6px; color:var(--ink); background:var(--paper); font:inherit; font-size:12px; cursor:pointer; }.extension-tabs button:disabled,.toolbar button:disabled,.extension-row button:disabled { cursor:not-allowed; opacity:.55; }.extension-tabs button { flex:1; border:0; background:transparent; }.extension-tabs button[aria-selected="true"] { background:var(--panel); box-shadow:0 1px 2px rgba(0,0,0,.1); }.toolbar { display:flex; align-items:center; justify-content:space-between; gap:12px; }.toolbar > span { display:flex; gap:7px; flex-wrap:wrap; justify-content:flex-end; }.toolbar button,.primary { display:inline-flex; align-items:center; gap:5px; }.primary { border-color:var(--accent)!important; color:var(--on-accent,#fff)!important; background:var(--accent)!important; }.paste-import { display:grid; gap:8px; padding:10px; border:1px solid var(--line); border-radius:7px; background:var(--soft); }.paste-import label { display:grid; gap:5px; font-size:12px; color:var(--muted); }.paste-import textarea { min-height:105px; padding:8px; border:1px solid var(--line); border-radius:6px; color:var(--ink); background:var(--paper); font:11px var(--mono,monospace); resize:vertical; }.paste-import span { display:flex; justify-content:flex-end; gap:7px; }.file-input { position:absolute; width:1px; height:1px; overflow:hidden; opacity:0; }.extension-row { display:grid; grid-template-columns:auto minmax(0,1fr) auto auto; align-items:center; gap:10px; padding:10px 0; border-top:1px solid var(--line); }.extension-row > div { display:grid; min-width:0; gap:2px; }.extension-row strong,.extension-row small { overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }.extension-row small { color:var(--muted); font:11px var(--mono,monospace); }.switch { display:grid; gap:2px; color:var(--muted); font-size:10px; text-align:center; }.switch input { accent-color:var(--accent); }.danger { color:#b84c44!important; }.empty,.manage-note,.plugin-note { margin:0; padding:12px; border:1px dashed var(--line); border-radius:7px; color:var(--muted); font-size:12px; line-height:1.5; }.plugin-note strong { color:var(--ink); }.plugin-note p { margin:4px 0 0; }.local-error { margin:0; padding:9px 11px; border:1px solid #b84c44; border-radius:7px; color:#b84c44; font-size:12px; }.saved { display:flex; align-items:flex-start; gap:6px; margin:0; color:var(--accent-ink,var(--accent)); font-size:12px; line-height:1.4; }.editor { gap:12px; }.editor label { display:grid; gap:5px; color:var(--muted); font-size:12px; }.editor label small { font-size:10.5px; line-height:1.35; }.editor input,.editor select,.editor textarea { width:100%; box-sizing:border-box; padding:8px 9px; border:1px solid var(--line); border-radius:6px; color:var(--ink); background:var(--paper); font:12px var(--interface-font,sans-serif); }.editor textarea { min-height:64px; resize:vertical; font-family:var(--mono,monospace); }.editor .skill-content { min-height:250px; }.editor fieldset { display:flex; flex-wrap:wrap; gap:8px 16px; margin:0; padding:10px; border:1px solid var(--line); border-radius:6px; }.editor legend { padding:0 4px; color:var(--muted); font-size:11px; }.editor .agent-choice { display:flex; align-items:center; gap:6px; color:var(--ink); }.editor .agent-choice input { width:auto; }.inline-heading button { min-height:28px; }.hint { margin:0; }.manage-note { border-style:solid; } @container (max-width:530px) { .toolbar { align-items:stretch; flex-direction:column; }.toolbar > span { justify-content:flex-start; }.extension-row { grid-template-columns:auto minmax(0,1fr) auto; }.extension-row .danger { grid-column:3; }.extension-row > button:nth-last-child(2) { grid-column:2; justify-self:start; }.extension-tabs { overflow:auto; }.extension-tabs button { white-space:nowrap; } }
</style>
