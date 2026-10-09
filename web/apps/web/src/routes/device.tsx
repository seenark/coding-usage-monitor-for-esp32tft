import { createFileRoute } from "@tanstack/react-router";
import { Button } from "@web/ui/components/button";
import { Input } from "@web/ui/components/input";
import { cn } from "@web/ui/lib/utils";
import { useId, useState } from "react";
import { toast } from "sonner";

import { DeviceDot } from "@/components/app-shell";
import { LayoutPreview } from "@/components/layout-previews";
import { Loading, LoadError, PageHeader, Panel } from "@/components/page";
import { useLoad } from "@/hooks/use-load";
import { useStatus } from "@/hooks/use-status";
import { api, ApiError, LAYOUTS, type DeviceConfig, type Layout } from "@/lib/api";
import { sentence } from "@/lib/format";
import { validateDevice, type FieldErrors } from "@/lib/validate";

export const Route = createFileRoute("/device")({ component: DevicePage });

const LAYOUT_INFO: Record<Layout, { name: string; text: string }> = {
  cards: { name: "Cards", text: "One card per provider with both windows." },
  gauges: { name: "Gauges", text: "Two large rings with the 5-hour value in the middle." },
  terminal: { name: "Terminal", text: "Dense monospace lines, one per window." },
};

const TIMEZONE_PRESETS = [
  { label: "Bangkok", value: "ICT-7" },
  { label: "UTC", value: "UTC0" },
  { label: "Tokyo", value: "JST-9" },
  { label: "Central Europe", value: "CET-1CEST,M3.5.0,M10.5.0/3" },
  { label: "US Eastern", value: "EST5EDT,M3.2.0,M11.1.0" },
  { label: "US Pacific", value: "PST8PDT,M3.2.0,M11.1.0" },
];

function DevicePage() {
  const { status } = useStatus();
  const { data, error, reload } = useLoad(api.device);
  return (
    <>
      <PageHeader
        title="Device"
        description="How the touchscreen looks. The Device picks changes up within a few seconds when it is online."
        actions={<DeviceDot state={status?.device.state ?? "unknown"} />}
      />
      {error ? <LoadError error={error} onRetry={reload} /> : data ? <DeviceForm initial={data} /> : <Loading what="Device settings" />}
    </>
  );
}

function DeviceForm({ initial }: { initial: DeviceConfig }) {
  const [layout, setLayout] = useState<Layout>(initial.layout);
  const [timezone, setTimezone] = useState(initial.timezone);
  const [brightness, setBrightness] = useState(initial.brightness);
  const [errors, setErrors] = useState<FieldErrors>({});
  const [busy, setBusy] = useState(false);
  const timezoneId = useId();
  const brightnessId = useId();

  const clearTimezoneError = () =>
    setErrors((current) => Object.fromEntries(Object.entries(current).filter(([key]) => key !== "timezone")));

  async function save(event: React.FormEvent) {
    event.preventDefault();
    const value: DeviceConfig = { layout, timezone: timezone.trim(), brightness };
    const local = validateDevice(value);
    if (Object.keys(local).length > 0) {
      setErrors(local);
      toast.error("Not saved. Fix the highlighted fields.");
      return;
    }
    setBusy(true);
    try {
      await api.saveDevice(value);
      setTimezone(value.timezone);
      setErrors({});
      toast.success("Device settings saved and sent to the Device.");
    } catch (failure) {
      if (failure instanceof ApiError && failure.status === 400) {
        setErrors(failure.fields);
        toast.error("The server rejected these values. Nothing was changed.");
      } else {
        toast.error("Could not reach the server. Nothing was changed.");
      }
    } finally {
      setBusy(false);
    }
  }

  return (
    <form onSubmit={(event) => void save(event)} noValidate className="flex flex-col gap-4">
      <Panel>
        <fieldset aria-describedby={errors.layout ? "layout-error" : undefined}>
          <legend className="mb-3 text-base font-semibold">Display layout</legend>
          <div className="grid gap-3 sm:grid-cols-3">
            {LAYOUTS.map((option) => (
              <label
                key={option}
                className="group flex cursor-pointer flex-col gap-2 rounded-xl border bg-background p-3 transition-colors has-checked:border-primary has-checked:bg-primary/10 has-focus-visible:ring-2 has-focus-visible:ring-ring"
              >
                <input
                  type="radio"
                  name="layout"
                  value={option}
                  checked={layout === option}
                  onChange={() => setLayout(option)}
                  className="sr-only"
                />
                <LayoutPreview layout={option} />
                <span className="flex flex-col gap-0.5 min-h-11">
                  <span className="text-sm font-medium group-has-checked:text-primary">{LAYOUT_INFO[option].name}</span>
                  <span className="text-xs text-muted-foreground">{LAYOUT_INFO[option].text}</span>
                </span>
              </label>
            ))}
          </div>
          {errors.layout && (
            <p id="layout-error" className="mt-2 text-sm text-bad">
              {sentence(errors.layout)}
            </p>
          )}
        </fieldset>
      </Panel>

      <Panel className="flex flex-col gap-3">
        <label htmlFor={timezoneId} className="text-base font-semibold">
          Timezone
        </label>
        <Input
          id={timezoneId}
          value={timezone}
          onChange={(event) => {
            setTimezone(event.target.value);
            clearTimezoneError();
          }}
          autoCapitalize="off"
          autoCorrect="off"
          spellCheck={false}
          aria-invalid={!!errors.timezone || undefined}
          aria-describedby={`${timezoneId}-help`}
          className="h-11 max-w-md rounded-lg font-mono text-base md:h-10 md:text-sm"
        />
        <p id={`${timezoneId}-help`} className={cn("text-sm", errors.timezone ? "text-bad" : "text-muted-foreground")}>
          {errors.timezone ? sentence(errors.timezone) : "A POSIX TZ string, for example ICT-7. It sets the clock the Device shows."}
        </p>
        <div className="flex flex-wrap gap-2" role="group" aria-label="Timezone presets">
          {TIMEZONE_PRESETS.map((preset) => (
            <button
              key={preset.value}
              type="button"
              onClick={() => {
                setTimezone(preset.value);
                clearTimezoneError();
              }}
              aria-pressed={timezone === preset.value}
              className="inline-flex min-h-11 items-center rounded-lg border px-3 text-sm text-muted-foreground hover:bg-muted hover:text-foreground aria-pressed:border-primary aria-pressed:text-primary md:min-h-9"
            >
              {preset.label}
              <span className="ml-2 font-mono text-xs opacity-80">{preset.value}</span>
            </button>
          ))}
        </div>
      </Panel>

      <Panel className="flex flex-col gap-1">
        <div className="flex items-baseline justify-between">
          <label htmlFor={brightnessId} className="text-base font-semibold">
            Brightness
          </label>
          <output htmlFor={brightnessId} className="text-sm tabular-nums">
            {brightness}%
          </output>
        </div>
        <input
          id={brightnessId}
          type="range"
          min={10}
          max={100}
          step={1}
          value={brightness}
          onChange={(event) => setBrightness(Number(event.target.value))}
          aria-describedby={errors.brightness ? `${brightnessId}-error` : undefined}
        />
        {errors.brightness && (
          <p id={`${brightnessId}-error`} className="text-sm text-bad">
            {sentence(errors.brightness)}
          </p>
        )}
      </Panel>

      <div className="flex justify-end">
        <Button type="submit" disabled={busy} className="h-11 w-full rounded-lg px-6 text-sm sm:w-auto md:h-10">
          {busy ? "Saving…" : "Save Device settings"}
        </Button>
      </div>
    </form>
  );
}
