import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const root = process.cwd();
const [messagePane, thinkingStatus, virtualList, terminalRuntime] = await Promise.all([
  readFile(`${root}/src/lib/components/MessagePane.svelte`, 'utf8'),
  readFile(`${root}/src/lib/components/ThinkingStatus.svelte`, 'utf8'),
  readFile(`${root}/src/lib/components/TranscriptVirtualList.svelte`, 'utf8'),
  readFile(`${root}/src/lib/terminal-runtime.ts`, 'utf8'),
]);

assert.doesNotMatch(messagePane, /setInterval\s*\(/, 'MessagePane must not retain a layout-repair poll');
assert.doesNotMatch(thinkingStatus, /setInterval\s*\(/, 'thinking timers must share the activity clock');
assert.match(messagePane, /active = true/);
assert.match(messagePane, /visibilitychange/);
assert.match(virtualList, /generics="T"/);
assert.match(virtualList, /estimateHeight/);
assert.match(virtualList, /stickyKey/);
assert.match(virtualList, /stickyBeforeWindow/);
assert.match(virtualList, /stickyGap/);
assert.doesNotMatch(virtualList, /start\s*=\s*Math\.min\(start,\s*stickyIndex\)/,
  'a pinned request must not expand the virtual window across every later row');
assert.match(virtualList, /role="feed"/);
assert.match(virtualList, /ResizeObserver/);
assert.doesNotMatch(terminalRuntime, /setInterval\s*\(/, 'terminal runtime must not keep a global interval');
assert.match(terminalRuntime, /runtimeCanPoll/);
assert.match(terminalRuntime, /documentVisibilityChanged/);
assert.match(terminalRuntime, /}, 5_000\)/, 'active title refresh should follow the backend five-second settle window');

console.log('transcript performance regression: bounded generic rows, visibility-aware observers, and active-only terminal scheduling are present.');
