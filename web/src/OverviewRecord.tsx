import { XIcon } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import type { Actions } from "./actions";
import { MissionChip } from "./components/mission-chip";
import { Button } from "./components/ui/button";
import { Hint } from "./components/ui/tooltip";
import { deviceConnected, frontDeviceId } from "./devices";
import { useInterfaceTranslation } from "./i18n/client";
import { boardProjects } from "./navigation";
import { cardOfPane, everyAgent } from "./overviewTasks";
import { machineName } from "./screenMachine";
import { SessionRecord, type RecordFrame } from "./SessionRecord";
import { sessionOfAgentOn, workKey } from "./sessionWork";
import { localDeviceId, type AgentRow, type WorkSession } from "./snapshot";
import { useShellStore } from "./store";
import { useUiStore, type OverviewPanel } from "./ui";

/**
 * A session's record beside the Overview's tabs (PRD overview-device-tasks
 * D-07, B16-B21; PRD sidebar-task-lens B17, B40): the same record the lane's
 * 보기 slot draws, at Home where no Workspace is in front, under the panel's
 * own head of ✕ and the ways to the work. A record of a task whose root runs
 * on another device than the one the Overview shows carries the note that
 * says whose task it is (B14).
 */
export function TaskRecordPanel({ panel, rows, actions }: { panel: Extract<OverviewPanel, { kind: "record" }>; rows: ReadonlyMap<string, AgentRow>; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const rest = useShellStore((s) => s.rest);
  const agents = useShellStore((s) => s.agents);
  const everyRow = useMemo(() => (rows.size > 0 ? rows : everyAgent(rest, agents)), [rows, rest, agents]);
  const agent = [...everyRow.values()].find((row) => {
    const current = sessionOfAgentOn(row, localDeviceId(rest));
    return current !== null && workKey(current) === workKey(panel.session);
  }) ?? null;
  const session = panel.session;
  const key = workKey(session);
  const [trail, setTrail] = useState<readonly WorkSession[]>([]);
  useEffect(() => setTrail([]), [key, panel.locator?.request_offset, panel.locator?.query]);
  // The task may live on another device than the one the Overview shows, so it is found among every device's projects.
  const pane = agent?.pane_id ?? null;
  const owner = useMemo(() => (pane ? cardOfPane(boardProjects(rest, agents), pane) : null), [rest, agents, pane]);
  const from = owner ? owner.project.device_id : undefined;
  const viewing = useUiStore((s) => s.overview.deviceId) ?? frontDeviceId(rest);
  useEffect(() => {
    actions.readSessionRecord(session);
    // The read follows the session's identity, not each snapshot's copy of it.
  }, [actions, key]);
  const close = () => {
    const ui = useUiStore.getState();
    if (panel.returnTo) ui.setMainView(panel.returnTo);
    ui.setOverview({ panel: null });
  };
  const project = owner?.project ?? null;
  const checkout = project && owner ? (owner.card.checkouts.map((id) => project.checkouts.find((row) => row.id === id)).find((row) => row?.exists) ?? null) : null;
  const elsewhere = project !== null && project.device_id !== frontDeviceId(rest);
  const deviceName = project ? machineName(rest?.navigator?.devices, project.device_id, t) : "";
  const head = (
    <div className="flex shrink-0 items-center gap-xs px-sm py-xs" data-record-head="overview">
      <Hint label={t("record.close")}>
        <Button variant="ghost" size="icon-sm" aria-label={t("record.close")} onClick={close} data-record-close="true"><XIcon /></Button>
      </Hint>
      {/* A Conductor's record leads to its mission's map, also once the session has closed (PRD software-factory-conductor B46). */}
      <MissionChip agent={{ session }} onOpen={(mission) => actions.openMission(mission)} />
      <span className="flex-1" />
      {project && checkout && agent && deviceConnected(rest,session.device_id) ? (
        elsewhere ? (
          <Button size="sm" variant="secondary" onClick={() => { actions.closeOverview(); actions.focusDevice(project.device_id); }} data-record-go-device={project.device_id}>
            {t("record.goToDevice", { device: deviceName })}
          </Button>
        ) : (
          <>
            <Button size="sm" onClick={() => { actions.closeOverview(); actions.openAgent(agent.pane_id); }} data-record-open-workspace="true">{t("record.openWorkspace")}</Button>
            <Button size="sm" variant="outline" onClick={() => actions.viewRecordInLane(project.device_id, project.id, checkout.id, session)} data-record-in-lane="true">{t("record.viewInLane")}</Button>
          </>
        )
      ) : null}
    </div>
  );
  const note = from && from !== viewing
    ? t("record.incomingNote", { device: machineName(rest?.navigator?.devices, viewing, t), root: machineName(rest?.navigator?.devices, from, t) })
    : null;
  const frame = useMemo<RecordFrame>(() => ({
    kind: "overview",
    head,
    note,
    locator: panel.locator,
    trail,
    onTrail: setTrail,
    onRead: () => actions.readSessionRecord(session),
    onLeave: () => actions.leaveOverviewRecord(),
  }), [head, note, panel.locator, trail, session, actions]);
  return <SessionRecord key={key} actions={actions} target={session} frame={frame} />;
}
