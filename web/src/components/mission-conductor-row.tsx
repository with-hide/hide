import { MissionTakeOver } from "../MissionTakeOver";
import { useMemo } from "react";
import type { Actions } from "../actions";
import type { MessageKey } from "../i18n/catalogs";
import { AgentMark } from "../AgentMark";
import { useInterfaceTranslation } from "../i18n/client";
import { cn } from "../lib/utils";
import { RunText, useRunWords } from "../missionParts";
import { conductorState, type ConductorState } from "../missions";
import { allAgents, agentsByPane } from "../paneAgents";
import type { Mission } from "../snapshot";
import { useShellStore } from "../store";
import { Button } from "./ui/button";

// The mission's Conductor under the map's head (PRD mission-tool D-04, B12,
// B13): its agent's mark, `Conductor`, the model its pane was launched with,
// its state, and `터미널로 ›`, which brings its pane forward with the keyboard
// in it, on whichever device it runs. An open mission no Conductor holds
// shows `Conductor 없음` and `이어받기` instead. No record button: the task
// row and the Overview open the record.

const STATE_WORD: Record<Exclude<ConductorState, "gone" | "unknown">, MessageKey> = {
  working: "mission.conductor.state.working",
  deciding: "mission.conductor.state.deciding",
  resting: "mission.conductor.state.resting",
  asleep: "mission.conductor.state.asleep",
};

const STATE_TONE: Partial<Record<ConductorState, string>> = { working: "text-mission-working", deciding: "text-mission-hold" };

export function ConductorRow({ mission, actions }: { mission: Mission; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const local = useShellStore((s) => s.agents);
  const remote = useShellStore((s) => s.rest?.status?.remote);
  const row = useMemo(() => agentsByPane(allAgents(local, remote)).get(mission.conductor.pane_id) ?? null, [local, remote, mission.conductor.pane_id]);
  const state = conductorState(mission, row);
  // Only the launch Hide proved for the Conductor's pane names its model; the plan names none for it.
  const run = useRunWords(mission, mission.conductor.pane_id, mission.device_id, null);
  if (!state) return null;
  const frame = "mx-md mb-sm flex min-w-0 items-center gap-sm rounded-md border border-border bg-card px-sm py-xs text-body";
  if (state === "gone") {
    return (
      <div className={frame} data-mission-conductor="gone">
        <span aria-hidden="true" className="size-(--size-agent-mark) shrink-0 rounded-full border border-dashed border-muted-foreground" />
        <span className="min-w-0 flex-1 truncate text-muted-foreground">{t("mission.noConductor")}</span>
        <MissionTakeOver mission={mission} actions={actions} />
      </div>
    );
  }
  const word = state === "unknown" ? null : t(STATE_WORD[state]);
  const terminal = t("mission.toTerminal");
  return (
    <div className={frame} data-mission-conductor={state}>
      <AgentMark kind={mission.conductor.agent_kind} />
      <span className="shrink-0 font-semibold">{t("mission.conductor")}</span>
      <span className="flex min-w-0 flex-1 items-center gap-xs text-caption text-muted-foreground">
        {run ? <RunText run={run} className="min-w-0 shrink" /> : null}
        {run && word ? <span aria-hidden="true" className="shrink-0">·</span> : null}
        {word ? <span className={cn("shrink-0", STATE_TONE[state])} data-mission-conductor-state={state}>{word}</span> : null}
      </span>
      <Button
        size="sm"
        variant="ghost"
        aria-label={[terminal, t("mission.conductor"), run?.text, word].filter(Boolean).join(" · ")}
        onClick={() => actions.openAgent(mission.conductor.pane_id)}
        data-mission-conductor-terminal="true"
      >
        {terminal}
      </Button>
    </div>
  );
}
