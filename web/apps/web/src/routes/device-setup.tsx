import { createFileRoute } from "@tanstack/react-router";
import { Input } from "@web/ui/components/input";
import { useId } from "react";

import { CopyButton } from "@/components/copy-button";
import { Loading, LoadError, PageHeader, Panel } from "@/components/page";
import { useLoad } from "@/hooks/use-load";
import { api, type DeviceSetup } from "@/lib/api";

export const Route = createFileRoute("/device-setup")({ component: DeviceSetupPage });

function DeviceSetupPage() {
  const { data, error, reload } = useLoad(api.deviceSetup);
  return (
    <>
      <PageHeader
        title="Device setup"
        description="The broker details to type into the Device's setup page. Anyone who can open this page can read the password."
      />
      {error ? <LoadError error={error} onRetry={reload} /> : data ? <Details setup={data} /> : <Loading what="broker details" />}
    </>
  );
}

function Details({ setup }: { setup: DeviceSetup }) {
  return (
    <div className="flex flex-col gap-4">
      <Panel aria-labelledby="broker" className="flex flex-col gap-4">
        <h2 id="broker" className="text-base font-semibold">
          Broker details
        </h2>
        <Field label="Host" value={setup.host} copyLabel="Copy host" />
        <Field label="Port" value={String(setup.port)} copyLabel="Copy port" />
        <Field label="Username" value={setup.username} copyLabel="Copy username" />
        <Field label="Password" value={setup.password} copyLabel="Copy password" mono />
        <p className="text-sm text-muted-foreground">
          Connection: {setup.tls ? "TLS on. Turn on TLS in the setup form." : "plain MQTT, no TLS. Leave TLS off in the setup form."}
        </p>
      </Panel>

      <Panel aria-labelledby="steps" className="flex flex-col gap-3">
        <h2 id="steps" className="text-base font-semibold">
          Set up the Device
        </h2>
        <ol className="flex list-decimal flex-col gap-3 pl-5 marker:text-primary">
          <li>
            On your phone or laptop, join the Wi-Fi network <span className="font-mono">Usage-Setup-XXXXXX</span> shown on the Device
            screen. Its password is the 8 digits shown there.
          </li>
          <li>
            Open <span className="font-mono">http://192.168.4.1/</span> in a browser.
          </li>
          <li>Fill in your home Wi-Fi name and password, then the broker details above.</li>
          <li>Tap Connect. The Device restarts and joins your network.</li>
        </ol>
        <p className="text-muted-foreground">
          The timezone is not part of this form any more. Set it on the Device page; the Device receives it from the server.
        </p>
      </Panel>

      <Panel aria-labelledby="again" className="flex flex-col gap-2">
        <h2 id="again" className="text-base font-semibold">
          Start over later
        </h2>
        <p className="text-muted-foreground">
          To change the Wi-Fi network or broker, open the gear on the Device, scroll to Reconfigure Wi-Fi and tap it. Confirm Erase and
          the Device shows a new setup network. If the Device cannot join Wi-Fi, it keeps showing the last numbers and says to open Settings, then Reconfigure Wi-Fi.
        </p>
      </Panel>
    </div>
  );
}

function Field({ label, value, copyLabel, mono }: { label: string; value: string; copyLabel: string; mono?: boolean }) {
  const id = useId();
  return (
    <div>
      <label htmlFor={id} className="mb-1.5 block text-sm font-medium text-muted-foreground">
        {label}
      </label>
      <div className="flex items-center gap-2">
        <Input
          id={id}
          readOnly
          value={value}
          onFocus={(event) => event.currentTarget.select()}
          className={`h-11 min-w-0 flex-1 rounded-lg text-base md:h-10 md:text-sm ${mono ? "font-mono tracking-wider" : ""}`}
        />
        <CopyButton text={value} label={copyLabel} />
      </div>
    </div>
  );
}
