import { describe, expect, test } from "vite-plus/test";

import type { ProviderStatus, ServerSettings, Snapshot, UsageWindow } from "./api";
import {
  health,
  joinInterval,
  localTime,
  overviewWindows,
  nextRefreshLine,
  resetLine,
  shortDuration,
  splitInterval,
  updatedLine,
} from "./format";
import { derivePill } from "./status";
import { validateDevice, validateSettings } from "./validate";

const NOW = 1_791_500_000; // Thu 2026-10-08 22:53:20 UTC

const windowAt = (resets_at: number | null, remaining: number | null): UsageWindow => ({
  limit_id: "x",
  key: "x",
  label: "5-hour",
  duration_minutes: 300,
  used_percent: remaining === null ? null : 100 - remaining,
  remaining_percent: remaining,
  resets_at,
});

describe("durations", () => {
  test("shortDuration matches the Device", () => {
    expect(shortDuration(30)).toBe("<1m");
    expect(shortDuration(59 * 60 + 40)).toBe("59m");
    expect(shortDuration(3 * 3600 + 12 * 60)).toBe("3h 12m");
    expect(shortDuration(2 * 86400 + 3 * 3600 + 59 * 60)).toBe("2d 3h");
  });

  test("resetLine never claims a refill", () => {
    expect(resetLine(windowAt(null, 50), NOW)).toBe("Reset time not reported");
    expect(resetLine(windowAt(NOW - 5, 50), NOW)).toBe("Waiting for refresh");
    expect(resetLine(windowAt(NOW + 3540, 50), NOW, true, "UTC")).toBe(
      "Resets in 59m (Thu 23:52)",
    );
    expect(resetLine(windowAt(NOW + 3540, 50), NOW, false)).toBe("Resets in 59m");
  });

  test("localTime uses a date beyond a week", () => {
    expect(localTime(NOW + 8 * 86400, NOW, "UTC")).toBe("16 Oct 22:53");
  });

  test("updated and next refresh", () => {
    expect(updatedLine(null, NOW)).toBe("Never updated");
    expect(updatedLine(NOW - 20, NOW)).toBe("Updated just now");
    expect(updatedLine(NOW - 125, NOW)).toBe("Updated 2m ago");
    expect(nextRefreshLine(null, NOW)).toBe("No refresh scheduled");
    expect(nextRefreshLine(NOW - 1, NOW)).toBe("Refreshing now");
    expect(nextRefreshLine(NOW + 600, NOW)).toBe("Next refresh in 10m");
  });
});

describe("overviewWindows", () => {
  const w = (key: string, minutes: number | null): UsageWindow => ({ ...windowAt(null, 50), key, duration_minutes: minutes });
  test("short and weekly first, then fill from the rest", () => {
    const keys = (list: UsageWindow[]) => overviewWindows(list).map((x) => x.key);
    expect(keys([w("week", 10080), w("five", 300), w("opus", 10080)])).toEqual(["five", "week"]);
    expect(keys([w("a", null), w("b", null), w("c", null)])).toEqual(["a", "b"]);
    expect(keys([w("week", 10080), w("a", null)])).toEqual(["week", "a"]);
    expect(keys([])).toEqual([]);
  });
});

describe("health and intervals", () => {
  test("thresholds are 50 and 20; unknown is neutral", () => {
    expect(health(windowAt(null, 50))).toBe("good");
    expect(health(windowAt(null, 49.9))).toBe("warn");
    expect(health(windowAt(null, 20))).toBe("warn");
    expect(health(windowAt(null, 19))).toBe("bad");
    expect(health(windowAt(null, null))).toBe("off");
  });

  test("minutes+seconds round trip, bad input is NaN", () => {
    expect(splitInterval(605)).toEqual({ minutes: 10, seconds: 5 });
    expect(joinInterval("10", "5")).toBe(605);
    expect(joinInterval("", "90")).toBe(90);
    expect(joinInterval("1.5", "0")).toBeNaN();
    expect(joinInterval("-1", "0")).toBeNaN();
  });
});

const snapshot = (patch: Partial<Snapshot>): Snapshot => ({
  schema_version: 1,
  provider: "claude",
  state: "ok",
  fetched_at: NOW - 60,
  published_at: NOW - 60,
  stale_after_seconds: 900,
  windows: [],
  banked_resets: null,
  error_code: null,
  ...patch,
});
const provider = (patch: Partial<ProviderStatus>, snap: Partial<Snapshot> = {}): ProviderStatus => ({
  enabled: true,
  snapshot: snapshot(snap),
  next_refresh_at: NOW + 60,
  retry_after_seconds: null,
  ...patch,
});
const live = { reachable: true, serverMock: false, now: NOW };

