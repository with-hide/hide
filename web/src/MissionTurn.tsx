import { useMemo, type ReactNode } from "react";
import type { Actions } from "./actions";
import { MissionNodeMark } from "./components/mission-node-mark";
import { MissionPrMark } from "./components/mission-pr";
import { StatusMark } from "./components/status-mark";
import { Button } from "./components/ui/button";
import { useInterfaceTranslation } from "./i18n/client";
import { DecisionBanner } from "./MissionDecision";
import { turnItems, type TurnItem } from "./missions";
import { allAgents, agentsByPane } from "./paneAgents";
import type { Mission, MissionNode } from "./snapshot";
import { useShellStore } from "./store";

// What only the operator can do in a mission now, above its map (PRD
// mission-map-git-criteria D-03, D-18, B18, B20, B25): the Conductor's
// decision, answered here as it always was, a worker the core raised past its
// Conductor, an outside pull request closed without its merge, the order to
// merge or join what waits for main, and a mission no Conductor holds. Work
// an agent carries is not here, and with nothing to do the section is gone.

export function MissionTurn({ mission, device, actions, onSelect }: { mission: Mission; device: string | null; actions: Actions; onSelect: (key: string) => void }) {
  const { t } = useInterfaceTranslation();
  const local = useShellStore((s) => s.agents);
  const remote = useShellStore((s) => s.rest?.status?.remote);
  const rows = useMemo(() => agentsByPane(allAgents(local, remote)), [local, remote]);
  // A mission no Conductor holds says so on its Conductor row, with 이어받기 there (PRD mission-tool B13).
  const items = turnItems(mission, (node) => (node.pane_id ? (rows.get(node.pane_id) ?? null) : null)).filter((item) => item.kind !== "takeover");
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
    case "outside_closed":
      return (
        <Row data-mission-turn-item="outside_closed">
          <MissionNodeMark look="failed" label={t("mission.look.failed")} />
          <NodeChip node={item.node} onSelect={onSelect} />
          <span className="min-w-0 flex-1 truncate text-body">{t("mission.outside.closed")}</span>
          {conversation}
        </Row>
      );
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
function NodeChip({ node, onSelect }: { node: MissionNode; onSelect: (key: string) => void }) {
  const pr = node.pr ?? (node.external && "pr" in node.external ? node.external.pr : null);
  return (
    <button type="button" onClick={() => onSelect(node.key)} title={node.title} className="flex min-w-0 max-w-(--size-flow-title) shrink items-center gap-xxs rounded-xs bg-muted px-xs text-caption outline-none hover:bg-accent focus-visible:ring-1 focus-visible:ring-ring" data-mission-turn-node={node.key}>
      <span className="min-w-0 truncate">{node.title}</span>
      {pr != null ? <MissionPrMark number={pr} state={node.pr_state} /> : null}
    </button>
  );
}
