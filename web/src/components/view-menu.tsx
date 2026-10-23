// The sidebar footer's View menu (PRD agent-logo-view-mode D-01, D-06, B1-B3):
// whether list rows and tabs draw the agent's logo, with a line that names the
// key which shows them for as long as it is held, then how agent panes draw
// their turn row. The core owns both choices; each check always reads what the
// core confirmed.

import { EyeIcon } from "lucide-react";
import type { Actions } from "../actions";
import { AgentMark } from "../AgentMark";
import type { AgentLogoMode, PaneTurnRowMode } from "../generated/hided-ws";
import { usePaneTurnRowMode } from "../PaneHeaderBand";
import { useInterfaceTranslation } from "../i18n/client";
import { agentPeekKey } from "../shortcutLabels";
import { useShellStore } from "../store";
import { Button } from "./ui/button";
import { DropdownMenu, DropdownMenuCheckboxItem, DropdownMenuContent, DropdownMenuLabel, DropdownMenuSeparator, DropdownMenuShortcut, DropdownMenuTrigger } from "./ui/dropdown-menu";
import { Hint } from "./ui/tooltip";

const TURN_ROW_MODES = [
  ["one_line", "sidebar.view.turnRowOneLine"],
  ["two_lines", "sidebar.view.turnRowTwoLines"],
  ["off", "sidebar.view.turnRowOff"],
] as const satisfies readonly (readonly [PaneTurnRowMode, string])[];

export function ViewMenu({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const mode: AgentLogoMode = useShellStore((s) => s.rest?.ui_state?.agent_logo_mode) ?? "hidden";
  const connected = useShellStore((s) => s.connection === "live");
  const key = useShellStore((s) => agentPeekKey(s.rest?.ui_state));
  // The core's value is the only authority; choosing what is already chosen only closes the menu (B9).
  const choose = (next: AgentLogoMode) => {
    if (next !== mode) actions.setAgentLogoMode(next);
  };
  const turnRow = usePaneTurnRowMode();
  const chooseTurnRow = (next: PaneTurnRowMode) => {
    if (next !== turnRow) actions.setPaneTurnRowMode(next);
  };
  return (
    <DropdownMenu>
      <Hint label={t("sidebar.view")}>
        <DropdownMenuTrigger asChild>
          <Button variant="ghost" size="icon-sm" aria-label={t("sidebar.view")} data-view-menu-trigger="true">
            <EyeIcon />
          </Button>
        </DropdownMenuTrigger>
      </Hint>
      <DropdownMenuContent align="end" side="top" aria-label={t("sidebar.view")} data-view-menu="true">
        <DropdownMenuLabel className="text-caption font-normal text-muted-foreground">{t("sidebar.view.logos")}</DropdownMenuLabel>
        <DropdownMenuCheckboxItem checked={mode === "shown"} disabled={!connected} data-view-item="shown" onSelect={() => choose("shown")}>
          {t("sidebar.view.logosShown")}
          <DropdownMenuShortcut className="flex items-center gap-xs"><AgentMark kind="claude" /><AgentMark kind="codex" /></DropdownMenuShortcut>
        </DropdownMenuCheckboxItem>
        <DropdownMenuCheckboxItem checked={mode === "hidden"} disabled={!connected} data-view-item="hidden" onSelect={() => choose("hidden")}>
          {t("sidebar.view.logosHidden")}
        </DropdownMenuCheckboxItem>
        {key ? (
          <DropdownMenuLabel className="text-caption font-normal text-muted-foreground" data-view-peek={key}>
            {t("sidebar.view.logosPeek", { key })}
          </DropdownMenuLabel>
        ) : null}
        <DropdownMenuSeparator />
        <DropdownMenuLabel className="text-caption font-normal text-muted-foreground">{t("sidebar.view.turnRow")}</DropdownMenuLabel>
        {TURN_ROW_MODES.map(([value, label]) => (
          <DropdownMenuCheckboxItem key={value} checked={turnRow === value} disabled={!connected} data-view-turn-row={value} onSelect={() => chooseTurnRow(value)}>
            {t(label)}
            {value === "one_line" ? <DropdownMenuShortcut>{t("sidebar.view.turnRowDefault")}</DropdownMenuShortcut> : null}
          </DropdownMenuCheckboxItem>
        ))}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
