import { readable } from 'svelte/store';

const DAY_MS = 86_400_000;
const weekdayFormatter = new Intl.DateTimeFormat('en-US', { weekday: 'long' });
const dateFormatter = new Intl.DateTimeFormat('en-GB', { day: '2-digit', month: '2-digit', year: 'numeric' });
const timeFormatter = new Intl.DateTimeFormat('en-US', { hour: 'numeric', minute: '2-digit', hour12: true });

/** One clock for every visible timestamp and date divider. */
export const chatDateClock = readable(Date.now(), set => {
  let timer: ReturnType<typeof setTimeout> | undefined;
  const refresh = () => {
    clearTimeout(timer);
    const now = new Date();
    set(now.getTime());
    const nextMidnight = new Date(now);
    nextMidnight.setHours(24, 0, 0, 0);
    timer = setTimeout(refresh, Math.max(1_000, nextMidnight.getTime() - now.getTime()));
  };
  refresh();
  if (typeof document !== 'undefined') document.addEventListener('visibilitychange', refresh);
  return () => {
    clearTimeout(timer);
    if (typeof document !== 'undefined') document.removeEventListener('visibilitychange', refresh);
  };
});

/** Calendar-day identity in the viewer's timezone, unaffected by DST length. */
export function chatDay(timestamp: number): number | null {
  const date = new Date(timestamp);
  if (!Number.isFinite(date.getTime())) return null;
  return Math.floor(Date.UTC(date.getFullYear(), date.getMonth(), date.getDate()) / DAY_MS);
}

export function chatDateLabel(timestamp: number, now = Date.now()): string {
  const day = chatDay(timestamp);
  const today = chatDay(now);
  if (day === null || today === null) return '';
  const distance = today - day;
  if (distance === 0) return 'Today';
  if (distance === 1) return 'Yesterday';
  if (distance === 2) return '2 days ago';
  if (distance >= 3 && distance <= 6) {
    const weekday = weekdayFormatter.format(new Date(timestamp));
    return `Last ${weekday}`;
  }
  const date = new Date(timestamp);
  return dateFormatter.format(date);
}

export function chatTimeLabel(timestamp: number): string {
  const date = new Date(timestamp);
  if (!Number.isFinite(date.getTime())) return '';
  return timeFormatter.format(date).replace(/\s+/g, ' ').toLowerCase();
}

export function chatDateTimeLabel(timestamp: number, now = Date.now()): string {
  const time = chatTimeLabel(timestamp);
  const date = chatDateLabel(timestamp, now);
  return time && date ? `${time} - ${date}` : '';
}

export function chatDateTimeAttribute(timestamp: number): string {
  const date = new Date(timestamp);
  if (!Number.isFinite(date.getTime())) return '';
  const year = date.getFullYear();
  const month = String(date.getMonth() + 1).padStart(2, '0');
  const day = String(date.getDate()).padStart(2, '0');
  return `${year}-${month}-${day}`;
}
