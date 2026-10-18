// The View areas' rules (PRD S7 B6-B13, B20; contract sections 2 and 6): the
// core owns the tree of areas and displays (`workspace_view.layout`), and
// these pure functions answer what the page asks of it - where each area and
// divider sits for a body rectangle, which area lies in a direction, whether
// a split fits, where a dragged tab would land, which commands a display's
// menu offers, and what a narrow window shows. They take values and return
// values, so every rule is testable without a page; nothing here dispatches.

import type { TFunction } from "i18next";
import type { ViewDisplaySnapshot, ViewLayoutSnapshot, ViewNode } from "./snapshot";
import { revealExternalEntry, type RevealHost } from "./revealExternal";
import { translate } from "./i18n/client";
import type { MenuEntry } from "./components/entry-menu";
import type { MessageKey } from "./i18n/catalogs";

/** The order a display's menu offers directions in. */
const MENU_EDGES: readonly Edge[] = ["right", "left", "up", "down"];

// --- the Workspace an action names ---------------------------------------------

/** The Workspace a View action was taken on, as its frame's `workspace_view` names it (contract 4.1). */
export type ViewWorkspace = { device_id: string; path: string };

/** What the operator acted on: one Workspace's View areas as a frame drew them. */
export type ViewFrame = { workspace: ViewWorkspace; layout: ViewLayoutSnapshot };

/** One string per Workspace, for keys and comparisons. */
export function workspaceKey(workspace: ViewWorkspace): string {
  return `${workspace.device_id}\u0000${workspace.path}`;
}

/**
 * A `view_layout` payload (contract 4.1): the action and its fields with the
 * Workspace of the frame it was taken on. Display, area and split ids repeat
 * across Workspaces and the front can move before the event lands, so the
 * core applies the action only while that Workspace is still in front.
 */
export function viewLayoutPayload(workspace: ViewWorkspace, action: { action: string } & Record<string, unknown>): Record<string, unknown> {
  return { workspace: { device_id: workspace.device_id, path: workspace.path }, ...action };
}

/**
 * What the operator is told when the core refuses a View action or open
 * (B19): the core's reason, for every `view_layout.*` refusal. An action
 * that arrived for a Workspace no longer in front (`stale_workspace`) is the
 * log's alone, since the screen already shows another Workspace.
 */
export function viewRefusal(error: { kind: string; message: string } | null | undefined): string | null {
  if (!error || (!error.kind.startsWith("view_layout.") && error.kind !== "agent_layout.display_limit") || error.kind === "view_layout.stale_workspace") return null;
  return error.message;
}

/** Whether two displays show one document: its path and kind, and a diff's History group (the core's `Display::shows`). */
export function showsSameDocument(a: ViewDisplaySnapshot, b: ViewDisplaySnapshot): boolean {
  return a.path === b.path && a.kind === b.kind && (a.kind === "file" || a.committed === b.committed);
}

/** Every display that shows one document (editor tab), in tree order. */
export function displaysOfDocument(root: ViewNode, tabId: string): ViewDisplaySnapshot[] {
  return areasOf(root).flatMap((area) => area.displays.filter((display) => display.tab_id === tabId));
}

import {
  activeDisplay, areasOf, locateDisplay, neighbourArea,
  splitEligibility as areaSplitEligibility, dropTarget as areaDropTarget,
  resizeTarget as areaResizeTarget, roomToSplit, areaSentence, splitLabel, moveLabel, type AreaWords,
  type LocatedItem, type Edge, type LayoutSizes,
  type Geometry, type DropTarget, type Eligibility,
} from "./areaLayout";
export {
  areasOf, findArea, locateDisplay, activeDisplay, parentSplit, neighbourArea,
  shownDisplays, areaDepth, adjacentInOrder, minimumSize, areaGeometry as viewGeometry,
  singleAreaGeometry, revealedScroll, ratioAtOffset, steppedRatio, ratioForFirst,
  RATIO_MIN, RATIO_MAX, RESIZE_STEP, EDGE_ZONE, sameTarget,
} from "./areaLayout";
export type { Rect, Point, Edge, LayoutSizes, Geometry, AreaBox, DividerBox, TabSlot, DropTarget, Eligibility } from "./areaLayout";
export type LocatedDisplay = LocatedItem<ViewDisplaySnapshot>;
export const VIEW_WORDS: AreaWords = { kind: "view" };
const refuse = (reason: string): Eligibility => ({ ok: false, reason });

