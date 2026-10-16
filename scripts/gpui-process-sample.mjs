// Read-only macOS process observations. No launch, signal, UI action or profile read.
import { execFileSync } from 'node:child_process';
import { dirname, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';
import { setTimeout } from 'node:timers/promises';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const [pidText, countText = '5', intervalText = '1000'] = process.argv.slice(2);
const pid = Number(pidText);
const count = Number(countText);
const interval = Number(intervalText);
if (!Number.isInteger(pid) || pid < 1 || !Number.isInteger(count) || count < 1 || count > 60
    || !Number.isInteger(interval) || interval < 100 || interval > 5000) {
  throw new Error('Usage: node scripts/gpui-process-sample.mjs PID [samples 1..60] [intervalMs 100..5000]');
}
if (process.platform !== 'darwin') throw new Error('This sampler uses macOS ps fields.');
const ps = (args) => execFileSync('/bin/ps', args, { encoding: 'utf8' }).trim();
const identity = () => ({
  executable: ps(['-p', String(pid), '-o', 'comm=']),
  started: ps(['-p', String(pid), '-o', 'lstart=']),
});
const initial = identity();
if (!initial.executable.startsWith(resolve(root, 'artifacts', 'gpui') + sep)
    || !initial.executable.endsWith('/Contents/MacOS/monitter-gpui-desktop')) {
  throw new Error('PID is not a packaged GPUI experimental executable in this fork.');
}
const observations = [];
for (let index = 0; index < count; index += 1) {
  if (index) await setTimeout(interval);
  const current = identity();
  if (current.executable !== initial.executable || current.started !== initial.started) {
    throw new Error('Process identity changed; refusing to mix observations.');
  }
  // Avoid command arguments: child processes may have sensitive invocation data.
  const rows = ps(['-axo', 'pid=,ppid=,rss=,%cpu=,time=']).split('\n').map((line) => {
    const [processId, parentId, rssKiB, cpuPercent, cpuTime] = line.trim().split(/\s+/);
    return { pid: Number(processId), parentPid: Number(parentId), rssKiB: Number(rssKiB),
      cpuPercent: Number(cpuPercent), cpuTime };
  });
  const selected = new Set([pid]);
  let previous;
  do {
    previous = selected.size;
    for (const row of rows) if (selected.has(row.parentPid)) selected.add(row.pid);
  } while (selected.size !== previous);
  const processes = rows.filter((row) => selected.has(row.pid));
  if (!processes.some((row) => row.pid === pid)) throw new Error('Experimental process exited.');
  observations.push({ at: new Date().toISOString(), processes,
    summedRssKiB: processes.reduce((sum, row) => sum + row.rssKiB, 0) });
}
console.log(JSON.stringify({
  executable: initial.executable, started: initial.started, intervalMs: interval,
  limitations: [
    'Observed workload and window state are not controlled by this sampler.',
    'RSS sums can double-count shared pages; they are not physical footprint.',
    'Only process-tree descendants are included; XPC WebKit/GPU services may be excluded.',
    'ps CPU percent is an OS-reported moving average, not interval CPU or frame latency.',
    'Debug-build observations cannot establish a release-build speed or memory comparison.',
  ], observations,
}, null, 2));
