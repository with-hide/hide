import { MailIcon, XIcon } from "lucide-react";
import { useState, type ReactNode } from "react";
import type { Actions } from "./actions";
import { AttentionChoices, AttentionFacts, failedLine, failedLines } from "./components/mission-attention";
import { MissionPrMark } from "./components/mission-pr";
import { StatusMark } from "./components/status-mark";
import { Button } from "./components/ui/button";
import { useInterfaceTranslation } from "./i18n/client";
import { DecisionBanner } from "./MissionDecision";
import { attentionReasonText, heldLettersText, turnItems, type TurnItem } from "./missions";
import { useWorkerEscalation } from "./missionWorkers";
import type { Mission, MissionAttention, MissionNode } from "./snapshot";

// What only the operator can do in a mission now, above its map (PRD
// mission-map-git-criteria D-03, D-18, B18, B20, B25; PRD mission-node-simple
// B14, B15): the Conductor's decision, answered here as it always was, a node
// the core raised to the operator with the choices its reason takes, a worker
// the core raised past its Conductor, the order to merge or join what waits
// for main, and a mission no Conductor holds. Work an agent carries is not
// here, and with nothing to do the section is gone.

export function MissionTurn({ mission, device, actions, onSelect }: { mission: Mission; device: string | null; actions: Actions; onSelect: (key: string) => void }) {
  const { t } = useInterfaceTranslation();
  const escalationOf = useWorkerEscalation();
  // A mission no Conductor holds says so on its Conductor row, with 이어받기 there (PRD mission-tool B13).
  const items = turnItems(mission, escalationOf).filter((item) => item.kind !== "takeover");
  if (items.length === 0) return null;
  return (
    <section aria-label={t("mission.turn", { count: items.length })} className="flex min-w-0 flex-col gap-xs px-md pb-sm" data-mission-turn={items.map((item) => item.kind).join(" ")}>
      <h3 className="text-caption font-semibold text-mission-hold">{t("mission.turn", { count: items.length })}</h3>
      {items.map((item, index) => <Item key={`${item.kind}-${index}`} item={item} mission={mission} device={device} actions={actions} onSelect={onSelect} />)}
    </section>
  );
}

function Item({ item, mission, device, actions, onSelect }: { item: TurnItem; mission: Mission; device: string | null; actions: Actions; onSelect: (key: string) => void }) {
  const { t } = useInterfaceTranslation();
  const conversation = (
    <Button size="sm" variant="ghost" onClick={() => actions.openAgent(mission.conductor.pane_id)} data-mission-conversation="true">
      {t("mission.toTerminal")}
    </Button>
  );
  switch (item.kind) {
    case "decision":
      // A new question is a new banner: nothing written for the one before carries over to it (B26).
      return <DecisionBanner key={mission.question?.key ?? 0} mission={mission} device={device} actions={actions} onNode={onSelect} />;
    case "letters": {
      // The letters the Conductor has not got, and what the operator does: the Conductor's pane is where each reason is lifted (hide-private issue 678).
      const text = heldLettersText(item.letters, t);
      return (
        <Row data-mission-turn-item="letters" data-mission-turn-reason={item.letters.reason}>
          <MailIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0 text-mission-hold" />
          <span className="min-w-0 flex-1 truncate text-body" title={text}>{text}</span>
          {conversation}
        </Row>
      );
    }
    case "worker_blocked":
      return (
        <Row data-mission-turn-item="worker_blocked">
          <StatusMark mark="turn" />
          <NodeChip node={item.node} onSelect={onSelect} />
          <span className="min-w-0 flex-1 truncate text-body" title={item.line ?? undefined} data-mission-turn-cause={item.cause}>
            {t("mission.turn.workerBlocked")}: {item.line ?? t(`mission.turn.cause.${item.cause}`)}
          </span>
          {item.node.pane_id ? <Button size="sm" variant="ghost" onClick={() => actions.openAgent(item.node.pane_id!)} data-mission-turn-terminal="true">{t("mission.toTerminal")}</Button> : null}
        </Row>
      );
    case "attention":
      return <AttentionCard key={`${item.node.key}-${item.attention.since_unix_ms}`} mission={mission} node={item.node} attention={item.attention} actions={actions} onSelect={onSelect} />;
    case "landing":
      return (
        <Row data-mission-turn-item={item.git ? "join" : "merge"}>
          <span className="shrink-0 text-body font-semibold">{t(item.git ? "mission.turn.join" : "mission.turn.merge")}</span>
          <span className="flex min-w-0 flex-1 flex-wrap items-center gap-xxs" data-mission-landing-order={item.nodes.map((node) => node.key).join(" ")}>
            {item.nodes.map((node, index) => (
              <span key={node.key} className="flex min-w-0 items-center gap-xxs">
                {index > 0 ? <span aria-hidden="true" className="text-muted-foreground">→</span> : null}
                <NodeChip node={node} onSelect={onSelect} />
              </span>
            ))}
          </span>
          {conversation}
        </Row>
      );
  }
}

