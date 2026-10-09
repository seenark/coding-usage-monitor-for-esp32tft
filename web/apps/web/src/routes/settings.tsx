import { createFileRoute } from "@tanstack/react-router";
import { Button } from "@web/ui/components/button";
import { Input } from "@web/ui/components/input";
import { Switch } from "@web/ui/components/switch";
import { cn } from "@web/ui/lib/utils";
import { useId, useState } from "react";
import type { ReactNode } from "react";
import { toast } from "sonner";

import { Loading, LoadError, PageHeader, Panel } from "@/components/page";
import { providerName } from "@/components/refresh-button";
import { useLoad } from "@/hooks/use-load";
import { api, ApiError, PROVIDERS, type ProviderId, type ServerSettings } from "@/lib/api";
import { describeInterval, joinInterval, parseWhole, sentence, splitInterval } from "@/lib/format";
import { validateSettings, type FieldErrors } from "@/lib/validate";

export const Route = createFileRoute("/settings")({ component: ServerSettingsPage });

interface Form {
  providers: Record<ProviderId, { enabled: boolean; minutes: string; seconds: string }>;
  stale_after_seconds: string;
  max_backoff_seconds: string;
  request_timeout_seconds: string;
}

function toForm(settings: ServerSettings): Form {
  const provider = (id: ProviderId) => {
    const { minutes, seconds } = splitInterval(settings.providers[id].refresh_interval_seconds);
    return { enabled: settings.providers[id].enabled, minutes: String(minutes), seconds: String(seconds) };
  };
  return {
    providers: { claude: provider("claude"), codex: provider("codex") },
    stale_after_seconds: String(settings.stale_after_seconds),
    max_backoff_seconds: String(settings.max_backoff_seconds),
    request_timeout_seconds: String(settings.request_timeout_seconds),
  };
}

function toSettings(form: Form): ServerSettings {
  const provider = (id: ProviderId) => ({
    enabled: form.providers[id].enabled,
    refresh_interval_seconds: joinInterval(form.providers[id].minutes, form.providers[id].seconds),
  });
  return {
    providers: { claude: provider("claude"), codex: provider("codex") },
    stale_after_seconds: parseWhole(form.stale_after_seconds),
    max_backoff_seconds: parseWhole(form.max_backoff_seconds),
    request_timeout_seconds: parseWhole(form.request_timeout_seconds),
  };
}

function ServerSettingsPage() {
  const { data, error, reload } = useLoad(api.settings);
  return (
    <>
      <PageHeader
        title="Server settings"
        description="How often the server asks each provider for usage, and how it treats old or failing data. Changes apply without a restart."
      />
      {error ? <LoadError error={error} onRetry={reload} /> : data ? <SettingsForm initial={data} /> : <Loading what="settings" />}
    </>
  );
}

