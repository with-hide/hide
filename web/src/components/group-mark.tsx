import { BriefcaseBusinessIcon } from "lucide-react";
import type { AgentGroup } from "../agentLayout";
import type { MenuSymbol } from "./menu-symbols";
import { cn } from "../lib/utils";
import { DeskMark } from "./desk/desk-entry";

/**
 * A tab group's mark wherever the group is named (the tab bar's group
 * selector and its list, the sidebar's group line, a tab's Move to group
 * list, the last two through `groupSymbol`): the briefcase for the operator's own
 * group, and the group's surface's own mark for a reserved one, Desk's
 * diamond for the group Desk files its sessions in.
 */
export function GroupMark({ group, className }: { group: Pick<AgentGroup, "role"> | null | undefined; className?: string }) {
  if (group?.role === "desk") return <span className={cn("flex shrink-0", className)} data-group-mark="desk"><DeskMark /></span>;
  return <BriefcaseBusinessIcon className={cn("size-(--size-icon-sm) shrink-0", className)} />;
}

/** The same mark as a menu entry draws it, for the group lists in menus. */
export function groupSymbol(group: Pick<AgentGroup, "role">): MenuSymbol {
  return group.role === "desk" ? "desk" : "group";
}
