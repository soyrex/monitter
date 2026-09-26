<script lang="ts">
  import type { RunEvent } from '$lib/types';

  type JsonRecord = Record<string, unknown>;
  type DecisionRecord = {
    toolName: string;
    question: string;
    labels: Record<string, string>;
    response: JsonRecord;
    provider: string | null;
    model: string | null;
    latencyMs: number | null;
    inputTokens: number | null;
    outputTokens: number | null;
    costUsd: number | null;
    createdAt: number;
    compact: boolean;
  };
  type Segment = { id: string; label: string; probability: number; selected: boolean };
  type DetailLoader = (event: RunEvent, onChunk: (detail: string) => void) => Promise<string>;

  let { event, onloaddetail }: { event: RunEvent; onloaddetail?: DetailLoader } = $props();
  let expandedDetail = $state<string | null>(null);
  let loadingDetail = $state(false);
  let detailError = $state('');

  function object(value: unknown): value is JsonRecord {
    return !!value && typeof value === 'object' && !Array.isArray(value);
  }
  function parseRecord(raw: string): DecisionRecord | null {
    try {
      const value: unknown = JSON.parse(raw);
      if (!object(value) || typeof value.toolName !== 'string' || typeof value.question !== 'string' || !object(value.response)) return null;
      const labels = object(value.labels)
        ? Object.fromEntries(Object.entries(value.labels).filter((entry): entry is [string, string] => typeof entry[1] === 'string'))
        : {};
      const numberOrNull = (key: string): number | null => typeof value[key] === 'number' && Number.isFinite(value[key]) ? value[key] as number : null;
      return {
        toolName: value.toolName,
        question: value.question,
        labels,
        response: value.response,
        provider: typeof value.provider === 'string' ? value.provider : null,
        model: typeof value.model === 'string' ? value.model : null,
        latencyMs: numberOrNull('latencyMs'),
        inputTokens: numberOrNull('inputTokens'),
        outputTokens: numberOrNull('outputTokens'),
        costUsd: numberOrNull('costUsd'),
        createdAt: numberOrNull('createdAt') ?? event.createdAt,
        compact: value.compact === true,
      };
    } catch { return null; }
  }

  const record = $derived(parseRecord(expandedDetail ?? event.detail));
  const response = $derived(record?.response ?? null);
  const toolLabel = $derived(record?.toolName === 'jev_route' ? 'Route' : record?.toolName === 'jev_choose' ? 'Choice' : response?.kind === 'score' ? 'Score' : response?.kind === 'noul' ? 'Noul' : 'Assessment');
  function numberField(key: string): number | null {
    const value = response?.[key];
    return typeof value === 'number' && Number.isFinite(value) ? value : null;
  }
  const probabilities = $derived.by(() => object(response?.probabilities)
    ? Object.fromEntries(Object.entries(response?.probabilities as JsonRecord).filter((entry): entry is [string, number] => typeof entry[1] === 'number' && Number.isFinite(entry[1]) && entry[1] >= 0 && entry[1] <= 1))
    : {});
  const segments = $derived.by((): Segment[] => {
    if (!record || !response || response.status !== 'ok') return [];
    if (response.kind === 'noul') {
      const yes = numberField('probabilityYes');
      return yes === null ? [] : [
        { id: 'yes', label: 'Yes', probability: yes, selected: true },
        { id: 'no', label: 'No', probability: 1 - yes, selected: false },
      ];
    }
    const candidateId = typeof response.candidateId === 'string' ? response.candidateId : '';
    const entries = Object.entries(probabilities);
    const compactOther = numberField('otherProbability');
    if (record.compact && record.toolName === 'jev_choose' && compactOther !== null) {
      const selected = entries[0];
      return [
        ...(selected ? [{ id: selected[0], label: record.labels[selected[0]] ?? selected[0], probability: selected[1], selected: true }] : []),
        { id: '__other_summary__', label: 'Other options', probability: compactOther, selected: false },
      ].filter(segment => segment.probability > 0);
    }
    return entries
      .map(([id, probability]) => ({ id, label: record.labels[id] ?? id, probability, selected: id === candidateId }))
      .filter(segment => segment.probability > 0)
      .sort((a, b) => response.kind === 'score' ? Number(a.id) - Number(b.id) : b.probability - a.probability);
  });

  function outcomeText(): string {
    if (!record || !response) return 'Decision details unavailable';
    if (response.status === 'unavailable') return 'Jev unavailable';
    if (record.toolName === 'jev_route') return typeof response.outcome === 'string' ? response.outcome : 'Route outcome unavailable';
    if (record.toolName === 'jev_choose') {
      const id = typeof response.candidateId === 'string' ? response.candidateId : '';
      return id === 'abstain' ? 'Abstained' : (record.labels[id] ?? id) || 'No outcome';
    }
    if (response.kind === 'noul') {
      const probability = numberField('probabilityYes');
      return probability === null ? 'No probability returned' : `P(yes) ${Math.round(probability * 100)}%`;
    }
    const score = numberField('score');
    return score === null ? 'No score returned' : `Score ${score.toFixed(2)}`;
  }

  const confidence = $derived(numberField('confidence'));
  function routeText(key: string): string {
    const value = response?.[key];
    return typeof value === 'string' ? value.replaceAll('_', ' ') : 'n/a';
  }
  function routeReasoningOutcome(): string {
    const level = routeText('reasoningLevel');
    switch (response?.reasoningStatus) {
      case 'selected': return `${level} reasoning selected for this chat`;
      case 'applied': return `${level} reasoning applied by the harness`;
      case 'already_current': return `${level} reasoning already active`;
      case 'pending': return `${level} reasoning · checking harness support`;
      case 'unsupported': return `${level} recommended · harness default kept`;
      case 'rejected': return `${level} rejected · harness default kept`;
      case 'unconfirmed': return `${level} reasoning · harness response unconfirmed`;
      default: return `${level} reasoning recommended`;
    }
  }
  function routeModelOutcome(): string {
    const model = routeText('selectedModel');
    switch (response?.modelStatus) {
      case 'selected': return `${model} selected for this chat`;
      case 'applied': return `${model} applied by the harness`;
      case 'already_current': return `${model} already active`;
      case 'pending': return `${model} requested · checking harness`;
      case 'unsupported': return `${model} unavailable to the harness`;
      case 'rejected': return `${model} rejected by the harness`;
      case 'unconfirmed': return `${model} · harness response unconfirmed`;
      case 'recommended': return 'Jev recommendation only';
      default: return response?.applied !== true ? 'Default kept' : response.modelChanged === true ? 'Selected for this chat' : 'Default model kept';
    }
  }
  function routePermissionSignal(): string {
    switch (response?.permissionTier) {
      case 'human_review_required': return 'Consequential action';
      case 'workspace_write': return 'Workspace changes';
      case 'read_only': return 'Read only';
      default: return 'Unspecified';
    }
  }
  const timeText = $derived(new Intl.DateTimeFormat(undefined, { hour: '2-digit', minute: '2-digit' }).format(record?.createdAt ?? event.createdAt));
  const fullTimeText = $derived(new Intl.DateTimeFormat(undefined, { dateStyle: 'medium', timeStyle: 'short' }).format(record?.createdAt ?? event.createdAt));
  function latencyText(value: number | null): string { return value === null ? 'Latency n/a' : value < 1000 ? `${Math.round(value)} ms` : `${(value / 1000).toFixed(2)} s`; }
  function costText(value: number | null): string {
    if (value === null) return 'Cost n/a';
    return new Intl.NumberFormat(undefined, { style: 'currency', currency: 'USD', maximumSignificantDigits: 3 }).format(value);
  }
  function segmentColor(segment: Segment, index: number): string {
    if (segment.selected || segment.id === 'yes') return 'var(--accent)';
    const tint = Math.max(18, 48 - index * 5);
    return `color-mix(in srgb, var(--accent) ${tint}%, var(--line))`;
  }
  function distributionLabel(values = segments): string {
    return values.map(segment => `${segment.label} ${Math.round(segment.probability * 100)}%`).join(', ');
  }
  function detailRows(): Segment[] {
    if (!record || !response || response.status !== 'ok') return [];
    if (response.kind === 'noul') return segments;
    const candidateId = typeof response.candidateId === 'string' ? response.candidateId : '';
    const rows = Object.entries(probabilities)
      .map(([id, probability]) => ({ id, label: record.labels[id] ?? id, probability, selected: id === candidateId }))
      .sort((a, b) => response.kind === 'score' ? Number(a.id) - Number(b.id) : b.probability - a.probability);
    const otherProbability = numberField('otherProbability');
    if (record.compact && otherProbability !== null && otherProbability > 0) {
      rows.push({ id: '__other_summary__', label: 'Other options', probability: otherProbability, selected: false });
    }
    return rows;
  }
  async function onToggle(eventValue: Event) {
    const details = eventValue.currentTarget as HTMLDetailsElement;
    if (!details.open || !record?.compact || !onloaddetail || expandedDetail !== null || loadingDetail) return;
    loadingDetail = true;
    detailError = '';
    try {
      expandedDetail = await onloaddetail(event, () => {});
    } catch (error) {
      detailError = error instanceof Error ? error.message : String(error);
    } finally {
      loadingDetail = false;
    }
  }
