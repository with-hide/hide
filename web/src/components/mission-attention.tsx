import { useState } from "react";
import type { Actions } from "../actions";
import { useInterfaceTranslation } from "../i18n/client";
import { attentionChoices, attentionPr, criterionName } from "../missions";
import { catalogWorkspaces, type Mission, type MissionAttention, type MissionNode } from "../snapshot";
import { useShellStore } from "../store";
import { MissionPrMark } from "./mission-pr";
import { Button } from "./ui/button";

// The operator's choices for a node that waits on someone to look (PRD
// mission-node-simple B15, B16, B26), the same in 내 차례, the node's panel
// and the mission's own Needs You row: the pull request the raise is about
// and its state first, and a line of the node that failed, so the choice
// is made with them in view, then each
// button saying what it does, Conductor에게 맡기기 first. 노드 제외 never
// goes on one press: the first turns it into 제외 확인 in place, and only
// that second press sends. Each choice sends one `mission_attention` event,
// and the core settles the node and tells the Conductor (B17). Once one is
// sent every choice is off, so a second press sends nothing; the item goes
// when the core's next snapshot settles it.

export function AttentionChoices({ mission, node, attention, actions }: { mission: Mission; node: MissionNode; attention: MissionAttention; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const [sent, setSent] = useState(false);
  const [confirming, setConfirming] = useState(false);
  const pulls = useShellStore((s) => catalogWorkspaces(s.rest).find((project) => project.id === mission.workspace_id)?.pull_requests);
  const pr = attentionPr(node, attention, pulls);
  const prWords = !pr ? null : pr.why?.kind === "queued" ? t("work.reason.queued", { count: pr.why.position }) : pr.why ? t(`mission.fix.${pr.why.kind}`) : pr.state ? t(`agentSessions.pr.${pr.state}`) : null;
  // A failed line is the worker's to fix, so it is the reason a node with no worker waits on someone.
  const failed = node.criteria.filter((line) => line.state === "fail").map((line) => criterionName(line, t));
  return (
    <span className="flex min-w-0 shrink-0 flex-wrap items-center gap-xs" data-attention-choices={attention.reason}>
      {pr ? (
        <span className="flex min-w-0 items-center gap-xxs text-caption text-muted-foreground" data-attention-pr={pr.number} data-attention-pr-state={pr.why?.kind ?? pr.state ?? "unread"}>
          <MissionPrMark number={pr.number} state={pr.state} />
          {prWords ? <span className="min-w-0 truncate">{prWords}</span> : null}
        </span>
      ) : null}
      {failed.length > 0 ? (
        <span className="min-w-0 truncate text-caption text-destructive" data-attention-failed={failed.length}>
          {t("mission.attention.lineFailed", { line: failed.join(", ") })}
        </span>
      ) : null}
      {attentionChoices(attention.reason, mission.conductor.present).map((choice) => {
        const confirm = choice === "drop" && confirming;
        return (
          <Button
            key={choice}
            size="sm"
            variant={confirm ? "destructive" : "secondary"}
            disabled={sent}
            onClick={() => {
              if (choice === "drop" && !confirming) {
                setConfirming(true);
                return;
              }
              setSent(true);
              actions.settleMissionNode(mission.id, node.key, choice, attention.since_unix_ms);
            }}
            onBlur={choice === "drop" ? () => setConfirming(false) : undefined}
            data-attention-choice={choice}
            data-attention-confirm={confirm ? "true" : undefined}
          >
            {confirm ? t("mission.operator.dropConfirm") : t(`mission.operator.${choice}`)}
          </Button>
        );
      })}
    </span>
  );
}
