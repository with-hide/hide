import { ChevronDownIcon, PinIcon } from "lucide-react";
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

// The desk's own pair view controls (PRD lane-desk-pair-view B17, B24, D-08,
// D-41, D-42): the pin beside the desk's ✕, and in pair view the one menu
// the other agents' tabs fold into.

/** The desk's pin, a way into pair view only (D-42): the tab in front beside its agent's pane. The desk draws none in pair view, whose ways out are the strip's, ⌘., the pressed ◧ and closing the desk. */
export function DeskPin({ displayId, actions }: { displayId: string | null; actions: Actions }) {
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
        data-desk-pin="off"
        onClick={() => displayId !== null && actions.pinPairView(displayId)}
      >
        <PinIcon />
      </Button>
    </Hint>
  );
}

/**
 * In pair view the strip shows the left agent's tabs and the operator's own
 * (D-41); every other agent's fold into this one menu at the strip's end,
 * listed under their agent. Choosing one brings it in front, and the left
 * pane follows it to its agent (B24, D-09).
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
                <span className="truncate">{agent?.identity_label ?? owner}</span>
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
