import { cn } from "@web/ui/lib/utils";

import { CopyButton } from "@/components/copy-button";
import { Meter, TONE_TEXT } from "@/components/meter";
import { PageHeader, Panel } from "@/components/page";
import { bankedCountLabel } from "@/components/provider-card";
import { providerName, RefreshButton } from "@/components/refresh-button";
import { StatusPill } from "@/components/status-pill";
import { useStatus } from "@/hooks/use-status";
import type { BankedResets, ProviderId } from "@/lib/api";
import {
  health,
  localTime,
  nextRefreshLine,
  percentLeft,
  percentUsed,
  resetLine,
  shortDuration,
  updatedLine,
} from "@/lib/format";
import { derivePill, isLastKnown, loginCommand, needsLogin, problemText } from "@/lib/status";

export function ProviderPage({ id }: { id: ProviderId }) {
  const { status, reachable, loaded, now } = useStatus();
  const name = providerName(id);
  const provider = status?.providers[id];
  const snapshot = provider?.snapshot ?? null;
  const serverMock = status?.server.mock ?? false;
  const pill = derivePill(provider, { reachable, serverMock, now });
  const problem = provider ? problemText(provider, name) : null;
  const windows = pill.label === "Off" ? [] : (snapshot?.windows ?? []);

  if (!provider) {
    return (
      <>
        <PageHeader title={name} />
        <p className="text-sm text-muted-foreground">
          {loaded ? "No data. The server is not reachable." : "Loading…"}
        </p>
      </>
    );
  }

  return (
    <>
      <PageHeader
        title={name}
        description={
          <span className="flex flex-wrap items-center gap-x-3 gap-y-1">
            <StatusPill pill={pill} />
            <span>{updatedLine(snapshot?.fetched_at ?? null, now)}</span>
          </span>
        }
        actions={<RefreshButton provider={id} enabled={provider.enabled} />}
      />

      <div className="flex flex-col gap-4">
        {problem && (
          <Panel role="status" className={cn(pill.tone === "bad" ? "border-bad/40" : "border-warn/40")}>
            <p className={cn("font-medium", pill.tone === "bad" ? "text-bad" : "text-warn")}>{problem}</p>
          </Panel>
        )}

        {needsLogin(snapshot) && <LoginHelp id={id} name={name} />}

        {windows.length > 0 && isLastKnown(pill) && (
          <p className="text-sm text-muted-foreground">Showing last-known usage, not a fresh reading.</p>
        )}

        {snapshot?.source === "mock" && (
          <p className="text-sm text-warn">Demo data, not from your account.</p>
        )}

        {snapshot && windows.length === 0 && pill.label !== "Off" && (
          <Panel>
            <p className="text-muted-foreground">No usage windows yet.</p>
          </Panel>
        )}

        <div className="grid gap-4 md:grid-cols-2">
          {windows.map((window) => (
            <Panel key={`${window.limit_id}/${window.key}`} className="flex flex-col gap-3">
              <div className="flex items-baseline justify-between gap-3">
                <h2 className="text-base font-semibold">{window.label}</h2>
                <p className={cn("text-3xl font-semibold leading-none tabular-nums", TONE_TEXT[health(window)])}>
                  {percentLeft(window)}
                  <span className="ml-1.5 text-sm font-normal text-muted-foreground">left</span>
                </p>
              </div>
              <Meter window={window} />
              <p className="text-sm text-muted-foreground">
                {percentUsed(window)} used · {resetLine(window, now)}
              </p>
            </Panel>
          ))}
          {snapshot?.banked_resets && pill.label !== "Off" && <BankedPanel bank={snapshot.banked_resets} now={now} />}
        </div>

        <Panel className="flex flex-col gap-1">
          <h2 className="text-base font-semibold">Refresh schedule</h2>
          <p className="text-muted-foreground">
            {provider.enabled ? nextRefreshLine(provider.next_refresh_at, now) : "Turned off in Server settings."}
          </p>
          {provider.retry_after_seconds ? (
            <p className="text-caution">
              {name} asked the server to wait {shortDuration(provider.retry_after_seconds)}. The server never asks earlier.
            </p>
          ) : null}
          <p className="text-muted-foreground">Change the interval in Server settings.</p>
        </Panel>
      </div>
    </>
  );
}

function LoginHelp({ id, name }: { id: ProviderId; name: string }) {
  const command = loginCommand(id);
  return (
    <Panel className="flex flex-col gap-3">
      <h2 className="text-base font-semibold">Sign in to {name} again</h2>
      <p className="text-muted-foreground">
        Run this on the machine that runs quota-server, then follow the prompts. Usage returns on the next refresh.
      </p>
      <div className="flex items-center gap-2">
        <code className="min-w-0 flex-1 overflow-x-auto rounded-lg bg-background px-3 py-2.5 font-mono text-sm whitespace-nowrap">
          {command}
        </code>
        <CopyButton text={command} label={`Copy ${name} sign-in command`} />
      </div>
    </Panel>
  );
}

function BankedPanel({ bank, now }: { bank: BankedResets; now: number }) {
  const expirations = [...bank.known_expirations_at].sort((a, b) => a - b);
  const partial = bank.details_status === "partial";
  return (
    <Panel className="flex flex-col gap-3">
      <div className="flex items-baseline justify-between gap-3">
        <h2 className="text-base font-semibold">Banked resets</h2>
        <p className="text-3xl font-semibold leading-none tabular-nums text-codex">
          {bank.available_count ?? "?"}
        </p>
      </div>
      <p className="sr-only">{bankedCountLabel(bank)}</p>
      <ul className="flex flex-col gap-1 text-sm text-muted-foreground">
        {expirations.map((expiry, index) => {
          const left = expiry - now;
          const when = left <= 0 ? "Expired, waiting for refresh" : `in ${shortDuration(left)} (${localTime(expiry, now)})`;
          const label = partial && index === 0 ? "Known expiry" : "Expires";
          return <li key={expiry}>{left <= 0 ? when : `${label} ${when}`}</li>;
        })}
        {bank.known_nonexpiring_count > 0 && <li>{bank.known_nonexpiring_count} never expire</li>}
      </ul>
      {partial && (
        <p className="text-sm text-muted-foreground">
          Some expiry dates were not reported. The nearest known date may not be the earliest of all.
        </p>
      )}
      {bank.details_status === "unavailable" && (
        <p className="text-sm text-muted-foreground">Expiry dates were not reported.</p>
      )}
      <p className="text-sm text-muted-foreground">Use them on the Codex website.</p>
    </Panel>
  );
}
