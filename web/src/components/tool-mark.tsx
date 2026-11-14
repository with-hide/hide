import { createContext, useContext } from "react";
import { useInterfaceTranslation } from "../i18n/client";
import { cn } from "../lib/utils";
import type { ToolWork } from "../snapshot";
import { Badge } from "./ui/badge";
import { Hint } from "./ui/tooltip";

/**
 * Whether the view draws an external tool's cell (PRD hide-external-control B40, D-45): every row in it is that
 * tool's work, so neither the badge nor the label names the tool there, and the tool's turn is not the operator's,
 * so nothing in it takes the decision colour.
 */
export const InToolCell = createContext(false);

/**
 * The external tool whose work this is (PRD hide-external-control B31, D-15): its name, cut short, after the
 * title of the agent's pane head and its ⌘K result, and of the mission its agent conducts, on the mission's row
 * and map head. It stays once the tool let go, as where the work came from; the hint says which.
 */
export function ToolMark({ tool, className }: { tool: ToolWork | undefined; className?: string }) {
  const { t } = useInterfaceTranslation();
  const inCell = useContext(InToolCell);
  if (!tool || inCell) return null;
  const label = t(tool.owns ? "common.tool.kept" : "common.tool.started", { name: tool.name });
  return (
    <Hint label={label}>
      <Badge variant="outline" className={cn("min-w-0 shrink", className)} aria-label={label} data-tool-mark={tool.connection} data-tool-mark-owns={tool.owns ? "" : undefined}>
        <span className="min-w-0 truncate">{tool.name}</span>
      </Badge>
    </Hint>
  );
}

/**
 * The same name where a sidebar row's first line has no room for the badge (the 태스크 list's and the projects
 * list's rows, operator decision 2026-11-14): muted words leading the row's second line, `nightly · …`.
 */
export function ToolLabel({ tool, className }: { tool: ToolWork | undefined; className?: string }) {
  const { t } = useInterfaceTranslation();
  const inCell = useContext(InToolCell);
  if (!tool || inCell) return null;
  const label = t(tool.owns ? "common.tool.kept" : "common.tool.started", { name: tool.name });
  return (
    <span className={cn("flex min-w-0 max-w-2/5 shrink-0 items-baseline gap-xs text-muted-foreground", className)} title={label}>
      <span className="min-w-0 truncate" data-tool-mark={tool.connection} data-tool-mark-owns={tool.owns ? "" : undefined}>{tool.name}</span>
      <span aria-hidden="true">·</span>
    </span>
  );
}
