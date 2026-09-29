<script lang="ts">
  /**
   * A small pacing chart: how the reported used% compares to an even
   * (linear) rate across the window, so a fast burn is visible before the
   * window resets. There is no historical series to plot, only the current
   * snapshot, so "used so far" is drawn as a straight line from window start
   * to now and projected forward at the same rate to the reset.
   */
  let { usedPercent, resetsAt, now, windowDays = 7 }: {
    usedPercent: number;
    resetsAt: number;
    now: number;
    windowDays?: number;
  } = $props();

  const windowMs = $derived(windowDays * 24 * 60 * 60 * 1000);
  const start = $derived(resetsAt - windowMs);
  const elapsedFraction = $derived(windowMs > 0 ? Math.max(0, Math.min(1, (now - start) / windowMs)) : 0);
  const idealPercent = $derived(elapsedFraction * 100);
  const used = $derived(Math.max(0, Math.min(100, usedPercent)));
  // Below ~3% elapsed the projection is too noisy to be useful (a single
  // early check reads as an extreme rate); show the current split instead.
  const projectedPercent = $derived(elapsedFraction > 0.03 ? used / elapsedFraction : used);
  const delta = $derived(used - idealPercent);

  type Status = 'ahead' | 'on' | 'behind';
  const status: Status = $derived(delta > 6 ? 'ahead' : delta < -6 ? 'behind' : 'on');

  const statusColor: Record<Status, string> = {
    ahead: 'rgb(205 76 76)',
    on: 'rgb(63 157 106)',
    behind: 'rgb(90 150 220)',
  };
  const statusLabel: Record<Status, string> = {
    ahead: 'Ahead of pace',
    on: 'On pace',
    behind: 'Behind pace',
  };
  const statusHint: Record<Status, string> = {
    ahead: 'Using faster than an even weekly rate — may run out before reset.',
    on: 'Tracking an even weekly rate.',
    behind: 'Using slower than an even weekly rate — headroom to use more.',
  };

  const color = $derived(statusColor[status]);

  // Plot geometry: x runs window-start (0) to reset (W); y runs 0% (bottom,
  // H) to 100% (top, 0). Kept in a fixed local box, independent of layout.
  const W = 280;
  const H = 64;
  const nowX = $derived(elapsedFraction * W);
  const usedY = $derived(H - (used / 100) * H);
  const overshoots = $derived(projectedPercent > 100.5);
  // The dashed projection stops where it crosses 100% used, rather than
  // running off the top of the plot, so an overrun reads as "before reset".
  const projectedEndX = $derived(overshoots && projectedPercent > used
    ? nowX + (W - nowX) * ((100 - used) / (projectedPercent - used))
    : W);
  const projectedEndY = $derived(overshoots ? 0 : H - (projectedPercent / 100) * H);

  const actualPath = $derived(`M 0 ${H} L ${nowX.toFixed(1)} ${usedY.toFixed(1)}`);
  const projectedPath = $derived(`M ${nowX.toFixed(1)} ${usedY.toFixed(1)} L ${projectedEndX.toFixed(1)} ${projectedEndY.toFixed(1)}`);

  const percentFormat = (value: number) => `${Math.round(value)}%`;
  const endTooltip = $derived(`Used ${percentFormat(used)} vs an even-pace target of ${percentFormat(idealPercent)} at this point in the week`);
  const projectionTooltip = $derived(overshoots
    ? `At this rate you'd hit 100% before the window resets`
    : `At this rate you'd finish the week around ${percentFormat(projectedPercent)} used`);
</script>

<div class="pacing-chart">
  <div class="pacing-head">
    <span class="pacing-status" style={`color:${color}`}>
      <span class="dot" style={`background:${color}`} aria-hidden="true"></span>
      {statusLabel[status]}
    </span>
    <span class="pacing-projection">
      {overshoots ? 'Projected to exceed 100% before reset' : `Projected ${percentFormat(projectedPercent)} by reset`}
    </span>
  </div>

  <svg viewBox={`0 0 ${W} ${H}`} class="pacing-svg" role="img" aria-label={`${statusLabel[status]}. ${statusHint[status]}`}>
    <line x1="0" y1={H} x2={W} y2={H} class="axis" />
    <line x1="0" y1="0" x2={W} y2="0" class="axis" />
    <line x1="0" y1={H} x2={W} y2="0" class="ideal">
      <title>Even pace: using the allowance at a constant rate across the week</title>
    </line>
    <line x1={nowX} y1="0" x2={nowX} y2={H} class="now-marker" />
    <path d={actualPath} class="actual" style={`stroke:${color}`} />
    <path d={projectedPath} class="projected" style={`stroke:${color}`} />
    <circle cx={nowX} cy={usedY} r="4" style={`fill:${color}`} class="end-dot"><title>{endTooltip}</title></circle>
    <circle cx={projectedEndX} cy={projectedEndY} r="4" style={`fill:${color}`} class="end-dot projected-dot"><title>{projectionTooltip}</title></circle>
  </svg>

  <p class="pacing-hint">{statusHint[status]}</p>
</div>

<style>
  .pacing-chart { display:grid; gap:8px; }
  .pacing-head { display:flex; align-items:baseline; justify-content:space-between; gap:8px; flex-wrap:wrap; }
  .pacing-status { display:inline-flex; align-items:center; gap:6px; font-size:calc(11.5px * var(--interface-font-ratio, 1)); font-weight:600; }
  .pacing-status .dot { width:8px; height:8px; border-radius:50%; flex:none; }
  .pacing-projection { color:var(--muted); font-size:calc(10.5px * var(--interface-font-ratio, 1)); }
  .pacing-svg { display:block; width:100%; height:auto; overflow:visible; }
  .axis { stroke:var(--line); stroke-width:1; }
  .ideal { stroke:var(--muted); stroke-width:1.5; stroke-dasharray:4 3; }
  .now-marker { stroke:var(--line); stroke-width:1; stroke-dasharray:2 2; }
  .actual { fill:none; stroke-width:2; stroke-linecap:round; }
  .projected { fill:none; stroke-width:2; stroke-linecap:round; stroke-dasharray:3 3; opacity:.55; }
  .end-dot { stroke:var(--soft); stroke-width:2; }
  .pacing-hint { margin:0; color:var(--muted); font-size:calc(10.5px * var(--interface-font-ratio, 1)); line-height:1.5; }
</style>
