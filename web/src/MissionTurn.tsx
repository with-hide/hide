import { MailIcon } from "lucide-react";
import type { ReactNode } from "react";
import type { Actions } from "./actions";
import { AttentionChoices } from "./components/mission-attention";
import { MissionPrMark } from "./components/mission-pr";
import { StatusMark } from "./components/status-mark";
import { Button } from "./components/ui/button";
import { useInterfaceTranslation } from "./i18n/client";
import { DecisionBanner } from "./MissionDecision";
import { attentionReasonText, heldLettersText, turnItems, type TurnItem } from "./missions";
import { useWorkerEscalation } from "./missionWorkers";
import type { Mission, MissionNode } from "./snapshot";

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
    case "attention": {
      const why = attentionReasonText(item.attention.reason, item.attention.pr, t, item.attention.unmet_nodes);
      return (
        // What happened on its line, and under it what the operator can do, so the choices never crowd the reason out (B30).
        <div className="flex min-w-0 flex-col gap-xxs" data-mission-turn-item="attention" data-mission-turn-reason={item.attention.reason}>
          <Row>
            <StatusMark mark="turn" />
            {/* The pull request stands beside the choices, with its state. */}
            <NodeChip node={item.node} onSelect={onSelect} pr={false} />
            <span className="min-w-0 flex-1 truncate text-body" title={why}>{why}</span>
          </Row>
          <AttentionChoices key={`${item.node.key}-${item.attention.since_unix_ms}`} mission={mission} node={item.node} attention={item.attention} actions={actions} className="pl-(--size-icon-sm) ml-sm" />
        </div>
      );
    }
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
