import { MailIcon } from "lucide-react";
import type { ReactNode } from "react";
import type { Actions } from "./actions";
import { AttentionChoices } from "./components/mission-attention";
import { useElapsed } from "./components/elapsed";
import { MissionPrMark } from "./components/mission-pr";
import { StatusMark } from "./components/status-mark";
import { Button } from "./components/ui/button";
import { useInterfaceTranslation } from "./i18n/client";
import { DecisionBanner } from "./MissionDecision";
import { mergePull, attentionReasonText, heldLettersText, turnItems, type TurnItem } from "./missions";
import { MissionTakeOver } from "./MissionTakeOver";
import { useWorkerEscalation } from "./missionWorkers";
import type { Mission, MissionHeldNode, MissionNode, MissionSafetyNet } from "./snapshot";

// What only the operator can do in a mission now, above its map (PRD
// mission-map-git-criteria D-03, D-18, B18, B20, B25; PRD mission-node-simple
// B14; the principle of 2026-11-11 in hide-private docs/missions.md): the
// Conductor's decision, answered here as it always was, the one safety net
// card for the raises the Conductor has not settled, a raise only the
// operator settles with the choices its reason takes, a worker the core raised
// past its Conductor, the order to merge or join what waits for main, and,
// while the mission runs, a ready pull request the plan left to the operator
// (PRD mission-destination D-11). A
// node raised to the Conductor is not here, nor is work an agent carries, and
// with nothing to do the section is gone.

export function MissionTurn({ mission, device, actions, onSelect }: { mission: Mission; device: string | null; actions: Actions; onSelect: (key: string) => void }) {
  const { t } = useInterfaceTranslation();
  const escalationOf = useWorkerEscalation();
  // A mission no Conductor holds says so on its Conductor row, with 지휘 다시 열기 there (PRD mission-tool B13), unless raises stand: then the safety net card says it.
  const items = turnItems(mission, escalationOf).filter((item) => item.kind !== "takeover" || item.net !== null);
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
    case "takeover":
    case "safety_net":
      return item.net ? <SafetyNetCard mission={mission} net={item.net} actions={actions} onSelect={onSelect} /> : null;
    case "attention": {
      // Only a raise the operator settles first comes here: the Conductor's own failure, or a gate reached with no Conductor, which says what to look at (PRD mission-human-gate D-23).
      const what = item.attention.reason === "human_gate" && item.node.gate ? item.node.gate.look : attentionReasonText(item.attention.reason, item.attention.pr, t, item.attention.unmet_nodes);
      return (
        <Row data-mission-turn-item="attention" data-mission-turn-reason={item.attention.reason}>
          <StatusMark mark="turn" />
          <NodeChip node={item.node} onSelect={onSelect} />
          <span className="min-w-0 shrink truncate text-body" title={what} data-mission-turn-what="true">{what}</span>
          <AttentionChoices key={`${item.node.key}-${item.attention.since_unix_ms}`} mission={mission} node={item.node} attention={item.attention} actions={actions} className="ml-auto" />
        </Row>
      );
    }
    case "landing":
      return (
        <Row data-mission-turn-item={item.git ? "join" : "merge"}>
          <span className="shrink-0 text-body font-semibold">{t(item.git ? "mission.turn.join" : item.due ? "mission.turn.mergeDue" : "mission.turn.merge")}</span>
          <span className="flex min-w-0 flex-1 flex-wrap items-center gap-xxs" data-mission-landing-order={item.nodes.map((node) => node.key).join(" ")}>
            {item.nodes.map((node, index) => (
              <span key={node.key} className="flex min-w-0 items-center gap-xxs">
                {index > 0 ? <span aria-hidden="true" className="text-muted-foreground">→</span> : null}
                <NodeChip node={node} onSelect={onSelect} merging={!item.git} />
              </span>
            ))}
          </span>
          {conversation}
        </Row>
      );
  }
}

/**
 * The mission's one card for the raises its Conductor has not settled (the
 * principle of 2026-11-11, hide-private docs/missions.md): how many stand
 * and since when, with no Conductor that it has none, a row per held node
 * with its own raise's reason and age, and the one way out, the Conductor's
 * pane or 지휘 다시 열기. It stands for the Conductor row while no Conductor
 * holds the mission, so the screen says that once.
 */
