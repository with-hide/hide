// Desk's own desk of Views (PRD desk-own-desk D-01-D-04, D-10): the same View
// areas a Workspace's Agent tab has, kept by the core apart from every
// Workspace under one reserved workspace, which every `view_layout`,
// `browser_open` and `browser_state` names it by (`desk_view` in the rest):
// the core's own machine, where Desk runs and its pages load, and a folder
// no checkout can be.

import { catalogWorkspaces, checkoutAtRoot, checkoutPlaces, localDeviceId, type DeskOutput, type DeskViewSnapshot, type DetailItem, type SnapshotRest } from "./snapshot";
import type { ViewWorkspace } from "./viewLayout";

/** The folder Desk's desk goes by on the wire (`hide-core/src/runtime/desk/views.rs`, `DESK_VIEWS_PATH`). */
const DESK_PATH = "hide:desk";

/**
 * The keyboard owner's name for Desk's room (`data-workspace-screen`), where
 * a Workspace's screen names its checkout id; no checkout id has this shape.
 */
export const DESK_SCREEN = "hide:desk";

export function isDeskWorkspace(workspace: ViewWorkspace | null | undefined): boolean {
  return workspace?.path === DESK_PATH;
}

/** Desk's desk, absent while Desk is off. */
export function deskViewOf(rest: SnapshotRest | null | undefined): DeskViewSnapshot | null {
  return rest?.desk_view ?? null;
}

/** The workspace Desk's desk goes by, as the core names it; null while Desk is off. */
export function deskWorkspaceOf(rest: SnapshotRest | null | undefined): ViewWorkspace | null {
  const view = deskViewOf(rest);
  return view ? { device_id: view.device_id, path: view.path } : null;
}

/** Desk's desk as its View areas draw it: its workspace and its layout. */
export type DeskWorkspace = DeskViewSnapshot;

/** How many tabs Desk's desk holds, the `▤ N` of Desk's head (D-06). */
export function deskTabCount(view: DeskViewSnapshot | null): number {
  return view?.layout.display_count ?? 0;
}

/**
 * What a press in Desk's room opens on Desk's desk (D-12, D-15, B10, B11,
 * B39): a file of a checkout, an output a session opened, or one of Hide's
 * cards (a mission's map, a pull request). `project` is the project of the
 * session whose output it is, where a pull request the catalog does not list
 * is looked for.
 */
export type DeskOpen =
  | { kind: "file"; device_id: string; root: string; path: string }
  | { kind: "output"; output: DeskOutput; project?: string | null }
  | { kind: "card"; card: DetailItem };

/** A `desk_view_open` payload, a page to open outside Hide, or why nothing opens (the log's). */
export type DeskOpenSend =
  | { send: Record<string, unknown>; focus: { card: DetailItem } | { opened: { path: string; kind: "file"; committed: null } } | null }
  | { link: string }
  | { refused: string };

/**
 * The event an open is (D-12): a View a session listed opens the same file or
 * address as a tab of Desk's own, through the core, which marks the
 * session's View seen; a file is named by its checkout; a pull request is
 * its card, in the project that lists it.
 */
export function deskOpenSend(rest: SnapshotRest | null | undefined, open: DeskOpen, listed: (output: DeskOutput) => boolean, options: { preview?: boolean; beside?: boolean } = {}): DeskOpenSend {
  if (open.kind === "card") return { send: { kind: "card", card: open.card }, focus: { card: open.card } };
  if (open.kind === "file") return fileSend(rest, open.device_id, open.root, open.path, options);
  const output = open.output;
  if (output.kind === "pr") {
    const url = output.url ?? "";
    const project = catalogWorkspaces(rest ?? null).find((row) => row.pull_requests?.some((pr) => pr.url === url));
    const number = project?.pull_requests?.find((pr) => pr.url === url)?.number ?? pullNumber(url);
    const workspace = project?.id ?? open.project ?? null;
    if (workspace && number !== null) {
      const card: DetailItem = { kind: "pull_request", workspace_id: workspace, number };
      return { send: { kind: "card", card }, focus: { card } };
    }
    return url ? { link: url } : { refused: "desk open: a pull request with no address" };
  }
  if (output.device_id !== null && output.checkout !== null && listed(output)) {
    return { send: { kind: "output", device_id: output.device_id, checkout: output.checkout, id: output.id }, focus: null };
  }
  if (output.path === null) return { refused: `desk open: output ${output.id} names no file Hide lists` };
  const device = output.device_id ?? localDeviceId(rest ?? null);
  const root = output.checkout ?? checkoutHolding(rest, device, output.path);
  if (!root) return { refused: "desk open: the output's file is in no registered checkout" };
  return fileSend(rest, device, root, output.path, options);
}

function fileSend(rest: SnapshotRest | null | undefined, device: string, root: string, path: string, options: { preview?: boolean; beside?: boolean }): DeskOpenSend {
  const place = checkoutAtRoot(rest ?? null, device, root);
  if (!place) return { refused: "desk file: its checkout is not registered" };
  return {
    send: { kind: "file", workspace_id: place.workspace, checkout_id: place.checkout.id, device_id: device, path, preview: options.preview === true, beside: options.beside === true },
    focus: { opened: { path, kind: "file", committed: null } },
  };
}

/** The checkout of `device` whose folder holds `path`, the deepest one. */
function checkoutHolding(rest: SnapshotRest | null | undefined, device: string, path: string): string | null {
  const roots = checkoutPlaces(rest ?? null).filter((place) => place.device === device && (path === place.checkout.path || path.startsWith(`${place.checkout.path.replace(/\/$/u, "")}/`)));
  return roots.sort((a, b) => b.checkout.path.length - a.checkout.path.length)[0]?.checkout.path ?? null;
}

/** A pull request's number from its address's last part (`…/pull/12`, `…/merge_requests/12`). */
function pullNumber(url: string): number | null {
  const found = /\/(\d+)\/?(?:[?#].*)?$/u.exec(url);
  return found ? Number(found[1]) : null;
}
