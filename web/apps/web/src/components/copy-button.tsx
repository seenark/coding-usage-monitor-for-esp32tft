import { Button } from "@web/ui/components/button";
import { cn } from "@web/ui/lib/utils";
import { Check, Copy } from "lucide-react";
import { useState } from "react";
import { toast } from "sonner";

import { copyText } from "@/lib/clipboard";

export function CopyButton({
  text,
  label,
  className,
}: {
  text: string;
  /** Accessible name, e.g. "Copy password". */
  label: string;
  className?: string;
}) {
  const [copied, setCopied] = useState(false);

  async function copy() {
    if (await copyText(text)) {
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1500);
    } else {
      toast.error("Copy is not available here. Select the text and copy it by hand.");
    }
  }

  return (
    <Button
      type="button"
      variant="outline"
      onClick={() => void copy()}
      aria-label={label}
      className={cn("h-11 min-w-11 gap-1.5 rounded-lg px-3 text-sm md:h-9 md:min-w-9", className)}
    >
      {copied ? <Check className="text-good" /> : <Copy />}
      <span className="hidden sm:inline">{copied ? "Copied" : "Copy"}</span>
    </Button>
  );
}
