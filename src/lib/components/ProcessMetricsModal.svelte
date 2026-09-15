<script lang="ts">
  import { Bot, Code, Monitor, Network, Terminal, Wrench } from '@lucide/svelte';
  import Modal from './Modal.svelte';
  import type { ProcessMetricsProcess, ProcessMetricsSample } from '$lib/types';

  let { open = false, samples = [], error = '', onclose }: {
    open?: boolean;
    samples?: ProcessMetricsSample[];
    error?: string;
    onclose: () => void;
  } = $props();

  const chartWidth = 640;
  const chartHeight = 112;
  const latest = $derived(samples.at(-1) ?? null);
  const previous = $derived(samples.at(-2) ?? null);
  const processKey = (process: ProcessMetricsProcess) => `${process.pid}:${process.startedAt}`;
  function processIcon(process: ProcessMetricsProcess) {
    if (process.pid === latest?.rootPid) return Monitor;
    const name = process.name.toLowerCase();
    if (/codex|claude|opencode|open-code|hermes|acp|agent|minimax|mmx/.test(name)) return Bot;
    if (/node|npm|npx|bun|deno|python|ruby|perl|java/.test(name)) return Code;
    if (/ssh|sshd|tunnel|relay|websocket/.test(name)) return Network;
    if (/zsh|bash|fish|shell|terminal|(^|\/)sh$/.test(name)) return Terminal;
    return Wrench;
  }
  const formatMemory = (bytes: number) => {
    const mib = bytes / (1024 * 1024);
    return mib >= 1024 ? `${(mib / 1024).toFixed(2)} GB` : `${Math.round(mib)} MB`;
  };
  const formatCpu = (value: number | null) => value === null ? '···' : `${value.toFixed(value >= 100 ? 0 : 1)}%`;
  function cpuDelta(current: ProcessMetricsSample, before: ProcessMetricsSample) {
    const elapsed = current.sampledAt - before.sampledAt;
    const cpu = current.cpuTimeMs - before.cpuTimeMs;
    return elapsed > 0 && cpu >= 0 ? cpu / elapsed * 100 : null;
  }
  function processCpu(current: ProcessMetricsSample, before: ProcessMetricsSample, process: ProcessMetricsProcess) {
    const prior = before.processes.find(item => processKey(item) === processKey(process));
    const elapsed = current.sampledAt - before.sampledAt;
    const cpu = prior ? process.cpuTimeMs - prior.cpuTimeMs : -1;
    return elapsed > 0 && cpu >= 0 ? cpu / elapsed * 100 : null;
  }
  const totalCpu = $derived(latest && previous ? cpuDelta(latest, previous) : null);
  const cpuHistory = $derived(samples.slice(1).flatMap((sample, index) => {
    const value = cpuDelta(sample, samples[index]);
    return value === null ? [] : [value];
  }));
  const memoryHistory = $derived(samples.map(sample => sample.residentMemoryBytes / (1024 * 1024)));
  const cpuCeiling = $derived(Math.max(100, ...cpuHistory) * 1.08);
  const memoryCeiling = $derived(Math.max(128, ...memoryHistory) * 1.08);
  function chartPoints(values: number[], ceiling: number, width = chartWidth, height = chartHeight) {
    if (!values.length) return '';
    const xRange = width - 4, yRange = height - 6, divisor = Math.max(1, values.length - 1);
    return values.map((value, index) => {
      const x = 2 + index / divisor * xRange;
      const y = 3 + (1 - Math.min(Math.max(value, 0), ceiling) / ceiling) * yRange;
      return `${x.toFixed(1)},${y.toFixed(1)}`;
    }).join(' ');
  }
  const cpuPoints = $derived(chartPoints(cpuHistory, cpuCeiling));
  const memoryPoints = $derived(chartPoints(memoryHistory, memoryCeiling));
  const historyLabel = $derived(samples.length < 2 ? 'Collecting history…' : `${Math.max(0, Math.round((samples.at(-1)!.sampledAt - samples[0].sampledAt) / 1000))}s history`);

  const rows = $derived.by(() => {
    if (!latest) return [];
    const previousByKey = new Map(previous?.processes.map(process => [processKey(process), process]) ?? []);
    const children = new Map<number, ProcessMetricsProcess[]>();
    for (const process of latest.processes) {
      const siblings = children.get(process.parentPid) ?? [];
      siblings.push(process);
      children.set(process.parentPid, siblings);
    }
    const cpu = (process: ProcessMetricsProcess) => {
      const prior = previousByKey.get(processKey(process));
      const elapsed = latest.sampledAt - (previous?.sampledAt ?? latest.sampledAt);
      return prior && elapsed > 0 ? Math.max(0, process.cpuTimeMs - prior.cpuTimeMs) / elapsed * 100 : null;
    };
    const output: Array<{ process: ProcessMetricsProcess; depth: number; cpu: number | null }> = [];
    const visited = new Set<number>();
    const visit = (process: ProcessMetricsProcess, depth: number) => {
      if (visited.has(process.pid)) return;
      visited.add(process.pid);
      output.push({ process, depth, cpu: cpu(process) });
      for (const child of (children.get(process.pid) ?? []).toSorted((a, b) => (cpu(b) ?? -1) - (cpu(a) ?? -1))) visit(child, depth + 1);
    };
    const root = latest.processes.find(process => process.pid === latest.rootPid);
    if (root) visit(root, 0);
    for (const process of latest.processes) if (!visited.has(process.pid)) visit(process, 0);
    return output;
  });

  function processHistory(process: ProcessMetricsProcess) {
    const values: number[] = [];
    for (let index = 1; index < samples.length; index += 1) {
      const current = samples[index], before = samples[index - 1];
      const currentProcess = current.processes.find(item => processKey(item) === processKey(process));
      if (!currentProcess) continue;
      const value = processCpu(current, before, currentProcess);
      if (value !== null) values.push(value);
    }
    return chartPoints(values, Math.max(100, ...values) * 1.08, 92, 24);
  }
