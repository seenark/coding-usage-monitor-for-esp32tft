import { LAYOUTS, PROVIDERS, type DeviceConfig, type ServerSettings } from "./api";

/** Field path -> message. Paths match the server's 400 `fields` keys. */
export type FieldErrors = Record<string, string>;

const between = (value: number, min: number, max: number) =>
  Number.isInteger(value) && value >= min && value <= max;

const range = (min: number, max: number) => `must be between ${min} and ${max}`;

/** Mirrors the server rules in the Admin API contract. The server stays the authority. */
export function validateSettings(settings: ServerSettings): FieldErrors {
  const errors: FieldErrors = {};
  for (const id of PROVIDERS) {
    if (!between(settings.providers[id].refresh_interval_seconds, 60, 3600)) {
      errors[`providers.${id}.refresh_interval_seconds`] = range(60, 3600);
    }
  }
  if (!between(settings.stale_after_seconds, 60, 86400)) {
    errors.stale_after_seconds = range(60, 86400);
  }
  if (!between(settings.max_backoff_seconds, 60, 86400)) {
    errors.max_backoff_seconds = range(60, 86400);
  } else if (!PROVIDERS.some((id) => errors[`providers.${id}.refresh_interval_seconds`])) {
    // Only comparable once every interval is itself valid.
    const longest = Math.max(
      ...PROVIDERS.map((id) => settings.providers[id].refresh_interval_seconds),
    );
    if (settings.max_backoff_seconds < longest) {
      errors.max_backoff_seconds = "must be at least the longest refresh interval";
    }
  }
  if (!between(settings.request_timeout_seconds, 1, 60)) {
    errors.request_timeout_seconds = range(1, 60);
  }
  return errors;
}

export const TIMEZONE_PATTERN = /^[A-Za-z0-9+\-:,./<>]{1,64}$/;

export function validateDevice(device: DeviceConfig): FieldErrors {
  const errors: FieldErrors = {};
  if (!LAYOUTS.includes(device.layout)) errors.layout = "must be cards, gauges or terminal";
  if (!TIMEZONE_PATTERN.test(device.timezone)) {
    errors.timezone = "use 1 to 64 letters, digits or + - : , . / < >";
  }
  if (!between(device.brightness, 10, 100)) errors.brightness = range(10, 100);
  return errors;
}
