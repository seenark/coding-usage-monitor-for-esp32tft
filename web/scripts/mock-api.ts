// Mock of the quota-server Admin API for developing the Admin app without the real server.
//
//   bun scripts/mock-api.ts            (PORT=8081 by default, bound to 127.0.0.1)
//
// Switch what the app sees while it runs, no restart:
//   curl 'http://127.0.0.1:8081/__mock/set?scenario=attention&device=offline&fail=save'
//   scenario: live | attention | limited | demo | disabled   (default: MOCK_SCENARIO or live)
//   device:   online | offline | unknown                     (default: scenario's own)
//   fail:     save | none     "save" makes PUT /api/settings and /api/device answer 400 field errors
// GET /__mock/state shows the current choice.

import type {
  BankedResets,
  DeviceConfig,
  DeviceSetup,
  DeviceState,
  ProviderId,
  ProviderStatus,
  ServerSettings,
  Snapshot,
  Status,
  UsageWindow,
} from "../apps/web/src/lib/api";

const SCENARIOS = ["live", "attention", "limited", "demo", "disabled"] as const;
type Scenario = (typeof SCENARIOS)[number];

const state: { scenario: Scenario; device: DeviceState | null; fail: boolean } = {
  scenario: SCENARIOS.includes(process.env.MOCK_SCENARIO as Scenario)
    ? (process.env.MOCK_SCENARIO as Scenario)
    : "live",
  device: null,
  fail: false,
};

const now = () => Math.floor(Date.now() / 1000);

let settings: ServerSettings = {
  providers: {
    claude: { enabled: true, refresh_interval_seconds: 600 },
    codex: { enabled: true, refresh_interval_seconds: 300 },
  },
  stale_after_seconds: 900,
  max_backoff_seconds: 1800,
  request_timeout_seconds: 12,
};
settings.providers.claude.enabled = state.scenario !== "disabled";
let device: DeviceConfig = { layout: "cards", timezone: "ICT-7", brightness: 80 };
// Not a real credential: the real server reads DEVICE_PASSWORD_FILE.
const setup: DeviceSetup = {
  host: "192.168.1.112",
  port: 1883,
  username: "esp32",
  password: "482915037264",
  tls: false,
};

const win = (key: string, label: string, minutes: number, used: number | null, resetsIn: number | null): UsageWindow => ({
  limit_id: "default",
  key,
  label,
  duration_minutes: minutes,
  used_percent: used,
  remaining_percent: used === null ? null : Math.min(100, Math.max(0, 100 - used)),
  resets_at: resetsIn === null ? null : now() + resetsIn,
});

function snapshot(provider: ProviderId, patch: Partial<Snapshot>): Snapshot {
  return {
    schema_version: 1,
    provider,
    state: "ok",
    source: "provider",
    fetched_at: now() - 75,
    published_at: now() - 75,
    stale_after_seconds: settings.stale_after_seconds,
    windows: [],
    banked_resets: null,
    error_code: null,
    ...patch,
  };
}

const claudeWindows = (fiveUsed: number, weekUsed: number) => [
  win("five_hour", "5-hour", 300, fiveUsed, 59 * 60),
  win("seven_day", "Weekly", 10080, weekUsed, 2 * 86400 + 3 * 3600),
];
const codexWindows = (fiveUsed: number, weekUsed: number) => [
  win("primary", "5-hour", 300, fiveUsed, 2 * 3600 + 14 * 60),
  win("secondary", "Weekly", 10080, weekUsed, 4 * 86400 + 5 * 3600),
];

const bankedComplete: BankedResets = {
  available_count: 2,
  details_status: "complete",
  known_expirations_at: [now() + 9 * 86400, now() + 21 * 86400],
  known_nonexpiring_count: 0,
};
const bankedPartial: BankedResets = {
  available_count: 3,
  details_status: "partial",
  known_expirations_at: [now() + 5 * 86400 + 7200],
  known_nonexpiring_count: 0,
};

