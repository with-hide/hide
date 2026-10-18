import { translate } from "./i18n/client";
import { adjacentInOrder, areaSentence, locateDisplay, moveLabel, neighbourArea, resizeTarget, splitEligibility, splitLabel, type AreaLayout, type AreaWords, type Edge, type Geometry, type LayoutSizes } from "./areaLayout";
import type { MenuEntry } from "./components/entry-menu";

export type AgentItem = { id: string };
export type AgentGroup = { id: string | null; name: string | null; tabs: string[] };
export type AgentStartReceipt = { request_id: string; group_id: string | null; area_id: string; draft_tab_id: string | null; live_tab_id: string | null; pane_id: string | null; add_pane: boolean; state: "pending" | "failed" | "ready"; agent_kind: string | null; agent_phase: string | null };
export type CheckoutAgentGroups = { selected: string | null; groups: AgentGroup[]; drafts: string[]; start_receipts?: AgentStartReceipt[] };
export type AgentStartTarget = { group_id: string | null; area_id: string; tab_id: string | null; add_pane: boolean };
export type AgentLayout = AreaLayout<AgentItem> & { waiting?: number; canvases: Record<string, string>; group_id?: string | null; groups?: AgentGroup[]; drafts?: string[]; can_undo_group?: boolean; start_receipts?: AgentStartReceipt[] };
export const AGENT_WORDS: AreaWords = { kind: "agent" };
export const remoteGroupReason = () => translate("panes.agent.remoteGroupReason");
const EDGES: Edge[] = ["right", "left", "up", "down"];
export type AgentCommand = "rename_tab" | "new_tab" | "copy_name" | "close_tab" | `split_${Edge}` | `move_${Edge}` | "focus_next" | "focus_previous" | "grow" | "shrink";
/** One drawn Agent area tree; `remote` when its tabs are another device's Herdr, which splits no group. */
export type AgentFrame = { workspace: { device_id: string; path: string }; remote: boolean; layout: AgentLayout; geometry: Geometry; sizes: LayoutSizes };
export function agentMenu(frame: AgentFrame, id: string): MenuEntry<AgentCommand>[] {
  const located = locateDisplay(frame.layout.root, id);
  if (!located) return [];
  const entries: MenuEntry<AgentCommand>[] = [{ id: "new_tab", icon: "add", label: translate("panes.area.newTab"), unavailable: null }, { id: "rename_tab", icon: "rename", label: translate("panes.agent.rename"), unavailable: null }];
  for (const edge of EDGES) {
    const eligibility = splitEligibility(frame.layout, frame.geometry, frame.sizes, id, located.area.id, edge, AGENT_WORDS);
    entries.push({ id: `split_${edge}`, icon: `split_${edge}`, group: "split", menuLabel: translate(`menus.${edge}`), separated: edge === "right", label: splitLabel(edge), unavailable: frame.remote ? remoteGroupReason() : eligibility.ok ? null : eligibility.reason });
  }
  for (const edge of EDGES) {
    const neighbour = neighbourArea(frame.layout.root, located.area.id, edge);
    if (neighbour) entries.push({ id: `move_${edge}`, icon: edge, group: "move", menuLabel: translate(`menus.${edge}`), label: moveLabel(edge), unavailable: null });
  }
  entries.push({ id: "copy_name", icon: "copy", label: translate("panes.agent.copyName"), unavailable: null, separated: true }, { id: "close_tab", icon: "close", label: translate("panes.agent.closeTab"), unavailable: null, separated: true });
  return entries;
}
export type AgentAreaStep = "focus_next" | "focus_previous" | "grow" | "shrink";

/** Why an Agent area focus or resize step cannot run now, or null when it can. */
export function agentAreaStepUnavailable(frame: AgentFrame, step: AgentAreaStep): string | null {
  if (step === "focus_next" || step === "focus_previous") return adjacentInOrder(frame.layout.root, frame.layout.active_area, step === "focus_next" ? 1 : -1) ? null : areaSentence(AGENT_WORDS, "onlyOne");
  const target = resizeTarget(frame.layout, frame.geometry, step === "grow", AGENT_WORDS);
  return "reason" in target ? target.reason : null;
}

/** Shared one-line capacity notice; authoritative tabs remain available upstream. */
export function agentCapacityNotice(waiting: number): string {
  return translate("panes.agent.capacity", { count: waiting });
}
