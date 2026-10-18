import type { StoredBuffer } from "./buffers";
import type { TerminalFrame, UsageSnapshot } from "./generated/hided-ws";
import { create } from "zustand";
import type { ConnectionState } from "./connection";
import type { CoreLink, MoveView } from "./coreMove";
import { frontDeviceId, localDeviceId } from "./devices";
import { appliedAfter } from "./operatorFocus";
import { remoteContext, remoteView } from "./remote";
import { share } from "./share";
import {
  type AgentRow,
  type ChangesSnapshot,
  type DocumentsSection,
  type EditorDocumentSnapshot,
  type EditorSnapshot,
  type PaneFind,
  type RowMark,
  type SnapshotRest,
} from "./snapshot";

export type { AgentRow, SnapshotRest } from "./snapshot";
import type { MobileState } from "./mobileSettings";

/** The pane holding the keyboard on the device in front, as the core reports it. */
export function focusedPaneOf(rest: SnapshotRest): string | null {
  const remote = remoteContext(rest);
  if (remote) return remoteView(remote.session)?.focusedPaneId ?? null;
  return rest.terminal?.pane_id ?? rest.focused?.pane_id ?? null;
}

/** One pane's output in a `terminal` frame, from the screen-side hub. */
export type TerminalChunk = TerminalFrame["chunks"][number];

/** `inode` is the entry's own identity in a checkout listing, which a trash of the row confirms: decimal text, because a number would round a 64-bit id. */
export type DirectoryEntry = { name: string; path: string; is_directory: boolean; inode?: string };
export type DirectoryList = {
  /** The event that asked: `file_list`, the Explorer's. */
  kind: string;
  root_path: string;
  entries: DirectoryEntry[];
  truncated: boolean;
  /** The folder changed and is being read again; its rows stay drawn until the new listing lands. */
  stale?: boolean;
};
/** A device folder its helper could not list now (`directory_unavailable`); the reason is the helper's. */
export type DirectoryUnavailable = { device_id: string; root_path: string; code: string; message: string };
export type PathRefusal = { kind: string; path: string; reason: string };
/** hided's answer to where a clone would land: `path` when it may, `reason` when it may not. */
export type CloneTarget = { parent: string; name: string; path: string | null; reason: string | null };
/**
 * hided's answer to a `project_target` probe: where Create new project would
 * make `name` in `parent`, as sent, and what the create would be refused for.
 */
export type ProjectTarget = {
  parent: string;
  name: string;
  parent_path: string | null;
  /** The parent as `~` spells it. */
  parent_label: string | null;
  /** The folder the project takes; null until a name is checked. */
  path: string | null;
  /** An empty folder is already there and is used. */
  leftover: boolean;
  reason: string | null;
};
export type DirectoryChanged = { path: string };
export type FileIndexEntry = { path: string; relative_path: string };
export type FileIndexResult = {
  /** The device the root is on; the core's own node id for this Hide host. */
  device_id: string;
  root_path: string;
  query: string;
  /** Named `files`, not `entries`: the directory_list frame owns `entries`. */
  files: FileIndexEntry[];
  truncated: boolean;
  indexing: boolean;
  /** Why a device's helper could not walk the root; the next query walks again. */
  unavailable: string | null;
};

export type Frame = {
  type: string;
  message?: string;
  payload?: {
    revision?: number;
    terminal_sequence?: number;
    terminal_epoch?: string;
    rest?: SnapshotRest;
    rest_format?: "semantic-rest-v1";
    usage?: UsageSnapshot;
    editor?: EditorSnapshot | null;
    changes?: ChangesSnapshot | null;
    documents?: DocumentsSection | null;
    memory?: import("./snapshot").MemorySection;
    link_panel?: import("./snapshot").LinkPanel;
    link_summaries?: import("./snapshot").LinkSummaries;
    session_record?: import("./snapshot").SessionRecordSection | null;
    session_work_search?: import("./snapshot").SessionWorkSearch | null;
    factory?: import("./factory/model").FactorySection;
    /** Null when the open Task page closed. */
    factory_task?: import("./factory/model").FactoryTaskSection | null;
    find?: PaneFind;
    chunks?: TerminalChunk[];
  } & Partial<DirectoryList> &
    Partial<DirectoryUnavailable> &
    Partial<PathRefusal> &
    Partial<DirectoryChanged> &
    Partial<FileIndexResult>;
};

