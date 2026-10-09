import { Button } from "@web/ui/components/button";
import { RefreshCw } from "lucide-react";
import { useState } from "react";
import { toast } from "sonner";

import { useStatus } from "@/hooks/use-status";
import { api, ApiError, type ProviderId } from "@/lib/api";
import { refreshErrorText } from "@/lib/status";

const NAMES: Record<ProviderId, string> = { claude: "Claude", codex: "Codex" };
export const providerName = (id: ProviderId) => NAMES[id];

export function RefreshButton({ provider, enabled }: { provider: ProviderId; enabled: boolean }) {
  const { refetch } = useStatus();
  const [busy, setBusy] = useState(false);

  async function refresh() {
    setBusy(true);
    try {
      await api.refresh(provider);
      toast.success(`${NAMES[provider]} refresh requested. New numbers appear in a few seconds.`);
      window.setTimeout(() => void refetch(), 3000);
    } catch (error) {
      const e = error instanceof ApiError ? error : new ApiError(0, "unreachable");
      toast.error(refreshErrorText(e.status, e.code, e.retryAfterSeconds));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Button
      type="button"
      variant="outline"
      disabled={busy || !enabled}
      onClick={() => void refresh()}
      className="h-11 gap-2 rounded-lg px-4 text-sm md:h-9"
    >
      <RefreshCw className={busy ? "animate-spin" : undefined} />
      Refresh now
    </Button>
  );
}