export function splitEligibility(layout: ViewLayoutSnapshot, geometry: Geometry, sizes: LayoutSizes, displayId: string, areaId: string, edge: Edge): Eligibility {
  return areaSplitEligibility(layout, geometry, sizes, displayId, areaId, edge, VIEW_WORDS);
}
export function dropTarget(input: Omit<Parameters<typeof areaDropTarget<ViewDisplaySnapshot>>[0], "words" | "sameContent">): DropTarget {
  return areaDropTarget({ ...input, words: VIEW_WORDS, sameContent: showsSameDocument });
}
export function resizeTarget(layout: ViewLayoutSnapshot, geometry: Geometry | null, grow: boolean) {
  return areaResizeTarget(layout, geometry, grow, VIEW_WORDS);
}

// --- eligibility -------------------------------------------------------------

/**
 * Why Open to the side cannot land now, or null (B4, B9, D-06). With two
 * or more areas it goes to a neighbour and splits nothing, and into an
 * empty Views it opens in the empty area; from the only area with a view
 * it makes a new area on the right, so that area has to hold two minimums
 * side by side, exactly as Split right does. While the View areas are not
 * drawn (the side panel closed) their room is unknown, and the core's caps decide.
 */
export function besideUnavailable(
  layout: ViewLayoutSnapshot | null | undefined,
  drawn: { geometry: Geometry; sizes: LayoutSizes } | null,
): string | null {
  if (!layout || !drawn) return null;
  const areas = areasOf(layout.root);
  const only = areas.length === 1 ? areas[0] : undefined;
  if (!only || only.displays.length === 0) return null;
  const room = roomToSplit(layout, drawn.geometry, drawn.sizes, only.id, "right", VIEW_WORDS);
  if (room.ok) return null;
  return room.reason === areaSentence(VIEW_WORDS, "tooNarrow") ? translate("documents.view.besideTooNarrow") : room.reason;
}

// --- a display's menu --------------------------------------------------------

/** The OS file manager's reveal as a display's menu offers it: the host's item, the device the Workspace's files are on, and the core's own node. */
export type ExternalReveal = { host: RevealHost; device: string; node: string };

export type ViewMenuId =
  | "keep_open"
  | `split_${Edge}`
  | `move_${Edge}`
  | "copy_path"
  | "select_in_tree"
  | "reveal_external"
  | "close_view";

export type ViewMenuEntry = MenuEntry<ViewMenuId>;

/** The menu's items in docs/UI_BEHAVIOR.md's order, with its fixed labels. */
const MENU_ITEMS: readonly { id: ViewMenuId; label: () => string }[] = [
  { id: "keep_open", label: () => translate("documents.view.keepOpen") },
  ...MENU_EDGES.map((edge) => ({ id: `split_${edge}` as const, label: () => splitLabel(edge) })),
  ...MENU_EDGES.map((edge) => ({ id: `move_${edge}` as const, label: () => moveLabel(edge) })),
  { id: "copy_path", label: () => translate("workspace.menu.copyPath") },
  { id: "select_in_tree", label: () => translate("documents.view.selectInTree") },
  // Labelled by the host's OS (`revealLabel`).
  { id: "reveal_external", label: () => "" },
  { id: "close_view", label: () => translate("documents.view.closeView") },
];

const NO_AREA = {
  right: "documents.view.noAreaRight",
  left: "documents.view.noAreaLeft",
  up: "documents.view.noAreaUp",
  down: "documents.view.noAreaDown",
} as const satisfies Record<Edge, MessageKey>;

/**
 * Every command of a display's menu with the reason it cannot run now, or
 * null: Keep open for a preview, a split that can land (B9, B19), a move
 * toward an area that exists, Select in File Tree while the file can be read,
 * and the OS file manager's reveal where the host has one, for a file on this
 * computer that can be read (issue 324). `drawn` is what the page last drew;
 * without it a split's room cannot be judged.
 */