/** What the daemon says about itself after a handshake (hided `daemon` frame). */
export type DaemonInfo = {
  version: string;
  /** This daemon host's lasting identity; a draft is filed under it (S5.5 B9). */
  host_id: string;
  /** The machine the daemon runs on, which owns every value it stores (S5.5 B35); null when the system gives none. */
  host_name: string | null;
  schema_version: number;
  pid: number;
  started_at_unix: string;
  state_dir: string;
  core_state_path: string;
  herdr_bin_path: string | null;
  herdr_socket_path: string | null;
  keep_alive: boolean;
  idle_secs: number;
};

type Store = {
  connection: ConnectionState;
  /** The daemon this page is connected to; null until the first handshake. */
  daemon: DaemonInfo | null;
  /** Settings > Mobile as the daemon reports it (`mobile` frame); null until the first one. */
  mobile: MobileState | null;
  /** The core move as the process's supervisor reports it (`core_move` frame); null until the first one. */
  coreMove: MoveView | null;
  /** Why the last `core_move` event was refused (`move_busy`, ...); cleared by the next frame of the move. */
  coreMoveRefusal: string | null;
  /** Where this node's link to its core stands while its screen waits (`core_link` frame); null once the core draws it. */
  coreLink: CoreLink | null;
  revision: number;
  /** The hub's cursor after the last `terminal` frame; null after a whole snapshot until the hub names the new one. */
  terminalSequence: number | null;
  /** The epoch of the hub that gave `terminalSequence`: a cursor resumes only on that hub. */
  terminalEpoch: string | null;
  /** The core's rest section, structurally shared across frames (`share.ts`). */
  rest: SnapshotRest | null;
  /** Independently revisioned usage; history changes leave navigation untouched. */
  usage: UsageSnapshot | null;
  /**
   * The core's editor section, one of the three sections the core revisions on
   * its own. A delta carries it only when it changed, so a null in a delta
   * keeps what the store holds; a snapshot always carries every section.
   */
  editor: EditorSnapshot | null;
  /** The core's changes section, read the same way and for the same reason. */
  changes: ChangesSnapshot | null;
  /**
   * The documents the front Workspace's visible displays show, by editor tab
   * id (S7 contract 3.1). A snapshot replaces the map; a delta that carries
   * the section keeps the documents still visible, overwrites the ones that
   * changed and drops the rest; a delta without it keeps the map as it is.
   */
  documents: Documents;
  /**
   * The opened Project's Memory (`memory`), its own section: absent until a
   * shell opens one; a delta without it keeps what the store holds.
   */
  memory: import("./snapshot").MemorySection | null;
  /** The link record of the open PR or issue panel (`link_panel`); null until one opens. */
  linkPanel: import("./snapshot").LinkPanel | null;
  /** Each Project's link counts and chips (`link_summaries`). */
  linkSummaries: import("./snapshot").LinkSummaries | null;
  sessionRecord: import("./snapshot").SessionRecordSection | null;
  sessionWorkSearch: import("./snapshot").SessionWorkSearch | null;
  /**
   * The Factory screens' section (`factory`): the engine's summary and the
   * answers to this screen's requests. The core sends it only when the engine
   * moved, so a delta without it keeps what the store holds.
   */
  factory: import("./factory/model").FactorySection | null;
  /** The Task page the screen opened (`factory_task`); a delta's null closes it. */
  factoryTask: import("./factory/model").FactoryTaskSection | null;
  agents: AgentRow[];
  /**
   * The keyboard-focus pane of the context on screen: the core's
   * `terminal.pane_id` for this machine, the host's own focus for a selected
   * SSH device (`remote.ts`).
   */
  focusedPaneId: string | null;
  /**
   * The newest operator `focus_pane` number this page sent on the open
   * connection, and the newest the snapshot says the core applied. While the
   * second is behind the first, the snapshot's focused pane predates the
   * operator's last click and does not move DOM focus (`operatorFocus.ts`).
   */
  operatorFocusSent: number;
  operatorFocusApplied: number;
  herdrState: string | null;
  find: PaneFind | null;
  /** The Explorer's listings, one per expanded folder, oldest evicted past `LISTING_CAP`. */
  listings: Record<string, DirectoryList>;
  /** The last path hided refused; Add a project clears it before it sends a folder. */
  pathRefusal: PathRefusal | null;
  /** The last `clone_target` answer; Clone from URL reads the one for its own parent and name. */
  cloneTarget: CloneTarget | null;
  /** hided's last answer about where Create new project would make its folder. */
  projectTarget: ProjectTarget | null;
  /** The last device folder that could not be listed, until a listing for it arrives. */
  directoryUnavailable: DirectoryUnavailable | null;
  /** The ⌘P palette's last answer, keyed by the query it answered. */
  fileIndex: FileIndexResult | null;
  /** The last attachment the daemon refused, drawn as one line over its pane (B15). */
  attachmentRefusal: { pane_id: string; reason: string } | null;
  /** Tabs whose save is in flight; the strip shows only these (D-10). */
  savingTabs: Set<string>;
  /** Tabs whose buffer could not be stored; they say "kept in this tab only" (D-14). */
  bufferWarnings: Set<string>;
  /** The draft text last exported per tab, which no longer needs its tab to survive (B26, B44). */
  exportedDrafts: Map<string, string>;
  /** Stored drafts no open tab stands for, to open, export or discard (S5.5 B10-B12). */
  recoveryDrafts: StoredBuffer[];
  setRecoveryDrafts: (drafts: StoredBuffer[]) => void;
  /** Watch frames per folder, counted so the Explorer re-reads even a folder
   * whose listing was in flight when the change landed. */
  folderChanges: Record<string, number>;
  /** The newest `DIAGNOSTIC_CAP` entries; older ones are counted in `diagnosticsDropped`. */
  diagnostics: string[];
  diagnosticsDropped: number;
  /** Counts self-contained snapshots; every terminal re-requests its view on each. */
  viewGeneration: number;
  refused: boolean;
  setConnection: (connection: ConnectionState, refused?: boolean) => void;
  /** An operator focus left on the socket with this number. */
  noteOperatorFocusSent: (sequence: number) => void;
  /** A new connection starts with nothing in flight: what the old socket lost is not awaited. */
  releaseOperatorFocus: () => void;
  noteDiagnostic: (message: string) => void;
  clearPathRefusal: () => void;
  setAttachmentRefusal: (refusal: { pane_id: string; reason: string } | null) => void;
  noteSaving: (tabId: string, saving: boolean) => void;
  noteBufferWarning: (tabId: string, warned: boolean) => void;
  noteDraftExported: (tabId: string, contents: string) => void;
  /** Drops cached listings outright, for expanded folders past the watch cap; nothing is re-read for them. */
  invalidateListings: (paths: string[]) => void;
  /** Marks cached listings stale: the Explorer re-reads those folders and keeps drawing them meanwhile. */
  refreshListings: (paths: string[]) => void;
  applyFrame: (frame: Frame) => TerminalChunk[];
};

