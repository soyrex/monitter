import type { MonitterBridge } from '$lib/bridge';
import { isOwnerRelayDisconnectedError } from './owner-relay-error.js';
import type { TaskGitStatus } from '$lib/types';

type Subscriber = (status: TaskGitStatus) => void;
type Entry = { status?: TaskGitStatus; watching: boolean; probe?: Promise<TaskGitStatus | null>; subscribers: Set<Subscriber> };
const entries = new Map<string, Entry>();
const detectorSession = globalThis.crypto?.randomUUID?.() ?? `${Date.now()}-${Math.random()}`;

function entryFor(key: string) {
  let entry = entries.get(key);
  if (!entry) { entry = { watching: false, subscribers: new Set() }; entries.set(key, entry); }
  return entry;
}
function publish(entry: Entry) { if (entry.status) for (const subscriber of entry.subscribers) subscriber(entry.status); }

// Same host/folder tabs share one probe and marker waiter. Subscriptions are
// removed with the pane so a stale task cannot update the active one.
export function observeGit(bridge: Pick<MonitterBridge, 'getTaskGitStatus' | 'waitForTaskGitMarker'>, taskId: string, probeKey: string, subscriber: Subscriber): () => void {
  const entry = entryFor(probeKey);
  entry.subscribers.add(subscriber);
  if (entry.status) {
    subscriber(entry.status);
    if (!entry.status.repository) watchForMarker(bridge, taskId, probeKey, entry);
  } else void probe(bridge, taskId, probeKey, entry);
  return () => entry.subscribers.delete(subscriber);
}

async function probe(bridge: Pick<MonitterBridge, 'getTaskGitStatus' | 'waitForTaskGitMarker'>, taskId: string, key: string, entry: Entry) {
  if (entry.probe) return entry.probe;
  entry.probe = bridge.getTaskGitStatus(taskId, detectorSession).catch(reason => {
    // A transport failure gives no new evidence about the repository. Resolve
    // the shared probe quietly without publishing a missing repository.
    return isOwnerRelayDisconnectedError(reason) ? null : ({ repository: false } as TaskGitStatus);
  });
  const status = await entry.probe;
  entry.probe = undefined;
  if (status === null) return entry.status;
  entry.status = status;
  publish(entry);
  if (!status.repository) watchForMarker(bridge, taskId, key, entry);
  return status;
}

function watchForMarker(bridge: Pick<MonitterBridge, 'getTaskGitStatus' | 'waitForTaskGitMarker'>, taskId: string, key: string, entry: Entry) {
  if (entry.watching) return;
  entry.watching = true;
  void bridge.waitForTaskGitMarker(taskId, detectorSession).then(async result => {
    entry.watching = false;
    if (entries.get(key) !== entry) return;
    if (result === 'found') { entry.status = undefined; await probe(bridge, taskId, key, entry); }
    else if (result === 'timeout' && entry.subscribers.size) watchForMarker(bridge, taskId, key, entry);
  }).catch(() => { entry.watching = false; });
}

export async function refreshGit(bridge: Pick<MonitterBridge, 'getTaskGitStatus' | 'waitForTaskGitMarker'>, taskId: string, probeKey: string): Promise<TaskGitStatus> {
  const entry = entryFor(probeKey);
  let status: TaskGitStatus;
  try { status = await bridge.getTaskGitStatus(taskId, detectorSession); }
  catch (reason) {
    // GitPane can retain its cached selection/diff and decide whether a user
    // initiated refresh should show this actionable transport error.
    if (isOwnerRelayDisconnectedError(reason)) throw reason;
    status = { repository: false } as TaskGitStatus;
  }
  entry.status = status;
  publish(entry);
  if (!status.repository) watchForMarker(bridge, taskId, probeKey, entry);
  return status;
}
export function resetGitDetectorForTest() { entries.clear(); }
