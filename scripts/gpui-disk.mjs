// Read-only cache accounting. Cleanup is deliberately a separate, scoped step.
import { execFileSync } from 'node:child_process';
import { appendFileSync, existsSync, mkdirSync, statfsSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
export const defaultTarget = join(root, '.cargo-target', 'gpui');
const GiB = 1024 ** 3;
function bytes(path) {
  if (!existsSync(path)) return 0;
  return Number(execFileSync('/usr/bin/du', ['-sk', path], { encoding: 'utf8' }).trim().split(/\s+/)[0]) * 1024;
}
export function recordDisk(phase, target = process.env.CARGO_TARGET_DIR || defaultTarget) {
  const fs = statfsSync(root);
  const record = {
    at: new Date().toISOString(), phase,
    availableBytes: fs.bavail * fs.bsize,
    target: resolve(root, target),
    targetBytes: bytes(resolve(root, target)),
    legacyTargetBytes: bytes(join(root, 'desktop-gpui', 'target')),
    toolchainBytes: bytes(join(root, '.toolchain')),
    artifactBytes: bytes(join(root, 'artifacts', 'gpui')),
  };
  const logDir = join(root, 'artifacts', 'gpui');
  mkdirSync(logDir, { recursive: true });
  appendFileSync(join(logDir, 'disk-usage.jsonl'), `${JSON.stringify(record)}\n`);
  console.log(`GPUI disk (${phase}): ${(record.availableBytes / GiB).toFixed(1)} GiB free; ${(record.targetBytes / GiB).toFixed(1)} GiB build cache.`);
  return record;
}
export function checkBuildSpace(target, { warmDebug = false } = {}) {
  const record = recordDisk('before-build', target);
  // A warm debug rebuild reuses dependency outputs; retain 6 GiB for linking
  // and immutable packaging. Cold/release builds keep the larger reserve.
  const minimumGiB = warmDebug ? 6 : 10;
  if (record.availableBytes < minimumGiB * GiB) {
    throw new Error(`Less than ${minimumGiB} GiB free: inspect artifacts/gpui/disk-usage.jsonl and reclaim only idle, regenerable GPUI fork caches before building. Preserve production/shared caches, running builds, app bundles, data and backups.`);
  }
  return record;
}
if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) recordDisk('manual');
