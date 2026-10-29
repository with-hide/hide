// The links a terminal pane offers under the pointer, and where a click on
// one goes (docs/UI_BEHAVIOR.md, Terminal links).
//
// xterm asks for the links of a row only when the pointer reaches it, so the
// rows are read then and never per frame: `terminalLinks.ts` finds the URL
// and path-shaped tokens, and a path is a link only once the desktop host
// says it exists on this Mac. A path is looked for under the pane's working
// folder and then under its checkout's root, the answers are cached briefly
// (`PROBE_CACHE_CAP`, `PROBE_TTL_MS`) so moving along a row asks nothing
// twice, and a pane on an SSH device or a page with no desktop host offers
// URLs only, since its paths name files this Mac cannot see.
//
// A click opens a URL or HTML as a page, or another file in View.
// ⌘-click (Ctrl off macOS) hands it to the operating system.
// A right-click never opens: xterm activates a link on any button's
// release, and on Windows the right-click menu comes after that release.
// A program's own OSC 8 links take the same routes through xterm's link
// handler, which replaces xterm's confirm dialog.

import type { IBufferRange, ILink, ILinkHandler, Terminal } from "@xterm/xterm";
import { hostBridge, holdsCommandKey, type ProbedPath } from "./host";
import { remoteTargetOfPane } from "./remote";
import { windowDeviceId } from "./screenMachine";
import { bufferRow, type CellRow } from "./selection";
import { catalogWorkspaces, localDeviceId, type Checkout, type SnapshotRest } from "./snapshot";
import { useShellStore } from "./store";
import { absolutePathRoot, JOIN_ROWS, linkCandidates, osc8Target, type LinkCandidate, type LinkTarget } from "./terminalLinks";

/** A path the host found: its physical spelling and kind. */
export type FoundPath = NonNullable<ProbedPath>;

/** Where a clicked link goes; `actions.ts` implements it. */
export type TerminalLinkActions = {
  /** `pane` is the terminal the link was clicked in: its agent becomes the View's opener (PRD view-opener-feedback D-02). */
  openLink(url: string, external: boolean, pane?: string): void;
  openTerminalPath(found: FoundPath, line: number | null, column: number | null, external: boolean, mode?: "auto" | "source", pane?: string): void;
};

/** `MouseEvent.button` of the right button. */
const RIGHT_BUTTON = 2;

/** Paths whose answer is kept; the oldest goes first. */
export const PROBE_CACHE_CAP = 512;
/** How long an answer is trusted, so a file made after a miss becomes a link soon after. */
export const PROBE_TTL_MS = 10_000;
/** Paths one host call carries (`MAX_PROBE_PATHS` in the desktop host). */
const PROBE_BATCH = 64;
/** New unique logical paths one hover may send to the host. Cache hits are free. */
export const PROBE_NEW_LIMIT = 512;

type Cached = { answer: ProbedPath; at: number };
const cache = new Map<string, Cached>();

/**
 * What each path names on this Mac, from the cache or one host call per 64
 * missing paths. A failed call, or a refusal (an answer that does not cover
 * the batch), leaves its paths out of the result and caches nothing, so a
 * caller can tell "does not exist" (null) from "not known" (absent).
 */
export async function probePaths(paths: string[], probe: (paths: string[]) => Promise<ProbedPath[]>, now = Date.now()): Promise<Map<string, ProbedPath>> {
  const answers = new Map<string, ProbedPath>();
  const missing: string[] = [];
  for (const path of new Set(paths)) {
    const hit = cache.get(path);
    if (hit && now - hit.at < PROBE_TTL_MS) answers.set(path, hit.answer);
    else missing.push(path);
  }
  if (missing.length > PROBE_NEW_LIMIT) {
    useShellStore.getState().noteDiagnostic(`terminal link: path_budget_exceeded unique=${answers.size + missing.length} cache_misses=${missing.length} limit=${PROBE_NEW_LIMIT} skipped=${missing.length - PROBE_NEW_LIMIT}`);
  }
  const admitted = missing.slice(0, PROBE_NEW_LIMIT);
  // The existing opt-in terminal QA seam records counts, never paths.
  // Ordinary hover has no diagnostic publication for successful checks.
  if (typeof window !== "undefined" && window.__hideProbe) {
    useShellStore.getState().noteDiagnostic(`terminal link: path_probe unique=${answers.size + missing.length} cache_hits=${answers.size} cache_misses=${missing.length} admitted=${admitted.length} skipped=${missing.length - admitted.length}`);
  }
  for (let from = 0; from < admitted.length; from += PROBE_BATCH) {
    const batch = admitted.slice(from, from + PROBE_BATCH);
    let found: ProbedPath[];
    try {
      found = await probe(batch);
      if (found.length !== batch.length) throw new Error(`the host answered ${found.length} of ${batch.length} paths`);
    } catch (error) {
      useShellStore.getState().noteDiagnostic(`terminal link: the path check failed: ${String(error)}`);
      continue;
    }
    batch.forEach((path, index) => {
      const answer = found[index] ?? null;
      answers.set(path, answer);
      cache.delete(path);
      cache.set(path, { answer, at: now });
    });
  }
  while (cache.size > PROBE_CACHE_CAP) {
    const oldest = cache.keys().next();
    if (oldest.done) break;
    cache.delete(oldest.value);
  }
  return answers;
}

