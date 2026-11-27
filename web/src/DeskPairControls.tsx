import { ChevronDownIcon, Columns2Icon, Maximize2Icon, Minimize2Icon } from "lucide-react";
import type { Actions } from "./actions";
import { AgentMark } from "./AgentMark";
import { Button } from "./components/ui/button";
import { DropdownMenu, DropdownMenuContent, DropdownMenuGroup, DropdownMenuItem, DropdownMenuLabel, DropdownMenuTrigger } from "./components/ui/dropdown-menu";
import { Hint } from "./components/ui/tooltip";
import { useInterfaceTranslation } from "./i18n/client";
import { allAgents } from "./paneAgents";
import { commandLabel } from "./shortcutLabels";
import type { ViewDisplaySnapshot } from "./snapshot";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";

// The desk's own controls beside its ✕: Expand, on the usual screen and in
// pair view alike, and the pair view controls (PRD lane-desk-pair-view B17,
// B24, D-08, D-41, D-42, D-50): ◧ beside the ✕, and in pair view the one
// menu the other agents' tabs fold into.

/** The desk's Expand, the rail `…` menu's Expand and ⇧⌘↩'s on the desk (B15, D-51, D-59) drawn where the operator looks at the page: pressed, it fills the body with the desk, and pressed again it returns. */
export function DeskExpandButton() {
  const expanded = useUiStore((s) => s.laneExpanded);
  const chord = useShellStore((s) => commandLabel("toggle_zoom", s.rest?.ui_state));
  return <ExpandButton expanded={expanded} chord={chord || null} onToggle={() => useUiStore.getState().setLaneExpanded(!expanded)} />;
}

/** Desk's own desk's `⤢` (PRD desk-own-desk D-16, B40): pressed, the desk covers Desk's chat whole, and pressed again it returns. */
export function DeskWideButton({ expanded, onExpand }: { expanded: boolean; onExpand: () => void }) {
  return <ExpandButton expanded={expanded} chord={null} onToggle={onExpand} />;
}

function ExpandButton({ expanded, chord, onToggle }: { expanded: boolean; chord: string | null; onToggle: () => void }) {
  const { t } = useInterfaceTranslation();
  const label = expanded ? t("panes.lane.collapse") : t("panes.lane.expand");
  return (
    <Hint label={label} shortcut={chord ?? undefined}>
      <Button
        variant="ghost"
        size="icon-sm"
        className="text-muted-foreground"
        aria-label={label}
        aria-pressed={expanded}
        data-desk-expand={expanded ? "on" : "off"}
        onClick={onToggle}
      >
        {expanded ? <Minimize2Icon /> : <Maximize2Icon />}
      </Button>
    </Hint>
  );
}

/** The desk's ◧, a way into pair view only (D-42), drawn as the pane head's ◧ so one mark means one thing (D-50): the tab in front beside its agent's pane. The desk draws none in pair view, whose ways out are the strip's, ⌘., the pressed ◧ and closing the desk. */
export function DeskPairButton({ displayId, actions }: { displayId: string | null; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const label = t("panes.menu.pair");
  return (
    <Hint label={label} shortcut={commandLabel("pair_view") || undefined}>
      <Button
        variant="ghost"
        size="icon-sm"
        className="text-muted-foreground"
        aria-label={label}
        disabled={displayId === null}
        data-desk-pair="off"
        onClick={() => displayId !== null && actions.pinPairView(displayId)}
      >
        <Columns2Icon />
      </Button>
    </Hint>
  );
}

/**
 * In focus mode the strip shows the left pane's tabs and what the operator
 * opened since the mode came on; every other agent's, and the operator's
 * from before, fold into this one menu at the strip's end, listed under
 * their agent or under the operator (D-60, narrowing D-41). The core names
 * them. Choosing one brings it in front, and the left pane follows it to an
 * agent of the same Agent tab (B24, D-09).
 */
export function PairOthers({ hidden, label, actions }: { hidden: ViewDisplaySnapshot[]; label: (display: ViewDisplaySnapshot) => string; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const agents = useShellStore((s) => allAgents(s.agents, s.rest?.status?.remote));
  if (hidden.length === 0) return null;
  const groups = new Map<string, ViewDisplaySnapshot[]>();
  for (const display of hidden) {
    const owner = display.owner_pane ?? "";
    groups.set(owner, [...(groups.get(owner) ?? []), display]);
  }
  return (
    <DropdownMenu>
      <Hint label={t("panes.pair.othersOf")}>
        <DropdownMenuTrigger asChild>
          <Button variant="ghost" size="sm" className="h-full shrink-0 gap-xxs rounded-none px-sm text-caption text-subtle-foreground" aria-label={`${t("panes.pair.othersOf")} · ${hidden.length}`} data-pair-others={hidden.length}>
            {t("panes.pair.others", { count: hidden.length })}
            <ChevronDownIcon aria-hidden="true" />
          </Button>
        </DropdownMenuTrigger>
      </Hint>
      <DropdownMenuContent align="end">
        {[...groups].map(([owner, displays]) => {
          const agent = agents.find((row) => row.pane_id === owner);
          return (
            <DropdownMenuGroup key={owner} data-pair-others-agent={owner}>
              <DropdownMenuLabel className="flex items-center gap-xs">
                <AgentMark kind={agent?.agent_kind} profile={agent?.profile} />
                <span className="truncate">{owner === "" ? t("panes.pair.yours") : (agent?.identity_label ?? t("panes.pair.terminal"))}</span>
              </DropdownMenuLabel>
              {displays.map((display) => (
                <DropdownMenuItem key={display.id} onSelect={() => actions.focusView(display.id)} data-pair-others-item={display.id}>
                  <span className="truncate">{label(display)}</span>
                </DropdownMenuItem>
              ))}
            </DropdownMenuGroup>
          );
        })}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
