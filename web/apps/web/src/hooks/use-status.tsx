import { createContext, useCallback, useContext, useEffect, useMemo, useState } from "react";
import type { ReactNode } from "react";

import { api, type Status } from "@/lib/api";

const POLL_MS = 10_000;
const CLOCK_MS = 15_000;

interface StatusState {
  /** Last good /api/status answer. Kept when a later poll fails. */
  status: Status | null;
  /** False after a failed poll; the data is then last-known. */
  reachable: boolean;
  /** True once the first poll has finished, either way. */
  loaded: boolean;
  /** Unix seconds, refreshed every 15 s so "in 59m" labels stay current. */
  now: number;
  refetch: () => Promise<void>;
}

const StatusContext = createContext<StatusState | null>(null);
const unixNow = () => Math.floor(Date.now() / 1000);

export function StatusProvider({ children }: { children: ReactNode }) {
  const [status, setStatus] = useState<Status | null>(null);
  const [reachable, setReachable] = useState(true);
  const [loaded, setLoaded] = useState(false);
  const [now, setNow] = useState(unixNow);

  const refetch = useCallback(async () => {
    try {
      setStatus(await api.status());
      setReachable(true);
    } catch {
      setReachable(false);
    }
    setNow(unixNow());
    setLoaded(true);
  }, []);

  // Poll every 10 s, but only while the tab is visible; catch up on return.
  useEffect(() => {
    let poll: number | undefined;
    let clock: number | undefined;
    const start = () => {
      void refetch();
      poll = window.setInterval(() => void refetch(), POLL_MS);
      clock = window.setInterval(() => setNow(unixNow()), CLOCK_MS);
    };
    const stop = () => {
      clearInterval(poll);
      clearInterval(clock);
    };
    const onVisibility = () => {
      stop();
      if (!document.hidden) start();
    };
    if (!document.hidden) start();
    document.addEventListener("visibilitychange", onVisibility);
    return () => {
      stop();
      document.removeEventListener("visibilitychange", onVisibility);
    };
  }, [refetch]);

  const value = useMemo(
    () => ({ status, reachable, loaded, now, refetch }),
    [status, reachable, loaded, now, refetch],
  );
  return <StatusContext.Provider value={value}>{children}</StatusContext.Provider>;
}

export function useStatus(): StatusState {
  const value = useContext(StatusContext);
  if (!value) throw new Error("useStatus outside StatusProvider");
  return value;
}
