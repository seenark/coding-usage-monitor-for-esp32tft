import { createFileRoute } from "@tanstack/react-router";

import { ProviderPage } from "@/components/provider-page";

export const Route = createFileRoute("/codex")({
  component: () => <ProviderPage id="codex" />,
});
