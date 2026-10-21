// Native NSH Canvas charts. No DOM, timers, network, filesystem, or dependencies.
export const CHART_LIMITS = Object.freeze({ series: 8, points: 32 });
const TAU = Math.PI * 2;
const array = value => Array.isArray(value) ? value : [];
export const chartProps = value => value?.type === 'element' ? (value.props || {}) : (value || {});
const number = value => typeof value === 'number' && Number.isFinite(value) ? Math.max(-1e100, Math.min(1e100, value)) : 0;
const label = value => typeof value === 'string' || typeof value === 'number' ? String(value).slice(0, 72) : '';
const short = (value, limit = 18) => { const s = label(value); return s.length > limit ? s.slice(0, limit - 1) + '…' : s; };
const clamp = (v, lo, hi) => Math.max(lo, Math.min(hi, v));

export function normalizeChart(type, input) {
  const p = chartProps(input);
  if (type === 'ScatterChart') {
    return { ...p, datasets: array(p.datasets).slice(0, 8).map((entry, i) => {
      const d = chartProps(entry);
      return { name: label(d.name) || `Series ${i + 1}`, points: array(d.points).slice(0, 32).map(entry => {
        const point = chartProps(entry);
        return { x: number(point.x), y: number(point.y), z: number(point.z) };
      }) };
    }) };
  }
  const labels = array(p.labels).slice(0, 32).map(label);
  if (['PieChart', 'RadialChart', 'SingleStackedBarChart'].includes(type)) {
    const count = Math.min(32, Math.max(labels.length, array(p.values).length));
    return { ...p, labels: Array.from({length: count}, (_, i) => labels[i] || `${i + 1}`), values: Array.from({length: count}, (_, i) => number(p.values?.[i])) };
  }
  const series = array(p.series).slice(0, 8).map((entry, i) => {
    const s = chartProps(entry);
    return { category: label(s.category) || `Series ${i + 1}`, values: array(s.values).slice(0, 32).map(number) };
  });
  const count = Math.min(32, Math.max(labels.length, ...series.map(s => s.values.length), 0));
  return { ...p, labels: Array.from({length: count}, (_, i) => labels[i] || `${i + 1}`), series: series.map(s => ({ ...s, values: Array.from({length: count}, (_, i) => s.values[i] || 0) })) };
}

export function valueDomain(values, includeZero = true) {
  let lo = includeZero ? 0 : Infinity, hi = includeZero ? 0 : -Infinity;
  for (const value of values) { const v = typeof value === 'number' && Number.isFinite(value) ? value : 0; lo = Math.min(lo, v); hi = Math.max(hi, v); }
  if (!Number.isFinite(lo) || !Number.isFinite(hi)) return [0, 1];
  if (lo === hi) return lo === 0 ? [0, 1] : [lo - Math.max(1, Math.abs(lo) * .1), hi + Math.max(1, Math.abs(hi) * .1)];
  return [lo, hi];
}
export function stackSegments(series, count) {
  const positive = Array(count).fill(0), negative = Array(count).fill(0);
  return series.map(s => s.values.map((value, i) => {
    const v = number(value), totals = v < 0 ? negative : positive, from = totals[i];
    totals[i] += v;
    return { from, to: totals[i], value: v };
  }));
}

// Every series color is derived from the current theme accent. Alternate hue
// offsets keep adjacent series distinguishable in both light and dark themes.
export function chartPalette(accent = '#86a9ff', count = 8) {
  const match = /^#([0-9a-f]{3}|[0-9a-f]{6}|[0-9a-f]{8})$/i.exec(accent);
  let rgb = match?.[1] || '86a9ff';
  if (rgb.length === 3) rgb = [...rgb].map(c => c + c).join('');
  const channels = [0, 2, 4].map(i => parseInt(rgb.slice(i, i + 2), 16) / 255);
  const max = Math.max(...channels), min = Math.min(...channels), delta = max - min, light = (max + min) / 2;
  let hue = 0;
  if (delta) {
    const [r, g, b] = channels;
    hue = 60 * (max === r ? ((g - b) / delta) % 6 : max === g ? (b - r) / delta + 2 : (r - g) / delta + 4);
  }
  const saturation = clamp(delta ? delta / (1 - Math.abs(2 * light - 1)) * 100 : 55, 45, 80);
  return Array.from({length: Math.min(32, Math.max(0, count))}, (_, i) => {
    if (!i && match) return accent;
    const h = ((hue + i * 137.508) % 360 + 360) % 360;
    return `hsl(${h.toFixed(1)}, ${saturation.toFixed(1)}%, ${clamp(light * 100 + (i % 3 - 1) * 7, 38, 72).toFixed(1)}%)`;
  });
}

