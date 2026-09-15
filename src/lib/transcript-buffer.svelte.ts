/** Data-only display buffer for a detached transcript reader. */
export function createTranscriptBuffer<T>(key: () => string, live: () => T, fingerprint: () => string) {
  let frozen = $state<{ key: string; fingerprint: string; value: T; pending: boolean } | null>(null);

  function setFollowing(following: boolean) {
    if (following) { frozen = null; return; }
    const nextKey = key();
    if (frozen?.key === nextKey) return;
    // $state.snapshot removes Svelte's reactive proxies before structuredClone.
    // Callers supply plain transcript records, so snapshot preserves their shape.
    frozen = { key: nextKey, fingerprint: fingerprint(), value: structuredClone($state.snapshot(live())) as T, pending: false };
  }

  function reset() { frozen = null; }

  $effect(() => {
    // Do not serialize a live transcript while following it. The fingerprint is
    // only needed after the reader has asked us to hold a display snapshot.
    if (frozen?.key === key() && !frozen.pending && frozen.fingerprint !== fingerprint()) {
      frozen = { ...frozen, pending: true };
    }
  });

  return {
    setFollowing,
    reset,
    value: () => frozen?.key === key() ? frozen.value : live(),
    held: () => frozen?.key === key(),
    pendingUpdates: () => frozen?.key === key() && frozen.pending,
  };
}