</script>

<Modal title="Resource usage" {open} {onclose} wide>
  <div class="resource-modal">
    {#if error}<p class="metrics-error" role="alert">{error}</p>{/if}
    {#if latest}
      <div class="summary" aria-label="Current resource totals">
        <div><span>CPU</span><strong>{formatCpu(totalCpu)}</strong><small>Monitter + harnesses</small></div>
        <div><span>RAM</span><strong>{formatMemory(latest.residentMemoryBytes)}</strong><small>{latest.processes.length} live {latest.processes.length === 1 ? 'process' : 'processes'}</small></div>
        <div><span>WINDOW</span><strong>{historyLabel}</strong><small>Updated every 2 seconds</small></div>
      </div>
      <div class="charts">
        <section aria-label={`CPU usage graph, currently ${formatCpu(totalCpu)}`}>
          <header><span>CPU usage</span><strong>{formatCpu(totalCpu)}</strong></header>
          <svg viewBox={`0 0 ${chartWidth} ${chartHeight}`} preserveAspectRatio="none" role="img" aria-label="CPU usage over time">
            <line x1="0" y1={chartHeight - 1} x2={chartWidth} y2={chartHeight - 1}/>
            {#if cpuPoints}<polyline points={cpuPoints}/>{/if}
          </svg>
        </section>
        <section class="memory-chart" aria-label={`RAM usage graph, currently ${formatMemory(latest.residentMemoryBytes)}`}>
          <header><span>RAM usage</span><strong>{formatMemory(latest.residentMemoryBytes)}</strong></header>
          <svg viewBox={`0 0 ${chartWidth} ${chartHeight}`} preserveAspectRatio="none" role="img" aria-label="RAM usage over time">
            <line x1="0" y1={chartHeight - 1} x2={chartWidth} y2={chartHeight - 1}/>
            {#if memoryPoints}<polyline points={memoryPoints}/>{/if}
          </svg>
        </section>
      </div>
      <section class="process-list" aria-label="Live process breakdown">
        <header><span>PROCESS</span><span>CPU</span><span>RAM</span><span>RECENT CPU</span></header>
        <div class="process-scroll">
          {#if rows.length}
            {#each rows as row (processKey(row.process))}
              {@const points=processHistory(row.process)}
              {@const ProcessIcon=processIcon(row.process)}
              <div class="process-row">
                <div class="process-name" style:padding-left={`${row.depth * 17}px`}>
                  {#if row.depth}<i aria-hidden="true"></i>{/if}
                  <span class="process-icon" aria-hidden="true"><ProcessIcon size={14}/></span>
                  <span>{row.process.pid === latest.rootPid ? 'Monitter' : row.process.name || 'Process'}</span>
                  <small>PID {row.process.pid}</small>
                </div>
                <strong>{formatCpu(row.cpu)}</strong>
                <strong>{formatMemory(row.process.residentMemoryBytes)}</strong>
                <svg class="process-sparkline" viewBox="0 0 92 24" preserveAspectRatio="none" aria-hidden="true">
                  <line x1="0" y1="23" x2="92" y2="23"/>
                  {#if points}<polyline points={points}/>{/if}
                </svg>
              </div>
            {/each}
          {:else}
            <p class="process-empty">Per-process details will appear after the native app is rebuilt and relaunched.</p>
          {/if}
        </div>
      </section>
    {:else if !error}
      <p class="empty">Collecting the first process sample…</p>
    {/if}
  </div>
</Modal>

<style>
  .resource-modal{display:grid;gap:18px}.metrics-error,.empty{margin:0;padding:24px;color:var(--muted);text-align:center}.metrics-error{color:var(--danger)}
  .summary{display:grid;grid-template-columns:repeat(3,minmax(0,1fr));gap:10px}.summary>div{display:grid;gap:3px;padding:12px 14px;border:1px solid var(--line);border-radius:9px;background:var(--soft)}.summary span,.process-list>header{color:var(--muted);font:600 calc(9px * var(--interface-font-ratio,1)) var(--mono);letter-spacing:.08em}.summary strong{font:600 calc(19px * var(--interface-font-ratio,1)) var(--mono)}.summary small{overflow:hidden;color:var(--muted);font-size:calc(10px * var(--interface-font-ratio,1));text-overflow:ellipsis;white-space:nowrap}
  .charts{display:grid;grid-template-columns:1fr 1fr;gap:12px}.charts section{min-width:0;padding:11px 12px 8px;border:1px solid var(--line);border-radius:9px;background:color-mix(in srgb,var(--panel) 84%,var(--soft))}.charts header{display:flex;justify-content:space-between;gap:10px;margin-bottom:7px;font-size:calc(11px * var(--interface-font-ratio,1))}.charts header span{color:var(--muted)}.charts header strong{font:500 calc(11px * var(--interface-font-ratio,1)) var(--mono)}.charts svg{display:block;width:100%;height:112px;overflow:visible}.charts line,.process-sparkline line{stroke:var(--line);stroke-width:1;vector-effect:non-scaling-stroke}.charts polyline,.process-sparkline polyline{fill:none;stroke:var(--accent);stroke-width:1.7;stroke-linecap:round;stroke-linejoin:round;vector-effect:non-scaling-stroke}.memory-chart polyline{stroke:#6e9f82}
  .process-list{overflow:hidden;border:1px solid var(--line);border-radius:9px}.process-list>header,.process-row{display:grid;grid-template-columns:minmax(190px,1fr) 72px 82px 100px;align-items:center;gap:10px}.process-list>header{padding:8px 12px;border-bottom:1px solid var(--line);background:var(--soft)}.process-list>header span:not(:first-child){text-align:right}.process-scroll{max-height:min(36vh,360px);overflow-y:auto;overscroll-behavior:contain;scrollbar-gutter:stable}.process-row{min-height:42px;padding:4px 12px;border-bottom:1px solid color-mix(in srgb,var(--line) 65%,transparent)}.process-row:last-child{border-bottom:0}.process-row>strong{text-align:right;font:500 calc(11px * var(--interface-font-ratio,1)) var(--mono)}.process-name{position:relative;display:flex;align-items:center;gap:8px;min-width:0}.process-name i{position:absolute;left:3px;width:9px;height:9px;border-bottom:1px solid var(--line);border-left:1px solid var(--line)}.process-name>span:not(.process-icon){overflow:hidden;text-overflow:ellipsis;white-space:nowrap;font-size:calc(12px * var(--interface-font-ratio,1));font-weight:500}.process-icon{display:grid;flex:none;place-items:center;width:24px;height:24px;border:1px solid color-mix(in srgb,var(--line) 78%,transparent);border-radius:6px;color:var(--muted);background:color-mix(in srgb,var(--soft) 68%,transparent)}.process-name small{flex:none;color:var(--muted);font:calc(9px * var(--interface-font-ratio,1)) var(--mono)}.process-sparkline{justify-self:end;width:92px;height:24px}
  .process-empty{margin:0;padding:24px;color:var(--muted);font-size:calc(11px * var(--interface-font-ratio,1));line-height:1.5;text-align:center}
  @media(max-width:700px){.summary{grid-template-columns:1fr}.charts{grid-template-columns:1fr}.process-list>header,.process-row{grid-template-columns:minmax(130px,1fr) 58px 72px}.process-list>header span:last-child,.process-sparkline{display:none}}
</style>