function displayCommands(
  layout: ViewLayoutSnapshot,
  drawn: { geometry: Geometry; sizes: LayoutSizes } | null,
  located: LocatedDisplay,
  external: ExternalReveal,
): (ViewMenuEntry & { hidden: boolean })[] {
  const { area, display } = located;
  return MENU_ITEMS.map(({ id, label: labelOf }) => {
    const label = labelOf();
    if (id === "reveal_external") {
      // A page is not a file of the checkout.
      const [entry] = display.kind === "browser" ? [] : revealExternalEntry(external.host, external.device, external.node, translate, revealBlocked(display));
      return entry ? { ...entry, hidden: false } : { id, label, unavailable: null, hidden: true };
    }
    const edge = menuEdge(id);
    let unavailable: string | null = null;
    let hidden = false;
    if (id === "keep_open") {
      unavailable = display.preview ? null : translate("documents.view.alreadyKept");
      hidden = !display.preview;
    } else if (edge && id.startsWith("split_")) {
      const eligibility = drawn ? splitEligibility(layout, drawn.geometry, drawn.sizes, display.id, area.id, edge) : refuse(translate("documents.view.notOnScreen"));
      unavailable = eligibility.ok ? null : eligibility.reason;
    } else if (edge) {
      unavailable = neighbourArea(layout.root, area.id, edge) ? null : translate(NO_AREA[edge]);
      hidden = unavailable !== null;
    } else if (id === "select_in_tree") {
      unavailable = revealBlocked(display);
      // A page is not a file of the checkout.
      hidden = display.kind === "browser";
    }
    const named = id === "copy_path" && display.kind === "browser" ? translate("documents.view.copyAddress") : label;
    return { id, label: named, unavailable, hidden, separated: id === "copy_path" || id === "close_view" };
  });
}

/**
 * The commands a display's tab menu and its area's overflow button offer, and
 * nothing else (B11, D-07): Keep open while it is a preview, a split in each
 * direction (disabled with the reason when it cannot land), a move toward
 * each direction that has an area, the path, Select in File Tree, the OS file
 * manager's reveal, and Close view. There is no file deletion here.
 */
export function displayMenu(layout: ViewLayoutSnapshot, geometry: Geometry, sizes: LayoutSizes, displayId: string, external: ExternalReveal): ViewMenuEntry[] {
  const located = locateDisplay(layout.root, displayId);
  if (!located) return [];
  return displayCommands(layout, { geometry, sizes }, located, external)
    .filter((entry) => !entry.hidden)
    .map(({ id, label, unavailable, separated }): ViewMenuEntry => {
      const entry = { id, label, unavailable, ...(separated ? { separated } : {}) };
      const edge = menuEdge(id);
      if (edge) return { ...entry, icon: id.startsWith("split_") ? `split_${edge}` : edge, group: id.startsWith("split_") ? "split" : "move", menuLabel: translate(`menus.${edge}`), separated: id === "split_right" };
      const icons = { keep_open: "pin", copy_path: "copy", select_in_tree: "folder", reveal_external: "external", close_view: "close" } as const;
      return { ...entry, icon: icons[id as keyof typeof icons] };
    });
}

/**
 * Where a display's selection and scroll are remembered: per Workspace,
 * display and document. A preview display is retargeted in place to the
 * next document, which starts at its own top rather than at the place the
 * last one was left (B1, B4).
 */
export function placeKey(workspace: string, display: Pick<ViewDisplaySnapshot, "id" | "tab_id">): string {
  return `${workspace}\u0000${display.id}\u0000${display.tab_id ?? ""}`;
}

function revealBlocked(display: ViewDisplaySnapshot): string | null {
  if (display.state === "unavailable") return translate("documents.view.fileUnavailable");
  if (display.state === "waiting") return display.reason ?? translate("documents.view.fileWaiting");
  return null;
}

/** The edge a `split_*` or `move_*` menu id names. */
export function menuEdge(id: ViewMenuId): Edge | null {
  const match = /^(?:split|move)_(left|right|up|down)$/.exec(id);
  return match ? (match[1] as Edge) : null;
}

