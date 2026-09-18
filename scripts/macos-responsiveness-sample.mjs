#!/usr/bin/env node
import assert from 'node:assert/strict';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';

const execFileAsync = promisify(execFile);

export function parseVmStat(text) {
  const pageMatch = text.match(/page size of (\d+) bytes/i);
  if (!pageMatch) throw new Error('vm_stat page-size header unavailable');
  const pageSize = Number(pageMatch[1]);
  if (!Number.isSafeInteger(pageSize) || pageSize <= 0) throw new Error('vm_stat page size is invalid');
  const wanted = new Set(['swapins', 'swapouts', 'compressions', 'decompressions', 'pageins', 'pageouts']);
  const counters = {};
  for (const line of text.split(/\r?\n/)) {
    const match = line.match(/^Pages?\s+([^:]+):\s*(\d+)/i) ?? line.match(/^([^:]+):\s*(\d+)/);
    if (!match) continue;
    const name = match[1].trim().toLowerCase().replace(/\s+/g, '_');
    if (wanted.has(name)) counters[name] = Number(match[2]);
  }
  return { pageSize, counters, unavailable: [...wanted].filter(name => !(name in counters)) };
}

export function counterDelta(before, after) {
  const keys = new Set([...Object.keys(before), ...Object.keys(after)]);
  const delta = {};
  const resets = [];
  const unavailable = [];
  for (const key of keys) {
    if (!(key in before) || !(key in after)) {
      unavailable.push(key);
      delta[key] = null;
      continue;
    }
    const oldValue = before[key];
    const newValue = after[key];
    if (newValue < oldValue) { resets.push(key); delta[key] = null; }
    else delta[key] = (newValue - oldValue);
  }
  return { delta, resets, unavailable };
}

export function parsePs(text) {
  return text.split(/\r?\n/).map(line => line.trim()).filter(Boolean).map(line => {
    const fields = line.split(/\s+/);
    return { pid: Number(fields[0]), ppid: Number(fields[1]), cpu: Number(fields[2]), rssKb: Number(fields[3]), command: fields.slice(4).join(' ') };
  }).filter(row => Number.isInteger(row.pid) && Number.isFinite(row.cpu) && Number.isFinite(row.rssKb));
}

async function readVmStat() {
  const { stdout } = await execFileAsync('/usr/bin/vm_stat', [], { maxBuffer: 1024 * 1024 });
  return parseVmStat(stdout);
}
async function readProcesses(pids) {
  try {
    const { stdout } = await execFileAsync('/bin/ps', ['-p', pids.join(','), '-o', 'pid=,ppid=,%cpu=,rss=,comm='], { maxBuffer: 128 * 1024 });
    return parsePs(stdout);
  } catch (error) {
    if (error.code === 1) return []; // ps returns 1 when no requested PID exists.
    throw error;
  }
}

function usage() {
  console.error('Usage: macos-responsiveness-sample.mjs --pid=<numeric> [--pid=<numeric> ...] [--seconds=15] [--interval-ms=1000]');
  process.exitCode = 2;
}
function args(argv) {
  const known = /^(--pid=|--seconds=|--interval-ms=)/;
  if (argv.some(value => !known.test(value))) return null;
  const pids = argv.filter(value => value.startsWith('--pid=')).map(value => Number(value.slice(6)));
  const seconds = Number(argv.find(value => value.startsWith('--seconds='))?.slice(10) ?? 15);
  const intervalMs = Number(argv.find(value => value.startsWith('--interval-ms='))?.slice(14) ?? 1000);
  if (!pids.length || pids.length > 16 || pids.some(pid => !Number.isSafeInteger(pid) || pid <= 0) || !Number.isFinite(seconds) || seconds < 1 || seconds > 60 || !Number.isFinite(intervalMs) || intervalMs < 500 || intervalMs > 60000) return null;
  return { pids, seconds, intervalMs };
}
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));

async function sample({ pids, seconds, intervalMs }) {
  const start = Date.now();
  const startedAt = new Date(start).toISOString();
  const firstVm = await readVmStat();
  const maxByPid = new Map();
  const samples = [];
  while (Date.now() - start < seconds * 1000) {
    const processes = await readProcesses(pids);
    const at = Date.now();
    samples.push({ elapsedMs: at - start, processes });
    for (const process of processes) {
      const previous = maxByPid.get(process.pid) ?? { cpu: 0, rssKb: 0 };
      maxByPid.set(process.pid, { cpu: Math.max(previous.cpu, process.cpu), rssKb: Math.max(previous.rssKb, process.rssKb) });
    }
    await sleep(Math.min(intervalMs, Math.max(0, seconds * 1000 - (Date.now() - start))));
  }
  const lastVm = await readVmStat();
  const swap = await execFileAsync('/usr/sbin/sysctl', ['-n', 'vm.swapusage'], { maxBuffer: 32 * 1024 }).then(result => result.stdout.trim()).catch(() => null);
  const { delta, resets, unavailable } = counterDelta(firstVm.counters, lastVm.counters);
  if (firstVm.pageSize !== lastVm.pageSize) throw new Error('vm_stat page size changed during sampling');
  const deltaBytes = Object.fromEntries(Object.entries(delta).map(([key, value]) => [key, value === null ? null : value * lastVm.pageSize]));
  const missing = [...new Set([...unavailable, ...firstVm.unavailable, ...lastVm.unavailable])];
  console.log(JSON.stringify({ startedAt, endedAt: new Date().toISOString(), elapsedMs: Date.now() - start, pids, samples, maxByPid: Object.fromEntries(maxByPid), vmStat: { pageSize: lastVm.pageSize, deltaPages: delta, deltaBytes, resets, unavailable: missing }, swapusage: swap, note: 'ps %CPU is rolling/process-time based, not instantaneous; swap used alone does not prove current thrashing.' }));
}

if (process.argv.includes('--self-test')) {
  const a = parseVmStat('Mach Virtual Memory Statistics: (page size of 16384 bytes)\nPageins: 20\nSwapins: 4\nSwapouts: 2\nCompressions: 8\nDecompressions: 3\nPageouts: 1');
  const b = parseVmStat('Mach Virtual Memory Statistics: (page size of 16384 bytes)\nPageins: 23\nSwapins: 2\nSwapouts: 3\nCompressions: 10\nDecompressions: 4\nPageouts: 2');
  assert.equal(a.pageSize, 16384); assert.deepEqual(counterDelta(a.counters, b.counters).delta, { pageins: 3, swapins: null, swapouts: 1, compressions: 2, decompressions: 1, pageouts: 1 });
  assert.equal(counterDelta(a.counters, b.counters).delta.pageins * b.pageSize, 49152);
  assert.deepEqual(counterDelta({ pageins: 9 }, { pageins: 2 }).resets, ['pageins']);
  assert.deepEqual(counterDelta({ pageins: 9 }, {}).unavailable, ['pageins']);
  assert.equal(counterDelta({ pageins: 9 }, {}).delta.pageins, null);
  assert.throws(() => parseVmStat('Pageins: 3'), /page-size/);
  assert.deepEqual(parsePs(' 123 1 4.2 5120 /Applications/Monitter.app/Contents/MacOS/monitter\n'), [{ pid: 123, ppid: 1, cpu: 4.2, rssKb: 5120, command: '/Applications/Monitter.app/Contents/MacOS/monitter' }]);
  console.log('macOS responsiveness sampler self-test passed');
} else {
  const options = args(process.argv.slice(2));
  if (!options) usage(); else if (process.platform !== 'darwin') { console.error('macOS sampler requires darwin'); process.exitCode = 1; } else await sample(options);
}
