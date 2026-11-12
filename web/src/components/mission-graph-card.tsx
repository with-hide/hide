import type { ComponentProps, ReactNode } from "react";
import { useInterfaceTranslation } from "../i18n/translator";
import { cn } from "../lib/utils";
import type { NodeLook } from "../missions";
import type { Mission, MissionNode } from "../snapshot";
import { Button } from "./ui/button";
import { MissionNodeMark } from "./mission-node-mark";

/** A card's border and surface by its look (B26): finished work a faint green surface inside its plain border, never a green border, in main or not. */
const LOOK_CARD: Record<NodeLook, string> = {
  waiting: "border-dashed border-mission-waiting bg-transparent",
  working: "border-mission-working ring-2 ring-mission-working/20",
  done: "border-border bg-mission-done/10",
  external: "border-dotted border-mission-waiting bg-transparent",
  cancelled: "border-dashed border-mission-cancelled bg-transparent",
  dropped: "border-dashed border-border bg-transparent",
};

/** A card waiting on the operator wears 내 차례's card colour as a thick border and ring over its look, readable at the map's smallest zoom (PRD mission-node-simple B2). */
const ATTENTION_CARD = "border-2 border-mission-hold ring-4 ring-mission-hold/20";

/** The same card frame, state, title and touch target on the desktop and phone; its rows stand one spacing token apart, carry no gaps of their own and all start at the card's left edge (PRD mission-destination D-51). */
export function MissionGraphCard({ node, look, word, yours, selected = false, dimmed = false, still, failed, title, accessory, children, ...props }: {
  node: Pick<MissionNode, "key" | "title" | "kind" | "status" | "blocked">;
  look: NodeLook;
  word: string;
  yours: boolean;
  selected?: boolean;
  dimmed?: boolean;
  still?: boolean;
  /** A done node that ended on a failed verdict: its mark is red (PRD node-presets D-08). */
  failed?: boolean;
  title?: ReactNode;
  accessory?: ReactNode;
  children?: ReactNode;
} & Omit<ComponentProps<typeof Button>, "children" | "title" | "node">) {
  return (
    <Button
      variant="ghost" size="inline"
      aria-pressed={selected}
      data-mission-node={node.key} data-node-status={node.status} data-node-look={look}
      data-node-blocked={node.blocked ?? undefined} data-node-dimmed={dimmed || undefined}
      className={cn(
        "relative flex h-auto w-(--size-mission-node) min-w-0 min-h-(--size-touch-target) flex-col items-stretch gap-xs rounded-md border bg-card px-sm py-xs text-left font-normal text-foreground hover:text-foreground whitespace-normal outline-none transition-opacity hover:bg-accent focus-visible:ring-1 focus-visible:ring-ring",
        yours ? ATTENTION_CARD : LOOK_CARD[look], node.kind === "external" && "border-dotted",
        selected && "outline-2 outline-offset-2 outline-primary", dimmed && "opacity-(--opacity-dimmed)",
      )}
      {...props}
    >
      {yours ? <span aria-hidden="true" className="absolute -top-xs -right-xs size-(--size-agent-mark) rounded-full border-2 border-card bg-mission-hold" data-node-turn="true" /> : null}
      <span className="flex min-w-0 items-center gap-xs" data-node-head="true">
        <MissionNodeMark look={look} label={word} still={still} failed={failed} />
        {title ?? <span className={cn("min-w-0 flex-1 truncate text-body font-semibold", (look === "external" || look === "cancelled" || look === "waiting") && "text-muted-foreground", look === "dropped" && "text-muted-foreground line-through")}>{node.title}</span>}
        {accessory}
      </span>
      {children}
    </Button>
  );
}

export function MissionGraphReport({ status, note, look, dimmed = false }: { status: Mission["status"]; note: string; look: NodeLook; dimmed?: boolean }) {
  const { t } = useInterfaceTranslation();
  return (
    <div data-mission-report={status} className={cn("flex w-(--size-mission-node) items-center justify-center gap-xs rounded-xl border px-md py-sm transition-opacity", LOOK_CARD[look], dimmed && "opacity-(--opacity-dimmed)")}>
      <MissionNodeMark look={look} label={note} />
      <span className="text-body font-semibold">{t("mission.report")}</span>
      <span className="text-caption text-muted-foreground">{note}</span>
    </div>
  );
}