describe("derivePill", () => {
  test("states", () => {
    expect(derivePill(provider({}), live).label).toBe("Live");
    expect(derivePill(provider({}, { fetched_at: NOW - 901 }), live).label).toBe("Stale");
    expect(derivePill(provider({}, { state: "stale" }), live).label).toBe("Stale");
    expect(derivePill(provider({}, { state: "reauth_required" }), live).label).toBe("Sign in");
    expect(derivePill(provider({}, { state: "rate_limited" }), live).label).toBe("Rate limited");
    expect(derivePill(provider({}, { state: "unavailable" }), live).label).toBe("Unavailable");
    expect(derivePill(provider({}, { state: "error" }), live).label).toBe("Error");
  });

  test("off, offline, demo, waiting", () => {
    expect(derivePill(provider({ enabled: false }), live).label).toBe("Off");
    expect(
      derivePill(provider({}, { state: "unavailable", error_code: "disabled" }), live).label,
    ).toBe("Off");
    expect(derivePill(provider({}), { ...live, reachable: false }).label).toBe("Offline");
    expect(derivePill(provider({}, { source: "mock" }), live).label).toBe("Demo");
    expect(derivePill(provider({}), { ...live, serverMock: true }).label).toBe("Demo");
    expect(derivePill(provider({ snapshot: null }), live).label).toBe("Waiting");
    expect(derivePill(undefined, live).label).toBe("Waiting");
  });

  test("a problem beats the demo label", () => {
    expect(derivePill(provider({}, { state: "reauth_required", source: "mock" }), live).label).toBe(
      "Sign in",
    );
  });
});

const goodSettings = (): ServerSettings => ({
  providers: {
    claude: { enabled: true, refresh_interval_seconds: 600 },
    codex: { enabled: true, refresh_interval_seconds: 300 },
  },
  stale_after_seconds: 900,
  max_backoff_seconds: 1800,
  request_timeout_seconds: 12,
});

describe("validation mirrors the server", () => {
  test("defaults pass", () => {
    expect(validateSettings(goodSettings())).toEqual({});
  });

  test("ranges and max_backoff against intervals", () => {
    const settings = goodSettings();
    settings.providers.claude.refresh_interval_seconds = 59;
    settings.providers.codex.refresh_interval_seconds = 3601;
    settings.stale_after_seconds = 86401;
    settings.request_timeout_seconds = 0;
    expect(validateSettings(settings)).toEqual({
      "providers.claude.refresh_interval_seconds": "must be between 60 and 3600",
      "providers.codex.refresh_interval_seconds": "must be between 60 and 3600",
      stale_after_seconds: "must be between 60 and 86400",
      request_timeout_seconds: "must be between 1 and 60",
    });
    const tight = goodSettings();
    tight.max_backoff_seconds = 599;
    expect(validateSettings(tight).max_backoff_seconds).toBe(
      "must be at least the longest refresh interval",
    );
    tight.max_backoff_seconds = 600;
    expect(validateSettings(tight)).toEqual({});
  });

  test("NaN and fractions are rejected", () => {
    const settings = goodSettings();
    settings.providers.claude.refresh_interval_seconds = NaN;
    settings.stale_after_seconds = 100.5;
    expect(Object.keys(validateSettings(settings)).sort()).toEqual([
      "providers.claude.refresh_interval_seconds",
      "stale_after_seconds",
    ]);
  });

  test("device", () => {
    expect(validateDevice({ layout: "cards", timezone: "ICT-7", brightness: 80 })).toEqual({});
    expect(
      validateDevice({
        layout: "cards",
        timezone: "CET-1CEST,M3.5.0,M10.5.0/3",
        brightness: 10,
      }),
    ).toEqual({});
    const bad = validateDevice({ layout: "wide" as never, timezone: "UTC 0", brightness: 9 });
    expect(Object.keys(bad).sort()).toEqual(["brightness", "layout", "timezone"]);
    expect(validateDevice({ layout: "gauges", timezone: "", brightness: 100 }).timezone).toBeTruthy();
    expect(
      validateDevice({ layout: "gauges", timezone: "A".repeat(65), brightness: 100 }).timezone,
    ).toBeTruthy();
  });
});
