<script lang="ts">
  import { onMount } from 'svelte';
  import { Check, Eye, EyeOff, KeyRound, LoaderCircle, Pencil, Plus, Trash2, X } from '@lucide/svelte';
  import { getBridge } from '$lib/bridge';
  import { isLanBrowser } from '$lib/lan';
  import type { EnvironmentSecretMetadata, EnvironmentSecretsConfig } from '$lib/types';

  const bridge = getBridge();
  const desktopAvailable = !isLanBrowser() && bridge.available;
  let config = $state<EnvironmentSecretsConfig>({ revision: '', entries: [] });
  let loading = $state(desktopAvailable);
  let saving = $state(false);
  let error = $state('');
  let editing = $state<EnvironmentSecretMetadata | null>(null);
  let name = $state('');
  let description = $state('');
  let value = $state('');
  let showDraft = $state(false);

  onMount(() => { if (desktopAvailable) void load(); });

  async function load() {
    loading = true;
    error = '';
    try { config = await bridge.listEnvironmentSecrets(); }
    catch (reason) { error = message(reason); }
    finally { loading = false; }
  }

  function message(reason: unknown) {
    return reason instanceof Error ? reason.message : String(reason);
  }

  function beginAdd() {
    editing = { name: '', description: '', updatedAt: 0 };
    name = '';
    description = '';
    value = '';
    showDraft = false;
    error = '';
  }

  function beginReplace(entry: EnvironmentSecretMetadata) {
    editing = entry;
    name = entry.name;
    description = entry.description;
    value = '';
    showDraft = false;
    error = '';
  }

  function cancelEdit() {
    editing = null;
    value = '';
    showDraft = false;
  }

  async function save() {
    if (!editing || !name.trim() || !value) return;
    saving = true;
    error = '';
    try {
      config = await bridge.setEnvironmentSecret(config.revision, name.trim(), value, description.trim());
      cancelEdit();
    } catch (reason) { error = message(reason); }
    finally { saving = false; }
  }

  async function remove(entry: EnvironmentSecretMetadata) {
    if (!confirm(`Remove ${entry.name} from macOS Keychain? Local agents launched afterwards will no longer receive it.`)) return;
    saving = true;
    error = '';
    try {
      config = await bridge.deleteEnvironmentSecret(config.revision, entry.name);
      if (editing?.name === entry.name) cancelEdit();
    } catch (reason) { error = message(reason); }
    finally { saving = false; }
  }
</script>

