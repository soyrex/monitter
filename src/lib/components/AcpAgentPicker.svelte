<script lang="ts">
  import { Plus, Search, RefreshCw, X, Plug, Check } from '@lucide/svelte';
  import { getBridge } from '../bridge';
  import type { AcpCandidate, AcpLaunch, AcpProbeResult } from '../types';

  let { hostId, launch, disabled = false, onchange }: {
    hostId: string; launch?: AcpLaunch | null; disabled?: boolean;
    onchange: (launch: AcpLaunch, suggestedName?: string) => void;
  } = $props();
  const bridge = getBridge();
  let candidates = $state<AcpCandidate[]>([]);
  let query = $state('');
  let loading = $state(false);
  let error = $state('');
  let generation = 0;
  let resultHost = $state('');
  let verifying = $state(false);
  let verified = $state<AcpProbeResult | null>(null);
  let verifyError = $state('');
  const verifyKey = $derived(JSON.stringify([hostId, launch]));
  $effect(() => { verifyKey; verified = null; verifyError = ''; });
  const matches = $derived(candidates.filter(c =>
    `${c.name} ${c.description} ${c.integration}`.toLowerCase().includes(query.trim().toLowerCase())));

  async function discover(target: string) {
    const request = ++generation;
    loading = true;
    error = '';
    candidates = [];
    resultHost = '';
    try {
      const result = await bridge.discoverAcpAgents(target);
      if (request !== generation || target !== hostId) return;
      candidates = result;
      resultHost = target;
    } catch (reason) {
      if (request === generation) error = String(reason);
    } finally {
      if (request === generation) loading = false;
    }
  }

  $effect(() => {
    const target = hostId;
    if (target) void discover(target);
    return () => { generation++; };
  });

  function patch(value: Partial<AcpLaunch>) {
    onchange({ command: launch?.command ?? '', args: [...(launch?.args ?? [])], ...value });
  }
  function changeArgument(index: number, value: string) {
    const args = [...(launch?.args ?? [])];
    args[index] = value;
    patch({ args });
  }
  async function verify() {
    if (!launch?.command.trim() || verifying) return;
    const key = verifyKey;
    verifying = true; verifyError = ''; verified = null;
    try {
      const result = await bridge.verifyAcpAgent(hostId, {command:launch.command,args:[...launch.args]});
      if (key === verifyKey) verified = result;
    } catch (reason) {
      if (key === verifyKey) verifyError = String(reason);
    } finally { verifying = false; }
  }
</script>

