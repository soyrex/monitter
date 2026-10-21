// Generated OpenUI is untrusted. Only the host's validated public-media
// mapping can supply an NSH image path; model paths and URLs stay blocked.
const CACHED_IMAGE = /^media:[a-f0-9]{64}\.(?:png|jpg|webp)$/;

export function safeImageSource(value, approved = {}) {
  return typeof value === 'string' && CACHED_IMAGE.test(value) && Object.hasOwn(approved, value)
    ? approved[value] : null;
}
