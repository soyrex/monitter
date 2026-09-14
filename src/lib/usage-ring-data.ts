import type { SubscriptionUsageSource, UsageOverview } from './types';

type UsageRingWindow = { label: string; usedPercent: number | null; unlimited?: boolean; resetsAt?: number | null };
type UsageRingData = {
  status: 'ready' | 'loading' | 'unavailable' | 'error' | 'stale';
  active?: UsageRingWindow | null;
  weekly?: UsageRingWindow | null;
  message?: string | null;
  updatedAt?: number | null;
};
type UsageRingProvider = 'codex' | 'claude' | 'minimax' | 'opencode-go';
type UsageRingMap = Record<UsageRingProvider, UsageRingData>;

const providerSource = (overview: UsageOverview | null, provider: UsageRingProvider) =>
  overview?.subscriptions.find(source => source.provider === provider || (provider === 'minimax' && source.source.includes('mmx')));

function window(source: SubscriptionUsageSource, weekly: boolean): UsageRingWindow | null {
  const matching = source.windows.find(item => {
    const key = `${item.key} ${item.label}`.toLowerCase();
    return weekly ? /week|seven.day|secondary/.test(key) : /5.hour|five.hour|current.interval|primary/.test(key);
  }) ?? (!weekly ? source.windows.find(item => !/week|seven.day|secondary/i.test(`${item.key} ${item.label}`)) : undefined);
  if (!matching) return null;
  return {
    label: weekly ? 'Week' : matching.label,
    usedPercent: matching.usedPercent,
    unlimited: /unlimited/i.test(matching.label),
    resetsAt: matching.resetsAt,
  };
}

function ring(source: SubscriptionUsageSource | undefined, loading: boolean, requestError: string, now: number): UsageRingData {
  if (!source) {
    return loading
      ? { status: 'loading', message: 'Checking allowance…' }
      : { status: requestError ? 'error' : 'unavailable', message: requestError || 'Quota unavailable.' };
  }
  const activeWindow = window(source, false);
  const weeklyWindow = window(source, true);
  // Some plans expose only a weekly allowance. Keep that useful value in the
  // ring instead of turning a valid source into an unavailable widget.
  const active = activeWindow ?? weeklyWindow;
  const weekly = activeWindow ? weeklyWindow : null;
  const status = source.state === 'available'
    ? (source.staleAfter !== null && source.staleAfter < now ? 'stale' : 'ready')
    : source.state === 'error' ? 'error' : 'unavailable';
  return {
    status,
    active,
    weekly,
    message: source.error ?? (source.provider === 'claude' && source.state === 'unsupported'
      ? 'Claude does not expose an on-demand quota read.'
      : null),
    updatedAt: source.fetchedAt,
  };
}

export function usageRingMap(
  overview: UsageOverview | null,
  loading = false,
  requestError = '',
  now = Date.now(),
): UsageRingMap {
  return {
    codex: ring(providerSource(overview, 'codex'), loading, requestError, now),
    claude: ring(providerSource(overview, 'claude'), loading, requestError, now),
    minimax: ring(providerSource(overview, 'minimax'), loading, requestError, now),
    'opencode-go': ring(providerSource(overview, 'opencode-go'), loading, requestError, now),
  };
}