export type Documents = Readonly<Record<string, EditorDocumentSnapshot>>;

const NO_DOCUMENTS: Documents = {};

/**
 * One `documents` section applied to the map the store holds. An unchanged
 * document keeps its object, and a section that changes nothing returns the
 * same map, so a view of another document does not redraw for this one.
 */
function mergeDocuments(previous: Documents, section: DocumentsSection): Documents {
  const next: Record<string, EditorDocumentSnapshot> = {};
  for (const id of section.visible) {
    const kept = previous[id];
    if (kept) next[id] = kept;
  }
  for (const change of section.changed) next[change.tab_id] = change.document;
  const ids = Object.keys(next);
  const same = ids.length === Object.keys(previous).length && ids.every((id) => previous[id] === next[id]);
  return same ? previous : next;
}

const KNOWN_GROUPS = new Set(["needs_you", "done", "working", "seen"]);
// Every mark the type names, so a mark the core adds cannot be read as unknown here.
const KNOWN_MARKS = new Set(Object.keys({ turn: true, going: true, waiting: true, done: true, unread: true, quiet: true, unknown: true, asleep: true } satisfies Record<RowMark, true>));
export const DIAGNOSTIC_CAP = 200;

/** Expanded folders the store keeps a listing for; the PRD's watch cap is the same number. */
export const LISTING_CAP = 64;