const format = value => Math.abs(value) >= 1e6 || (Math.abs(value) > 0 && Math.abs(value) < .001) ? value.toExponential(1) : Math.abs(value) >= 1000 ? `${+(value / 1000).toFixed(1)}k` : String(+value.toFixed(2));
function text(ctx, value, x, y, color, align = 'left', maxWidth) {
  ctx.fillStyle = color; ctx.textAlign = align; ctx.textBaseline = 'middle';
  if (maxWidth !== undefined) ctx.fillText(label(value), x, y, Math.max(1, maxWidth));
  else ctx.fillText(label(value), x, y);
}
function line(ctx, x1, y1, x2, y2) { ctx.moveTo(x1, y1); ctx.lineTo(x2, y2); }
function legend(ctx, entries, colors, width, height, theme) {
  const shown = entries.slice(0, 8), columns = Math.min(4, Math.max(1, Math.floor(width / 110)));
  const rows = Math.ceil(shown.length / columns), cell = (width - 16) / columns;
  shown.forEach((entry, i) => {
    const x = 8 + i % columns * cell, y = height - rows * 17 + Math.floor(i / columns) * 17 + 7;
    ctx.fillStyle = colors[i]; ctx.fillRect(x, y - 4, 8, 8);
    text(ctx, short(entry, Math.max(5, Math.floor((cell - 22) / 6))), x + 13, y, theme.muted, 'left', cell - 18);
  });
  return rows * 17 + 8;
}
function cartesian(ctx, size, p, domainX, domainY, theme, horizontal = false) {
  const categorical = domainX == null;
  const left = horizontal ? Math.min(92, size.width * .25) : 53;
  const plot = { x: left, y: 12, w: Math.max(1, size.width - left - 16), h: Math.max(1, size.height - 52 - (p.xLabel ? 16 : 0)) };
  const scale = (value, domain, span) => (value - domain[0]) / (domain[1] - domain[0]) * span;
  const y = v => plot.y + plot.h - scale(v, domainY, plot.h);
  const x = v => plot.x + scale(v, domainX || domainY, plot.w);
  ctx.strokeStyle = theme.line; ctx.lineWidth = 1; ctx.beginPath();
  for (let i = 0; i <= 4; i++) {
    const ratio = i / 4;
    if (horizontal) line(ctx, plot.x + plot.w * ratio, plot.y, plot.x + plot.w * ratio, plot.y + plot.h);
    else line(ctx, plot.x, plot.y + plot.h * ratio, plot.x + plot.w, plot.y + plot.h * ratio);
  }
  ctx.stroke();
  for (let i = 0; i <= 4; i++) {
    const ratio = i / 4;
    if (horizontal) text(ctx, format(domainY[0] + ratio * (domainY[1] - domainY[0])), plot.x + ratio * plot.w, plot.y + plot.h + 13, theme.muted, 'center', plot.w / 4);
    else text(ctx, format(domainY[1] - ratio * (domainY[1] - domainY[0])), plot.x - 7, plot.y + ratio * plot.h, theme.muted, 'right', left - 10);
  }
  if (categorical) {
    const stride = Math.max(1, Math.ceil(p.labels.length / (horizontal ? Math.max(1, Math.floor(plot.h / 22)) : Math.max(1, Math.min(8, Math.floor(plot.w / 48))))));
    p.labels.forEach((value, i) => {
      if (i % stride) return;
      if (horizontal) text(ctx, short(value, 14), plot.x - 7, plot.y + (i + .5) / p.labels.length * plot.h, theme.muted, 'right', left - 10);
      else text(ctx, short(value, 10), plot.x + (i + .5) / p.labels.length * plot.w, plot.y + plot.h + 13, theme.muted, 'center', plot.w * stride / p.labels.length - 3);
    });
  } else {
    for (let i = 0; i <= 4; i++) text(ctx, format(domainX[0] + i / 4 * (domainX[1] - domainX[0])), plot.x + i / 4 * plot.w, plot.y + plot.h + 13, theme.muted, 'center', plot.w / 4);
  }
  if (p.xLabel) text(ctx, short(p.xLabel, 42), plot.x + plot.w / 2, size.height - 8, theme.muted, 'center', plot.w);
  if (p.yLabel) {
    ctx.save(); ctx.translate(10, plot.y + plot.h / 2); ctx.rotate(-Math.PI / 2);
    text(ctx, short(p.yLabel, 32), 0, 0, theme.muted, 'center', plot.h); ctx.restore();
  }
  ctx.strokeStyle = theme.muted; ctx.beginPath();
  if (horizontal) line(ctx, x(0), plot.y, x(0), plot.y + plot.h);
  else line(ctx, plot.x, y(0), plot.x + plot.w, y(0));
  ctx.stroke();
  return { ...plot, sx: x, sy: y };
}
function clipPlot(ctx, plot) { ctx.save(); ctx.beginPath(); ctx.rect(plot.x, plot.y, plot.w, plot.h); ctx.clip(); }
function curve(ctx, points, variant) {
  ctx.moveTo(points[0].x, points[0].y);
  for (let i = 1; i < points.length; i++) {
    const a = points[i - 1], b = points[i];
    if (variant === 'step') { ctx.lineTo(b.x, a.y); ctx.lineTo(b.x, b.y); }
    else if (variant === 'natural') {
      // Monotone horizontal tangents avoid overshoot, including signed data.
      const middle = (a.x + b.x) / 2;
      ctx.bezierCurveTo(middle, a.y, middle, b.y, b.x, b.y);
    } else ctx.lineTo(b.x, b.y);
  }
}