/** What a display is, for its tooltip and accessible name (B21): kind, full path and state. */
export function displayIdentity(display: ViewDisplaySnapshot, t: TFunction<"translation">): string {
  if (display.kind === "browser" && !display.url) return t("panes.area.newTab");
  if (display.kind === "browser") return t("documents.view.identityPage", { target: `${display.title ? `${display.title} · ` : ""}${display.url ?? ""}` });
  const kind = display.kind === "diff" ? (display.committed ? t("documents.view.kindBranchDiff") : t("documents.view.kindWorkingDiff")) : t("documents.view.kindFile");
  const state =
    display.state === "unavailable"
      ? ` · ${display.reason ? t("documents.view.stateUnavailableReason", { reason: display.reason }) : t("documents.view.stateUnavailable")}`
      : display.state === "waiting"
        ? ` · ${display.reason ? t("documents.view.stateWaitingReason", { reason: display.reason }) : t("documents.view.stateWaiting")}`
        : display.state === "opening"
          ? ` · ${t("documents.view.stateOpening")}`
          : display.preview
            ? ` · ${t("documents.view.statePreview")}`
            : "";
  return `${t("documents.view.identity", { kind, path: display.path })}${state}`;
}

// --- area commands -------------------------------------------------------------

/**
 * The area commands the registry carries (PRD cmdk-navigation D-05, B24):
 * focus to the next or previous View area and growing or shrinking the one
 * in use.
 */
export type ViewAreaStep = "focus_next" | "focus_previous" | "grow" | "shrink";

/**
 * Why a View area step cannot run now, or null. `drawn` is what the page last
 * drew of the areas; a resize judges its room on it, and without it only the
 * core's own range limits it.
 */
export function viewAreaStepUnavailable(layout: ViewLayoutSnapshot, drawn: { geometry: Geometry } | null, step: ViewAreaStep): string | null {
  if (step === "focus_next" || step === "focus_previous") return areasOf(layout.root).length < 2 ? areaSentence(VIEW_WORDS, "onlyOne") : null;
  const target = resizeTarget(layout, drawn?.geometry ?? null, step === "grow");
  return "reason" in target ? target.reason : null;
}

// --- where the keyboard goes -------------------------------------------------

/**
 * Where the keyboard goes once the core shows it (B20): a display a menu,
 * the palette or a drop moved or split, with where it stood when asked; an
 * area a focus command chose; or a file or diff the operator opened, which
 * has no display id until the core places it; each in the Workspace it was
 * asked in.
 */
export type ViewFocusRequest =
  | { workspace: string; displayId: string; from: { areaId: string; index: number } | null }
  | { workspace: string; areaId: string }
  | { workspace: string; opened: { path: string; kind: "file" | "diff"; committed: boolean | null } }
  | { workspace: string; browserOpen: string };

/** The core's answer to a `browser_open` that named itself (`status.browser_opens`). */
export type BrowserOpenReceipt = { request_id: string; ok: boolean; display_id: string | null };

/**
 * Whether the core now shows what a focus request asked for, so the keyboard
 * can follow: the named area active, the display active in the active area
 * and no longer where it stood when asked, or the opened file or diff (of
 * its own History group) the active area's view once it is past opening,
 * or the page a named browser open answered with, active in the active area.
 * A move or split of the active view therefore resolves once it has landed,
 * never on the frame it was asked from, and a refused one never resolves.
 */
export function focusRequestArrived(request: ViewFocusRequest, workspace: string, layout: ViewLayoutSnapshot, opens: readonly BrowserOpenReceipt[] = []): boolean {
  if (request.workspace !== workspace) return false;
  if ("areaId" in request) return layout.active_area === request.areaId;
  if ("browserOpen" in request) {
    const page = opens.find((receipt) => receipt.request_id === request.browserOpen);
    const located = page?.ok && page.display_id ? locateDisplay(layout.root, page.display_id) : null;
    return !!located && layout.active_area === located.area.id && located.area.active === located.display.id;
  }
  if ("opened" in request) {
    const shown = activeDisplay(layout)?.display;
    const { path, kind, committed } = request.opened;
    return shown?.kind === kind && shown.path === path && shown.committed === committed && shown.state !== "opening";
  }
  const located = locateDisplay(layout.root, request.displayId);
  if (!located || layout.active_area !== located.area.id || located.area.active !== request.displayId) return false;
  return !request.from || located.area.id !== request.from.areaId || located.index !== request.from.index;
}
