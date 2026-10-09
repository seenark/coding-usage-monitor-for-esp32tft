import { useCallback, useEffect, useState } from "react";

import { ApiError } from "@/lib/api";

/** Load one resource on mount. `reload` retries after a failure. */
export function useLoad<T>(load: () => Promise<T>) {
  const [data, setData] = useState<T | null>(null);
  const [error, setError] = useState<ApiError | null>(null);
  const [attempt, setAttempt] = useState(0);

  useEffect(() => {
    let cancelled = false;
    setError(null);
    load().then(
      (value) => !cancelled && setData(value),
      (reason) => !cancelled && setError(reason instanceof ApiError ? reason : new ApiError(0, "unreachable")),
    );
    return () => {
      cancelled = true;
    };
    // `load` is a stable api method per call site.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [attempt]);

  const reload = useCallback(() => setAttempt((count) => count + 1), []);
  return { data, error, reload };
}
