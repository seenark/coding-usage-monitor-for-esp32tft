import type { UsageWindow } from "./api";

export type Tone = "good" | "warn" | "bad" | "off";

/** Minute precision, same shape as the Device: "2d 3h", "3h 12m", "59m", "<1m". */
export function shortDuration(seconds: number): string {
  if (seconds < 60) return "<1m";
  const days = Math.floor(seconds / 86400);
  const hours = Math.floor(seconds / 3600) % 24;
  const minutes = Math.floor(seconds / 60) % 60;
  if (days) return `${days}d ${hours}h`;
  if (hours) return `${hours}h ${minutes}m`;
  return `${minutes}m`;
}

/** "Fri 08:00" within the coming week, otherwise "09 Oct 08:00". `timeZone` defaults to the browser's. */
export function localTime(utc: number, now: number, timeZone?: string): string {
  const parts = Object.fromEntries(
    new Intl.DateTimeFormat("en-GB", {
      timeZone,
      weekday: "short",
      day: "2-digit",
      month: "short",
      hour: "2-digit",
      minute: "2-digit",
      hourCycle: "h23",
    })
      .formatToParts(new Date(utc * 1000))
      .map((part) => [part.type, part.value]),
  );
  const clock = `${parts.hour}:${parts.minute}`;
  const thisWeek = utc >= now && utc - now < 6 * 86400;
  return thisWeek ? `${parts.weekday} ${clock}` : `${parts.day} ${parts.month} ${clock}`;
}

/** "Resets in 59m (Fri 08:00)". Never claims the quota refilled once the countdown ends. */
export function resetLine(window: UsageWindow, now: number, withTime = true, timeZone?: string) {
  if (window.resets_at === null) return "Reset time not reported";
  const left = window.resets_at - now;
  if (left <= 0) return "Waiting for refresh";
  const line = `Resets in ${shortDuration(left)}`;
  return withTime ? `${line} (${localTime(window.resets_at, now, timeZone)})` : line;
}

export function updatedLine(fetchedAt: number | null, now: number): string {
  if (fetchedAt === null) return "Never updated";
  const age = now - fetchedAt;
  if (age < 60) return "Updated just now";
  return `Updated ${shortDuration(age)} ago`;
}

/** Clock-time of the next scheduled refresh, or a plain statement when none is planned. */
export function nextRefreshLine(nextRefreshAt: number | null, now: number): string {
  if (nextRefreshAt === null) return "No refresh scheduled";
  const left = nextRefreshAt - now;
  if (left <= 0) return "Refreshing now";
  return `Next refresh in ${shortDuration(left)}`;
}

export function percentLeft(window: UsageWindow): string {
  return window.remaining_percent === null ? "--" : `${Math.round(window.remaining_percent)}%`;
}

export function percentUsed(window: UsageWindow): string {
  return window.used_percent === null ? "--" : `${Math.round(window.used_percent)}%`;
}

/** Green at 50% or more left, yellow at 20% or more, red below. Unknown is neutral, never zero. */
export function health(window: UsageWindow): Tone {
  const left = window.remaining_percent;
  if (left === null) return "off";
  if (left >= 50) return "good";
  return left >= 20 ? "warn" : "bad";
}

export function splitInterval(totalSeconds: number): { minutes: number; seconds: number } {
  return { minutes: Math.floor(totalSeconds / 60), seconds: totalSeconds % 60 };
}

/** Whole non-negative number from form text; anything else is NaN so validation rejects it. */
export function parseWhole(text: string): number {
  return /^\d+$/.test(text.trim()) ? Number(text.trim()) : NaN;
}

/** Empty minutes or seconds count as zero. */
export function joinInterval(minutes: string, seconds: string): number {
  return (minutes.trim() ? parseWhole(minutes) : 0) * 60 + (seconds.trim() ? parseWhole(seconds) : 0);
}

export function describeInterval(totalSeconds: number): string {
  const { minutes, seconds } = splitInterval(totalSeconds);
  if (!minutes) return `${seconds} s`;
  return seconds ? `${minutes} min ${seconds} s` : `${minutes} min`;
}

/** The short (usually 5-hour) and weekly windows, else the first two. Same pick as the Device. */
export function overviewWindows(windows: UsageWindow[]): UsageWindow[] {
  const short = windows.find((w) => w.duration_minutes !== null && w.duration_minutes < 10080);
  const weekly = windows.find((w) => w.duration_minutes === 10080);
  const picked = [short, weekly].filter((w): w is UsageWindow => w !== undefined);
  for (const w of windows) if (picked.length < 2 && !picked.includes(w)) picked.push(w);
  return picked;
}

/** Server field messages are lower case ("must be between…"); show them as sentences. */
export function sentence(text: string): string {
  return text.charAt(0).toUpperCase() + text.slice(1) + (text.endsWith(".") ? "" : ".");
}
