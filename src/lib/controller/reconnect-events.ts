/** Coalesce browser and native lifecycle hints without reloading or forgetting trust. */
export function watchReconnectEvents(reconnect: () => void): () => void {
  let timer: ReturnType<typeof setTimeout> | undefined;
  const schedule = () => {
    if (document.visibilityState === 'hidden') return;
    if (timer !== undefined) clearTimeout(timer);
    timer = setTimeout(() => { timer = undefined; if (document.visibilityState !== 'hidden') reconnect(); }, 250);
  };
  const events = ['online', 'pageshow', 'monitter:resume'] as const;
  for (const event of events) window.addEventListener(event, schedule);
  document.addEventListener('visibilitychange', schedule);
  return () => {
    if (timer !== undefined) clearTimeout(timer);
    for (const event of events) window.removeEventListener(event, schedule);
    document.removeEventListener('visibilitychange', schedule);
  };
}
