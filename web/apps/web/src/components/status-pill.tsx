import { Badge } from "@web/ui/components/badge";
import { cn } from "@web/ui/lib/utils";

import type { Pill, PillTone } from "@/lib/status";

const TONES: Record<PillTone, string> = {
  good: "bg-good/15 text-good",
  warn: "bg-warn/15 text-warn",
  caution: "bg-caution/15 text-caution",
  bad: "bg-bad/15 text-bad",
  off: "bg-off/15 text-off",
  demo: "bg-primary/15 text-primary",
};

export function StatusPill({ pill, className }: { pill: Pill; className?: string }) {
  return (
    <Badge className={cn("h-6 gap-1.5 rounded-full px-2.5 text-xs", TONES[pill.tone], className)}>
      <span aria-hidden className="size-1.5 rounded-full bg-current" />
      {pill.label}
    </Badge>
  );
}
