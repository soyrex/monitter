<script lang="ts">
  import { onMount } from 'svelte';
  import GeometryPane from './transcript-geometry-pane.svelte';
  import { perfMark } from '../../src/lib/perf-phases';

  type Message = { id: string; role: 'user' | 'assistant'; text: string; createdAt: number; streamStatus: 'streaming' | 'complete' };
  type Chat = { id: string; title: string; messages: Message[] };
  type Geometry = { pane: string; top: number; height: number; gap: number; following: boolean; held: boolean };
  type Summary = { count: number; total: number; max: number; samples: number[] };

  const chatCount = 4;
  const messagesPerChat = 1_000;
  const now = Date.now();
  const content = (chat: number, index: number) => index % 9 === 0
    ? `### Synthetic transcript ${chat + 1}/${index + 1}\n\nThis is **synthetic Markdown** for virtualized geometry measurement. ${'A wrapped content segment. '.repeat(15)}`
    : `Synthetic transcript ${chat + 1}, message ${index + 1}. ${'Virtualized Markdown content wraps across the pane. '.repeat(8)}`;
  const makeChat = (chat: number): Chat => ({
    id: `synthetic-geometry-${chat}`,
    title: `Synthetic chat ${chat + 1}`,
    messages: Array.from({ length: messagesPerChat }, (_, index) => ({
      id: `synthetic-geometry-${chat}-${index}`,
      role: index % 3 === 0 ? 'user' : 'assistant',
      text: content(chat, index),
      createdAt: now - (messagesPerChat - index) * 1_000,
      streamStatus: index === messagesPerChat - 1 ? 'streaming' : 'complete',
    })),
  });

  // Like production bridge snapshots, this large immutable fixture must not
  // acquire one reactive proxy per nested transcript field.
  let chats = $state.raw<Chat[]>(Array.from({ length: chatCount }, (_, index) => makeChat(index)));
  let selected = $state(0);
  let twoPanes = $state(false);
  let running = $state(false);
  let resetRevision = $state(0);
  let streamTimer: number | undefined;
  let stopTimer: number | undefined;
  let streamTicks = $state(0);
  let geometry = $state<Record<string, Geometry>>({});
  let frameMax = $state(0);
  let frameSampleCount = $state(0);
  let phases = $state<Record<string, Summary>>({});
  let eventSummaries = $state<Record<string, Summary>>({ typing: emptySummary(), wheel: emptySummary(), chatClick: emptySummary() });
  let frameLast: number | undefined;
  let frameRequest: number | undefined;
  let frameMaxRaw = 0;
  let frameLastPublishedAt = 0;
  const frameSamples: number[] = [];
  let fixtureError = $state<string | null>(null);

  const selectedChat = $derived(chats[selected]!);
  const secondaryIndex = $derived((selected + 1) % chats.length);
  const secondaryChat = $derived(chats[secondaryIndex]!);
  function emptySummary(): Summary { return { count: 0, total: 0, max: 0, samples: [] }; }
  function quantile(values: number[], percentile: number) {
    if (!values.length) return 0;
    const sorted = [...values].sort((left, right) => left - right);
    // Nearest-rank quantiles: p95 of a two/three-event sample must include its
    // maximum, not silently select the lower order statistic.
    return sorted[Math.max(0, Math.min(sorted.length - 1, Math.ceil(sorted.length * percentile) - 1))]!;
  }
  function nextSummary(previous: Summary | undefined, value: number): Summary {
    const current = previous ?? emptySummary();
    return { count: current.count + 1, total: current.total + value, max: Math.max(current.max, value), samples: [...current.samples.slice(-119), value] };
  }
  const formatSummary = (summary: Summary | undefined) => {
    if (!summary?.count) return 'n 0';
    return `n ${summary.count}, p50 ${quantile(summary.samples, .5).toFixed(1)}ms, p95 ${quantile(summary.samples, .95).toFixed(1)}ms, max ${summary.max.toFixed(1)}ms`;
  };
  const formatGeometry = (value: Geometry | undefined) => value
    ? `top ${value.top}px · height ${value.height}px · gap ${value.gap}px · ${value.following ? 'following' : 'reader detached'} · ${value.held ? 'held snapshot' : 'live'}`
    : 'waiting for visible pane geometry';

  function addSummary(kind: string, value: number) {
    eventSummaries = { ...eventSummaries, [kind]: nextSummary(eventSummaries[kind], value) };
  }

  function sampleEvent(kind: string) {
    const start = performance.now();
    requestAnimationFrame(() => requestAnimationFrame(() => addSummary(kind, performance.now() - start)));
  }

  function selectChat(index: number) {
    sampleEvent('chatClick');
    // Mirrors AppSurface's opt-in selection probe. The keyed primary pane
    // then gives TranscriptVirtualList a fresh mount to measure.
    perfMark('chat-open');
    selected = index;
    resetRevision += 1;
  }

  function appendStreams() {
    const tick = streamTicks + 1;
    chats = chats.map((chat, chatIndex) => ({
      ...chat,
      messages: chat.messages.map((message, index) => index === chat.messages.length - 1
        ? { ...message, streamStatus: 'streaming', text: `${message.text}${tick % 30 === 0 ? '\n\n' : ' '}token-${chatIndex + 1}-${tick}` }
        : message),
    }));
    streamTicks = tick;
  }

  function stopStreaming() {
    if (streamTimer !== undefined) window.clearInterval(streamTimer);
    if (stopTimer !== undefined) window.clearTimeout(stopTimer);
    streamTimer = undefined;
    stopTimer = undefined;
    running = false;
  }

  function startStreaming() {
    if (streamTimer !== undefined) return;
    running = true;
    appendStreams();
    streamTimer = window.setInterval(appendStreams, 100);
    stopTimer = window.setTimeout(stopStreaming, 60_000);
  }

  function setGeometry(value: Geometry) {
    geometry = { ...geometry, [value.pane]: value };
  }

  onMount(() => {
    const showError = (reason: unknown) => {
      const detail = reason instanceof Error ? reason.message : typeof reason === 'string' ? reason : 'Unknown runtime error';
      fixtureError = `Fixture runtime error: ${detail}`;
    };
    const handleError = (event: ErrorEvent) => showError(event.error ?? event.message);
    const handleRejection = (event: PromiseRejectionEvent) => showError(event.reason);
    window.addEventListener('error', handleError);
    window.addEventListener('unhandledrejection', handleRejection);
    const observeMeasures = new PerformanceObserver((list) => {
      const next = { ...phases };
      for (const entry of list.getEntries()) {
        if (!entry.name.startsWith('monitter.chat-switch.') && !entry.name.startsWith('monitter.geometry.')) continue;
        next[entry.name] = nextSummary(next[entry.name], entry.duration);
      }
      phases = next;
    });
    observeMeasures.observe({ type: 'measure', buffered: true });

    const frame = (time: number) => {
      if (frameLast !== undefined) {
        const gap = Math.max(0, time - frameLast);
        frameMaxRaw = Math.max(frameMaxRaw, gap);
        frameSamples.push(gap);
        if (frameSamples.length > 240) frameSamples.shift();
        // Do not make the observer itself a per-frame reactive workload.
        if (time - frameLastPublishedAt >= 100) {
          frameLastPublishedAt = time;
          frameMax = frameMaxRaw;
          frameSampleCount = frameSamples.length;
        }
      }
      frameLast = time;
      frameRequest = requestAnimationFrame(frame);
    };
    frameRequest = requestAnimationFrame(frame);

    return () => {
      window.removeEventListener('error', handleError);
      window.removeEventListener('unhandledrejection', handleRejection);
      observeMeasures.disconnect();
      stopStreaming();
      if (frameRequest !== undefined) cancelAnimationFrame(frameRequest);
    };
  });
