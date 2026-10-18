import { useState } from "react";
import type { Actions } from "./actions";
import { Button } from "./components/ui/button";
import { Hint } from "./components/ui/tooltip";
import { EntryPointMenu, type MenuEntry } from "./components/entry-menu";
import { relationEntries, relationState } from "./lineage";
import { herdrPaneId } from "./remote";
import { type PaneRow } from "./snapshot";
import { useShellStore } from "./store";
import { copySelection, pasteClipboard, selectAllText } from "./terminals";
import { useUiStore } from "./ui";
import { translate, useInterfaceTranslation } from "./i18n/client";
import { splitLabel } from "./areaLayout";
import { isHtmlFile } from "./browserViews";
import { revealHost } from "./host";
import type { TerminalContextLink } from "./terminalLinkProvider";
import type { RevealHost } from "./revealExternal";
import { archiveOffer, type ArchiveOffer } from "./archive";
import { agentsByPane, allAgents } from "./paneAgents";

// The delegation tree where the operator works (PRD S6 D-07, B14-B16): a
// child pane's header returns to its parent, a parent's badge lists every
// direct child in a popover, and the pane menu lists parent and
// siblings, each moved to only by its explicit Open. Every move
// is one tracked focus; its pending and failed states show in the Agent area,
// which stays on screen while the core moves the visible tab to the target,
// and a failure never splits the parent or makes a pane.

/**
 * A child pane's ancestors left of its title, root first (PRD
 * agent-hierarchy-screens D-14, B20): each name opens that pane. A narrow
 * pane drops the names and keeps each arrow, whose accessible name and
 * tooltip still say where it goes. A root pane has none.
 */
export function AncestorPath({ pane, actions }: { pane: PaneRow; actions: Actions }) {
  const ancestors = (pane.lineage_path ?? []).slice(0, -1);
  if (ancestors.length === 0) return null;
  return (
    <nav className="flex min-w-0 shrink items-center" data-pane-path={pane.id}>
      {ancestors.map((step) => <AncestorStep key={step.pane_id} pane={pane} step={step} actions={actions} />)}
    </nav>
  );
}

