import { useInterfaceTranslation } from "./i18n/client";
import type { MessageKey } from "./i18n/catalogs";
import { cn } from "./lib/utils";
import type { PaneNotice } from "./snapshot";

const NOTICE_LABELS: Record<string, MessageKey> = {
  sleeping: "panes.sleep.captionSleeping", waking: "panes.sleep.captionWaking", failed: "panes.sleep.captionFailed", restore_failed: "panes.restore.failed",
  disconnected: "panes.transport.disconnected", closing: "panes.transport.closing", starting: "panes.transport.starting", reconnecting: "panes.transport.reconnecting", helper_not_allowed: "panes.transport.helperNotAllowed",
  unavailable: "panes.transport.remoteUnavailable", terminated: "panes.transport.remoteEnded", exit: "agentSessions.exitCode",
  controlled_elsewhere: "panes.transport.scrollElsewhere", device_offline: "panes.transport.disconnected",
};
const NOTICE_TONES: Record<PaneNotice["tone"], string> = { muted: "text-subtle-foreground", warning: "text-warning", error: "text-destructive" };

/**
 * Why the terminal is not live, in the terminal's place (docs/UI_BEHAVIOR.md,
 * Agent panes and the Agents explorer): the core's notice for a state no other body here draws.
 * A dead terminal is covered; a start or close that passes by itself is a
 * small caption over whatever the terminal shows, and a wheel another client
 * holds sits in a corner, because that terminal is live.
 */
export function PaneNoticeBody({ notice }: { notice: PaneNotice }) {
  const { t } = useInterfaceTranslation();
  const key = NOTICE_LABELS[notice.kind];
  if (!key) throw new Error(`Unknown core pane notice: ${notice.kind}`);
  const label = notice.kind === "exit" ? t("agentSessions.exitCode", { exitCode: notice.exit_code! }) : t(key as Exclude<MessageKey, "agentSessions.exitCode">);
  const text = notice.kind === "device_offline" && notice.reason ? `${label} · ${notice.reason}` : label;
  const tone = NOTICE_TONES[notice.tone];
  if (notice.kind === "controlled_elsewhere") return <p className={cn("pointer-events-none absolute right-sm bottom-sm rounded-xs bg-card px-xs py-xxs text-micro", tone)} role="status" data-pane-notice={notice.kind}>{text}</p>;
  if (notice.kind === "starting" || notice.kind === "closing") return <div className="pointer-events-none absolute inset-0 flex items-center justify-center"><p className={cn("rounded-xs bg-card px-sm py-xxs text-caption", tone)} role="status" data-pane-notice={notice.kind}>{text}</p></div>;
  return <div className={cn("absolute inset-0 flex items-center justify-center bg-card px-lg text-center text-caption", tone)} role="status" data-pane-notice={notice.kind}>{text}</div>;
}
