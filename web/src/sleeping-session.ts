import type { MessageKey } from "./i18n/catalogs";
import type { SleepingSession } from "./snapshot";

export function sleepingCaption(session: Pick<SleepingSession, "phase" | "checking" | "cause">): MessageKey {
  if (session.checking) return "agents.checking";
  switch (session.phase) {
    case "close_unknown":
    case "wake_unknown": return "common.unknown";
    // The words a sleeping row says on its line (status-marks B16).
    case "sleeping": return session.cause ? `agents.sleepCause.${session.cause}` : "agents.status.sleeping";
    case "failed": return "agents.status.sleep_failed";
    case "saving_close":
    case "saving_close_ready":
    case "closing": return "panes.transport.closing";
    default: return "agents.status.waking";
  }
}
