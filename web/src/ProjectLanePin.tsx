import type { Actions } from "./actions";
import { LanePinButton } from "./components/lane-pin";
import { useInterfaceTranslation } from "./i18n/client";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";
import type { WorkspaceView } from "./workspace";

/**
 * The lane's pin (B15, B19; PRD session-docs-and-assistant D-73, B33), in
 * the head of the column at the lane's edge beside its ✕: the tool column's
 * while one is drawn, else the viewing column's. Filled and pressed while
 * the lane is pinned, its hint says what a press does, or that a pinned lane
 * is drawn over the agents because the body is too narrow to dock it.
 */
export function ProjectLanePin({ column, actions }: { column: "tool" | "view"; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const pinned = useShellStore((s) => (s.rest?.workspace_view as WorkspaceView | undefined)?.pinned ?? null);
  const narrow = useUiStore((s) => s.drawnLane.mode === "overlay");
  const toolDrawn = useUiStore((s) => s.drawnLane.tool !== null);
  if (pinned === null || (column === "view" && toolDrawn)) return null;
  const hint = pinned ? (narrow ? t("panes.lane.narrowPinHint") : t("panes.lane.unpinHint")) : t("panes.lane.pinHint");
  return <LanePinButton pinned={pinned} label={t("panes.lane.pin")} hint={hint} state={pinned ? (narrow ? "narrow" : "on") : "off"} onPin={() => actions.setLanePinned(!pinned)} />;
}