/** Forgets every cached answer (tests). */
export function clearProbeCache(): void {
  cache.clear();
}

/** `.` and `..` resolved while keeping the wire path's drive, share or POSIX root. */
export function normalize(path: string): string {
  const root = absolutePathRoot(path);
  if (!root) throw new Error("terminal link: normalization needs an absolute wire path");
  const parts: string[] = [];
  for (const part of path.slice(root.length).split("/")) {
    if (part === "" || part === ".") continue;
    if (part === "..") parts.pop();
    else parts.push(part);
  }
  return `${root}${parts.join("/")}`;
}

/** The spellings to look a written path up under, in order: as written when absolute, else under the pane's folder and then the checkout's root. */
export function pathLookups(written: string, cwd: string | null, root: string | null): string[] {
  if (absolutePathRoot(written)) return [normalize(written)];
  if (written.startsWith("~/")) return [written];
  const bases = [cwd, root].filter((base): base is string => !!base && absolutePathRoot(base) !== null);
  return [...new Set(bases.map((base) => normalize(`${base}/${written}`)))];
}

/**
 * The pane's working folder and its checkout's root, or null for a pane
 * that is not on this window's machine (`screen`, `windowDeviceId`): the
 * core's own unless this window's machine dials a core elsewhere.
 */
export function paneContext(rest: SnapshotRest | null, paneId: string, screen: string): { cwd: string | null; root: string | null } | null {
  const target = remoteTargetOfPane(rest, paneId);
  if ((target ?? localDeviceId(rest)) !== screen) return null;
  for (const workspace of catalogWorkspaces(rest)) {
    if (workspace.device_id !== screen) continue;
    for (const checkout of workspace.checkouts) {
      const pane = checkout.tabs.flatMap((tab) => tab.panes).find((row) => row.id === paneId);
      if (pane) return { cwd: pane.cwd || null, root: checkout.path };
    }
  }
  return { cwd: null, root: null };
}

/**
 * The checkout a physical path belongs to: the one whose physical root holds
 * it and is longest, so a worktree nested in another checkout wins; null when
 * no root does. `roots` answers each checkout's path.
 */
export function owningCheckout<C extends { path: string }>(real: string, checkouts: C[], roots: Map<string, ProbedPath>): { checkout: C; root: string } | null {
  let best: { checkout: C; root: string } | null = null;
  for (const checkout of checkouts) {
    const root = roots.get(checkout.path);
    if (root?.kind !== "directory") continue;
    if (real !== root.real && !real.startsWith(`${root.real}/`)) continue;
    if (!best || root.real.length > best.root.length) best = { checkout, root: root.real };
  }
  return best;
}

/** `sourceInHide`: an HTML file's Open source opens its text in Hide, in a checkout or as an outside file. */
export type TerminalContextLink = { target: LinkTarget; found: FoundPath | null; inCheckout?: boolean; sourceInHide?: boolean };

