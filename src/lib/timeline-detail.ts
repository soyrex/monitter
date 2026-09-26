import { readableToolDetail } from './activity-grouping';
import type { RunEvent } from './types';

const MAX_LINES = 80;
const MAX_CHARACTERS = 6_000;
const MAX_DEPTH = 5;
const MAX_ITEMS = 10;
const MAX_FIELD_CHARACTERS = 1_200;

const priority = ['error', 'message', 'summary', 'command', 'cmd', 'query', 'path', 'status', 'phase', 'tool', 'name', 'model', 'output', 'result', 'content', 'update', 'usage', 'changes'];
const diagnosticKeys = new Set(['id', 'requestId', 'sessionId', 'threadId', 'parentThreadId', 'senderThreadId', 'receiverThreadIds', 'providerTurnId', 'traceId', 'createdAt', 'updatedAt']);
const unhelpfulToolSummaries = new Set(['No additional details.', 'Completed without additional output.', 'Tool completed without additional output.']);

type JsonRecord = Record<string, unknown>;
const isRecord = (value: unknown): value is JsonRecord => value !== null && typeof value === 'object' && !Array.isArray(value);

function label(key: string): string {
  const words = key.replace(/([a-z\d])([A-Z])/g, '$1 $2').replace(/([A-Z])([A-Z][a-z])/g, '$1 $2').replace(/[_-]+/g, ' ').trim();
  return (words.charAt(0).toUpperCase() + words.slice(1)).replace(/\b(id|url|uri|cpu|ram|api|mcp|acp)\b/gi, word => word.toUpperCase());
}

function scalar(value: string | number | boolean, key: string): string {
  if (typeof value === 'boolean') return value ? 'Yes' : 'No';
  if (typeof value === 'number') return `${value}${/percent$/i.test(key) ? '%' : ''}`;
  const text = /^(status|phase|type|kind|sessionUpdate)$/.test(key) ? label(value) : value.trim();
  if (/^data:[^,]+;base64,/i.test(text) || /^[A-Za-z\d+/]{256,}={0,2}$/.test(text)) return 'Binary data available in raw details';
  return text.length <= MAX_FIELD_CHARACTERS ? text : `${text.slice(0, MAX_FIELD_CHARACTERS).trimEnd()}…`;
}

function humanize(value: unknown): string {
  const lines: string[] = [];
  let characters = 0;
  let omitted = false;
  function add(text: string): boolean {
    if (omitted) return false;
    if (lines.length >= MAX_LINES || characters + text.length > MAX_CHARACTERS) { omitted = true; return false; }
    lines.push(text);
    characters += text.length;
    return true;
  }
  function render(key: string, item: unknown, depth: number, indent = ''): void {
    if (omitted || item === null || item === undefined || item === '') return;
    const heading = key ? label(key) : '';
    if (typeof item === 'string' && depth < MAX_DEPTH && /^[\[{]/.test(item.trim())) {
      try {
        const nested = JSON.parse(item);
        if (isRecord(nested) || Array.isArray(nested)) { render(key, nested, depth + 1, indent); return; }
      } catch { /* Keep ordinary text as text. */ }
    }
    if (typeof item === 'string' || typeof item === 'number' || typeof item === 'boolean') {
      const displayed = scalar(item, key);
      const parts = displayed.split(/\r?\n/);
      if (parts.length === 1) add(`${indent}${heading ? `${heading}: ` : ''}${parts[0]}`);
      else {
        if (heading) add(`${indent}${heading}:`);
        for (const part of parts) if (!add(`${indent}${heading ? '  ' : ''}${part}`)) break;
      }
      return;
    }
    if (depth >= MAX_DEPTH) { add(`${indent}${heading}: Further details available in raw data`); return; }
    if (Array.isArray(item)) {
      if (!item.length) return;
      if (item.every(entry => entry === null || ['string', 'number', 'boolean'].includes(typeof entry))) {
        const values = item.slice(0, MAX_ITEMS).filter(entry => entry !== null).map(entry => scalar(entry as string | number | boolean, key));
        const summary = values.join(', ');
        add(`${indent}${heading}: ${summary.length > MAX_FIELD_CHARACTERS ? `${summary.slice(0, MAX_FIELD_CHARACTERS).trimEnd()}…` : summary}${item.length > MAX_ITEMS ? `, and ${item.length - MAX_ITEMS} more` : ''}`);
      } else {
        if (heading) add(`${indent}${heading}:`);
        for (const [index, entry] of item.slice(0, MAX_ITEMS).entries()) render(`Item ${index + 1}`, entry, depth + 1, `${indent}  `);
        if (item.length > MAX_ITEMS) add(`${indent}  … ${item.length - MAX_ITEMS} more items available in raw data`);
      }
      return;
    }
    if (!isRecord(item)) return;
    const entries = Object.entries(item)
      .filter(([name, entry]) => !diagnosticKeys.has(name) && entry !== null && entry !== undefined && entry !== '' && (!Array.isArray(entry) || entry.length > 0))
      .sort(([left], [right]) => {
        const leftRank = priority.indexOf(left), rightRank = priority.indexOf(right);
        return (leftRank < 0 ? priority.length : leftRank) - (rightRank < 0 ? priority.length : rightRank);
      });
    if (!entries.length) return;
    if (heading) add(`${indent}${heading}:`);
    for (const [name, entry] of entries) render(name, entry, depth + 1, `${indent}${heading ? '  ' : ''}`);
  }
  render('', value, 0);
  if (omitted) lines.push('… additional details available in raw data');
  return lines.join('\n') || 'No additional details.';
}

function parsedJson(value: string): unknown {
  let parsed: unknown = JSON.parse(value);
  // A few harnesses serialize JSON inside a JSON string transport envelope.
  for (let depth = 0; depth < 2 && typeof parsed === 'string'; depth += 1) {
    try { parsed = JSON.parse(parsed); }
    catch { break; }
  }
  return parsed;
}

export function formatTimelineDetail(event: RunEvent): { text: string; structured: boolean } {
  const raw = event.detail.trim();
  if (!raw) return { text: 'No additional details.', structured: false };
  let parsed: unknown;
  try { parsed = parsedJson(raw); }
  catch { return { text: raw.length <= MAX_CHARACTERS ? raw : `${raw.slice(0, MAX_CHARACTERS).trimEnd()}\n… additional details omitted`, structured: false }; }
  if (parsed === null) return { text: 'No additional details.', structured: false };
  if (!isRecord(parsed) && !Array.isArray(parsed)) return { text: scalar(parsed as string | number | boolean, ''), structured: false };
  if (event.kind === 'tool') {
    const specific = readableToolDetail(event).trim();
    if (specific && !unhelpfulToolSummaries.has(specific)) {
      const text = specific.split('\n').map(line => {
        const trimmed = line.trim();
        if (!/^[\[{]/.test(trimmed)) return line;
        try {
          const nested = JSON.parse(trimmed);
          return isRecord(nested) || Array.isArray(nested) ? humanize(nested) : line;
        } catch { return line; }
      }).join('\n');
      return { text, structured: true };
    }
  }
  return { text: humanize(parsed), structured: true };
}

export function rawTimelineDetail(value: string): string {
  try { return JSON.stringify(parsedJson(value), null, 2); }
  catch { return value; }
}