function bars(ctx, size, type, p, colors, theme) {
  const horizontal = type === 'HorizontalBarChart', count = p.labels.length, stacked = p.variant === 'stacked';
  const segments = stackSegments(p.series, count);
  const domain = valueDomain(stacked ? segments.flatMap(s => s.flatMap(v => [v.from, v.to])) : p.series.flatMap(s => s.values));
  const plot = cartesian(ctx, size, p, null, domain, theme, horizontal);
  clipPlot(ctx, plot);
  const band = (horizontal ? plot.h : plot.w) / count, groups = stacked ? 1 : p.series.length, thickness = band * .72 / groups;
  p.series.forEach((s, si) => {
    ctx.fillStyle = colors[si];
    s.values.forEach((value, i) => {
      const segment = stacked ? segments[si][i] : {from: 0, to: value};
      const offset = i * band + band * .14 + (stacked ? 0 : si * thickness);
      if (horizontal) { const a = plot.sx(segment.from), b = plot.sx(segment.to); ctx.fillRect(Math.min(a, b), plot.y + offset, Math.abs(a - b), Math.max(.5, thickness - 1)); }
      else { const a = plot.sy(segment.from), b = plot.sy(segment.to); ctx.fillRect(plot.x + offset, Math.min(a, b), Math.max(.5, thickness - 1), Math.abs(a - b)); }
    });
  });
  ctx.restore();
}
function lines(ctx, size, type, p, colors, theme) {
  const plot = cartesian(ctx, size, p, null, valueDomain(p.series.flatMap(s => s.values)), theme);
  clipPlot(ctx, plot);
  p.series.forEach((s, si) => {
    const points = s.values.map((v, i) => ({x: plot.x + (i + .5) / p.labels.length * plot.w, y: plot.sy(v)}));
    ctx.strokeStyle = colors[si]; ctx.fillStyle = colors[si]; ctx.lineWidth = 2;
    ctx.beginPath(); curve(ctx, points, p.variant);
    if (type === 'AreaChart') {
      ctx.lineTo(points.at(-1).x, plot.sy(0)); ctx.lineTo(points[0].x, plot.sy(0)); ctx.closePath();
      ctx.globalAlpha = .2; ctx.fill(); ctx.globalAlpha = 1;
    }
    ctx.stroke();
    if (type !== 'AreaChart' || points.length === 1) {
      const stride = Math.max(1, Math.ceil(points.length / 8));
      ctx.beginPath(); points.forEach((pt, i) => { if (i % stride && i !== points.length - 1) return; ctx.moveTo(pt.x + 2.5, pt.y); ctx.arc(pt.x, pt.y, 2.5, 0, TAU); }); ctx.fill();
    }
  });
  ctx.restore();
}
function scatter(ctx, size, p, colors, theme) {
  const points = p.datasets.flatMap(d => d.points);
  const plot = cartesian(ctx, size, p, valueDomain(points.map(pt => pt.x)), valueDomain(points.map(pt => pt.y)), theme);
  const maxZ = Math.max(1, ...points.map(pt => Math.abs(pt.z)));
  clipPlot(ctx, plot);
  p.datasets.forEach((dataset, si) => {
    ctx.fillStyle = colors[si]; ctx.globalAlpha = .7; ctx.beginPath();
    dataset.points.forEach(pt => { const x = plot.sx(pt.x), y = plot.sy(pt.y), r = pt.z ? 3 + 9 * Math.sqrt(Math.abs(pt.z) / maxZ) : 4; ctx.moveTo(x + r, y); ctx.arc(x, y, r, 0, TAU); });
    ctx.fill();
  });
  ctx.globalAlpha = 1; ctx.restore();
}
function circles(ctx, size, type, p, colors, theme) {
  const semi = type === 'PieChart' && p.appearance === 'semiCircular';
  const radius = Math.max(1, Math.min((size.width - 34) / 2, (size.height - 26) / (semi ? 1 : 2)));
  const cx = size.width / 2, cy = semi ? size.height - 14 : size.height / 2;
  if (type === 'RadialChart') {
    const max = Math.max(1, ...p.values.map(Math.abs)), band = radius / Math.max(1, p.values.length);
    p.values.forEach((v, i) => {
      const r = radius - (i + .5) * band;
      ctx.lineWidth = Math.max(.5, band * .7); ctx.strokeStyle = theme.line; ctx.beginPath(); ctx.arc(cx, cy, r, -Math.PI / 2, Math.PI * 1.5); ctx.stroke();
      if (v) { ctx.strokeStyle = colors[i]; ctx.beginPath(); ctx.arc(cx, cy, r, -Math.PI / 2, -Math.PI / 2 + TAU * Math.abs(v) / max); ctx.stroke(); }
    });
    return;
  }
  const total = p.values.reduce((sum, v) => sum + Math.abs(v), 0), span = semi ? Math.PI : TAU;
  let angle = semi ? -Math.PI : -Math.PI / 2;
  if (!total) { ctx.strokeStyle = theme.line; ctx.lineWidth = p.variant === 'donut' ? radius * .35 : 2; ctx.beginPath(); ctx.arc(cx, cy, radius * (p.variant === 'donut' ? .8 : 1), angle, angle + span); ctx.stroke(); text(ctx, 'No data', cx, semi ? cy - radius / 2 : cy, theme.muted, 'center'); return; }
  p.values.forEach((value, i) => {
    if (!value) return;
    const end = angle + Math.abs(value) / total * span, inner = p.variant === 'donut' ? radius * .6 : 0;
    ctx.fillStyle = colors[i]; ctx.beginPath(); ctx.moveTo(cx + Math.cos(angle) * radius, cy + Math.sin(angle) * radius); ctx.arc(cx, cy, radius, angle, end);
    if (inner) { ctx.lineTo(cx + Math.cos(end) * inner, cy + Math.sin(end) * inner); ctx.arc(cx, cy, inner, end, angle, true); }
    else ctx.lineTo(cx, cy);
    ctx.closePath(); ctx.fill(); angle = end;
  });
}
function singleStack(ctx, size, p, colors, theme) {
  const segments = stackSegments(p.values.map(value => ({values: [value]})), 1);
  const domain = valueDomain(segments.flatMap(s => [s[0].from, s[0].to]));
  const left = 16, width = Math.max(1, size.width - 32), y = size.height / 2 - 12;
  const x = value => left + (value - domain[0]) / (domain[1] - domain[0]) * width;
  ctx.fillStyle = theme.line; ctx.fillRect(left, y, width, 24);
  segments.forEach((s, i) => { ctx.fillStyle = colors[i]; ctx.fillRect(x(Math.min(s[0].from, s[0].to)), y, Math.abs(x(s[0].to) - x(s[0].from)), 24); });
  ctx.strokeStyle = theme.ink; ctx.lineWidth = 1; ctx.beginPath(); line(ctx, x(0), y - 4, x(0), y + 28); ctx.stroke();
  text(ctx, format(domain[0]), left, y + 39, theme.muted);
  text(ctx, format(domain[1]), left + width, y + 39, theme.muted, 'right');
}
function radar(ctx, size, p, colors, theme) {
  const cx = size.width / 2, cy = size.height / 2, radius = Math.max(1, Math.min((size.width - 110) / 2, (size.height - 48) / 2));
  const domain = valueDomain(p.series.flatMap(s => s.values));
  const point = (i, r) => ({ x: cx + Math.sin(i / p.labels.length * TAU) * r, y: cy - Math.cos(i / p.labels.length * TAU) * r });
  const polygon = points => { ctx.moveTo(points[0].x, points[0].y); points.slice(1).forEach(p => ctx.lineTo(p.x, p.y)); ctx.closePath(); };
  ctx.strokeStyle = theme.line; ctx.lineWidth = 1; ctx.beginPath();
  for (let ring = 1; ring <= 4; ring++) polygon(p.labels.map((_, i) => point(i, radius * ring / 4)));
  p.labels.forEach((_, i) => { const pt = point(i, radius); line(ctx, cx, cy, pt.x, pt.y); }); ctx.stroke();
  const stride = Math.max(1, Math.ceil(p.labels.length / 12));
  p.labels.forEach((name, i) => { if (i % stride) return; const pt = point(i, radius + 12); text(ctx, short(name, 12), pt.x, pt.y, theme.muted, pt.x < cx - 4 ? 'right' : pt.x > cx + 4 ? 'left' : 'center', 54); });
  // Label the radial scale so negative and zero values have an explicit position.
  for (let i = 0; i <= 4; i++) text(ctx, format(domain[0] + (domain[1] - domain[0]) * i / 4), cx + 4, cy - radius * i / 4, theme.muted, 'left', 45);
  p.series.forEach((series, si) => {
    ctx.beginPath(); polygon(series.values.map((v, i) => point(i, (v - domain[0]) / (domain[1] - domain[0]) * radius)));
    ctx.fillStyle = colors[si]; ctx.globalAlpha = .15; ctx.fill(); ctx.globalAlpha = 1; ctx.strokeStyle = colors[si]; ctx.lineWidth = 2; ctx.stroke();
  });
}