function AncestorStep({ pane, step, actions }: { pane: PaneRow; step: NonNullable<PaneRow["lineage_path"]>[number]; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const progress = useRelationProgress(pane.id, step.pane_id);
  const pending = progress?.phase === "pending";
  const failed = progress?.phase === "failed";
  const label = t("panes.relation.return", { name: step.label });
  return (
    <Hint label={failed ? progress.message : label}>
      <button
        type="button"
        className={`flex min-w-0 shrink items-center gap-xxs rounded-xs px-xxs outline-none hover:bg-popover hover:text-foreground focus-visible:ring-1 focus-visible:ring-ring disabled:opacity-50 ${failed ? "text-destructive" : ""}`}
        aria-label={label}
        aria-busy={pending}
        data-pane-return={step.pane_id}
        disabled={pending || (failed && !progress.retryable)}
        onClick={() => actions.followRelation(pane.id, step.pane_id, step.label)}
      >
        <span className="hidden min-w-0 max-w-(--size-pane-child-chip-max) truncate @min-[var(--size-pane-parent-breakpoint)]/pane:inline">{failed && progress.retryable ? t("common.retry") : step.label}</span>
        <span aria-hidden="true" className="shrink-0 text-muted-foreground">{pending ? "…" : "›"}</span>
      </button>
    </Hint>
  );
}

/**
 * The relationship focus the operator asked for, while it is in flight or
 * after it failed (B15). A visible source band owns this feedback; the Agent
 * area keeps it reachable when the core has moved that source out of view.
 */
export function RelationStatus({ actions, visiblePaneIds }: { actions: Actions; visiblePaneIds: readonly string[] }) {
  const { t } = useInterfaceTranslation();
  const relation = useUiStore((s) => s.relation);
  const outcome = useShellStore((s) => s.rest?.status?.pane_focus_request);
  const band = useShellStore((s) => s.rest?.terminal?.headers?.[relation?.sourcePaneId ?? ""]?.band);
  if (!relation) return null;
  // A visible source band owns its move's feedback. Retain the existing
  // fallback only when that band is no longer on screen or cannot show it.
  if (visiblePaneIds.includes(relation.sourcePaneId) && band?.action?.pane_id === relation.targetPaneId) return null;
  const state = relationState(relation, outcome, t);
  if (!state) return null;
  if (state.phase === "pending") {
    return (
      <div className="flex shrink-0 items-center gap-sm bg-card px-sm py-xxs text-caption text-muted-foreground" role="status" data-relation-status="pending">
        <span className="min-w-0 flex-1 truncate">{t("panes.relation.opening", { name: relation.label })}</span>
        <Button variant="ghost" size="sm" className="h-auto shrink-0 px-none text-muted-foreground hover:bg-transparent hover:text-foreground" data-relation-dismiss="true" onClick={() => actions.dismissRelation()}>
          {t("workspace.dismiss")}
        </Button>
      </div>
    );
  }
  return (
    <div className="flex shrink-0 items-center gap-sm bg-card px-sm py-xxs text-caption" role="alert" data-relation-status="failed">
      <Hint label={state.message} reveals>
      <span className="min-w-0 flex-1 truncate text-destructive">
        {state.message}
      </span>
      </Hint>
      {state.retryable ? (
        <Button variant="ghost" size="sm" className="h-auto shrink-0 px-none text-subtle-foreground hover:bg-transparent hover:text-foreground" data-relation-retry="true" onClick={() => actions.retryRelation()}>
          {t("common.retry")}
        </Button>
      ) : null}
      <Button variant="ghost" size="sm" className="h-auto shrink-0 px-none text-muted-foreground hover:bg-transparent hover:text-foreground" data-relation-dismiss="true" onClick={() => actions.dismissRelation()}>
        {t("workspace.dismiss")}
      </Button>
    </div>
  );
}

type PaneMenuId =
  | `open:${string}`
  | "sleep_agent"
  | "archive_session"
  | "fork_agent"
  | "fork_agent_side"
  | "copy_name"
  | "copy_pane_id"
  | "close_pane"
  | "copy"
  | "paste"
  | "select_all"
  | "find"
  | "split_right"
  | "split_down"
  | "toggle_zoom"
  | "link_open"
  | "link_source"
  | "link_external"
  | "link_copy"
  | "link_reveal";

/**
 * What the pane header offers about this pane (B16, B18): its relatives,
 * each opened only by choosing it, its name and Herdr id to copy, and closing it. Opening
 * the menu moves no focus and marks nothing read.
 */
export function paneMenuItems(pane: PaneRow, title: string, archive: ArchiveOffer = { kind: "hidden" }): MenuEntry<PaneMenuId>[] {
  const relationLabel = { parent: "panes.relation.parent", sibling: "panes.relation.sibling" } as const;
  const items: MenuEntry<PaneMenuId>[] = relationEntries(pane).map((entry) => ({
    id: `open:${entry.paneId}` as const,
    label: translate(relationLabel[entry.relation], { name: entry.label }),
    icon: "terminal", group: "relations", menuLabel: translate(`menus.${entry.relation}`, { name: entry.label }),
    unavailable: null,
  }));
  // Sleep agent (PRD agent-sleep B15): offered on a local agent pane that is
  // awake, disabled with the core's reason when this agent cannot sleep now.
  const agentActions: MenuEntry<PaneMenuId>[] = [];
  if (pane.sleep_action) {
    agentActions.push({
      id: "sleep_agent", menuLabel: translate("menus.sleep"), icon: "sleep",
      label: translate("panes.menu.sleep"),
      unavailable: pane.sleep_action.available ? null : (pane.sleep_action.reason ?? translate("panes.menu.sleepUnavailable")),
    });
  }
  // Archive session (PRD session-archive B1-B4, D-08): below Sleep agent on a
  // root agent's pane, disabled with why on a child, an unknown conversation
  // or a disconnected device; a shell, a Factory worker and Hide AI get none.
  const archiveEntry: MenuEntry<PaneMenuId> | null = archive.kind === "offered" ? { id: "archive_session", icon: "archive", label: translate("archive.menu.archive"), unavailable: archive.unavailable } : null;
  if (archiveEntry && pane.sleep_action) agentActions.push(archiveEntry);
  // Fork agent and Fork agent to the side (PRD fork-operator-root B2, B3, D-03):
  // offered on an agent pane the core can fork, and both drawn disabled with the
  // core's reason on an agent that can fork but has not reported its conversation
  // yet; an agent with no fork command, a shell and a device pane get neither.
  if (pane.fork?.available || pane.fork?.reason) {
    const unavailable = pane.fork.available ? null : (pane.fork.reason ?? translate("panes.menu.forkUnavailable"));
    agentActions.push(
      { id: "fork_agent", group: "fork", menuLabel: translate("menus.forkTab"), icon: "fork", label: translate("panes.menu.fork"), unavailable },
      { id: "fork_agent_side", group: "fork", menuLabel: translate("menus.forkSide"), icon: "split_right", label: translate("panes.menu.forkSide"), unavailable },
    );
  }
  if (archiveEntry && !pane.sleep_action) agentActions.push(archiveEntry);
  if (agentActions.length > 0) items.push(...agentActions.map((item, index) => (index === 0 ? { ...item, separated: items.length > 0 } : item)));
  items.push({ id: "copy_name", group: "copy", menuLabel: translate("menus.terminalName"), icon: "copy", label: translate("panes.menu.copyName"), unavailable: null, separated: items.length > 0 });
  items.push({ id: "copy_pane_id", group: "copy", menuLabel: translate("menus.terminalId"), icon: "id", label: translate("panes.menu.copyPaneId"), unavailable: null });
  items.push({ id: "close_pane", menuLabel: translate("menus.closeTerminal"), icon: "close", label: translate("panes.close", { name: title }), unavailable: null, separated: true });
  return items;
}

/** What a right-click in the terminal knows when it opens. */
export type TerminalMenuContext = {
  /** The pane has a drag selection, so Copy has something to copy. */
  selection: boolean;
  zoomed: boolean;
  /** Panes in the tab, the zoomed one's hidden siblings included. */
  paneCount: number;
  /** The chords the registry and the terminal bind here; "" draws none. */
  chords: { copy: string; paste: string; find: string; splitRight: string; splitDown: string; zoom: string };
  /** The exact target right-clicked, retained even after the pointer leaves it. */
  link?: TerminalContextLink | null;
  reveal?: RevealHost;
};

/**
 * A right-click in the terminal: its link first, editing the text, then the tab's
 * layout around this pane, then everything the header menu offers.
 */
export function terminalMenuItems(pane: PaneRow, title: string, context: TerminalMenuContext, archive: ArchiveOffer = { kind: "hidden" }): MenuEntry<PaneMenuId>[] {
  const { chords } = context;
  const items: MenuEntry<PaneMenuId>[] = [];
  const { target, found } = context.link ?? {};
  if (target?.kind === "url") {
    items.push(
      { id: "link_open", icon: "record", label: translate("panes.menu.openInHide"), unavailable: null },
      { id: "link_external", icon: "external", label: translate("panes.menu.openDefaultBrowser"), unavailable: null },
      { id: "link_copy", icon: "copy", label: translate("panes.menu.copyLink"), unavailable: null },
    );
  } else if (target?.kind === "path" && found) {
    const html = found.kind === "file" && isHtmlFile(found.real);
    items.push({ id: "link_open", icon: "record", label: translate(html ? "documents.openInBrowser" : "common.open"), unavailable: null });
    if (html && context.link?.inCheckout) items.push({ id: "link_source", icon: "source", label: translate("panes.menu.openSource"), unavailable: null });
    items.push({ id: "link_external", icon: "external", label: translate("panes.menu.openDefaultApp"), unavailable: null });
    const host = context.reveal === undefined ? revealHost() : context.reveal;
    if (host) items.push({ id: "link_reveal", icon: "external", label: translate(host.label), unavailable: null });
    items.push({ id: "link_copy", icon: "copy", label: translate("workspace.menu.copyPath"), unavailable: null });
  }
  const editingStart = items.length;
  if (context.selection) items.push({ id: "copy", icon: "copy", label: translate("common.copy"), unavailable: null, shortcut: chords.copy });
  items.push(
    { id: "paste", icon: "paste", label: translate("panes.menu.paste"), unavailable: null, shortcut: chords.paste },
    { id: "select_all", icon: "select", label: translate("panes.menu.selectAll"), unavailable: null },
    { id: "find", icon: "find", label: translate("panes.menu.find"), unavailable: null, shortcut: chords.find },
    { id: "split_right", icon: "split_right", label: splitLabel("right"), unavailable: null, separated: true, shortcut: chords.splitRight },
    { id: "split_down", icon: "split_down", label: splitLabel("down"), unavailable: null, shortcut: chords.splitDown },
    {
      id: "toggle_zoom", menuLabel: translate(context.zoomed ? "menus.unzoom" : "menus.zoom"), icon: context.zoomed ? "unzoom" : "zoom",
      label: context.zoomed ? translate("panes.unzoom") : translate("panes.menu.zoom"),
      unavailable: context.zoomed || context.paneCount > 1 ? null : translate("panes.menu.onlyPane"),
      shortcut: chords.zoom,
    },
  );
  if (editingStart > 0) items[editingStart] = { ...items[editingStart]!, separated: true };
  const paneItems = paneMenuItems(pane, title, archive).map((item, index) => (index === 0 ? { ...item, separated: true } : item));
  return [...items, ...paneItems];
}

/** Where the menu opened, and what a terminal right-click knew then; its items are drawn from the pane as it is now. */
type MenuOpen = { x: number; y: number; terminal: TerminalMenuContext | null };


export function usePaneMenu(pane: PaneRow, title: string, actions: Actions) {
  const { t } = useInterfaceTranslation();
  const [open, setOpen] = useState<MenuOpen | null>(null);
  const select = (id: PaneMenuId) => {
    if (id.startsWith("link_")) {
      const link = open?.terminal?.link;
      if (!link) return;
      const { target, found } = link;
      if (id === "link_copy") {
        if (target.kind === "url") return actions.copyText(target.url, "terminal link");
        if (found) return actions.copyText(found.real, "terminal path");
        return;
      }
      if (target.kind === "url") return actions.openLink(target.url, id === "link_external", pane.id);
      if (!found) return;
      if (id === "link_reveal") return actions.revealExternal(found.real);
      return actions.openTerminalPath(found, target.line, target.column, id === "link_external", id === "link_source" ? "source" : "auto", pane.id);
    }
    switch (id) {
      case "copy_name":
        return actions.copyText(title, "pane name");
      case "copy_pane_id":
        return actions.copyText(herdrPaneId(pane.id), "pane id");
      case "close_pane":
        return actions.closePane(pane.id);
      case "sleep_agent":
        return actions.sleepAgent(pane.id);
      case "archive_session":
        return actions.archiveSession(pane.id);
      case "fork_agent":
        return actions.forkPane(pane.id, "tab");
      case "fork_agent_side":
        return actions.forkPane(pane.id, "side");
      case "copy":
        void copySelection(pane.id, "pane menu");
        return;
      case "paste":
        void pasteClipboard(pane.id, "pane menu");
        return;
      case "select_all":
        return selectAllText(pane.id);
      // The right-click focused this pane; each command still names it.
      case "find":
        return actions.openFind(pane.id);
      case "split_right":
        return actions.split("right", pane.id);
      case "split_down":
        return actions.split("down", pane.id);
      case "toggle_zoom":
        return actions.toggleZoom(pane.id);
    }
    const target = relationEntries(pane).find((entry) => `open:${entry.paneId}` === id);
    if (target) actions.followRelation(pane.id, target.paneId, target.label);
  };
  // The name, the fork and sleep availability and the relatives change while a
  // menu is open (the session's label arrives after its first turn), and the
  // menu says what the header says now.
  const archive = useArchiveOffer(pane.id, open !== null);
  const items = open === null ? [] : open.terminal ? terminalMenuItems(pane, title, open.terminal, archive) : paneMenuItems(pane, title, archive);
  const menu = <EntryPointMenu title={title} label={t("panes.menu.aria", { name: title })} items={items} onSelect={select} at={open} onClose={() => setOpen(null)} />;
  return {
    menu,
    open: open !== null,
    openAt: (x: number, y: number) => setOpen({ x, y, terminal: null }),
    openTerminalAt: (x: number, y: number, context: TerminalMenuContext, link?: Promise<TerminalContextLink | null>) => {
      const opened = { x, y, terminal: context };
      setOpen(opened);
      void link?.then((resolved) => {
        // A late filesystem answer must not reopen a dismissed or newer menu.
        setOpen((current) => current === opened ? { ...opened, terminal: { ...context, link: resolved } } : current);
      }).catch((error: unknown) => useShellStore.getState().noteDiagnostic(`terminal menu: the link check failed: ${String(error)}`));
    },
  };
}

/** `세션 보관` for this pane's agent, read only while its menu is open. */
function useArchiveOffer(paneId: string, open: boolean): ArchiveOffer {
  const { t } = useInterfaceTranslation();
  // A string, so a snapshot that moves nothing of the offer leaves the menu alone.
  const offer = useShellStore((s) => {
    if (!open) return "hidden";
    const agent = agentsByPane(allAgents(s.agents, s.rest?.status?.remote)).get(paneId);
    const found = archiveOffer(s.rest, agent, t);
    return found.kind === "hidden" ? "hidden" : `offered:${found.unavailable ?? ""}`;
  });
  if (offer === "hidden") return { kind: "hidden" };
  const reason = offer.slice("offered:".length);
  return { kind: "offered", unavailable: reason || null };
}

export function useRelationProgress(sourcePaneId: string, targetPaneId: string | null) {
  const { t } = useInterfaceTranslation();
  const relation = useUiStore((s) => s.relation);
  const outcome = useShellStore((s) => s.rest?.status?.pane_focus_request);
  if (!targetPaneId || relation?.sourcePaneId !== sourcePaneId || relation.targetPaneId !== targetPaneId) return null;
  return relationState(relation, outcome, t);
}