/** Watch-frame counts the store keeps; far above the 64-folder watch set. */
export const FOLDER_CHANGE_CAP = 512;

function withListing(
  listings: Record<string, DirectoryList>,
  listing: DirectoryList,
): Record<string, DirectoryList> {
  const next: Record<string, DirectoryList> = { ...listings, [listing.root_path]: listing };
  const paths = Object.keys(next);
  const overflow = Math.max(0, paths.length - LISTING_CAP);
  for (const path of paths.slice(0, overflow)) delete next[path];
  return next;
}

function withDiagnostics(
  diagnostics: string[],
  dropped: number,
  added: string[],
): { diagnostics: string[]; diagnosticsDropped: number } {
  if (added.length === 0) return { diagnostics, diagnosticsDropped: dropped };
  const all = [...diagnostics, ...added];
  const overflow = Math.max(0, all.length - DIAGNOSTIC_CAP);
  return { diagnostics: all.slice(overflow), diagnosticsDropped: dropped + overflow };
}

export const useShellStore = create<Store>((set, get) => ({
  connection: "connecting",
  daemon: null,
  mobile: null,
  coreMove: null,
  coreMoveRefusal: null,
  coreLink: null,
  revision: 0,
  terminalSequence: null,
  terminalEpoch: null,
  rest: null,
  usage: null,
  editor: null,
  changes: null,
  documents: NO_DOCUMENTS,
  memory: null,
  linkPanel: null,
  linkSummaries: null,
  sessionRecord: null,
  sessionWorkSearch: null,
  factory: null,
  factoryTask: null,
  agents: [],
  focusedPaneId: null,
  operatorFocusSent: 0,
  operatorFocusApplied: 0,
  herdrState: null,
  find: null,
  listings: {},
  pathRefusal: null,
  cloneTarget: null,
  projectTarget: null,
  directoryUnavailable: null,
  fileIndex: null,
  attachmentRefusal: null,
  savingTabs: new Set<string>(),
  bufferWarnings: new Set<string>(),
  exportedDrafts: new Map<string, string>(),
  recoveryDrafts: [],
  setRecoveryDrafts: (recoveryDrafts) => set({ recoveryDrafts }),
  folderChanges: {},
  diagnostics: [],
  diagnosticsDropped: 0,
  viewGeneration: 0,
  refused: false,
  setConnection: (connection, refused = false) => set({ connection, refused }),
  noteOperatorFocusSent: (sequence) => set({ operatorFocusSent: sequence }),
  releaseOperatorFocus: () => set({ operatorFocusSent: get().operatorFocusApplied }),
  noteDiagnostic: (message) =>
    set(withDiagnostics(get().diagnostics, get().diagnosticsDropped, [message])),
  clearPathRefusal: () => {
    if (get().pathRefusal) set({ pathRefusal: null });
  },
  setAttachmentRefusal: (attachmentRefusal) => set({ attachmentRefusal }),
  noteDraftExported: (tabId, contents) => {
    const next = new Map(get().exportedDrafts);
    next.set(tabId, contents);
    set({ exportedDrafts: next });
  },
  noteBufferWarning: (tabId, warned) => {
    const current = get().bufferWarnings;
    if (warned === current.has(tabId)) return;
    const next = new Set(current);
    if (warned) next.add(tabId);
    else next.delete(tabId);
    // A draft stored here again no longer needs the text it was exported as.
    if (!warned && get().exportedDrafts.has(tabId)) {
      const exportedDrafts = new Map(get().exportedDrafts);
      exportedDrafts.delete(tabId);
      set({ bufferWarnings: next, exportedDrafts });
      return;
    }
    set({ bufferWarnings: next });
  },
  noteSaving: (tabId, saving) => {
    const current = get().savingTabs;
    if (saving === current.has(tabId)) return;
    const next = new Set(current);
    if (saving) next.add(tabId);
    else next.delete(tabId);
    set({ savingTabs: next });
  },
  invalidateListings: (paths) => {
    const listings = get().listings;
    if (!paths.some((path) => path in listings)) return;
    const next = { ...listings };
    for (const path of paths) delete next[path];
    set({ listings: next });
  },
  refreshListings: (paths) => {
    const listings = get().listings;
    const changed = paths.filter((path) => listings[path] && !listings[path].stale);
    if (changed.length === 0) return;
    const next = { ...listings };
    for (const path of changed) next[path] = { ...listings[path]!, stale: true };
    set({ listings: next });
  },
  applyFrame: (frame) => {
    const payload = frame.payload ?? {};
    // The core revisions `editor`, `changes`, `documents` and the named
    // the link and record sections on their own, beside `rest`: a snapshot
    // carries every section it has, a delta only the ones that changed since
    // the client's revision. They land before the routing below, which
    // answers a listing or a refusal with an early return.
    if (frame.type === "snapshot") {
      set({
        // The core draws this screen again: the node's wait is over.
        coreLink: null,
        editor: payload.editor ?? null,
        changes: payload.changes ?? null,
        documents: payload.documents ? mergeDocuments(NO_DOCUMENTS, payload.documents) : NO_DOCUMENTS,
        memory: payload.memory ?? null,
        linkPanel: payload.link_panel ?? null,
        linkSummaries: payload.link_summaries ?? null,
        sessionRecord: payload.session_record ?? null,
        sessionWorkSearch: payload.session_work_search ?? null,
        factory: payload.factory ?? null,
        factoryTask: payload.factory_task ?? null,
      });
    } else if (
      payload.editor !== undefined ||
      payload.changes !== undefined ||
      payload.documents !== undefined ||
      payload.memory !== undefined ||
      payload.link_panel !== undefined || payload.link_summaries !== undefined ||
      payload.session_record !== undefined || payload.session_work_search !== undefined ||
      payload.factory !== undefined || payload.factory_task !== undefined
    ) {
      set({
        editor: payload.editor ?? get().editor,
        changes: payload.changes ?? get().changes,
        documents: payload.documents ? mergeDocuments(get().documents, payload.documents) : get().documents,
        memory: payload.memory ? share(get().memory, payload.memory) : get().memory,
        linkPanel: payload.link_panel ? share(get().linkPanel, payload.link_panel) : get().linkPanel,
        linkSummaries: payload.link_summaries ? share(get().linkSummaries, payload.link_summaries) : get().linkSummaries,
        sessionRecord: payload.session_record !== undefined ? payload.session_record && share(get().sessionRecord, payload.session_record) : get().sessionRecord,
        sessionWorkSearch: payload.session_work_search !== undefined ? payload.session_work_search && share(get().sessionWorkSearch, payload.session_work_search) : get().sessionWorkSearch,
        factory: payload.factory ? share(get().factory, payload.factory) : get().factory,
        factoryTask: payload.factory_task !== undefined ? payload.factory_task && share(get().factoryTask, payload.factory_task) : get().factoryTask,
      });
    }
    if (frame.type === "directory_list") {
      const listing: DirectoryList = {
        kind: payload.kind ?? "",
        root_path: payload.root_path ?? "",
        entries: payload.entries ?? [],
        truncated: payload.truncated ?? false,
      };
      if (listing.kind === "file_list") {
        // A listing belongs to the device that answered it; one for a device
        // no longer selected would show its rows under this device's paths.
        if ((payload.device_id || localDeviceId(get().rest)) !== frontDeviceId(get().rest)) return [];
        const unavailable = get().directoryUnavailable;
        set({
          listings: withListing(get().listings, listing),
          directoryUnavailable: unavailable?.root_path === listing.root_path ? null : unavailable,
        });
      } else {
        get().noteDiagnostic(`directory_list without a known kind=${listing.kind || "none"}`);
      }
      return [];
    }
    if (frame.type === "directory_unavailable") {
      set({
        directoryUnavailable: {
          device_id: payload.device_id ?? "",
          root_path: payload.root_path ?? "",
          code: payload.code ?? "",
          message: payload.message ?? "",
        },
      });
      return [];
    }
    if (frame.type === "path_refused") {
      set({
        pathRefusal: { kind: payload.kind ?? "", path: payload.path ?? "", reason: payload.reason ?? "" },
      });
      return [];
    }
    if (frame.type === "clone_target") {
      const answer = payload as Partial<CloneTarget>;
      set({
        cloneTarget: { parent: answer.parent ?? "", name: answer.name ?? "", path: answer.path ?? null, reason: answer.reason ?? null },
      });
      return [];
    }
    if (frame.type === "project_target") {
      const answer = payload as Partial<ProjectTarget>;
      set({
        projectTarget: {
          parent: answer.parent ?? "",
          name: answer.name ?? "",
          parent_path: answer.parent_path ?? null,
          parent_label: answer.parent_label ?? null,
          path: answer.path ?? null,
          leftover: answer.leftover ?? false,
          reason: answer.reason ?? null,
        },
      });
      return [];
    }
    if (frame.type === "directory_changed") {
      // A watched folder moved; its cached listing turns stale, so its rows
      // stay drawn until the re-read lands instead of vanishing for the round
      // trip, and the count lets the Explorer re-read even a listing that was
      // still in flight.
      // Listings are the shown device's, so another device's frame names a
      // path that is not the one listed here (S5.5 B2).
      const path = payload.path;
      const shownDevice = frontDeviceId(get().rest);
      if (path && (payload.device_id || localDeviceId(get().rest)) === shownDevice) {
        get().refreshListings([path]);
        // A bounded map ordered by recency: the count is re-inserted so a
        // folder that keeps changing is the last one evicted.
        const next = { ...get().folderChanges };
        delete next[path];
        next[path] = (get().folderChanges[path] ?? 0) + 1;
        const paths = Object.keys(next);
        const overflow = paths.length - FOLDER_CHANGE_CAP;
        for (const stale of overflow > 0 ? paths.slice(0, overflow) : []) delete next[stale];
        set({ folderChanges: next });
      }
      return [];
    }
    if (frame.type === "open_external_result") return [];
    if (frame.type === "daemon") {
      set({ daemon: frame.payload as unknown as DaemonInfo });
      return [];
    }
    if (frame.type === "mobile") {
      set({ mobile: frame.payload as unknown as MobileState });
      return [];
    }
    if (frame.type === "core_move") {
      set({ coreMove: frame.payload as unknown as MoveView, coreMoveRefusal: null });
      return [];
    }
    if (frame.type === "core_link") {
      set({ coreLink: frame.payload as unknown as CoreLink });
      return [];
    }
    if (frame.type === "file_index_result") {
      set({
        fileIndex: {
          device_id: payload.device_id || localDeviceId(get().rest),
          unavailable: payload.unavailable ?? null,
          root_path: payload.root_path ?? "",
          query: payload.query ?? "",
          files: payload.files ?? [],
          truncated: payload.truncated ?? false,
          indexing: payload.indexing ?? false,
        },
      });
      return [];
    }
    if (frame.type === "error") {
      const refused = frame.payload as { kind?: string; reason?: string } | undefined;
      if (refused?.kind === "core_move" && refused.reason) set({ coreMoveRefusal: refused.reason });
      get().noteDiagnostic(`hided error: ${frame.message ?? "unknown"}`);
      return [];
    }
    // Pane output comes from the hub beside the core's frames, never inside them.
    if (frame.type === "terminal") {
      set({
        terminalSequence: payload.terminal_sequence ?? get().terminalSequence,
        terminalEpoch: payload.terminal_epoch ?? get().terminalEpoch,
      });
      return payload.chunks ?? [];
    }
    const cursors = {
      usage: frame.type === "snapshot"
        ? payload.usage ?? null
        : payload.usage && (payload.revision ?? get().revision) >= get().revision ? share(get().usage, payload.usage) : get().usage,
      revision: frame.type === "snapshot" ? (payload.revision ?? get().revision) : Math.max(payload.revision ?? 0, get().revision),
      // A whole snapshot resets every terminal, so the old cursor names
      // output those terminals no longer hold.
      terminalSequence: frame.type === "snapshot" ? null : get().terminalSequence,
      terminalEpoch: frame.type === "snapshot" ? null : get().terminalEpoch,
      find: payload.find ? share(get().find, payload.find) : get().find,
    };
    if (frame.type === "snapshot" || payload.rest) {
      const previous = get().rest;
      // Negotiated semantic frames retain omitted nested fields. Legacy
      // schema-2 frames carry complete nested sections and omitted Workspace
      // still clears it, including when connecting to an older daemon.
      const incoming =
        frame.type === "snapshot"
          ? payload.rest ?? {}
          : payload.rest_format === "semantic-rest-v1"
            ? {
                ...previous,
                ...payload.rest,
                ...(payload.rest?.navigator ? { navigator: { ...previous?.navigator, ...payload.rest.navigator } } : {}),
                ...(payload.rest?.terminal ? { terminal: { ...previous?.terminal, ...payload.rest.terminal } } : {}),
                ...(payload.rest?.status ? { status: { ...previous?.status, ...payload.rest.status } } : {}),
              }
            : { ...previous, ...payload.rest, workspace_view: payload.rest?.workspace_view };
      const rest = share(previous, incoming);
      const diagnostics: string[] = [];
      const agents =
        rest.navigator?.agents === previous?.navigator?.agents
          ? get().agents
          : (rest.navigator?.agents ?? []).map((agent) => {
              // A value this shell cannot read draws as a state Hide cannot
              // vouch for: no mark, dimmed, `상태 모름` (status-marks B16).
              if (agent.group && !KNOWN_GROUPS.has(agent.group)) {
                diagnostics.push(`unknown enum group=${agent.group}`);
                return { ...agent, group: "unknown", unknown: true, mark: "unknown" as const, status_code: "unknown" as const };
              }
              if (!KNOWN_MARKS.has(agent.mark)) {
                diagnostics.push(`unknown enum mark=${String(agent.mark)}`);
                return { ...agent, unknown: true, mark: "unknown" as const, status_code: "unknown" as const };
              }
              return agent;
            });
      const lastError = rest.status?.last_error;
      if (lastError && lastError.occurred_at !== previous?.status?.last_error?.occurred_at) {
        diagnostics.push(`${lastError.kind}: ${lastError.message}`);
      }
      // Listings are one device's folders; switching devices starts empty.
      const deviceChanged = frontDeviceId(rest) !== frontDeviceId(previous ?? null);
      set({
        rest,
        agents,
        ...(deviceChanged ? { listings: {}, directoryUnavailable: null } : {}),
        ...withDiagnostics(get().diagnostics, get().diagnosticsDropped, diagnostics),
        viewGeneration: frame.type === "snapshot" ? get().viewGeneration + 1 : get().viewGeneration,
        ...cursors,
        focusedPaneId: focusedPaneOf(rest),
        operatorFocusApplied: appliedAfter(rest, get()),
        herdrState: rest.status?.herdr?.state ?? get().herdrState,
      });
    } else {
      set(cursors);
    }
    return [];
  },
}));
