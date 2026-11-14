import { useInterfaceTranslation } from "../i18n/client";
import { cn } from "../lib/utils";
import type { ToolWork } from "../snapshot";
import { Badge } from "./ui/badge";
import { Hint } from "./ui/tooltip";

/**
 * The external tool whose work this is (PRD hide-external-control B31, D-15): its name, cut short, after the
 * title of the agent's pane head, its 내 차례 row and ⌘K result, and of the mission its agent conducts, on the
 * mission's row and map head. It stays once the tool let go, as where the work came from; the hint says which.
 */
export function ToolMark({ tool, className }: { tool: ToolWork | undefined; className?: string }) {
  const { t } = useInterfaceTranslation();
  if (!tool) return null;
  const label = t(tool.owns ? "common.tool.kept" : "common.tool.started", { name: tool.name });
  return (
    <Hint label={label}>
      <Badge variant="outline" className={cn("min-w-0 shrink", className)} aria-label={label} data-tool-mark={tool.connection} data-tool-mark-owns={tool.owns ? "" : undefined}>
        <span className="min-w-0 truncate">{tool.name}</span>
      </Badge>
    </Hint>
  );
}