function providers(): Record<ProviderId, ProviderStatus> {
  const enabled = (id: ProviderId) => settings.providers[id].enabled;
  const next = (id: ProviderId) => (enabled(id) ? now() + (id === "claude" ? 412 : 187) : null);
  const status = (id: ProviderId, snap: Snapshot, retry: number | null = null): ProviderStatus => ({
    enabled: enabled(id),
    snapshot: enabled(id)
      ? snap
      : snapshot(id, { state: "unavailable", error_code: "disabled", fetched_at: snap.fetched_at, windows: snap.windows }),
    next_refresh_at: next(id),
    retry_after_seconds: retry,
  });

  switch (state.scenario) {
    case "attention":
      return {
        claude: status(
          "claude",
          snapshot("claude", {
            state: "reauth_required",
            error_code: "auth_rejected",
            fetched_at: now() - 3 * 3600,
            windows: claudeWindows(28, 59),
          }),
        ),
        codex: status("codex", snapshot("codex", { windows: codexWindows(36, 12), banked_resets: bankedPartial })),
      };
    case "limited":
      return {
        claude: status(
          "claude",
          snapshot("claude", { state: "rate_limited", error_code: "rate_limited", fetched_at: now() - 640, windows: claudeWindows(44, 71) }),
          780,
        ),
        codex: status("codex", snapshot("codex", { windows: codexWindows(82, 91), banked_resets: bankedComplete })),
      };
    case "demo":
      return {
        claude: status("claude", snapshot("claude", { source: "mock", windows: claudeWindows(28, 59) })),
        codex: status("codex", snapshot("codex", { source: "mock", windows: codexWindows(36, 12), banked_resets: bankedComplete })),
      };
    case "disabled":
      return {
        claude: status("claude", snapshot("claude", { windows: claudeWindows(28, 59) })),
        codex: status("codex", snapshot("codex", { windows: codexWindows(36, 12), banked_resets: bankedComplete })),
      };
    default:
      return {
        claude: status("claude", snapshot("claude", { windows: claudeWindows(28, 59) })),
        codex: status("codex", snapshot("codex", { windows: codexWindows(36, 12), banked_resets: bankedComplete })),
      };
  }
}

const defaultDevice: Record<Scenario, DeviceState> = {
  live: "online",
  attention: "offline",
  limited: "unknown",
  demo: "online",
  disabled: "online",
};

const json = (body: unknown, status = 200) =>
  Response.json(body, { status, headers: { "Cache-Control": "no-store" } });
const invalid = (fields: Record<string, string>) => json({ error: "invalid", fields }, 400);

/** Same ranges as the real server (Admin API contract). */
function settingsFields(value: ServerSettings): Record<string, string> {
  const fields: Record<string, string> = {};
  const between = (n: unknown, lo: number, hi: number) => Number.isInteger(n) && (n as number) >= lo && (n as number) <= hi;
  for (const id of ["claude", "codex"] as const) {
    if (!between(value.providers?.[id]?.refresh_interval_seconds, 60, 3600)) {
      fields[`providers.${id}.refresh_interval_seconds`] = "must be between 60 and 3600";
    }
  }
  if (!between(value.stale_after_seconds, 60, 86400)) fields.stale_after_seconds = "must be between 60 and 86400";
  if (!between(value.request_timeout_seconds, 1, 60)) fields.request_timeout_seconds = "must be between 1 and 60";
  if (!between(value.max_backoff_seconds, 60, 86400)) fields.max_backoff_seconds = "must be between 60 and 86400";
  else if (Object.keys(fields).length === 0) {
    const longest = Math.max(value.providers.claude.refresh_interval_seconds, value.providers.codex.refresh_interval_seconds);
    if (value.max_backoff_seconds < longest) fields.max_backoff_seconds = "must be at least the longest refresh interval";
  }
  return fields;
}

function deviceFields(value: DeviceConfig): Record<string, string> {
  const fields: Record<string, string> = {};
  if (!["cards", "gauges", "terminal"].includes(value.layout)) fields.layout = "must be cards, gauges or terminal";
  if (typeof value.timezone !== "string" || !/^[A-Za-z0-9+\-:,./<>]{1,64}$/.test(value.timezone)) {
    fields.timezone = "must be 1 to 64 characters from A-Z a-z 0-9 + - : , . / < >";
  }
  if (!Number.isInteger(value.brightness) || value.brightness < 10 || value.brightness > 100) {
    fields.brightness = "must be between 10 and 100";
  }
  return fields;
}

