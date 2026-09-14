/** Coalesce localStorage writes while retaining an explicit crash/quit flush. */
export function createWorkspaceSaveScheduler(save: () => boolean, delay = 300) {
  let timer: ReturnType<typeof setTimeout> | undefined;
  const flush = () => {
    if (timer) { clearTimeout(timer); timer = undefined; }
    return save();
  };
  return {
    schedule() {
      if (!timer) timer = setTimeout(() => { timer = undefined; save(); }, delay);
    },
    flush,
    cancel() { if (timer) clearTimeout(timer); timer = undefined; },
    get pending() { return !!timer; },
  };
}