</script>

<details class="jev-card" ontoggle={onToggle}>
  <summary aria-label={`Jev ${toolLabel}: ${outcomeText()}`}>
    <div class="jev-top"><span class="jev-mark">Jev</span><span class="jev-kind">{toolLabel}</span><time title={fullTimeText}>{timeText}</time></div>
    {#if record}<div class="jev-question">{record.question}</div>{:else}<div class="jev-question">Jev decision details unavailable</div>{/if}
    <div class="jev-result-line"><strong title={outcomeText()}>{outcomeText()}</strong>{#if confidence !== null}<span>{record?.toolName === 'jev_route' ? 'Lowest confidence' : 'Confidence'} {Math.round(confidence * 100)}%</span>{/if}</div>
    {#if record?.toolName === 'jev_route'}<div class="jev-route-reasoning">{routeReasoningOutcome()}</div>{/if}
    {#if segments.length}<div class="jev-bar" role="img" aria-label={distributionLabel()} title={distributionLabel()}>{#each segments as segment, index (segment.id)}<span style={`width:${segment.probability * 100}%;background:${segmentColor(segment, index)}`}></span>{/each}</div>{/if}
    {#if record?.toolName === 'jev_route' && confidence !== null}<div class="jev-bar" role="img" aria-label={`Lowest routing confidence ${Math.round(confidence * 100)}%`}><span style={`width:${confidence * 100}%;background:var(--accent)`}></span></div>{/if}
    <div class="jev-meta"><span>{latencyText(record?.latencyMs ?? null)}</span><span>{costText(record?.costUsd ?? null)}</span><span class="jev-expand-hint">{record?.compact && expandedDetail === null ? 'Open for details' : 'Details'}</span></div>
  </summary>
  <div class="jev-details">
    {#if loadingDetail}<p class="jev-detail-state">Loading full result…</p>{/if}
    {#if detailError}<p class="jev-detail-state error">Full result unavailable: {detailError}</p>{/if}
    {#if record}<p class="jev-full-question"><span>Question</span>{record.question}</p>{/if}
    {#if response && response.status !== 'unavailable'}
      {#if detailRows().length}<div class="jev-breakdown" aria-label="Jev probability breakdown">{#each detailRows() as segment (segment.id)}<div class="jev-breakdown-row"><span class:selected={segment.selected}>{segment.label}</span><div class="jev-track"><i style={`width:${segment.probability * 100}%;background:${segmentColor(segment, 0)}`}></i></div><b>{Math.round(segment.probability * 100)}%</b></div>{/each}</div>{/if}
      {#if record?.toolName === 'jev_route'}
        <dl class="jev-route-detail">
          <div><dt>Model tier</dt><dd>{routeText('modelTier')}</dd></div>
          <div><dt>Reasoning</dt><dd>{routeText('reasoningLevel')}</dd></div>
          <div><dt>Chat model</dt><dd>{routeText('selectedModel')}</dd></div>
          <div><dt>Model selection</dt><dd>{routeModelOutcome()}</dd></div>
          <div><dt>Reasoning outcome</dt><dd>{routeReasoningOutcome()}</dd></div>
          <div><dt>Task kind</dt><dd>{routeText('taskKind')}</dd></div>
          <div><dt>Permission signal</dt><dd>{routePermissionSignal()} · advisory only; existing approvals apply</dd></div>
        </dl>
      {/if}
    {:else if response?.status === 'unavailable'}<p class="jev-detail-state">Jev could not return a decision for this request.</p>{/if}
    {#if record}<dl class="jev-usage"><div><dt>Model</dt><dd>{record.model ?? 'n/a'}</dd></div><div><dt>Provider</dt><dd>{record.provider ?? 'n/a'}</dd></div><div><dt>Time</dt><dd>{fullTimeText}</dd></div><div><dt>Latency</dt><dd>{latencyText(record.latencyMs)}</dd></div><div><dt>Cost</dt><dd>{costText(record.costUsd)}</dd></div><div><dt>Tokens</dt><dd>{record.inputTokens ?? 'n/a'} in · {record.outputTokens ?? 'n/a'} out</dd></div></dl>{/if}
  </div>
</details>

<style>
  .jev-card { width:min(100%,560px); margin:7px 0; border:1px solid var(--line); border-radius:9px; color:var(--ink); background:var(--panel); box-shadow:0 1px 2px #0000000a; font-size:calc(11px * var(--interface-font-ratio,1)); }
  .jev-card > summary { display:grid; gap:5px; padding:8px 10px; cursor:pointer; list-style:none; }
  .jev-card > summary::-webkit-details-marker { display:none; }
  .jev-top { display:flex; align-items:center; gap:6px; min-width:0; color:var(--muted); font:10px var(--mono); }
  .jev-mark { color:var(--accent); font-weight:700; }
  .jev-kind { padding:1px 5px; border-radius:4px; background:var(--soft); }
  .jev-top time { margin-left:auto; font-variant-numeric:tabular-nums; }
  .jev-question { display:-webkit-box; overflow:hidden; color:var(--ink); line-height:1.35; -webkit-box-orient:vertical; -webkit-line-clamp:2; }
  .jev-result-line { display:flex; align-items:center; gap:9px; min-width:0; }
  .jev-result-line strong { overflow:hidden; color:var(--ink); font-size:11px; font-weight:650; text-overflow:ellipsis; white-space:nowrap; }
  .jev-result-line span { flex:none; color:var(--accent); font:10px var(--mono); }
  .jev-route-reasoning { overflow:hidden; color:var(--muted); font:9px var(--mono); text-overflow:ellipsis; white-space:nowrap; }
  .jev-bar,.jev-track { display:flex; overflow:hidden; height:5px; border-radius:4px; background:var(--soft); }
  .jev-bar span { min-width:1px; height:100%; }
  .jev-meta { display:flex; align-items:center; gap:10px; color:var(--muted); font:9px var(--mono); font-variant-numeric:tabular-nums; }
  .jev-expand-hint { margin-left:auto; }
  .jev-details { display:grid; gap:9px; padding:0 10px 9px; }
  .jev-breakdown { display:grid; gap:5px; }
  .jev-breakdown-row { display:grid; grid-template-columns:minmax(90px,1fr) minmax(70px,1.5fr) 36px; align-items:center; gap:8px; min-width:0; }
  .jev-breakdown-row > span { overflow:hidden; color:var(--muted); text-overflow:ellipsis; white-space:nowrap; }
  .jev-breakdown-row > span.selected { color:var(--ink); font-weight:650; }
  .jev-breakdown-row b { color:var(--muted); text-align:right; font:10px var(--mono); font-variant-numeric:tabular-nums; }
  .jev-track { height:4px; }.jev-track i { display:block; height:100%; border-radius:inherit; }
  .jev-usage { display:flex; flex-wrap:wrap; gap:5px 12px; margin:0; padding-top:7px; border-top:1px solid var(--line); }
  .jev-route-detail { display:grid; grid-template-columns:repeat(2,minmax(0,1fr)); gap:5px 12px; margin:0; }
  .jev-route-detail div { display:grid; gap:2px; min-width:0; }
  .jev-route-detail dt { color:var(--muted); }
  .jev-route-detail dd { overflow:hidden; margin:0; color:var(--ink); text-overflow:ellipsis; white-space:nowrap; }
  .jev-usage div { display:flex; gap:4px; min-width:0; }
  .jev-usage dt { color:var(--muted); }.jev-usage dd { overflow:hidden; margin:0; color:var(--ink); text-overflow:ellipsis; white-space:nowrap; }
  .jev-full-question { display:grid; gap:3px; margin:0; color:var(--ink); font-size:10px; line-height:1.4; overflow-wrap:anywhere; }.jev-full-question span { color:var(--muted); font:9px var(--mono); }
  .jev-detail-state { margin:0; color:var(--muted); font-size:10px; line-height:1.4; }.jev-detail-state.error { color:var(--danger,#b84c44); }
  @media (prefers-reduced-motion:no-preference) { .jev-bar span { transition:width 160ms ease-out; } }
</style>
