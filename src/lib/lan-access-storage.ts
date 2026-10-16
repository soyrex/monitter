/** Retain only a server-validated reusable LAN bearer, scoped to this origin.
 * Pending input stays in the current tab. Browser storage may be unavailable. */
export function createLanAccessStore(
  session: () => Storage,
  persistent: () => Storage,
) {
  const storageKey = 'monitter.lan.access';
  let current: string | null = null;
  const read = (storage: () => Storage) => {
    try { return storage().getItem(storageKey) ?? ''; } catch { return ''; }
  };
  const write = (storage: () => Storage, value: string) => {
    try {
      if (value) storage().setItem(storageKey, value);
      else storage().removeItem(storageKey);
    } catch { /* A blocked storage origin can still connect for this page. */ }
  };
  const accessKey = () => current ?? (read(session) || read(persistent));
  return {
    accessKey,
    setAccessKey(key: string) {
      current = key.trim();
      write(session, current);
    },
    rememberAccessKey(key: string) {
      const validated = key.trim();
      if (!validated || validated !== accessKey()) return;
      write(persistent, validated);
    },
    clearAccessKey(rejected: string) {
      // A late response from an old request cannot revoke a newly validated
      // credential in this page. Transport errors do not call this method.
      if (accessKey() !== rejected) return false;
      current = '';
      write(session, '');
      // Another tab may already have validated a replacement bearer.
      if (read(persistent) === rejected) write(persistent, '');
      return true;
    },
  };
}
