import { createFileRoute } from "@tanstack/react-router";

import { ProviderPage } from "@/components/provider-page";

export const Route = createFileRoute("/claude")({
  component: () => <ProviderPage id="claude" />,
});