/** null is outside every checkout; undefined is an unanswered ownership check. */
export async function terminalPathOwner(found: FoundPath): Promise<{ checkout: Checkout; path: string; deviceId: string } | null | undefined> {
  const bridge = hostBridge();
  if (!bridge) return undefined;
  const rest = useShellStore.getState().rest;
  const deviceId = windowDeviceId(rest?.navigator?.devices);
  const checkouts = catalogWorkspaces(rest)
    .filter((workspace) => workspace.device_id === deviceId)
    .flatMap((workspace) => workspace.checkouts)
    .filter((checkout) => checkout.exists);
  const roots = await probePaths(checkouts.map((checkout) => checkout.path), (paths) => bridge.probePaths(paths));
  // An unanswered root might own the file: neither an external open nor a
  // source menu item is justified until ownership is known. The probe logs it.
  if (checkouts.some((checkout) => !roots.has(checkout.path))) return undefined;
  const owner = owningCheckout(found.real, checkouts, roots);
  return owner ? { checkout: owner.checkout, path: owner.checkout.path + found.real.slice(owner.root.length), deviceId } : null;
}

/**
 * The first candidate of each group that holds: a URL always, a path when
 * one of its lookups exists. Every lookup of every group goes out in one
 * probe.
 */
export async function resolveGroups(
  groups: LinkCandidate[][],
  context: { cwd: string | null; root: string | null } | null,
  probe: ((paths: string[]) => Promise<ProbedPath[]>) | null,
): Promise<{ candidate: LinkCandidate; resolved: TerminalContextLink }[]> {
  const lookups = new Map<LinkCandidate, string[]>();
  if (context && probe) {
    for (const candidate of groups.flat().sort((a, b) => Number(b.original) - Number(a.original))) {
      if (candidate.target.kind === "path") lookups.set(candidate, pathLookups(candidate.target.path, context.cwd, context.root));
    }
  }
  const answers = probe && lookups.size > 0 ? await probePaths([...lookups.values()].flat(), probe) : new Map<string, ProbedPath>();
  const chosen: { candidate: LinkCandidate; resolved: TerminalContextLink }[] = [];
  for (const group of groups) {
    for (const candidate of group) {
      if (candidate.target.kind === "url") {
        chosen.push({ candidate, resolved: { target: candidate.target, found: null } });
        break;
      }
      const paths = lookups.get(candidate) ?? [];
      let found: FoundPath | null = null;
      let unknown = false;
      for (const path of paths) {
        if (!answers.has(path)) { unknown = true; break; }
        const answer = answers.get(path);
        if (answer) { found = answer; break; }
      }
      // A skipped or failed higher-precedence spelling cannot justify a shorter link.
      if (unknown) break;
      if (found) {
        chosen.push({ candidate, resolved: { target: candidate.target, found } });
        break;
      }
    }
  }
  return chosen;
}

/** Per-terminal link state: whether the pointer is over a link, so a click on one is not also the program's. */
export type LinkState = { hovered: boolean; link?: { range: IBufferRange; resolved: TerminalContextLink } | null };

/** Whether a link's release is a click that opens it: any but the right button's, which opens the link's menu. */
function opens(event: MouseEvent): boolean {
  return event.button !== RIGHT_BUTTON;
}

function open(actions: TerminalLinkActions, resolved: TerminalContextLink, event: MouseEvent, selecting: boolean, paneId: string): void {
  // A press that dragged across the link selected its text; the release is not a click.
  if (selecting || !opens(event)) return;
  const external = holdsCommandKey(event);
  const { target, found } = resolved;
  if (target.kind === "url") actions.openLink(target.url, external, paneId);
  else if (found) actions.openTerminalPath(found, target.line, target.column, external, "auto", paneId);
}

function rangeOf(candidate: LinkCandidate, top: number): IBufferRange {
  const first = candidate.spans[0]!;
  const last = candidate.spans[candidate.spans.length - 1]!;
  return { start: { x: first.start + 1, y: top + first.row }, end: { x: last.end, y: top + last.row } };
}

function containsCell(range: IBufferRange, x: number, y: number): boolean {
  return y >= range.start.y && y <= range.end.y
    && (y !== range.start.y || x >= range.start.x)
    && (y !== range.end.y || x <= range.end.x);
}

/** The same bounded row read and cached path check for hover and right-click. */
async function linksOnRow(term: Terminal, paneId: string, y: number) {
  const buffer = term.buffer.active;
  const top = Math.max(1, y - JOIN_ROWS);
  const bottom = Math.min(buffer.length, y + JOIN_ROWS);
  const rows: CellRow[] = [];
  for (let line = top; line <= bottom; line += 1) rows.push(bufferRow(buffer.getLine(line - 1), term.cols));
  const groups = linkCandidates(rows, y - top);
  if (groups.length === 0) return [];
  const rest = useShellStore.getState().rest;
  const bridge = hostBridge();
  const chosen = await resolveGroups(groups, paneContext(rest, paneId, windowDeviceId(rest?.navigator?.devices)), bridge ? (paths) => bridge.probePaths(paths) : null);
  return chosen.map(({ candidate, resolved }) => ({ candidate, resolved, range: rangeOf(candidate, top) }));
}

