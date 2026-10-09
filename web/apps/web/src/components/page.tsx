import { Button } from "@web/ui/components/button";
import { cn } from "@web/ui/lib/utils";
import { TriangleAlert } from "lucide-react";
import type { ReactNode } from "react";

import type { ApiError } from "@/lib/api";

export function PageHeader({
  title,
  description,
  actions,
}: {
  title: string;
  description?: ReactNode;
  actions?: ReactNode;
}) {
  return (
    <header className="mb-6 flex flex-wrap items-start justify-between gap-x-6 gap-y-3">
      <div className="min-w-0">
        <h1 className="text-2xl font-semibold tracking-tight">{title}</h1>
        {description && <p className="mt-1 max-w-prose text-sm text-muted-foreground">{description}</p>}
      </div>
      {actions}
    </header>
  );
}

/** A surface for one idea. Plain div on purpose: shadcn Card is square-cornered and tiny-type. */
export function Panel({
  className,
  children,
  ...props
}: React.ComponentProps<"section">) {
  return (
    <section
      className={cn("rounded-xl border bg-card p-4 text-sm sm:p-5", className)}
      {...props}
    >
      {children}
    </section>
  );
}

export function LoadError({ error, onRetry }: { error: ApiError; onRetry: () => void }) {
  // Status 0 is a failed fetch; 502 and above is a proxy or tunnel with nothing behind it.
  const unreachable = error.status === 0 || error.status >= 502;
  return (
    <Panel role="alert" className="flex flex-col items-start gap-3">
      <p className="flex items-center gap-2 font-medium text-bad">
        <TriangleAlert className="size-4" />
        {unreachable ? "Cannot reach the server." : "The server could not answer."}
      </p>
      <p className="text-muted-foreground">
        {unreachable
          ? "Check that quota-server is running and this page is served by it."
          : `The server answered with an error (${error.status} ${error.code}).`}
      </p>
      <Button type="button" variant="outline" onClick={onRetry} className="h-11 rounded-lg px-4 text-sm md:h-9">
        Try again
      </Button>
    </Panel>
  );
}

export function Loading({ what }: { what: string }) {
  return (
    <p role="status" className="text-sm text-muted-foreground">
      Loading {what}…
    </p>
  );
}
