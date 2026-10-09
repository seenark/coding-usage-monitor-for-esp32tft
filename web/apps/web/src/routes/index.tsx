import { createFileRoute } from "@tanstack/react-router";

import { PageHeader } from "@/components/page";
import { ProviderCard } from "@/components/provider-card";
import { useStatus } from "@/hooks/use-status";
import { PROVIDERS } from "@/lib/api";

export const Route = createFileRoute("/")({ component: Overview });

function Overview() {
  const { status, reachable, loaded, now } = useStatus();
  return (
    <>
      <PageHeader title="Overview" description="Claude and Codex usage, kept separate. Updates every 10 seconds." />
      {!status && loaded ? (
        <p className="text-sm text-muted-foreground">No data yet. The server is not reachable.</p>
      ) : (
        <div className="grid gap-4 md:grid-cols-2">
          {PROVIDERS.map((id) => (
            <ProviderCard
              key={id}
              id={id}
              provider={status?.providers[id]}
              reachable={reachable}
              serverMock={status?.server.mock ?? false}
              now={now}
            />
          ))}
        </div>
      )}
    </>
  );
}
