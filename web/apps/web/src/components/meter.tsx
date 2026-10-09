import { cn } from "@web/ui/lib/utils";

import type { UsageWindow } from "@/lib/api";
import { health, type Tone } from "@/lib/format";

export const TONE_TEXT: Record<Tone, string> = {
  good: "text-good",
  warn: "text-warn",
  bad: "text-bad",
  off: "text-off",
};
const TONE_FILL: Record<Tone, string> = {
  good: "bg-good",
  warn: "bg-warn",
  bad: "bg-bad",
  off: "bg-off",
};

/** Remaining-usage bar. Unknown usage draws an empty track, never a full or zero reading. */
export function Meter({ window, className }: { window: UsageWindow; className?: string }) {
  const left = window.remaining_percent;
  return (
    <div
      role="meter"
      aria-label={`${window.label} remaining`}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={left ?? undefined}
      aria-valuetext={left === null ? "Unknown" : `${Math.round(left)}% left`}
      className={cn("h-2 w-full overflow-hidden rounded-full bg-track", className)}
    >
      <div
        className={cn("h-full rounded-full transition-[width]", TONE_FILL[health(window)])}
        style={{ width: `${left ?? 0}%` }}
      />
    </div>
  );
}