// fail=save: a rejection the client cannot predict, so the Admin app's server-error display can be seen.
const failFields = (kind: "settings" | "device"): Record<string, string> =>
  kind === "settings"
    ? {
        "providers.claude.refresh_interval_seconds": "rejected by the mock server (fail=save)",
        max_backoff_seconds: "rejected by the mock server (fail=save)",
      }
    : { timezone: "rejected by the mock server (fail=save)" };

async function handle(request: Request): Promise<Response> {
  const url = new URL(request.url);
  const { pathname } = url;
  const { method } = request;

  if (pathname === "/__mock/set") {
    const scenario = url.searchParams.get("scenario");
    if (scenario) {
      if (!SCENARIOS.includes(scenario as Scenario)) return json({ error: `scenario must be one of ${SCENARIOS.join(", ")}` }, 400);
      state.scenario = scenario as Scenario;
      settings.providers.claude.enabled = scenario !== "disabled";
    }
    const dev = url.searchParams.get("device");
    if (dev) state.device = dev === "auto" ? null : (dev as DeviceState);
    const fail = url.searchParams.get("fail");
    if (fail) state.fail = fail === "save";
    return json(state);
  }
  if (pathname === "/__mock/state") return json(state);

  if (!pathname.startsWith("/api/")) return json({ error: "not_found" }, 404);

  if (method === "PUT" || method === "POST") {
    // Real server: 16 KiB limit and application/json required for bodies.
    const length = Number(request.headers.get("content-length") ?? 0);
    if (length > 16 * 1024) return json({ error: "payload_too_large" }, 413);
    if (length > 0 && !request.headers.get("content-type")?.includes("application/json")) {
      return json({ error: "unsupported_media_type" }, 415);
    }
  }

  if (method === "GET" && pathname === "/api/status") {
    const body: Status = {
      server: { mock: state.scenario === "demo" },
      device: { state: state.device ?? defaultDevice[state.scenario] },
      providers: providers(),
    };
    return json(body);
  }

  if (pathname === "/api/settings") {
    if (method === "GET") return json(settings);
    if (method === "PUT") {
      const value = (await request.json().catch(() => null)) as ServerSettings | null;
      if (!value) return json({ error: "bad_json" }, 400);
      const fields = state.fail ? failFields("settings") : settingsFields(value);
      if (Object.keys(fields).length > 0) return invalid(fields);
      settings = value;
      return json(settings);
    }
  }

  if (pathname === "/api/device") {
    if (method === "GET") return json(device);
    if (method === "PUT") {
      const value = (await request.json().catch(() => null)) as DeviceConfig | null;
      if (!value) return json({ error: "bad_json" }, 400);
      const fields = state.fail ? failFields("device") : deviceFields(value);
      if (Object.keys(fields).length > 0) return invalid(fields);
      device = value;
      return json(device);
    }
  }

  if (method === "GET" && pathname === "/api/device-setup") return json(setup);

  const refresh = pathname.match(/^\/api\/refresh\/([^/]+)$/);
  if (method === "POST" && refresh) {
    const id = refresh[1] as ProviderId;
    if (id !== "claude" && id !== "codex") return json({ error: "not_found" }, 404);
    const provider = providers()[id];
    if (!provider.enabled) return json({ error: "disabled" }, 409);
    if (provider.retry_after_seconds) return json({ error: "rate_limited", retry_after_seconds: provider.retry_after_seconds }, 429);
    return json({ scheduled: true }, 202);
  }

  return json({ error: "not_found" }, 404);
}

const server = Bun.serve({
  hostname: "127.0.0.1",
  port: Number(process.env.PORT ?? 8081),
  fetch: handle,
});
console.log(`mock API on http://${server.hostname}:${server.port}  scenario=${state.scenario}`);