function SettingsForm({ initial }: { initial: ServerSettings }) {
  const [form, setForm] = useState(() => toForm(initial));
  const [errors, setErrors] = useState<FieldErrors>({});
  const [busy, setBusy] = useState(false);

  const clear = (...paths: string[]) =>
    setErrors((current) => Object.fromEntries(Object.entries(current).filter(([key]) => !paths.includes(key))));
  const setProvider = (id: ProviderId, patch: Partial<Form["providers"][ProviderId]>) => {
    setForm((current) => ({ ...current, providers: { ...current.providers, [id]: { ...current.providers[id], ...patch } } }));
    clear(`providers.${id}.refresh_interval_seconds`, "max_backoff_seconds");
  };
  const setNumber = (key: "stale_after_seconds" | "max_backoff_seconds" | "request_timeout_seconds", value: string) => {
    setForm((current) => ({ ...current, [key]: value }));
    clear(key);
  };

  async function save(event: React.FormEvent) {
    event.preventDefault();
    const settings = toSettings(form);
    const local = validateSettings(settings);
    if (Object.keys(local).length > 0) {
      setErrors(local);
      toast.error("Not saved. Fix the highlighted fields.");
      return;
    }
    setBusy(true);
    try {
      await api.saveSettings(settings);
      setErrors({});
      toast.success("Settings saved. The server uses them from its next cycle.");
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
      {PROVIDERS.map((id) => (
        <ProviderSection
          key={id}
          id={id}
          value={form.providers[id]}
          error={errors[`providers.${id}.refresh_interval_seconds`]}
          onChange={(patch) => setProvider(id, patch)}
        />
      ))}

      <Panel className="flex flex-col gap-5">
        <h2 className="text-base font-semibold">Limits</h2>
        <NumberField
          label="Mark data stale after"
          unit="seconds"
          value={form.stale_after_seconds}
          error={errors.stale_after_seconds}
          help="When the last good reading is older than this, the Device and this app call it Stale. 60 to 86400."
          onChange={(value) => setNumber("stale_after_seconds", value)}
        />
        <NumberField
          label="Longest wait after failures"
          unit="seconds"
          value={form.max_backoff_seconds}
          error={errors.max_backoff_seconds}
          help="Upper bound for the growing wait after errors. It must be at least the longest refresh interval. 60 to 86400."
          onChange={(value) => setNumber("max_backoff_seconds", value)}
        />
        <NumberField
          label="Request timeout"
          unit="seconds"
          value={form.request_timeout_seconds}
          error={errors.request_timeout_seconds}
          help="How long the server waits for one provider answer. 1 to 60."
          onChange={(value) => setNumber("request_timeout_seconds", value)}
        />
      </Panel>

      <div className="flex justify-end">
        <Button type="submit" disabled={busy} className="h-11 w-full rounded-lg px-6 text-sm sm:w-auto md:h-10">
          {busy ? "Saving…" : "Save settings"}
        </Button>
      </div>
    </form>
  );
}

function ProviderSection({
  id,
  value,
  error,
  onChange,
}: {
  id: ProviderId;
  value: Form["providers"][ProviderId];
  error: string | undefined;
  onChange: (patch: Partial<Form["providers"][ProviderId]>) => void;
}) {
  const name = providerName(id);
  const errorId = useId();
  const total = joinInterval(value.minutes, value.seconds);
  return (
    <Panel aria-labelledby={`settings-${id}`} className="flex flex-col gap-5">
      <div className="flex items-center justify-between gap-4">
        <h2 id={`settings-${id}`} className="text-base font-semibold">
          {name}
        </h2>
        <label className="flex min-h-11 items-center gap-3 text-sm">
          {value.enabled ? "On" : "Off"}
          <Switch
            aria-label={`${name} enabled`}
            checked={value.enabled}
            onCheckedChange={(enabled) => onChange({ enabled })}
            className="mr-1 origin-right scale-125"
          />
        </label>
      </div>

      <fieldset aria-describedby={error ? errorId : undefined}>
        <legend className="mb-2 text-sm font-medium">Refresh interval</legend>
        <div className="flex items-center gap-3">
          <IntervalInput label="Minutes" unit="min" value={value.minutes} invalid={!!error} onChange={(minutes) => onChange({ minutes })} />
          <IntervalInput label="Seconds" unit="sec" value={value.seconds} invalid={!!error} onChange={(seconds) => onChange({ seconds })} />
        </div>
        {error ? (
          <p id={errorId} className="mt-2 text-sm text-bad">
            {sentence(error)}
          </p>
        ) : (
          <p className="mt-2 text-sm text-muted-foreground">
            {Number.isFinite(total) ? `Every ${describeInterval(total)}. ` : ""}
            Between 1 min and 60 min.
          </p>
        )}
        {id === "claude" && (
          <p className="mt-2 text-sm text-muted-foreground">
            Claude limits how often usage can be read. Keep this at 10 minutes or longer; shorter intervals can trigger rate limiting. If Claude asks the server to wait, the server always waits that long.
          </p>
        )}
        {id === "codex" && (
          <p className="mt-2 text-sm text-muted-foreground">
            If Codex asks the server to wait, the server always waits that long, whatever is set here.
          </p>
        )}
      </fieldset>
    </Panel>
  );
}

function IntervalInput({
  label,
  unit,
  value,
  invalid,
  onChange,
}: {
  label: string;
  unit: string;
  value: string;
  invalid: boolean;
  onChange: (value: string) => void;
}) {
  return (
    <label className="flex flex-1 items-center gap-2 sm:max-w-40">
      <span className="sr-only">{label}</span>
      <Input
        inputMode="numeric"
        autoComplete="off"
        value={value}
        aria-invalid={invalid || undefined}
        onChange={(event) => onChange(event.target.value)}
        className="h-11 rounded-lg text-right text-base tabular-nums md:h-10 md:text-sm"
      />
      <span className="text-sm text-muted-foreground">{unit}</span>
    </label>
  );
}

function NumberField({
  label,
  unit,
  value,
  error,
  help,
  onChange,
}: {
  label: string;
  unit: string;
  value: string;
  error: string | undefined;
  help: ReactNode;
  onChange: (value: string) => void;
}) {
  const id = useId();
  const parsed = parseWhole(value);
  return (
    <div>
      <label htmlFor={id} className="mb-2 block text-sm font-medium">
        {label}
      </label>
      <div className="flex items-center gap-2">
        <Input
          id={id}
          inputMode="numeric"
          autoComplete="off"
          value={value}
          aria-invalid={!!error || undefined}
          aria-describedby={`${id}-help`}
          onChange={(event) => onChange(event.target.value)}
          className="h-11 max-w-40 rounded-lg text-right text-base tabular-nums md:h-10 md:text-sm"
        />
        <span className="text-sm text-muted-foreground">
          {unit}
          {Number.isFinite(parsed) && parsed >= 60 ? ` (${describeInterval(parsed)})` : ""}
        </span>
      </div>
      <p id={`${id}-help`} className={cn("mt-2 text-sm", error ? "text-bad" : "text-muted-foreground")}>
        {error ? sentence(error) : help}
      </p>
    </div>
  );
}
