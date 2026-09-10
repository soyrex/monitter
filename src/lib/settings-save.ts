import { getBridge } from './bridge';
import type { Settings, Snapshot } from './types';

let pending: Promise<unknown> = Promise.resolve();

// Merge each edit into current saved preferences, including edits from another pane.
export function saveSettingsPatch(patch: Partial<Settings>): Promise<Snapshot> {
  const save = pending.then(async () => {
    const bridge = getBridge();
    const current = await bridge.getSnapshot();
    return bridge.saveSettings({ ...current.settings, ...patch });
  });
  pending = save.catch(() => {});
  return save;
}