function SafetyNetCard({ mission, net, actions, onSelect }: { mission: Mission; net: MissionSafetyNet; actions: Actions; onSelect: (key: string) => void }) {
  const { t } = useInterfaceTranslation();
  // While the mission's device does not answer, nothing is counted on.
  const age = useElapsed(mission.stale ? null : net.since_unix_ms);
  const count = net.nodes.length;
  const head = !net.conductor_present ? t("mission.net.noConductor", { count }) : age ? t("mission.net.title", { count, age }) : t("mission.net.line", { count });
  return (
    <div className="tone-hold flex min-w-0 flex-col gap-xs rounded-md border border-mission-hold/30 bg-mission-hold/8 px-sm py-xs [&>:not(:first-child)]:ml-[calc(var(--size-agent-mark)+var(--spacing-sm))]" data-mission-turn-item="safety_net" data-mission-safety-net={net.conductor_present ? "present" : "absent"}>
      <div className="flex min-w-0 items-center gap-sm">
        <span aria-hidden="true" className="size-(--size-agent-mark) shrink-0 rounded-full bg-mission-hold" />
        <span className="flex min-w-0 flex-1 flex-col">
          <span className="truncate text-body font-semibold" title={head} data-mission-safety-net-head="true">{head}</span>
          {net.conductor_present && age ? <span className="truncate text-caption text-muted-foreground">{t("mission.net.unsettled")}</span> : null}
        </span>
        {net.conductor_present ? (
          <Button size="sm" variant="ghost" onClick={() => actions.openAgent(mission.conductor.pane_id)} data-mission-conversation="true">{t("mission.toTerminal")}</Button>
        ) : (
          <MissionTakeOver mission={mission} actions={actions} disabled={mission.stale} />
        )}
      </div>
      <ul className="flex min-w-0 flex-col gap-xxs" data-mission-safety-net-nodes={count}>
        {net.nodes.map((held) => <HeldRow key={held.key} mission={mission} held={held} onSelect={onSelect} />)}
      </ul>
    </div>
  );
}

/** One node the safety net holds: the node, its raise's reason, and how long that raise stood. */
function HeldRow({ mission, held, onSelect }: { mission: Mission; held: MissionHeldNode; onSelect: (key: string) => void }) {
  const { t } = useInterfaceTranslation();
  const age = useElapsed(mission.stale ? null : held.since_unix_ms);
  const node = mission.nodes.find((one) => one.key === held.key) ?? null;
  const reason = attentionReasonText(held.reason, held.pr, t);
  return (
    <li className="flex min-w-0 items-center gap-sm text-body" data-mission-held-node={held.key} data-mission-turn-reason={held.reason}>
      {node ? <NodeChip node={node} onSelect={onSelect} /> : <span className="min-w-0 shrink truncate text-caption">{held.title}</span>}
      <span className="min-w-0 flex-1 truncate" title={reason}>{reason}</span>
      {age ? <span className="shrink-0 text-caption text-muted-foreground">{t("mission.net.age", { age })}</span> : null}
    </li>
  );
}

function Row({ children, ...data }: { children: ReactNode } & Record<`data-${string}`, string>) {
  return <div className="flex min-w-0 items-center gap-sm" {...data}>{children}</div>;
}

/** A node named in 내 차례, which picks it on the map (B25): its title, with its pull request's mark when it has one (PRD mission-tool B18). */
function NodeChip({ node, onSelect, merging = false }: { node: MissionNode; onSelect: (key: string) => void; merging?: boolean }) {
  const pull = merging ? mergePull(node) : { number: node.pr ?? (node.external && "pr" in node.external ? node.external.pr : null), state: node.pr_state, more: 0 };
  return (
    <button type="button" onClick={() => onSelect(node.key)} title={node.title} className="flex min-w-0 max-w-(--size-flow-title) shrink items-center gap-xxs rounded-xs bg-muted px-xs text-caption outline-none hover:bg-accent focus-visible:ring-1 focus-visible:ring-ring" data-mission-turn-node={node.key}>
      <span className="min-w-0 truncate">{node.title}</span>
      {pull.number != null ? <MissionPrMark number={pull.number} state={pull.state} /> : null}
      {pull.more > 0 ? <span>+{pull.more}</span> : null}
    </button>
  );
}
