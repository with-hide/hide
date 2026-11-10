// A mission node's review gate as the map card, the phone's step and both
// plan lists draw it (PRD mission-human-gate D-06, D-08, B1, B2, B13), so the
// desktop and the phone show one shape and one wording.

import { CheckIcon, EyeIcon } from "lucide-react";
import { useInterfaceTranslation } from "../i18n/translator";
import { cn } from "../lib/utils";
import type { MissionGate } from "../snapshot";
import { Button } from "./ui/button";

/**
 * What the operator will look at, as a strip of the card itself, up to two
 * lines and read without a hover: tinted ahead, filled in 내 차례's colour
 * once reached, grey with a check once looked at.
 */
export function GateStrip({ gate, className }: { gate: Pick<MissionGate, "state" | "look">; className?: string }) {
  return (
    <span
      className={cn(
        "flex min-w-0 items-start gap-xs rounded-xs px-xs py-xxs text-caption",
        gate.state === "open" ? "bg-mission-hold/20 font-semibold text-foreground" : gate.state === "ahead" ? "bg-mission-hold/10 text-foreground" : "text-muted-foreground",
        className,
      )}
      data-node-gate={gate.state}
    >
      {gate.state === "passed" ? <CheckIcon aria-hidden="true" className="mt-xxs size-(--size-icon-sm) shrink-0" /> : <EyeIcon aria-hidden="true" className="mt-xxs size-(--size-icon-sm) shrink-0 text-mission-hold" />}
      <span className="line-clamp-2 min-w-0 break-words">{gate.look}</span>
    </span>
  );
}

/**
 * Where the operator will look, while the plan waits for their approval
 * (D-08, B1): `내가 볼 곳 N`, then one row each with the node, what to look at
 * and when, which is once the node's work is done and before it finishes
 * (D-10). The operator turns a place on or off by saying so to the Conductor,
 * whose replan redraws the list; it is never the operator's turn (B9). With
 * `onSelect` the node is a button that picks it on the map.
 */
export function GatePlanList({ nodes, onSelect, className }: { nodes: readonly { key: string; title: string; gate?: Pick<MissionGate, "look"> | null }[]; onSelect?: (key: string) => void; className?: string }) {
  const { t } = useInterfaceTranslation();
  if (nodes.length === 0) return null;
  const title = t("mission.gate.planTitle", { count: nodes.length });
  return (
    <section aria-label={title} className={cn("flex min-w-0 flex-col gap-xs", className)} data-mission-gates={nodes.length}>
      <h3 className="flex min-w-0 items-center gap-xs text-caption font-semibold text-mission-hold">
        <EyeIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0" />
        {title}
      </h3>
      <ul className="flex min-w-0 flex-col gap-xxs">
        {nodes.map((node) => (
          <li key={node.key} className="flex min-w-0 flex-wrap items-center gap-x-xs text-body" data-mission-gate-row={node.key}>
            {onSelect ? (
              <Button variant="secondary" size="sm" onClick={() => onSelect(node.key)} title={node.title} className="max-w-(--size-flow-title) shrink-0 justify-start" data-mission-gate-node={node.key}>
                <span className="min-w-0 truncate">{node.title}</span>
              </Button>
            ) : (
              <span className="max-w-(--size-flow-title) shrink-0 truncate rounded-xs bg-secondary px-xs text-caption" data-mission-gate-node={node.key}>{node.title}</span>
            )}
            <span className="min-w-0 flex-1 truncate font-semibold" title={node.gate?.look}>{node.gate?.look}</span>
            <span className="shrink-0 truncate text-caption text-muted-foreground" data-mission-gate-when="true">{t("mission.gate.when")}</span>
          </li>
        ))}
      </ul>
    </section>
  );
}

/**
 * How many places the plan still has the operator look, a quiet grey eye and
 * number beside a mission row's progress (D-18, B18), named `내가 볼 곳 N`;
 * never in 내 차례's colour or count. None when none is ahead.
 */
export function GateAheadCount({ count, className }: { count: number; className?: string }) {
  const { t } = useInterfaceTranslation();
  if (count === 0) return null;
  return (
    <span className={cn("flex shrink-0 items-center gap-xxs text-caption text-muted-foreground", className)} role="img" aria-label={t("mission.gate.planTitle", { count })} data-mission-row-gates={count}>
      <EyeIcon aria-hidden="true" className="size-(--size-icon-sm)" />
      <span className="font-mono tabular-nums">{count}</span>
    </span>
  );
}