{#if !desktopAvailable}
  <div class="section-stack">
    <section class="setting-card locked" aria-labelledby="desktop-secrets-heading">
      <KeyRound size={20}/>
      <div><h2 id="desktop-secrets-heading">Manage secrets on the desktop</h2><p>Environment secrets stay on the Mac running Monitter. Browser, mobile, and shared visitors cannot list or change them.</p></div>
    </section>
  </div>
{:else}
  <div class="section-stack" data-environment-secrets-settings>
    <section class="setting-card intro" aria-labelledby="environment-secrets-heading">
      <div class="card-heading"><h2 id="environment-secrets-heading">Shared agent environment</h2><p>Store API keys and other sensitive environment variables in macOS Keychain. They are supplied to every current and future local user agent when its harness next launches.</p></div>
      <div class="scope-note"><KeyRound size={16}/><span>Values are never shown again after saving. Local agents and commands they start can read them. Existing sessions keep their current environment, and SSH agents do not receive these secrets.</span></div>
      <button type="button" class="primary add" disabled={loading || saving || !!editing} onclick={beginAdd}><Plus size={15}/> Add secret</button>
    </section>

    {#if error}<div class="error" role="alert"><span>{error}</span><button type="button" disabled={loading || saving} onclick={() => void load()}>Refresh</button></div>{/if}

    {#if editing}
      <section class="setting-card editor" aria-labelledby="secret-editor-heading">
        <div class="inline-heading"><strong id="secret-editor-heading">{editing.name ? `Replace ${editing.name}` : 'Add environment secret'}</strong><button type="button" class="icon" aria-label="Cancel secret edit" onclick={cancelEdit}><X size={16}/></button></div>
        <label>Variable name<input aria-label="Variable name" bind:value={name} disabled={!!editing.name} maxlength="64" autocapitalize="none" autocomplete="off" spellcheck="false" placeholder="OPENAI_API_KEY"/></label>
        <label>Description <small>Optional. Never include the secret itself.</small><input aria-label="Secret description" bind:value={description} maxlength="160" autocomplete="off" placeholder="Used for image generation"/></label>
        <label>Secret value
          <span class="secret-input"><input aria-label="Secret value" type={showDraft ? 'text' : 'password'} bind:value autocomplete="new-password" spellcheck="false"/><button type="button" aria-label={showDraft ? 'Hide entered value' : 'Show entered value'} aria-pressed={showDraft} onclick={() => showDraft = !showDraft}>{#if showDraft}<EyeOff size={16}/>{:else}<Eye size={16}/>{/if}</button></span>
        </label>
        <p class="hint">Variable names use letters, numbers, and underscores and cannot replace Monitter runtime variables. Saving an existing name replaces its value.</p>
        <button type="button" class="primary" disabled={saving || !name.trim() || !value} onclick={save}>{#if saving}<LoaderCircle class="spin" size={15}/> Saving…{:else}<Check size={15}/> {editing.name ? 'Replace secret' : 'Save secret'}{/if}</button>
      </section>
    {/if}

    <section class="setting-card" aria-labelledby="saved-secrets-heading">
      <div class="card-heading"><h2 id="saved-secrets-heading">Saved secrets</h2><p>Only variable names and descriptions are visible here.</p></div>
      {#if loading}<div class="empty"><LoaderCircle class="spin" size={18}/> Loading Keychain…</div>
      {:else if config.entries.length === 0}<div class="empty"><KeyRound size={18}/><span>No environment secrets saved.</span></div>
      {:else}<div class="secret-list">
        {#each config.entries as entry (entry.name)}
          <article class="secret-row">
            <div class="secret-copy"><code>{entry.name}</code><span class="masked" aria-label="Secret value stored">••••••••••••</span>{#if entry.description}<p>{entry.description}</p>{/if}</div>
            <div class="row-actions"><button type="button" aria-label={`Replace ${entry.name}`} title="Replace secret" disabled={saving || !!editing} onclick={() => beginReplace(entry)}><Pencil size={15}/></button><button type="button" class="danger" aria-label={`Remove ${entry.name}`} title="Remove secret" disabled={saving} onclick={() => void remove(entry)}><Trash2 size={15}/></button></div>
          </article>
        {/each}
      </div>{/if}
    </section>
  </div>
{/if}

<style>
  .section-stack{display:grid;gap:14px;max-width:760px;margin:0 auto}.setting-card{display:grid;gap:16px;padding:18px;border:1px solid var(--line);border-radius:10px;background:var(--panel);box-shadow:0 1px 2px rgba(0,0,0,.025)}.card-heading h2,.locked h2{margin:0 0 4px;font-size:calc(14px * var(--interface-font-ratio,1))}.card-heading p,.locked p,.hint{margin:0;color:var(--muted);font-size:calc(11.5px * var(--interface-font-ratio,1));line-height:1.5}.intro{grid-template-columns:minmax(0,1fr) auto}.intro .card-heading,.intro .scope-note{grid-column:1/-1}.scope-note{display:flex;align-items:flex-start;gap:9px;padding:11px 12px;border:1px solid color-mix(in srgb,var(--accent) 24%,var(--line));border-radius:8px;background:color-mix(in srgb,var(--accent) 7%,transparent);color:var(--muted);font-size:calc(11px * var(--interface-font-ratio,1));line-height:1.45}.scope-note :global(svg){flex:none;color:var(--accent)}button{font:inherit}.primary{display:inline-flex;width:max-content;align-items:center;justify-content:center;gap:7px;padding:8px 12px;border:1px solid var(--accent);border-radius:7px;color:var(--on-accent,#fff);background:var(--accent);cursor:pointer}.primary:disabled,button:disabled{cursor:not-allowed;opacity:.5}.add{grid-column:2;grid-row:1;align-self:start}.inline-heading{display:flex;align-items:center;justify-content:space-between;gap:12px}.inline-heading strong{color:var(--accent-ink,var(--accent));font:600 calc(13px * var(--interface-font-ratio,1)) var(--mono,monospace)}.icon,.row-actions button,.secret-input button{display:grid;place-items:center;padding:7px;border:1px solid var(--line);border-radius:6px;color:var(--muted);background:var(--paper);cursor:pointer}.editor label{display:grid;gap:6px;color:var(--ink);font-size:calc(11px * var(--interface-font-ratio,1));font-weight:600}.editor label small{color:var(--muted);font-weight:400}.editor input{box-sizing:border-box;width:100%;padding:8px 10px;border:1px solid var(--line);border-radius:6px;color:var(--ink);background:var(--paper);font:500 calc(12px * var(--interface-font-ratio,1)) var(--mono,monospace)}.editor input:focus{outline:2px solid color-mix(in srgb,var(--accent) 32%,transparent);border-color:var(--accent)}.secret-input{display:grid;grid-template-columns:minmax(0,1fr) auto;gap:7px}.secret-input button{width:36px}.secret-list{display:grid;gap:8px}.secret-row{display:flex;align-items:center;justify-content:space-between;gap:16px;padding:12px;border:1px solid var(--line);border-radius:8px;background:var(--paper)}.secret-copy{display:grid;grid-template-columns:auto auto;align-items:center;gap:5px 12px;min-width:0}.secret-copy code{overflow:hidden;color:var(--ink);font:600 calc(12px * var(--interface-font-ratio,1)) var(--mono,monospace);text-overflow:ellipsis}.masked{color:var(--muted);font:600 10px var(--mono,monospace);letter-spacing:.08em}.secret-copy p{grid-column:1/-1;margin:0;color:var(--muted);font-size:calc(11px * var(--interface-font-ratio,1))}.row-actions{display:flex;gap:6px;flex:none}.row-actions .danger{color:#b84c44}.empty{display:flex;align-items:center;justify-content:center;gap:8px;padding:18px;color:var(--muted);font-size:calc(11.5px * var(--interface-font-ratio,1))}.locked{grid-template-columns:auto minmax(0,1fr);align-items:start}.locked :global(svg){color:var(--accent)}.error{display:flex;align-items:center;justify-content:space-between;gap:12px;max-width:760px;margin:0 auto;padding:10px 12px;border:1px solid color-mix(in srgb,#b84c44 34%,var(--line));border-radius:7px;color:#b84c44;background:color-mix(in srgb,#b84c44 7%,transparent);font-size:calc(11.5px * var(--interface-font-ratio,1))}.error button{padding:5px 8px;border:1px solid currentColor;border-radius:5px;color:inherit;background:transparent;cursor:pointer}:global(svg.spin){animation:spin .85s linear infinite}@keyframes spin{to{transform:rotate(360deg)}}@container (max-width:520px){.intro{grid-template-columns:1fr}.add{grid-column:1;grid-row:auto}.secret-row{align-items:flex-start}.secret-copy{grid-template-columns:1fr}.masked{display:none}}
</style>