/** A right-click's target, including an OSC 8 address behind a program's label. */
export async function terminalLinkAtCell(term: Terminal, paneId: string, state: LinkState, x: number, y: number): Promise<TerminalContextLink | null> {
  const hovered = state.link;
  // An OSC 8 address we refuse does not inherit a different address from its label.
  if (state.hovered && !hovered) return null;
  if (hovered && containsCell(hovered.range, x, y)) {
    const { target, found } = hovered.resolved;
    if (target.kind === "url" || found) return hovered.resolved;
    const bridge = hostBridge();
    const rest = useShellStore.getState().rest;
    if (!bridge || !paneContext(rest, paneId, windowDeviceId(rest?.navigator?.devices))) return null;
    const answers = await probePaths([target.path], (paths) => bridge.probePaths(paths));
    const answer = answers.get(target.path);
    return answer ? { target, found: answer } : null;
  }
  const links = await linksOnRow(term, paneId, y);
  return links.find(({ range }) => containsCell(range, x, y))?.resolved ?? null;
}

/**
 * Offers the links of the hovered row. `actions` is read at click time, so a
 * terminal that moves between panes' hosts always opens with the current one.
 */
export function registerTerminalLinks(term: Terminal, pane: string | (() => string), state: LinkState, actions: () => TerminalLinkActions): { dispose(): void } {
  const currentPane = () => typeof pane === "string" ? pane : pane();
  return term.registerLinkProvider({
    provideLinks(y, callback) {
      const paneId = currentPane();
      void linksOnRow(term, paneId, y).then((chosen) => {
        const links: ILink[] = chosen.map(({ candidate, resolved, range }) => ({
          range,
          text: candidate.text,
          activate: (event) => open(actions(), resolved, event, term.hasSelection(), currentPane()),
          hover: () => {
            state.hovered = true;
            state.link = { range, resolved };
          },
          leave: () => {
            state.hovered = false;
            state.link = null;
          },
        }));
        callback(links.length > 0 ? links : undefined);
      });
    },
  });
}

/**
 * The handler for links a program marked itself (OSC 8), in place of xterm's
 * confirm dialog. `file://` and `vscode://file/` addresses are paths and go
 * the way a detected path does; any other address is left alone and logged.
 */
export function osc8Handler(pane: string | (() => string), state: LinkState, actions: () => TerminalLinkActions, hasSelection: () => boolean): ILinkHandler {
  return {
    allowNonHttpProtocols: true,
    activate(event, uri) {
      const paneId = typeof pane === "string" ? pane : pane();
      if (hasSelection() || !opens(event)) return;
      const target = osc8Target(uri);
      const diagnostic = useShellStore.getState().noteDiagnostic;
      if (!target) {
        // The address is the program's text, so the log names only the refusal.
        diagnostic(`terminal link: ${target === null ? "an unreadable file" : "a link that is not http(s) or a file"} is not opened`);
        return;
      }
      if (target.kind === "url") {
        open(actions(), { target, found: null }, event, false, paneId);
        return;
      }
      const bridge = hostBridge();
      const shown = useShellStore.getState().rest;
      const context = paneContext(shown, paneId, windowDeviceId(shown?.navigator?.devices));
      if (!bridge || !context) {
        diagnostic("terminal link: a file link names a path this Mac cannot open here");
        return;
      }
      void probePaths([target.path], (paths) => bridge.probePaths(paths)).then((answers) => {
        // An unanswered path was logged by the check itself.
        if (!answers.has(target.path)) return;
        const found = answers.get(target.path) ?? null;
        if (found) open(actions(), { target, found }, event, false, typeof pane === "string" ? pane : pane());
        else diagnostic("terminal link: the linked file does not exist");
      });
    },
    hover(_event, uri, range) {
      state.hovered = true;
      const target = osc8Target(uri);
      state.link = target ? { range, resolved: { target, found: null } } : null;
    },
    leave() {
      state.hovered = false;
      state.link = null;
    },
  };
}
