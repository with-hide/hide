import { movingTab } from "./paneMove";
import { PaneMoveProvider, PaneMovePreview, PaneMovePlus } from "./PaneMoveControls";
import { useMemo, useState } from "react";
import type { Actions } from "./actions";
import { AreaEmpty } from "./AreaEmpty";
import { createAreaTree, type AreaAdapter } from "./AreaTree";
import { AGENT_WORDS, agentMenu, remoteGroupReason, type AgentCommand, type AgentItem, type AgentLayout } from "./agentLayout";
import { noteAreaFrame } from "./areaFrames";
import { Button } from "./components/ui/button";
import { FindBar } from "./Overlays";
import { PaneCanvas } from "./PaneGrid";
import { RelationStatus } from "./PaneRelations";
import { commandLabel } from "./shortcutLabels";
import { localDeviceId, type Checkout } from "./snapshot";
import { useShellStore } from "./store";
import { AgentDraftTab, AgentTab } from "./TabBar";
import { AgentGroupSelector } from "./AgentGroupControls";
import { AgentTabStart } from "./AgentTabStart";
import { inlineStartKey, useInlineStarts } from "./startDraft";
import { receiptTarget } from "./inlineStart";
import { useGroupName } from "./groupTabs";
import type { MenuEntry } from "./components/entry-menu";
import { agentEntries, workspaceViewOf } from "./workspace";
import { areasOf } from "./areaLayout";
import { useKeyboardOwner } from "./viewFocus";
import { numberOf, numberedTabs } from "./numbering";
import { useUiStore } from "./ui";
import { useInterfaceTranslation } from "./i18n/client";

