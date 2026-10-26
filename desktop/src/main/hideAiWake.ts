// Hide AI's day (PRD hide-ai-suggestions B6): the core starts the day's
// suggestion runs a few minutes after the operator is back at the machine,
// and only the desktop host sees that: the app opening, the machine waking
// and the screen unlocking. The core runs a day once however many wakes it
// is sent, so the host sends each one and keeps no count of its own.
// Linux reports no unlock (docs/INSTALL.md, What each system supports).

import type { Attached } from "./cli";

export type WakeCause = "app_open" | "resume" | "unlock";

export const WAKE_TIMEOUT_MS = 3_000;

/** The request that tells the attached core the operator is back, or null when its address carries no token. */
export function wakeRequest(daemon: Attached, cause: WakeCause): { url: string; init: RequestInit } | null {
  const token = new URLSearchParams(new URL(daemon.url).hash.slice(1)).get("token");
  if (!token) return null;
  return {
    url: `${daemon.origin}/hide-ai-wake`,
    init: {
      method: "POST",
      headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/json" },
      body: JSON.stringify({ cause }),
    },
  };
}
