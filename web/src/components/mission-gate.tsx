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
 * (D-08, B1): `내가 볼 곳 N` with when, said once since it is the same for
 * every place (once the node's other finish lines all pass, D-10), then one
 * place each: what to look at on its first line and its node, quiet and
 * whole, under it. It wears no hold colour, since none of it is the
 * operator's turn (B9); the operator turns a place on or off by saying so to
 * the Conductor, whose replan redraws the list. With `onSelect` the node is a
 * word button that picks it on the map.
 */
export function GatePlanList({ nodes, onSelect, className }: { nodes: readonly { key: string; title: string; gate?: Pick<MissionGate, "look"> | null }[]; onSelect?: (key: string) => void; className?: string }) {
  const { t } = useInterfaceTranslation();
  if (nodes.length === 0) return null;
  const title = t("mission.gate.planTitle", { count: nodes.length });
  return (
    <section aria-label={title} className={cn("flex min-w-0 flex-col gap-xs", className)} data-mission-gates={nodes.length}>
      <h3 className="flex min-w-0 items-baseline gap-xs text-caption">
        <EyeIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0 self-center text-muted-foreground" />
        <span className="shrink-0 font-semibold">{title}</span>
        <span aria-hidden="true" className="shrink-0 text-muted-foreground">·</span>
        <span className="min-w-0 break-keep text-muted-foreground wrap-anywhere" data-mission-gate-when="true">{t("mission.gate.planWhen")}</span>
      </h3>
      <ul className="flex min-w-0 flex-col divide-y divide-border pl-[calc(var(--size-icon-sm)+var(--spacing-xs))]">
        {nodes.map((node) => (
          <li key={node.key} className="flex min-w-0 flex-col items-start gap-xxs py-xs text-body first:pt-none last:pb-none" data-mission-gate-row={node.key}>
            <span className="min-w-0 break-keep font-semibold wrap-anywhere" data-mission-gate-look="true">{node.gate?.look}</span>
            {onSelect ? (
              <Button variant="text-link" size="inline" onClick={() => onSelect(node.key)} className="min-w-0 justify-start whitespace-normal text-left text-caption break-keep wrap-anywhere" data-mission-gate-node={node.key}>
                {node.title}
              </Button>
            ) : (
              <span className="min-w-0 break-keep text-caption text-muted-foreground wrap-anywhere" data-mission-gate-node={node.key}>{node.title}</span>
            )}
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