const SharedAgentTree = createAreaTree<AgentItem>("agent");
/** `remoteBody` draws a device tab's panes: the tab an area shows, or `null` for the one its Herdr has in front. */
export function AgentAreas({ checkout, actions, deviceId, remoteBody }: { checkout: Checkout; actions: Actions; deviceId?: string; remoteBody?: (tabId: string | null) => React.ReactNode }) {
  const { t } = useInterfaceTranslation();
  const [renaming, setRenaming] = useState<string | null>(null);
  const startDrafts = useInlineStarts((state) => state.drafts);
  const saved = useShellStore((s) => workspaceViewOf(s.rest)?.agent_layout);
  const owner = useKeyboardOwner();
  // A ⌘ hold numbers the tabs ⌘n selects, the Agent area in use only (`numbering.ts`).
  const numberedHold = useUiStore((s) => s.hint === "tabs");
  const entries = agentEntries(checkout);
  const paneLayouts = useShellStore((s) => s.rest?.pane_layouts);
  const operations = useShellStore(s => s.rest?.status?.async_operations);
  const remoteSessions = useShellStore((s) => s.rest?.status?.remote);
  const node = useShellStore((s) => localDeviceId(s.rest));
  const device = deviceId ?? node;
  const remote = device !== node;
  const remoteLayout = useMemo<AgentLayout>(() => ({
    root: { area: { id: "a1", active: checkout.active_tab_id, displays: agentEntries(checkout).map((entry) => ({ id: entry.source_id })) } },
    active_area: "a1", canvases: {}, limits: { areas: 1, depth: 0, displays: 64 }, display_count: agentEntries(checkout).length,
  }), [checkout]);
  // A device this core dials shows the one tab its Herdr has in front; a
  // node that dialed in arranges its tabs as this machine does, and the core
  // publishes its Agent areas only then.
  const single = remote && !saved;
  const layout = single ? remoteLayout : saved;
  if (!layout) return <AreaEmpty state="agent-layout-missing" text={t("panes.agent.waiting")} />;
  const visiblePaneIds = areasOf(layout.root).flatMap(area => {
    const tabId = layout.canvases[area.id] ?? area.active;
    const geometry = remote
      ? remoteSessions?.find(row => row.target_id === device)?.session?.pane_layouts.find(row => row.tab_id === tabId)
      : paneLayouts?.find(row => row.tab_id === tabId);
    if (!geometry) return [];
    if (geometry.zoomed) return [geometry.focused_pane_id];
    return checkout.tabs.find(tab => tab.id === tabId)?.panes.map(pane => pane.id) ?? [];
  });
  const workspace = { device_id: device, path: checkout.path };
  const tabNumbers = numberedHold ? numberedTabs(layout) : null;
  const tabNumber = (id: string) => tabNumbers ? numberOf(tabNumbers, id) : null;
  const isDraft = (id: string) => layout.drafts?.includes(id) ?? false;
  const groupName = layout.groups?.find((group) => group.id === (layout.group_id ?? null))?.name ?? t("groups.ungrouped");
  const label = (id: string) => isDraft(id) ? t("groups.newTab") : entries.find((entry) => entry.source_id === id)?.label ?? id;
  const startForm = (tabId: string | null, areaId: string, addPane = false, recovery?: import("./agentLayout").AgentStartTarget) => <AgentTabStart key={`${layout.group_id}\0${tabId}\0${areaId}\0${addPane}`} workspace={workspace} target={recovery ?? { group_id: layout.group_id ?? null, area_id: areaId, tab_id: tabId, add_pane: addPane }} groupName={groupName} actions={actions} />;
  const adapter: AreaAdapter<AgentItem> = {
    words: AGENT_WORDS,
    keyboardArea: owner.kind !== "none" && owner.workspace === checkout.id
      ? owner.kind === "agent" ? owner.areaId ?? null
        : owner.kind === "pane" ? areasOf(layout.root).find((area) => {
          const shown = layout.canvases[area.id] ?? area.active;
          return area.displays.some((tab) => tab.id === shown) && checkout.tabs.find((tab) => tab.id === shown)?.panes.some((pane) => pane.id === owner.paneId);
        })?.id ?? null : null
      : null,
    barAttributes: { "data-tab-bar": checkout.id, ...(remote ? { "data-remote-tab-bar": "true" } : {}) },
    barStart: layout.groups ? () => <AgentGroupSelector layout={layout} workspace={workspace} actions={actions} /> : undefined,
    splitUnavailable: single ? remoteGroupReason() : undefined,
    label: (item) => label(item.id),
    sameContent: (a, b) => a.id === b.id,
    shown: (area) => {
      const id = layout.canvases[area.id] ?? area.active;
      return id ? { id } : null;
    },
    tab: (item, interaction) => {
      if (isDraft(item.id)) return <AgentDraftTab id={item.id} interaction={interaction} actions={actions} number={tabNumber(item.id)} />;
      const entry = entries.find((row) => row.source_id === item.id);
      return entry ? <AgentTab entry={entry} checkout={checkout} renaming={renaming === item.id} onCancelRename={() => setRenaming(null)} interaction={interaction} actions={actions} number={tabNumber(item.id)} /> : null;
    },
    body: (item, area) => isDraft(item.id) ? startForm(item.id, area.id) : <div className="relative flex min-h-0 flex-1 flex-col">
      {(() => {
        const receipt = layout.start_receipts?.find((row) => row.live_tab_id === item.id && startDrafts[inlineStartKey(device, checkout.path, receiptTarget(row))]?.request?.id === row.request_id);
        return receipt ? <div className="absolute inset-0 z-20 flex bg-background">{startForm(item.id, area.id, receipt.add_pane, receiptTarget(receipt))}</div> : null;
      })()}
      {area.id === layout.active_area ? <><RelationStatus actions={actions} visiblePaneIds={visiblePaneIds} /><FindBar actions={actions} /></> : null}
      <PaneMovePreview areaId={area.id} />
      {remote ? remoteBody?.(single ? null : item.id) : <PaneCanvas key={item.id} tab={checkout.tabs.find((tab) => tab.id === item.id) ?? null} actions={actions} />}
    </div>,
    empty: (area) => layout.groups ? startForm(null, area.id) : <AreaEmpty state="no-agent-tab" text={t("panes.agent.noTab")}><Button variant="secondary" onClick={() => actions.createTab(area.id)} data-empty-new-tab="true">{t("panes.area.newTab")}</Button></AreaEmpty>,
    floating: (item) => <span className="truncate">{label(item.id)}</span>,
    menu: (id, geometry, sizes) => {
      const items: MenuEntry[] = agentMenu({ workspace, remote: single, layout, geometry, sizes }, id).filter((item) => !isDraft(id) || (item.id !== "rename_tab" && item.id !== "copy_name"));
      if (layout.groups) {
        items.splice(1, 0, { id: "group:new", label: t("groups.newWithTab"), unavailable: null }, { id: "group:move", label: t("groups.move"), unavailable: null, choices: layout.groups.map((group) => ({ id: `group:to:${group.id ?? ""}`, label: group.name ?? t("groups.ungrouped"), checked: group.id === (layout.group_id ?? null) })) });
        if (layout.group_id) items.splice(3, 0, { id: "group:remove", label: t("groups.remove"), unavailable: null });
      }
      if (movingTab(operations, id)) for (const item of items) if (item.id !== "copy_name") item.unavailable = t("menus.paneMove.busy");
      return items;
    },
    runMenu: (command, id) => {
      if (command === "group:new") return useGroupName.getState().open({ workspace, groupId: null, name: "", tabId: id });
      if (command === "group:remove" || command.startsWith("group:to:")) return void actions.agentGroup(workspace, command === "group:remove" ? null : command.slice(9) || null, { action: "move", tab_id: id });
      if (command === "rename_tab") setRenaming(id);
      else actions.runAgentCommand(command as AgentCommand, id);
    },
    onMenuCloseAutoFocus: (event) => { if (renaming) event.preventDefault(); },
    focus: (id) => single ? actions.focusTab(id) : actions.agentLayout({ action: "focus", tab_id: id }),
    focusArea: (id) => { if (!single) actions.agentLayout({ action: "focus_area", area_id: id }); },
    move: (id, areaId, index) => {
      if (single) {
        const entry = entries.find((row) => row.source_id === id);
        const other = entries.filter((row) => row.source_id !== id);
        const before = other[index];
        const strip = checkout.strip.filter((row) => row.source_id !== id);
        const slot = before ? strip.findIndex((row) => row.id === before.id) : strip.length;
        if (entry) actions.reorderTab(entry.id, slot);
      } else actions.agentLayout({ action: "move", tab_id: id, area_id: areaId, index });
    },
    split: (id, areaId, edge) => actions.agentLayout({ action: "split", tab_id: id, area_id: areaId, edge, request_id: crypto.randomUUID() }),
    resize: (id, ratio) => actions.agentLayout({ action: "resize", split_id: id, ratio }),
    newTab: (areaId) => actions.createTab(areaId),
    newTabContent: <PaneMovePlus />,
    newTabLabel: layout.groups ? t("groups.newTab") : t("panes.agent.newTabNamed", { label: checkout.next_tab_label }),
    tabListLabel: t("panes.agent.tabList"), actionsLabel: t("panes.agent.tabActions"),
    newTabShortcut: commandLabel("new_tab"),
    onDraw: (frame) => noteAreaFrame("agent", frame ? { ...frame, layout, workspace, remote: single } : null),
  };
  return <PaneMoveProvider key={`${device}\0${checkout.path}\0${layout.group_id ?? ""}`} checkout={checkout} layout={layout} deviceId={device} actions={actions}><SharedAgentTree layout={layout} adapter={adapter} /></PaneMoveProvider>;
}
