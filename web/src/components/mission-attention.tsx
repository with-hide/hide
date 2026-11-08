import { useState } from "react";
import type { Actions } from "../actions";
import { useInterfaceTranslation } from "../i18n/client";
import { attentionPr, criterionName } from "../missions";
import { ReasonLine } from "../prReason";
import { catalogWorkspaces, type Mission, type MissionAttention, type MissionCriterion, type MissionNode } from "../snapshot";
import { useShellStore } from "../store";
import { MissionPrMark } from "./mission-pr";
import { Button } from "./ui/button";

// The operator's choices for a node that waits on someone to look (PRD
// mission-node-simple B15, B16, B26), the same in 내 차례, the node's panel
// and the mission's own Needs You row. First the pull request the raise is
// about with why it stands where it does, the reason the PRs list gives it
// (the core's judgment, conflict and merge queue included), and a line of
// the node that failed and that reason does not already say, so the choice
// is made with the cause in view. Then the choices the core offers for the
// raise, in its order (`AttentionReason::choices`): 지휘 에이전트에게 넘기기
// first, or 그대로 두기 with no Conductor, and 이 단계 빼기 last, which never
// goes on one press: the first turns it into 빼기 확인 in place, a press
// elsewhere puts it back, and only the second press sends. A refused close
// leads with 완료로 닫기 (`done` on that reason), which closes the mission and
// follows the same two presses (닫기 확인), and offers no 이 단계 빼기, since
// its work is finished. Each choice sends
// one `mission_attention` event, and the core settles the node and tells
// the Conductor (B17). Once one is sent every choice is off; the item goes
// when the core's next snapshot settles it.

export function AttentionChoices({ mission, node, attention, actions }: { mission: Mission; node: MissionNode; attention: MissionAttention; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const [sent, setSent] = useState(false);
  const [confirming, setConfirming] = useState(false);
  const project = useShellStore((s) => catalogWorkspaces(s.rest).find((row) => row.id === mission.workspace_id) ?? null);
  const pr = attentionPr(node, attention, project?.pull_requests);
  const row = pr ? (project?.agent_scope.prs.rows.find((candidate) => candidate.number === pr.number) ?? null) : null;
  // The pull request's own words already say a failed CI.
  const failed = node.criteria.filter((line) => line.state === "fail" && !(pr && line.kind === "ci")).map((line) => failedLine(line, t));
  return (
    <span className="flex min-w-0 shrink-0 flex-wrap items-center gap-xs" data-attention-choices={attention.reason}>
      {pr ? (
        <span className="flex min-w-0 items-center gap-xxs text-caption text-muted-foreground" data-attention-pr={pr.number} data-attention-pr-reason={row?.reason.kind ?? pr.state ?? "unread"}>
          <MissionPrMark number={pr.number} state={pr.state} />
          {row ? <ReasonLine reason={row.reason} group={row.group} className="text-caption" /> : pr.state ? <span className="min-w-0 truncate">{t(`agentSessions.pr.${pr.state}`)}</span> : null}
        </span>
      ) : null}
      {failed.length > 0 ? (
        <span className="min-w-0 truncate text-caption text-destructive" data-attention-failed={failed.length}>
          {failed.join(", ")}
        </span>
      ) : null}
      {attention.choices.map((choice) => {
        // A choice that ends something goes on a second press: dropping a node, or closing a mission whose empty lines the operator passes.
        const closes = choice === "done" && attention.reason === "close_refused";
        const twice = choice === "drop" || closes;
        const confirm = twice && confirming;
        const label = closes
          ? t(confirm ? "mission.operator.closeDoneConfirm" : "mission.operator.closeDone")
          : confirm
            ? t("mission.operator.dropConfirm")
            : choice === "leave" && !mission.conductor.present
              ? t("mission.operator.leaveAsIs")
              : t(`mission.operator.${choice}`);
        return (
          <Button
            key={choice}
            size="sm"
            variant={confirm ? "destructive" : "secondary"}
            disabled={sent}
            onClick={() => {
              if (twice && !confirming) {
                setConfirming(true);
                return;
              }
              setSent(true);
              actions.settleMissionNode(mission.id, node.key, choice, attention.since_unix_ms);
            }}
            onBlur={twice ? () => setConfirming(false) : undefined}
            data-attention-choice={choice}
            data-attention-confirm={confirm ? "true" : undefined}
          >
            {label}
          </Button>
        );
      })}
    </span>
  );
}

/** A failed line as `<line> 실패`: a review or a verification by its kind alone, a CI with no pull request to say it as `CI 실패`. */
function failedLine(line: MissionCriterion, t: ReturnType<typeof useInterfaceTranslation>["t"]): string {
  switch (line.kind) {
    case "ci":
      return t("mission.fix.ci_failed");
    case "review":
    case "verify":
      return t("mission.attention.lineFailed", { line: t(`mission.line.${line.kind}`) });
    default:
      return t("mission.attention.lineFailed", { line: criterionName(line, t) });
  }
}