/** Draw once into NSH's bounded canvas recorder. Returns normalized input for diagnostics. */
export function drawChart(ctx, size, type, props, inputTheme = {}) {
  const p = normalizeChart(type, props);
  const width = clamp(number(size?.width), 0, 16384), height = clamp(number(size?.height), 0, 16384);
  if (width < 100 || height < 90) return p;
  const theme = { accent: '#86a9ff', ink: '#e6e9ef', muted: '#a4aebd', line: '#384353', fontFamily: '.AppleSystemUIFont', ...inputTheme };
  const entries = type === 'ScatterChart' ? p.datasets.map(d => d.name) : p.series ? p.series.map(s => s.category) : p.labels.map((name, i) => `${name}: ${format(p.values[i])}`);
  const colors = chartPalette(theme.accent, Math.max(entries.length, 8));
  ctx.save(); ctx.beginPath(); ctx.rect(0, 0, width, height); ctx.clip(); ctx.font = `11px ${theme.fontFamily}`; ctx.globalAlpha = 1;
  const legendHeight = legend(ctx, entries, colors, width, height, theme);
  const drawingSize = {width, height: Math.max(50, height - legendHeight)};
  const empty = type === 'ScatterChart' ? !p.datasets.some(d => d.points.length) : !p.labels.length || (p.series && !p.series.length);
  if (empty) text(ctx, 'No data', width / 2, drawingSize.height / 2, theme.muted, 'center');
  else if (type === 'BarChart' || type === 'HorizontalBarChart') bars(ctx, drawingSize, type, p, colors, theme);
  else if (type === 'LineChart' || type === 'AreaChart') lines(ctx, drawingSize, type, p, colors, theme);
  else if (type === 'ScatterChart') scatter(ctx, drawingSize, p, colors, theme);
  else if (type === 'RadarChart') radar(ctx, drawingSize, p, colors, theme);
  else if (type === 'SingleStackedBarChart') singleStack(ctx, drawingSize, p, colors, theme);
  else if (type === 'PieChart' || type === 'RadialChart') circles(ctx, drawingSize, type, p, colors, theme);
  ctx.restore();
  return p;
}