</script>

<main>
  <header class="fixture-header">
    <div><strong>SYNTHETIC TRANSCRIPT GEOMETRY FIXTURE</strong><span>No real agents, native IPC, providers, or persistent workspace data.</span></div>
    <div class="chat-buttons" aria-label="Synthetic chats">
      {#each chats as chat, index (chat.id)}
        <button class:active={selected === index} class="task-select" type="button" onclick={() => selectChat(index)}>{chat.title}</button>
      {/each}
    </div>
    <div class="fixture-actions">
      <button type="button" onclick={() => twoPanes = !twoPanes}>{twoPanes ? 'Use one pane' : 'Use two panes'}</button>
      <button type="button" onclick={startStreaming} disabled={running}>Start 4 streams</button>
      <button type="button" onclick={stopStreaming} disabled={!running}>Stop streaming</button>
    </div>
  </header>

  <section class:two-panes={twoPanes} class="pane-grid">
    {#key selectedChat.id}<GeometryPane pane="primary" chatId={selectedChat.id} messages={selectedChat.messages} resetRevision={resetRevision} {running} ongeometry={setGeometry}/>{/key}
    {#if twoPanes}{#key secondaryChat.id}<GeometryPane pane="secondary" chatId={secondaryChat.id} messages={secondaryChat.messages} resetRevision={resetRevision} {running} ongeometry={setGeometry}/>{/key}{/if}
  </section>

  <label class="composer-label">Synthetic composer
    <textarea aria-label="Synthetic composer" placeholder="Type here to sample input to two animation frames" oninput={() => sampleEvent('typing')}></textarea>
  </label>

  <section class="metrics" aria-label="Synthetic geometry metrics">
    {#if fixtureError}<p class="fixture-error" role="alert">{fixtureError}</p>{/if}
    <p>Stream: {running ? 'running' : 'stopped'} · ticks {streamTicks} · 4 chats × 1,000 messages · immutable latest-message updates every 100ms · automatic stop at 60 seconds.</p>
    <p>Event-to-two-rAF proxy (not screen paint; nearest-rank percentiles of last 120 samples): input {formatSummary(eventSummaries.typing)}; wheel {formatSummary(eventSummaries.wheel)}; chat click {formatSummary(eventSummaries.chatClick)}.</p>
    <p>Frame gaps: whole-run max {frameMax.toFixed(1)}ms; bounded recent samples {frameSampleCount} (metrics publish at most 10Hz).</p>
    <p>Phase measures cover this fixture's keyed virtual-list remount only; it does not mount TaskTranscript or claim its mount phase.</p>
    <p data-geometry-pane="primary">Primary geometry: {formatGeometry(geometry.primary)}</p>
    {#if twoPanes}<p data-geometry-pane="secondary">Secondary geometry: {formatGeometry(geometry.secondary)}</p>{/if}
    <div aria-label="Performance phase measures">
      {#each Object.entries(phases) as [name, summary] (name)}<p data-phase={name}>{name}: {formatSummary(summary)}, total {summary.total.toFixed(1)}ms (nested phases overlap)</p>{/each}
    </div>
  </section>
</main>

<svelte:window onwheel={(event) => { if (event.target instanceof Element && event.target.closest('.messages')) sampleEvent('wheel'); }} />

<style>
  :global(html, body, #app) { height: 100%; margin: 0; }
  :global(body) { overflow: hidden; color: #1e2932; background: #e9eef0; font-family: system-ui, sans-serif; }
  main { --paper: #fff; --panel: #fff; --soft: #eef7f7; --line: #c8d3d7; --ink: #1e2932; --muted: #5e6f76; --accent: #08767c; --accent-ink: #075d62; --on-accent: #fff; --code: #edf1f2; --mono: ui-monospace, SFMono-Regular, Menlo, monospace; height: 100%; display: grid; grid-template-rows: auto minmax(0, 1fr) auto auto; gap: 8px; padding: 8px; box-sizing: border-box; }
  .fixture-header { display: flex; flex-wrap: wrap; align-items: center; gap: 8px 14px; padding: 9px; border: 2px solid #df8a27; border-radius: 8px; background: #fff4dd; }
  .fixture-header strong, .fixture-header span { display: block; }
  .fixture-header strong { font-size: 13px; letter-spacing: .02em; }
  .fixture-header span { margin-top: 2px; font-size: 11px; }
  .chat-buttons, .fixture-actions { display: flex; flex-wrap: wrap; gap: 5px; }
  button { min-height: 28px; border: 1px solid #aab9bd; border-radius: 5px; color: var(--ink); background: #fff; cursor: pointer; }
  button.active { color: #fff; border-color: var(--accent); background: var(--accent); }
  button:disabled { opacity: .55; cursor: default; }
  .pane-grid { min-height: 0; display: grid; grid-template-columns: minmax(0, 1fr); gap: 8px; }
  .pane-grid.two-panes { grid-template-columns: repeat(2, minmax(0, 1fr)); }
  .composer-label { display: grid; gap: 3px; color: var(--muted); font-size: 11px; }
  textarea { min-height: 42px; resize: vertical; box-sizing: border-box; padding: 7px; border: 1px solid var(--line); border-radius: 6px; font: 13px system-ui; }
  .metrics { max-height: 150px; overflow: auto; padding: 7px 9px; border: 1px solid var(--line); border-radius: 7px; background: #f7fafb; font: 11px/1.4 var(--mono); }
  .metrics p { margin: 0 0 3px; }
  .fixture-error { padding: 5px; border: 1px solid #b84343; border-radius: 4px; color: #8d2424; background: #fff0f0; font-weight: 700; }
  @media (max-width: 700px) { .pane-grid.two-panes { grid-template-columns: minmax(0, 1fr); } }
</style>
