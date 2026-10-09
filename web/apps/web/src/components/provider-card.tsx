import { Link } from "@tanstack/react-router";
import { cn } from "@web/ui/lib/utils";
import { ChevronRight, History } from "lucide-react";

import { Meter, TONE_TEXT } from "@/components/meter";
import { Panel } from "@/components/page";
import { providerName, RefreshButton } from "@/components/refresh-button";
import { StatusPill } from "@/components/status-pill";
import type { BankedResets, ProviderId, ProviderStatus } from "@/lib/api";
import { health, overviewWindows, percentLeft, resetLine, updatedLine } from "@/lib/format";
import { derivePill, isLastKnown, problemText } from "@/lib/status";

export function bankedCountLabel(bank: BankedResets): string {
  if (bank.available_count === null) return "Banked resets: count not reported";
  return `${bank.available_count} banked ${bank.available_count === 1 ? "reset" : "resets"}`;
}

export function ProviderCard({
  id,
  provider,
  reachable,
  serverMock,
  now,
}: {
  id: ProviderId;
  provider: ProviderStatus | undefined;
  reachable: boolean;
  serverMock: boolean;
  now: number;
}) {
  const name = providerName(id);
  const snapshot = provider?.snapshot ?? null;
  const pill = derivePill(provider, { reachable, serverMock, now });
  const problem = provider ? problemText(provider, name) : null;
  const off = pill.label === "Off";
  const windows = snapshot && !off ? overviewWindows(snapshot.windows) : [];

  return (
    <Panel aria-labelledby={`card-${id}`} className="flex flex-col gap-4">
      <div className="flex items-center justify-between gap-3">
        <h2 id={`card-${id}`} className="flex items-center gap-2 text-base font-semibold">
          <span aria-hidden className={cn("size-2.5 rounded-full", id === "claude" ? "bg-claude" : "bg-codex")} />
          {name}
        </h2>
        <StatusPill pill={pill} />
      </div>

      {problem && (
        <p className={cn("text-sm", pill.tone === "bad" ? "text-bad" : "text-warn")}>{problem}</p>
      )}

      {windows.length > 0 && isLastKnown(pill) && (
        <p className="text-sm text-muted-foreground">Showing last-known usage.</p>
      )}

      {off ? null : windows.length === 0 ? (
        <p className="py-6 text-sm text-muted-foreground">
          {snapshot ? "No usage reading yet." : "Waiting for the server to publish usage."}
        </p>
      ) : (
        <div className="flex flex-col gap-5">
          {windows.map((window) => (
            <div key={`${window.limit_id}/${window.key}`} className="flex flex-col gap-2">
              <div className="flex items-baseline gap-2">
                <span className={cn("text-4xl font-semibold leading-none tabular-nums", TONE_TEXT[health(window)])}>
                  {percentLeft(window)}
                </span>
                <span className="text-sm text-muted-foreground">{window.label} left</span>
              </div>
              <Meter window={window} />
              <p className="text-sm text-muted-foreground">{resetLine(window, now)}</p>
            </div>
          ))}
        </div>
      )}

      {snapshot?.banked_resets && (
        <p className="flex items-center gap-2 text-sm">
          <History className="size-4 text-muted-foreground" aria-hidden />
          {bankedCountLabel(snapshot.banked_resets)}
        </p>
      )}

      <div className="mt-auto flex flex-wrap items-center justify-between gap-3 border-t pt-4">
        <p className="text-sm text-muted-foreground">{updatedLine(snapshot?.fetched_at ?? null, now)}</p>
        <div className="flex items-center gap-2">
          <RefreshButton provider={id} enabled={provider?.enabled ?? false} />
          <Link
            to={id === "claude" ? "/claude" : "/codex"}
            aria-label={`${name} details`}
            className="inline-flex h-11 min-w-11 items-center justify-center rounded-lg border text-muted-foreground hover:bg-muted hover:text-foreground md:h-9 md:min-w-9"
          >
            <ChevronRight className="size-4" />
          </Link>
        </div>
      </div>
    </Panel>
  );
}
