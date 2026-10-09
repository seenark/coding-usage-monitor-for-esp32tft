// Typed client for the quota-server Admin API. Same-origin, JSON only.

export type ProviderId = "claude" | "codex";
export const PROVIDERS: readonly ProviderId[] = ["claude", "codex"];

export type ProviderState =
  | "ok"
  | "stale"
  | "reauth_required"
  | "rate_limited"
  | "unavailable"
  | "error";

export type ErrorCode =
  | "disabled"
  | "unauthenticated"
  | "auth_rejected"
  | "access_denied"
  | "rate_limited"
  | "network"
  | "timeout"
  | "invalid_response"
  | "process_unavailable"
  | "unsupported"
  | "cache"
  | "upstream";

export interface UsageWindow {
  limit_id: string;
  key: string;
  label: string;
  duration_minutes: number | null;
  used_percent: number | null;
  remaining_percent: number | null;
  resets_at: number | null;
}

export interface BankedResets {
  available_count: number | null;
  details_status: "complete" | "partial" | "unavailable";
  known_expirations_at: number[];
  known_nonexpiring_count: number;
}

export interface Snapshot {
  schema_version: 1;
  provider: ProviderId;
  state: ProviderState;
  source?: "provider" | "mock";
  fetched_at: number | null;
  published_at: number;
  stale_after_seconds: number;
  windows: UsageWindow[];
  banked_resets: BankedResets | null;
  error_code: ErrorCode | null;
}

export interface ProviderStatus {
  enabled: boolean;
  snapshot: Snapshot | null;
  next_refresh_at: number | null;
  retry_after_seconds: number | null;
}

export type DeviceState = "online" | "offline" | "unknown";

export interface Status {
  server: { mock: boolean };
  device: { state: DeviceState };
  providers: Record<ProviderId, ProviderStatus>;
}

export interface ProviderSettings {
  enabled: boolean;
  refresh_interval_seconds: number;
}

export interface ServerSettings {
  providers: Record<ProviderId, ProviderSettings>;
  stale_after_seconds: number;
  max_backoff_seconds: number;
  request_timeout_seconds: number;
}

export const LAYOUTS = ["cards", "gauges", "terminal"] as const;
export type Layout = (typeof LAYOUTS)[number];

export interface DeviceConfig {
  layout: Layout;
  timezone: string;
  brightness: number;
}

export interface DeviceSetup {
  host: string;
  port: number;
  username: string;
  password: string;
  tls: boolean;
}

/** Thrown for any non-2xx answer. `fields` is set for 400 `invalid`. */
export class ApiError extends Error {
  constructor(
    readonly status: number,
    readonly code: string,
    readonly fields: Record<string, string> = {},
    readonly retryAfterSeconds: number | null = null,
  ) {
    super(code);
  }
}

async function request<T>(method: string, path: string, body?: unknown): Promise<T> {
  let response: Response;
  try {
    response = await fetch(path, {
      method,
      headers: body === undefined ? undefined : { "Content-Type": "application/json" },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
  } catch {
    throw new ApiError(0, "unreachable");
  }
  const text = await response.text();
  let json: unknown = null;
  try {
    json = text ? JSON.parse(text) : null;
  } catch {
    // Not JSON (for example an HTML error page from a proxy); handled below.
  }
  if (!response.ok) {
    const error = (json ?? {}) as {
      error?: string;
      fields?: Record<string, string>;
      retry_after_seconds?: number;
    };
    throw new ApiError(
      response.status,
      error.error ?? "http_error",
      error.fields,
      error.retry_after_seconds ?? null,
    );
  }
  // GET must carry a body; PUT/POST may answer with an empty or unspecified body.
  if (json === null && method === "GET") throw new ApiError(response.status, "invalid_response");
  return json as T;
}

export const api = {
  status: () => request<Status>("GET", "/api/status"),
  settings: () => request<ServerSettings>("GET", "/api/settings"),
  saveSettings: (value: ServerSettings) => request<unknown>("PUT", "/api/settings", value),
  refresh: (provider: ProviderId) =>
    request<unknown>("POST", `/api/refresh/${provider}`),
  device: () => request<DeviceConfig>("GET", "/api/device"),
  saveDevice: (value: DeviceConfig) => request<unknown>("PUT", "/api/device", value),
  deviceSetup: () => request<DeviceSetup>("GET", "/api/device-setup"),
};
