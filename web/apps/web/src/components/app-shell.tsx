import { Link } from "@tanstack/react-router";
import { cn } from "@web/ui/lib/utils";
import {
  Gauge,
  LayoutDashboard,
  MonitorSmartphone,
  SlidersHorizontal,
  Wifi,
  WifiOff,
} from "lucide-react";
import type { ReactNode } from "react";

import { useStatus } from "@/hooks/use-status";
import type { DeviceState } from "@/lib/api";

const NAV = [
  { to: "/", label: "Overview", short: "Overview", icon: LayoutDashboard },
  { to: "/claude", label: "Claude", short: "Claude", icon: Gauge },
  { to: "/codex", label: "Codex", short: "Codex", icon: Gauge },
  { to: "/settings", label: "Server settings", short: "Server", icon: SlidersHorizontal },
  { to: "/device", label: "Device", short: "Device", icon: MonitorSmartphone },
  { to: "/device-setup", label: "Device setup", short: "Setup", icon: Wifi },
] as const;

const DEVICE_DOT: Record<DeviceState, { label: string; dot: string }> = {
  online: { label: "Device online", dot: "bg-good" },
  offline: { label: "Device offline", dot: "bg-bad" },
  unknown: { label: "Device status unknown", dot: "bg-off" },
};

export function DeviceDot({ state, className }: { state: DeviceState; className?: string }) {
  const { label, dot } = DEVICE_DOT[state];
  return (
    <span role="status" className={cn("inline-flex items-center gap-2 text-sm", className)}>
      <span aria-hidden className={cn("size-2.5 rounded-full", dot)} />
      {label}
    </span>
  );
}

export function AppShell({ children }: { children: ReactNode }) {
  const { status, reachable, loaded } = useStatus();
  const device = status?.device.state ?? "unknown";

  return (
    <div className="min-h-svh">
      {/* Sidebar, 768px and up */}
      <aside className="fixed inset-y-0 left-0 hidden w-60 flex-col border-r bg-sidebar p-4 md:flex">
        <Brand />
        <nav aria-label="Main" className="mt-6 flex flex-1 flex-col gap-1">
          {NAV.map(({ to, label, icon: Icon }) => (
            <Link
              key={to}
              to={to}
              activeOptions={{ exact: true }}
              className="flex h-10 items-center gap-3 rounded-lg px-3 text-sm text-muted-foreground hover:bg-sidebar-accent hover:text-foreground data-[status=active]:bg-sidebar-accent data-[status=active]:font-medium data-[status=active]:text-primary"
            >
              <Icon className="size-4" aria-hidden />
              {label}
            </Link>
          ))}
        </nav>
        <div className="border-t pt-4">
          <DeviceDot state={device} />
        </div>
      </aside>

      {/* Top bar, below 768px */}
      <header className="sticky top-0 z-20 flex h-12 items-center justify-between border-b bg-sidebar px-4 md:hidden">
        <Brand />
        <DeviceDot state={device} />
      </header>

      <main className="px-4 pt-6 pb-28 sm:px-6 md:ml-60 md:px-8 md:pb-10 lg:px-10">
        <div className="mx-auto max-w-4xl">
          {loaded && !reachable && (
            <p
              role="alert"
              className="mb-6 flex items-center gap-2 rounded-xl border border-bad/40 bg-bad/10 px-4 py-3 text-sm text-bad"
            >
              <WifiOff className="size-4 shrink-0" aria-hidden />
              Cannot reach the server. Showing the last numbers received.
            </p>
          )}
          {children}
        </div>
      </main>

      {/* Bottom tab bar, below 768px */}
      <nav
        aria-label="Main"
        className="fixed inset-x-0 bottom-0 z-20 grid grid-cols-6 border-t bg-sidebar pb-[env(safe-area-inset-bottom)] md:hidden"
      >
        {NAV.map(({ to, short, icon: Icon }) => (
          <Link
            key={to}
            to={to}
            activeOptions={{ exact: true }}
            className="flex min-h-14 flex-col items-center justify-center gap-1 text-[11px] text-muted-foreground data-[status=active]:text-primary"
          >
            <Icon className="size-5" aria-hidden />
            {short}
          </Link>
        ))}
      </nav>
    </div>
  );
}

function Brand() {
  return (
    <p className="flex items-center gap-2 text-sm font-semibold tracking-tight">
      <img src="/favicon.svg" alt="" className="size-6" />
      AI Quota Monitor
    </p>
  );
}
