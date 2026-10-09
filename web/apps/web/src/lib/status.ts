import type { ProviderId, ProviderStatus, Snapshot } from "./api";
import { shortDuration } from "./format";

export type PillTone = "good" | "warn" | "caution" | "bad" | "off" | "demo";
export interface Pill {
  label: string;
  tone: PillTone;
}

/**
 * What the Admin app calls a provider's state. `reachable` is false when the
 * last /api/status poll failed: the data shown is then last-known, not current.
 */
export function derivePill(
  provider: ProviderStatus | undefined,
  options: { reachable: boolean; serverMock: boolean; now: number },
): Pill {
  if (!options.reachable) return { label: "Offline", tone: "bad" };
  if (!provider) return { label: "Waiting", tone: "off" };
  const snapshot = provider.snapshot;
  if (!provider.enabled || snapshot?.error_code === "disabled") return { label: "Off", tone: "off" };
  if (!snapshot) return { label: "Waiting", tone: "off" };
  switch (snapshot.state) {
    case "ok": {
      const age = snapshot.fetched_at === null ? null : options.now - snapshot.fetched_at;
      if (age !== null && age > snapshot.stale_after_seconds) return { label: "Stale", tone: "warn" };
      return snapshot.source === "mock" || options.serverMock
        ? { label: "Demo", tone: "demo" }
        : { label: "Live", tone: "good" };
    }
    case "stale":
      return { label: "Stale", tone: "warn" };
    case "reauth_required":
      return { label: "Sign in", tone: "bad" };
    case "rate_limited":
      return { label: "Rate limited", tone: "caution" };
    case "unavailable":
      return { label: "Unavailable", tone: "off" };
    case "error":
      return { label: "Error", tone: "bad" };
  }
}

/** Windows shown under any state but Live or Demo are last-known readings, not fresh ones. */
export const isLastKnown = (pill: Pill) => pill.tone !== "good" && pill.tone !== "demo";

export function needsLogin(snapshot: Snapshot | null): boolean {
  return snapshot?.state === "reauth_required";
}

export const loginCommand = (provider: ProviderId) => `./scripts/auth.sh ${provider} login`;

/** One plain sentence about what is wrong and what to do, or null when nothing is. */
export function problemText(provider: ProviderStatus, providerName: string): string | null {
  const code = provider.snapshot?.error_code;
  if (!code) return null;
  switch (code) {
    case "disabled":
      return "Turned off in Server settings.";
    case "unauthenticated":
    case "auth_rejected":
      return `${providerName} needs a new sign-in on the server.`;
    case "access_denied":
      return "This account is not allowed to read usage.";
    case "rate_limited": {
      const wait = provider.retry_after_seconds;
      return wait
        ? `${providerName} is limiting requests. The server waits ${shortDuration(wait)} before it asks again.`
        : `${providerName} is limiting requests. The server waits before it asks again.`;
    }
    case "network":
    case "timeout":
    case "upstream":
      return `The server cannot reach ${providerName} right now. It keeps retrying.`;
    case "process_unavailable":
      return "The Codex helper is not running on the server.";
    case "invalid_response":
      return `${providerName} sent data the server did not understand.`;
    case "unsupported":
      return "This account type is not supported.";
    case "cache":
      return "The server could not read its saved usage.";
  }
}

/** Wording for a failed "Refresh now" request. */
export function refreshErrorText(status: number, code: string, retryAfter: number | null): string {
  if (status === 429) {
    return retryAfter
      ? `Rate limited. Try again in ${shortDuration(retryAfter)}.`
      : "Rate limited. Try again later.";
  }
  if (status === 409 || code === "disabled") return "This provider is turned off in Server settings.";
  if (status === 0) return "Cannot reach the server.";
  return "Refresh failed.";
}
