// A mission node's review gate as the map card, the phone's step and both
// plan lists draw it (PRD mission-human-gate D-06, D-08, B1, B2, B13), so the
// desktop and the phone show one shape and one wording.

import { CheckIcon, EyeIcon, UserRoundIcon } from "lucide-react";
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
        stripTone(gate.state),
        className,
      )}
      data-node-gate={gate.state}
    >
      {gate.state === "passed" ? <CheckIcon aria-hidden="true" className="mt-xxs size-(--size-icon-sm) shrink-0" /> : <EyeIcon aria-hidden="true" className="mt-xxs size-(--size-icon-sm) shrink-0 text-mission-hold" />}
      <span className="line-clamp-2 min-w-0 break-words">{gate.look}</span>
    </span>
  );
}

/** A strip's tint by where its human step stands: faint ahead, filled once it is the operator's turn, grey with a check after. */
function stripTone(state: "ahead" | "open" | "passed"): string {
  return state === "open" ? "bg-mission-hold/20 font-semibold text-foreground" : state === "ahead" ? "bg-mission-hold/10 text-foreground" : "text-muted-foreground";
}

/**
 * A merge the core says is the operator's now (PRD mission-destination D-53,
 * B56), in the gate strip's shape and its turn's tint: `머지할 차례`. The
 * number and branch are the card's branch line; a merge ahead or done is the
 * person on that line, never a strip.
 */
export function MergeTurnStrip({ className }: { className?: string }) {
  const { t } = useInterfaceTranslation();
  return (
    <span className={cn("flex min-w-0 items-start gap-xs rounded-xs px-xs py-xxs text-caption", stripTone("open"), className)} data-node-merge-turn="true">
      <UserRoundIcon aria-hidden="true" className="mt-xxs size-(--size-icon-sm) shrink-0 text-mission-hold" />
      <span className="min-w-0 truncate">{t("mission.merge.turn")}</span>
    </span>
  );
}

/**
 * The operator's band right under a card's title (D-51, D-53): the gate's
 * strip as it always was, the merge's turn in the same shape, or both on
 * one line joined by `·`, each part in its own state's tint.
 */
export function OperatorBand({ gate, mergeDue, className }: { gate: Pick<MissionGate, "state" | "look"> | null; mergeDue: boolean; className?: string }) {
  if (gate && !mergeDue) return <GateStrip gate={gate} className={className} />;
  if (!gate) return mergeDue ? <MergeTurnStrip className={className} /> : null;
  return (
    <span className={cn("flex min-w-0 items-start gap-xxs", className)} data-node-band="true">
      <GateStrip gate={gate} className="min-w-0 flex-1" />
      <span aria-hidden="true" className="py-xxs text-caption text-muted-foreground">·</span>
      <MergeTurnStrip className="shrink-0" />
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
 * word button that picks it on the map. A place's caption line leaves more
 * room under its words than its bold line leaves over them, so a row pads
 * less below than above and its divider sits midway between the two places.
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
          <li key={node.key} className="flex min-w-0 flex-col items-start gap-xxs pt-xs pb-xxs text-body first:pt-none last:pb-none" data-mission-gate-row={node.key}>
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