/**
 * A node raised to the operator, as a card in the 내 차례 colour (operator
 * report on M41): the node with 노드 보기, what happened with the facts behind
 * it, why it came to the operator and not the Conductor, the choices, and
 * 자세히 with the worker's one line on each failed finish line, the line after it.
 */
function AttentionCard({ mission, node, attention, actions, onSelect }: { mission: Mission; node: MissionNode; attention: MissionAttention; actions: Actions; onSelect: (key: string) => void }) {
  const { t } = useInterfaceTranslation();
  const [open, setOpen] = useState(false);
  const lines = failedLines(node, false);
  const what = attentionReasonText(attention.reason, attention.pr, t, attention.unmet_nodes);
  return (
    <div className="flex min-w-0 flex-col gap-xs rounded-md border border-mission-turn bg-mission-turn/10 px-sm py-xs" data-mission-turn-item="attention" data-mission-turn-reason={attention.reason}>
      <div className="flex min-w-0 items-center gap-sm">
        <span aria-hidden="true" className="size-(--size-agent-mark) shrink-0 rounded-full bg-mission-turn" />
        <span className="min-w-0 flex-1 truncate text-body font-semibold" title={node.title}>{node.title}</span>
        <Button size="sm" variant="ghost" onClick={() => onSelect(node.key)} data-mission-turn-node={node.key}>{t("mission.turn.openNode")}</Button>
      </div>
      <dl className="grid min-w-0 grid-cols-[auto_1fr] items-baseline gap-x-md gap-y-xxs text-body">
        <dt className="text-caption text-muted-foreground">{t("mission.turn.what")}</dt>
        <dd className="flex min-w-0 flex-wrap items-center gap-x-sm" data-mission-turn-what="true">
          <span className="min-w-0 truncate" title={what}>{what}</span>
          <AttentionFacts mission={mission} node={node} attention={attention} />
        </dd>
        <dt className="text-caption text-muted-foreground">{t("mission.turn.why")}</dt>
        <dd className="min-w-0 truncate" data-mission-turn-why={whyYou(mission, attention)}>{t(`mission.turn.why.${whyYou(mission, attention)}`)}</dd>
      </dl>
      <div className="flex min-w-0 flex-wrap items-center gap-xs">
        <AttentionChoices mission={mission} node={node} attention={attention} actions={actions} facts={false} />
        {lines.length > 0 ? (
          <Button variant="text" size="inline" className="text-caption" aria-expanded={open} onClick={() => setOpen(!open)} data-mission-turn-details-toggle="true">
            {t(open ? "mission.turn.less" : "mission.turn.more")}
          </Button>
        ) : null}
      </div>
      {open ? (
        <ul className="flex min-w-0 flex-col gap-xs text-caption" data-mission-turn-details={lines.length}>
          {lines.map((line) => (
            // The worker's one line on the result leads, in plain words; the finish line it failed follows quietly.
            <li key={line.key} className="flex min-w-0 items-start gap-xs">
              <XIcon aria-hidden="true" className="mt-xxs size-(--size-icon-sm) shrink-0 text-destructive" />
              <span className="flex min-w-0 flex-col">
                <span className="break-words text-body text-foreground">{line.summary ?? failedLine(line, t)}</span>
                {line.summary ? <span className="break-words text-muted-foreground">{failedLine(line, t)}</span> : null}
              </span>
            </li>
          ))}
        </ul>
      ) : null}
    </div>
  );
}

/** Why a raise reached the operator: the core gives a raise to the Conductor first and to the operator once it stood unsettled, at once with no Conductor or for the Conductor's own failure. */
function whyYou(mission: Mission, attention: MissionAttention): "failed" | "noConductor" | "waited" {
  if (attention.reason === "conductor_failed") return "failed";
  return mission.conductor.present ? "waited" : "noConductor";
}

function Row({ children, ...data }: { children: ReactNode } & Record<`data-${string}`, string>) {
  return <div className="flex min-w-0 items-center gap-sm" {...data}>{children}</div>;
}

/** A node named in 내 차례, which picks it on the map (B25): its title, with its pull request's mark when it has one (PRD mission-tool B18). */
function NodeChip({ node, onSelect, pr: withPr = true }: { node: MissionNode; onSelect: (key: string) => void; pr?: boolean }) {
  // The pull request a node waits on now is its last one (D-10).
  const pr = withPr ? (node.pr ?? (node.external && "pr" in node.external ? node.external.pr : null)) : null;
  return (
    <button type="button" onClick={() => onSelect(node.key)} title={node.title} className="flex min-w-0 max-w-(--size-flow-title) shrink items-center gap-xxs rounded-xs bg-muted px-xs text-caption outline-none hover:bg-accent focus-visible:ring-1 focus-visible:ring-ring" data-mission-turn-node={node.key}>
      <span className="min-w-0 truncate">{node.title}</span>
      {pr != null ? <MissionPrMark number={pr} state={node.pr_state} /> : null}
    </button>
  );
}
