import { HeadContent, Outlet, createRootRoute } from "@tanstack/react-router";
import { Toaster } from "@web/ui/components/sonner";

import { AppShell } from "@/components/app-shell";
import { StatusProvider } from "@/hooks/use-status";

import "../index.css";

export const Route = createRootRoute({
  component: RootComponent,
  head: () => ({
    meta: [
      { title: "AI Quota Monitor" },
      { name: "description", content: "Admin app for the AI quota monitor." },
    ],
  }),
});

function RootComponent() {
  return (
    <>
      <HeadContent />
      <StatusProvider>
        <AppShell>
          <Outlet />
        </AppShell>
      </StatusProvider>
      <Toaster position="top-center" richColors />
    </>
  );
}
