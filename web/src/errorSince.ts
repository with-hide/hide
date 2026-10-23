// The core's `last_error` as a surface that sent a request reads it: the
// failure that answers that request, worded by `coreErrorText`.

import { useEffect, useState } from "react";
import type { CoreError } from "./coreError";
import { useShellStore } from "./store";

/** The core's newest error, if it arrived after `since` and its kind starts with one of `prefixes`. */
export function useErrorSince(since: number | null, prefixes: readonly string[]): CoreError | null {
  const error = useShellStore((s) => s.rest?.status?.last_error ?? null);
  if (since === null || !error || error.occurred_at < since) return null;
  return prefixes.some((prefix) => error.kind.startsWith(prefix)) ? error : null;
}

/**
 * The same refusal, kept. The core clears `last_error` on its next event of
 * any kind, so a form that reads "no error yet" as "still pending" would
 * disable itself again the moment an unrelated event arrived. This returns
 * the refusal until the request it answers is replaced (`since` changes):
 * pending is "sent, not landed and not refused", and a refusal stays
 * refused (engineering 13: model the state).
 */
export function useRefusalSince(since: number | null, prefixes: readonly string[]): CoreError | null {
  const live = useErrorSince(since, prefixes);
  const [kept, setKept] = useState<{ since: number | null; error: CoreError } | null>(null);
  useEffect(() => {
    if (live !== null) setKept({ since, error: live });
  }, [live, since]);
  return live ?? (kept !== null && kept.since === since ? kept.error : null);
}