<section class="acp-setup" aria-label="Launch behaviour">
  <div class="catalog-heading"><div><b>Configure launch behaviour</b><p>Choose a preset or use any compatible executable.</p></div>
    <button type="button" class="refresh" disabled={disabled || loading || !hostId} onclick={() => discover(hostId)} aria-label="Refresh installed presets"><RefreshCw size={14}/>Refresh</button>
  </div>
  <label class="search"><Search size={15}/><input type="search" aria-label="Search presets" placeholder="Search presets…" bind:value={query} disabled={disabled}/></label>
  {#if loading}<p class="hint" role="status">Checking executable locations on this host…</p>{/if}
  {#if error}<p class="error" role="alert">{error} You can still enter a custom executable below.</p>{/if}
  <div class="catalog" aria-label="Presets">
    {#if resultHost === hostId}
      {#each matches as candidate (candidate.id)}
        <button type="button" class="candidate" disabled={disabled} onclick={() => onchange({ command: candidate.launch.command, args: [...candidate.launch.args] }, candidate.name)}>
          <span class="candidate-title"><b>{candidate.name}</b><small class:detected={candidate.detected}>{candidate.detected ? 'Detected' : 'Not found'}</small></span>
          <span class="description">{candidate.description}</span>
          <code class="launch-preview">{candidate.launch.command}{candidate.launch.args.length ? ` ${candidate.launch.args.join(' ')}` : ''}</code>
        </button>
      {:else}
        {#if !loading}<p class="hint">No presets match. You can still use a custom executable.</p>{/if}
      {/each}
    {/if}
    <button type="button" class="candidate custom" disabled={disabled} onclick={() => onchange({command:'',args:[]})}><Plug size={15}/><span><b>Custom executable</b><span class="description">Any compatible launch or installed bridge</span></span></button>
  </div>
  <p class="hint">Detection only locates executables. It does not start an agent, prove compatibility, or check sign-in. Nothing is installed automatically.</p>
  <label>Executable<input aria-label="Executable" value={launch?.command ?? ''} disabled={disabled} placeholder="/path/to/agent or executable-name" oninput={event => patch({command:event.currentTarget.value})}/></label>
  <div class="argument-heading"><b>Arguments</b><button type="button" disabled={disabled || (launch?.args.length ?? 0) >= 64} onclick={() => patch({args:[...(launch?.args ?? []),'']})}><Plus size={13}/>Add argument</button></div>
  {#each launch?.args ?? [] as argument, index}
    <div class="argument"><input aria-label={`Argument ${index+1}`} value={argument} disabled={disabled} oninput={event => changeArgument(index,event.currentTarget.value)}/><button type="button" aria-label={`Remove argument ${index+1}`} disabled={disabled} onclick={() => patch({args:launch!.args.filter((_,i)=>i!==index)})}><X size={14}/></button></div>
  {/each}
  <p class="hint">One argument per row, without shell quoting. Use the agent's existing sign-in or environment for credentials; do not put secrets in arguments.</p>
  <button type="button" class="verify" disabled={disabled || verifying || !launch?.command.trim()} onclick={verify}><Plug size={14}/>{verifying ? 'Checking…' : 'Verify connection'}</button>
  {#if verified}<p class="verified" role="status"><Check size={14}/>v{verified.protocolVersion} verified{verified.agentName ? ` · ${verified.agentName}` : ''}{verified.agentVersion ? ` ${verified.agentVersion}` : ''}</p>{/if}
  {#if verifyError}<p class="error" role="alert">{verifyError}</p>{/if}
  <p class="hint">Verification launches this executable for a handshake only. It does not sign in, create a chat, or send a model request.</p>
</section>

<style>
  .acp-setup{display:flex;flex-direction:column;gap:10px;border:1px solid var(--line);border-radius:8px;padding:14px;min-width:0}
  .catalog-heading,.argument-heading,.candidate-title,.argument{display:flex;align-items:center;justify-content:space-between;gap:10px}
  .catalog-heading p,.hint{margin:4px 0 0;font-size:12px;color:var(--muted);line-height:1.45}
  b{font-weight:500}.search{display:flex;align-items:center;gap:8px}.search input{width:100%;min-width:0}
  .catalog{display:flex;flex-direction:column;gap:5px;max-height:260px;overflow:auto;overscroll-behavior:contain}
  .candidate{text-align:left;display:flex;flex-direction:column;gap:5px;padding:10px;background:transparent;border:1px solid var(--line);border-radius:6px;color:inherit;flex-shrink:0}
  .candidate-title{width:100%}.candidate-title small{font-size:10px;color:var(--muted);white-space:nowrap}.candidate-title .detected{color:var(--accent-ink,var(--accent))}
  .description{display:block;color:var(--muted);font-size:12px;line-height:1.4}.custom{flex-direction:row;align-items:center;gap:9px}
  .launch-preview{color:var(--muted);font:10px/1.4 var(--mono);overflow-wrap:anywhere;white-space:normal}
  .candidate:focus-visible{outline:2px solid var(--accent);outline-offset:-2px}
  label{display:flex;flex-direction:column;gap:6px;font-size:12px}.search{flex-direction:row}
  input{min-width:0;background:var(--bg);border:1px solid var(--line);border-radius:5px;color:inherit;padding:8px;font:inherit}
  .argument input{flex:1}.argument-heading b{font-size:12px}
  .refresh,.argument-heading button,.argument button{display:inline-flex;align-items:center;justify-content:center;gap:5px;flex-shrink:0;color:inherit;background:transparent;border:1px solid var(--line);border-radius:5px;padding:6px;font-size:11px}
  .verify{display:inline-flex;align-items:center;justify-content:center;gap:7px;align-self:flex-start;padding:9px;border:1px solid var(--line);border-radius:5px;background:var(--panel);color:inherit;font:inherit;font-size:12px;min-height:40px}.verified{display:flex;align-items:center;gap:6px;font-size:12px;color:var(--accent-ink,var(--accent));margin:0;overflow-wrap:anywhere}
  button{cursor:pointer}button:disabled{opacity:.5;cursor:default}.error{color:var(--danger,#d96060);font-size:12px;margin:0}
  @media(hover:hover) and (pointer:fine){.candidate:hover{border-color:var(--accent)}}
  @media(max-width:760px),(pointer:coarse){input{font-size:16px}.candidate{min-height:48px}.refresh,.argument-heading button,.argument button{min-height:40px}}
</style>
