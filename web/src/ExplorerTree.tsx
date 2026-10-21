import { useVirtualizer } from "@tanstack/react-virtual";
import { ChevronRightIcon, ChevronsDownUpIcon, FilePlusIcon, FolderPlusIcon, LoaderCircleIcon, RefreshCwIcon, SearchIcon } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import type { Actions } from "./actions";
import { isHtmlFile } from "./browserViews";
import { holdShellDrag } from "./shellDrag";
import { EntryPointMenu } from "./components/entry-menu";
import { FileMark } from "./components/file-mark";
import { Badge } from "./components/ui/badge";
import { Button } from "./components/ui/button";
import { useEscapeLayer } from "./components/ui/layer";
import { Input } from "./components/ui/input";
import { Hint } from "./components/ui/tooltip";
import {
  explorerMenuItems,
  explorerRows,
  filteredRows,
  firstChildSelection,
  explorerGitMark,
  gitBadgeColor,
  moveSelection,
  parentPath,
  parentSelection,
  pasteRefusal,
  pasteTarget,
  rowAccessibilityLabel,
  relativeTo,
  rowForPath,
  rowTitle,
  selectionAfterRemoval,
  treeCommandFor,
  type ExplorerRow,
  helperNeedsSettings,
} from "./explorer";
import { windowDeviceId } from "./screenMachine";
import { changesFor, explorerContext, localDeviceId } from "./snapshot";
import { useShellStore } from "./store";
import { useUiStore, type ExplorerDraft } from "./ui";
import { captureDrop, carriesOsFiles } from "./explorerDrop";
import { drawnViews } from "./viewFocus";
import { besideUnavailable } from "./viewLayout";
import { expandedUnderRoot, watchedFolders } from "./watch";
import { workspaceViewOf } from "./workspace";
import { hostKind, keySystem, revealHost } from "./host";
import { chordFromEvent, hostRegistry } from "./shortcuts";
import { commandLabel } from "./shortcutLabels";
import { useInterfaceTranslation } from "./i18n/client";

// The Explorer tree (PRD B1, B3, B9, B10): a lazy tree over the focused
// checkout. The core owns which folders are expanded (`ui_state.expanded_paths`)
// and every filesystem change (`explorer_operation`); hided answers one listing
// per folder, and the rows are derived from those plus the core's changed-file
// set (`explorer.ts`). Nothing here reads the disk or runs git; a row is a pure
// function of state that already arrived.

const EMPTY_PATHS: string[] = [];
const EMPTY_ROWS: ExplorerRow[] = [];

/** The root row's creation and fold buttons show on hover and focus, and always where there is no hover; they keep their place in the layout either way, so the name never moves when they appear. */
const ROOT_CONTROL = "opacity-0 group-hover/root:opacity-100 group-focus-within/root:opacity-100 focus-visible:opacity-100 [@media(hover:none)]:opacity-100";

/** One drawn row: an entry, or the inline name field the operator is editing. */
type DisplayRow =
  | { kind: "entry"; row: ExplorerRow }
  | { kind: "draft"; depth: number; parent: string; path: string; initial: string; draftKind: ExplorerDraft["kind"] };

/** The rows with the inline field spliced in: after the folder a new entry
 * lands in, or in place of the entry being renamed. */
export function displayRows(rows: ExplorerRow[], draft: ExplorerDraft | null): DisplayRow[] {
  const base: DisplayRow[] = rows.map((row) => ({ kind: "entry", row }));
  if (!draft) return base;
  if (draft.kind === "rename") {
    const index = rows.findIndex((row) => row.path === draft.path);
    const row = index === -1 ? undefined : rows[index];
    if (!row) return base;
    base.splice(index, 1, { kind: "draft", depth: row.depth, parent: draft.parent, path: draft.path, initial: draft.initial, draftKind: "rename" });
    return base;
  }
  const index = rows.findIndex((row) => row.path === draft.parent);
  const anchor = index === -1 ? undefined : rows[index];
  const depth = anchor ? anchor.depth + 1 : 0;
  const at = index === -1 ? 0 : index + 1;
  base.splice(at, 0, { kind: "draft", depth, parent: draft.parent, path: draft.parent, initial: draft.initial, draftKind: draft.kind });
  return base;
}

/** The row height the virtual list lays out with; 22 is the outline row
 * token, read here rather than written as a second number. */
function rowHeight(): number {
  const value = getComputedStyle(document.documentElement).getPropertyValue("--size-pane-child-row");
  return Number.parseFloat(value) || 24;
}

/** Where a row of `depth` starts its chevron: the tree's inset, then one
 * indent per level, VS Code's twistie column. */
function rowInset(depth: number): string {
  return `calc(var(--spacing-sm) + var(--size-tree-indent) * ${depth})`;
}

/** One hairline per ancestor level, under that ancestor's chevron, so a deep
 * row shows which folder it sits in; each row draws its own segment. */
function IndentGuides({ depth }: { depth: number }) {
  return Array.from({ length: depth }, (_, level) => (
    <span
      key={level}
      aria-hidden="true"
      data-explorer-guide={level}
      className="pointer-events-none absolute inset-y-0 w-(--size-hairline) bg-border"
      style={{ left: `calc(${rowInset(level)} + var(--size-tree-mark) / 2)` }}
    />
  ));
}

function baseName(path: string): string {
  const parts = path.split("/").filter(Boolean);
  return parts[parts.length - 1] ?? path;
}

/**
 * `hint` is the line under the filter and `footer` the line at the bottom,
 * which the lane's tree column fills (PRD right-lane B9, B12).
 */
export function ExplorerTree({ actions, hint, footer }: { actions: Actions; hint?: string; footer?: string }) {
  const { t } = useInterfaceTranslation();
  const checkout = useShellStore((s) => explorerContext(s.rest).checkout);
  const device = useShellStore((s) => explorerContext(s.rest).device);
  const rootPath = checkout?.path ?? null;
  const expandedPaths = useShellStore((s) => explorerContext(s.rest).expanded ?? EMPTY_PATHS);
  const listings = useShellStore((s) => s.listings);
  // Git decorations come from the checkout's own host on either device; the
  // core drops a list read for another device before this could show it.
  const changes = useShellStore((s) => changesFor(s.changes, s.rest?.navigator?.changes_root_path ?? null));
  const unavailable = useShellStore((s) => (s.directoryUnavailable && s.directoryUnavailable.device_id === device ? s.directoryUnavailable : null));
  const needsSettings = useShellStore((s) => helperNeedsSettings(s.rest?.navigator?.devices?.find((row) => row.id === device)?.host));
  const selectedPath = useShellStore((s) => s.rest?.ui_state?.selected_path ?? null);
  const pathRefusal = useShellStore((s) => s.pathRefusal);
  const operation = useShellStore((s) => s.rest?.explorer_operation ?? null);
  const folderChanges = useShellStore((s) => s.folderChanges);
  const invalidateListings = useShellStore((s) => s.invalidateListings);
  const refreshListings = useShellStore((s) => s.refreshListings);
  const selection = useUiStore((s) => s.explorerSelection);
  const setSelection = useUiStore((s) => s.setExplorerSelection);
  const draft = useUiStore((s) => s.explorerDraft);
  const setDraft = useUiStore((s) => s.setExplorerDraft);
  const clipboard = useUiStore((s) => s.explorerClipboard);
  const setClipboard = useUiStore((s) => s.setExplorerClipboard);
  const notice = useUiStore((s) => s.explorerNotice);
  const setNotice = useUiStore((s) => s.setExplorerNotice);
  // The folder a drag from the file manager would land in, drawn like the folder an in-tree drag is over.
  const [osTarget, setOsTarget] = useState<string | null>(null);
  const scrollRef = useRef<HTMLDivElement>(null);
  // The filter narrows the rows to the files hided's index (the ⌘P source)
  // matched; it is page-local and leaves the core's expansion untouched.
  const [filter, setFilter] = useState("");
  const needle = filter.trim();
  const filtering = needle !== "";
  const overlay = useUiStore((s) => s.overlay);
  const answer = useShellStore((s) =>
    s.fileIndex && s.fileIndex.device_id === device && s.fileIndex.root_path === rootPath && s.fileIndex.query === needle ? s.fileIndex : null,
  );
  // The last answer stays drawn while the next keystroke's is in flight, so
  // the tree does not blank between letters.
  const [settled, setSettled] = useState<typeof answer>(null);
  const shownAnswer = answer ?? (filtering ? settled : null);
  useEffect(() => {
    if (!filtering) setSettled(null);
    else if (answer) setSettled(answer);
  }, [filtering, answer]);
  const [refreshTick, setRefreshTick] = useState(0);
  const [menu, setMenu] = useState<{ path: string; isDirectory: boolean; x: number; y: number } | null>(null);
  // Folders this pane has asked hided for and not yet heard about; a refusal
  // keeps its entry so a refused folder is not re-asked on every render.
  const pending = useRef<Set<string>>(new Set());
  const pendingName = useRef<ExplorerDraft | null>(null);
  /** The watch-frame count already acted on, per folder. */
  const seenChanges = useRef<Record<string, number>>({});

  const gitMark = explorerGitMark(changes, t);
  const expandedKey = expandedPaths.join("\n");
  const rows = useMemo(() => {
    if (!rootPath) return EMPTY_ROWS;
    if (filtering) return filteredRows({ rootPath, files: shownAnswer?.files ?? [], changes });
    return explorerRows({ rootPath, listings, expandedPaths, changes });
    // `expandedPaths` arrives as a fresh array per section; the joined key is
    // the identity that matters, and the listings/changes objects are shared.
  }, [rootPath, listings, expandedKey, changes, filtering, shownAnswer]);
  const shown = useMemo(() => displayRows(rows, filtering ? null : draft), [rows, draft, filtering]);
  const watchedNow = useMemo(
    () => new Set(rootPath ? watchedFolders(rootPath, expandedPaths) : []),
    // `expandedPaths` identity changes per section; the joined key is the one
    // that matters, exactly as the row memo above reads it.
    [rootPath, expandedKey],
  );

  // Only the watched folders are listed and kept live: the root and the most
  // recently expanded ones. A folder past the cap loses its listing and draws
  // the refresh badge instead (B2); pressing it makes that folder most recent.
  useEffect(() => {
    if (!rootPath) return;
    const watched = watchedFolders(rootPath, expandedPaths);
    const watchedNow = new Set(watched);
    // A watch frame that landed while a listing was in flight must be re-read:
    // its folder drops its pending marker and its (possibly older) listing
    // turns stale, still drawn until the new one lands.
    const bumped: string[] = [];
    for (const [path, count] of Object.entries(folderChanges)) {
      if ((seenChanges.current[path] ?? 0) === count) continue;
      seenChanges.current[path] = count;
      if (!watchedNow.has(path)) continue;
      pending.current.delete(path);
      bumped.push(path);
    }
    if (bumped.length > 0) refreshListings(bumped);
    // Keep only the counts the store still holds, so a capped map does not
    // leave this ref growing for the session.
    for (const path of Object.keys(seenChanges.current)) {
      if (!(path in folderChanges)) delete seenChanges.current[path];
    }
    invalidateListings(
      expandedUnderRoot(rootPath, expandedPaths).filter((path) => path !== rootPath && !watchedNow.has(path)),
    );
    const fresh = (folder: string) => listings[folder] !== undefined && !listings[folder].stale;
    for (const folder of watched) {
      if (fresh(folder) || pending.current.has(folder)) continue;
      pending.current.add(folder);
      actions.listChildren(rootPath, folder);
    }
    for (const folder of [...pending.current]) {
      if (fresh(folder)) pending.current.delete(folder);
    }
  }, [rootPath, expandedPaths, listings, refreshTick, folderChanges, actions, invalidateListings, refreshListings]);

  // A device folder refused while its helper was still starting is asked
  // again once the helper is ready, as the views read their files again then
  // (S7 B14, B16); otherwise a restored device Workspace kept its Explorer
  // on the helper's old refusal until the operator pressed Retry.
  const hostReady = useShellStore(
    (s) => device !== localDeviceId(s.rest) && s.rest?.navigator?.devices?.find((row) => row.id === device)?.host?.state === "ready",
  );
  const wasReady = useRef(hostReady);
  useEffect(() => {
    const became = hostReady && !wasReady.current;
    wasReady.current = hostReady;
    if (!became) return;
    pending.current.clear();
    setRefreshTick((tick) => tick + 1);
  }, [hostReady]);

  // One request per pause in typing, as the palette sends. The palette shares
  // the one answer slot, so a request waits while it is open and is sent again
  // when it closes. The first query for a checkout starts the walk and answers
  // `indexing`; it is asked again until the list is there.
  useEffect(() => {
    if (!rootPath || !filtering || overlay === "file_palette") return undefined;
    const delay = answer?.indexing ? 250 : answer ? null : 120;
    if (delay === null) return undefined;
    const timer = window.setTimeout(() => actions.requestFileIndex(rootPath, needle, device), delay);
    return () => window.clearTimeout(timer);
  }, [rootPath, needle, filtering, device, overlay, answer, actions]);

  // Escape clears the filter before any layer opened earlier.
  useEscapeLayer(filtering, () => setFilter(""));

  // A reveal (or an open from anywhere) sets the core's selected_path. A
  // palette pick also sets the local cursor before the core replies; a panel
  // remount must not replace that newer pick with the older snapshot path.
  const previousCoreSelection = useRef(selectedPath);
  useEffect(() => {
    if (selectedPath && (previousCoreSelection.current !== selectedPath || !useUiStore.getState().explorerSelection)) {
      setSelection(selectedPath);
    }
    previousCoreSelection.current = selectedPath;
  }, [selectedPath, setSelection]);

  // A panel remount keeps the current selection; only a real checkout switch
  // invalidates its cursor and pending inline name.
  const previousRoot = useRef(rootPath);
  useEffect(() => {
    if (previousRoot.current === rootPath) return;
    previousRoot.current = rootPath;
    const selected = useShellStore.getState().rest?.ui_state?.selected_path ?? null;
    setSelection(rootPath && selected?.startsWith(`${rootPath}/`) ? selected : null);
    pending.current.clear();
    setMenu(null);
    setDraft(null);
    setNotice(null);
    setFilter("");
    pendingName.current = null;
  }, [rootPath, setSelection, setDraft, setNotice]);

  // A new change replaces the failure line the page drew for the last one.
  useEffect(() => {
    if (operation) setNotice(null);
  }, [operation?.id, setNotice]);

  // A copy or a drop hided refused at the checkout boundary is a failure line too.
  useEffect(() => {
    if (pathRefusal && (pathRefusal.kind === "path_copy" || pathRefusal.kind === "path_import")) {
      // The reason is a code the operator cannot act on (the path left the checkout); the line says what did not happen.
      setNotice({ path: pathRefusal.path, message: t(pathRefusal.kind === "path_copy" ? "explorer.copyRefused" : "explorer.dropFailed") });
    }
  }, [pathRefusal, setNotice, t]);

  const lastOperation = useRef<number | null>(null);
  useEffect(() => {
    if (!operation || operation.phase === "working") return;
    if (operation.id === lastOperation.current) return;
    lastOperation.current = operation.id;
    const submitted = pendingName.current;
    if (submitted) {
      const expected = submitted.kind === "rename" ? submitted.path : `${submitted.parent}/${submitted.initial}`;
      const matches = operation.path === expected || (operation.kind === "refused" && operation.path === submitted.parent);
      if (matches) {
        pendingName.current = null;
        // The failed operation leaves its typed name in an editable row, so
        // the operator can correct a collision without starting over (B10).
        if (operation.phase === "failed") setDraft(submitted);
      }
    }
    // A settled change moved a row: the folders it touched are re-read, and
    // the created item is selected so the operator sees it (B9).
    const folders = [parentPath(operation.path), parentPath(operation.destination)];
    refreshListings(folders);
    for (const folder of folders) pending.current.delete(folder);
    if (operation.phase === "finished") {
      setSelection(operation.destination);
      // What a paste or a drop added is shown: the folder it landed in opens.
      if (operation.kind === "path_copy" || operation.kind === "path_import") expand(parentPath(operation.destination));
    }
    setRefreshTick((tick) => tick + 1);
  }, [operation, refreshListings, setSelection]);

  const virtualizer = useVirtualizer({
    count: shown.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: rowHeight,
    overscan: 12,
  });

  useEffect(() => {
    const index = rows.findIndex((row) => row.path === selection);
    if (index >= 0) virtualizer.scrollToIndex(index, { align: "auto" });
    // `shown` shifts when the inline field is spliced in; the scroll follows rows.
  }, [selection, rows, virtualizer]);

  // A filtered tree has every folder open; pressing one must not move the
  // expansion the operator will come back to.
  const toggleFolder = (path: string) => {
    if (filtering) return;
    const next = new Set(expandedPaths);
    if (next.has(path)) next.delete(path);
    else next.add(path);
    actions.setExpandedPaths([...next]);
  };

  const expand = (path: string) => {
    if (filtering || expandedPaths.includes(path)) return;
    actions.setExpandedPaths([...expandedPaths, path]);
  };

  const openRow = (row: ExplorerRow, preview: boolean) => {
    setSelection(row.path);
    if (row.isDirectory) toggleFolder(row.path);
    else if (isHtmlFile(row.path)) actions.openInBrowser(row.path);
    else actions.openFile(row.path, preview);
  };

  const beginCreate = (parent: string, kind: "file" | "folder") => {
    setFilter("");
    expand(parent);
    setDraft({ kind, parent, path: parent, initial: "" });
    setMenu(null);
  };

  const beginRename = (row: ExplorerRow) => {
    setFilter("");
    setDraft({ kind: "rename", parent: parentPath(row.path), path: row.path, initial: row.name });
    setMenu(null);
  };

  const requestTrash = (row: ExplorerRow) => {
    actions.requestTrash(row.path, row.name, row.isDirectory, selectionAfterRemoval(rows, row.path, rootPath ?? row.path), row.inode);
    setMenu(null);
  };

  const copyPathOf = (path: string, relative: boolean) => {
    if (!rootPath) return;
    actions.copyText(relative ? relativeTo(path, rootPath) : path, relative ? "relative path" : "path");
    setMenu(null);
  };

  // Cut and Copy take the item the menu or the cursor names; Paste puts it in a
  // folder of the same checkout. A cut item moves on paste (`path_move`), a copy
  // is copied (`path_copy`) under the name the host picks when one is taken.
  const takeItem = (row: ExplorerRow | null, mode: "copy" | "cut") => {
    setMenu(null);
    if (!row || !rootPath) return;
    setNotice(null);
    setClipboard({ mode, path: row.path, isDirectory: row.isDirectory, root: rootPath, device });
  };

  const pasteInto = (folder: string) => {
    setMenu(null);
    const held = useUiStore.getState().explorerClipboard;
    if (!held || !rootPath) return;
    setNotice(null);
    const refusal = pasteRefusal(held, folder, rootPath, device);
    if (refusal) {
      setNotice({ path: folder, message: t(refusal) });
      return;
    }
    if (held.mode === "copy") {
      actions.copyEntry(held.path, folder);
      return;
    }
    // A cut item pasted into the folder it is in has nowhere to go.
    setClipboard(null);
    if (parentPath(held.path) !== folder) actions.moveEntry(held.path, folder);
  };

  const duplicate = (row: ExplorerRow) => {
    setMenu(null);
    actions.copyEntry(row.path, parentPath(row.path));
  };

  const onKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    const row = rowForPath(rows, selection);
    // Move to Trash (⌘⌫, Delete on Windows and Linux) and the two path copies
    // are the Explorer's own chords: the registry leaves them to the tree,
    // because in a terminal ⌘⌫ is ^U (S2 passthrough).
    const registry = hostRegistry(useShellStore.getState().rest?.ui_state, hostKind(), keySystem()).registry;
    const command = treeCommandFor(chordFromEvent(event.nativeEvent), registry, hostKind());
    if (command) {
      event.preventDefault();
      if (command === "move_to_trash") {
        if (row) requestTrash(row);
      } else if (command === "explorer_cut" || command === "explorer_copy") {
        takeItem(row, command === "explorer_cut" ? "cut" : "copy");
      } else if (command === "explorer_paste") {
        if (rootPath) pasteInto(pasteTarget(row, rootPath));
      } else if (rootPath) {
        copyPathOf(row?.path ?? rootPath, command === "copy_relative_path");
      }
      return;
    }
    switch (event.key) {
      case "ArrowDown":
        event.preventDefault();
        setSelection(moveSelection(rows, selection, 1));
        return;
      case "ArrowUp":
        event.preventDefault();
        setSelection(moveSelection(rows, selection, -1));
        return;
      case "ArrowRight":
        event.preventDefault();
        if (row?.isDirectory && !row.expanded) toggleFolder(row.path);
        else setSelection(firstChildSelection(rows, selection) ?? selection);
        return;
      case "ArrowLeft":
        event.preventDefault();
        if (row?.isDirectory && row.expanded) toggleFolder(row.path);
        else setSelection(parentSelection(rows, selection));
        return;
      case "Enter":
        event.preventDefault();
        if (row) openRow(row, false);
        return;
      default:
    }
  };

  if (!checkout || !rootPath) {
    return (
      <div className="flex min-h-0 flex-1 items-center justify-center px-md text-center text-caption text-muted-foreground" data-explorer-state="no-checkout">
        {t("explorer.noCheckout")}
      </div>
    );
  }

  // The system's own clipboard commands (the native Edit menu, a right-click
  // menu of the browser) reach the focused tree as clipboard events when its
  // keydown did not answer them; a name field inside the tree keeps its own.
  const onClipboardEvent = (mode: "copy" | "cut" | "paste") => (event: React.ClipboardEvent<HTMLDivElement>) => {
    if ((event.target as HTMLElement).closest("input, textarea")) return;
    event.preventDefault();
    const row = rowForPath(rows, selection);
    if (mode === "paste") pasteInto(pasteTarget(row, rootPath));
    else takeItem(row, mode);
  };

  // Files dragged in from the file manager land in the folder under the
  // pointer, the root row, or the empty space (the root); a drag inside the
  // tree is `moveEntry`'s and never reaches these.
  const osDrag = (folder: string | null) => ({
    onDragOver: (event: React.DragEvent<HTMLElement>) => {
      if (dragPath.current || !carriesOsFiles(event.dataTransfer)) return;
      event.preventDefault();
      event.stopPropagation();
      event.dataTransfer.dropEffect = folder === null ? "none" : "copy";
      setOsTarget(folder);
    },
    onDrop: (event: React.DragEvent<HTMLElement>) => {
      if (dragPath.current || !carriesOsFiles(event.dataTransfer)) return;
      event.preventDefault();
      event.stopPropagation();
      setOsTarget(null);
      if (folder !== null) actions.importDropped(folder, captureDrop(event.dataTransfer));
    },
  });
  const leaveTree = (event: React.DragEvent<HTMLElement>) => {
    if (!event.currentTarget.contains(event.relatedTarget as Node | null)) setOsTarget(null);
  };

  const refused = pathRefusal?.kind === "file_list" && pathRefusal.path === rootPath ? pathRefusal.reason : null;
  const rootListing = listings[rootPath];
  const rootUnavailable = unavailable?.root_path === rootPath ? unavailable : null;
  // What stands in for the tree while a filter has no rows to draw.
  const filterNote = !filtering
    ? null
    : answer?.unavailable
      ? t("documents.filesUnavailable", { reason: answer.unavailable })
      : answer?.indexing && rows.length === 0
        ? t("documents.indexing")
        : answer && rows.length === 0
          ? t("documents.noMatchingFiles")
          : null;
  // A rename, a move or a creation fails under the row it started from. A
  // copy or a drop fails for a whole folder, whose open rows the line would
  // cover, so its line stands under the root row and names the folder.
  const operationFailure = operation?.phase === "failed" && operation.message ? operation : null;
  const failure: { path: string; message: string; onRow: boolean } | null = notice
    ? { ...notice, onRow: false }
    : operationFailure && operationFailure.message
      ? { path: operationFailure.path, message: operationFailure.message, onRow: operationFailure.kind !== "path_copy" && operationFailure.kind !== "path_import" }
      : null;

  const anchorTop = (path: string): number | null => {
    const index = shown.findIndex((entry) => (entry.kind === "entry" ? entry.row.path : entry.path) === path);
    if (index < 0) return null;
    const item = virtualizer.getVirtualItems().find((virtual) => virtual.index === index);
    return item ? item.start : null;
  };
  const failureTop = failure?.onRow ? anchorTop(failure.path) ?? anchorTop(parentPath(failure.path)) ?? 0 : null;

  return (
    <div className="flex min-h-0 flex-1 flex-col" data-explorer={rootPath}>
      <div className="flex shrink-0 items-center border-b border-border px-md py-xs">
        <span className="relative flex min-w-0 flex-1 items-center">
          <SearchIcon aria-hidden="true" className="pointer-events-none absolute left-xs size-(--size-icon-sm) text-muted-foreground" />
          <Input
            value={filter}
            onChange={(event) => setFilter(event.target.value)}
            placeholder={t("explorer.filter")}
            aria-label={t("explorer.filterAria")}
            className="h-(--size-control-sm) pl-xl"
            data-explorer-filter="true"
          />
        </span>
      </div>
      {hint ? (
        <p className="shrink-0 truncate px-md pt-xs text-micro text-muted-foreground" data-lane-tree-hint="true">
          {hint}
        </p>
      ) : null}
      <div
        className={`group/root flex shrink-0 items-center gap-xs border-b border-border px-md py-xs text-caption text-subtle-foreground ${osTarget === rootPath ? "bg-secondary" : ""}`}
        data-explorer-root-row="true"
        data-drop={osTarget === rootPath ? "true" : "false"}
        {...osDrag(rootPath)}
        onDragLeave={leaveTree}
        onContextMenu={(event) => {
          event.preventDefault();
          setMenu({ path: rootPath, isDirectory: true, x: event.clientX, y: event.clientY });
        }}
      >
        <Hint label={rootPath} reveals>
        <span className="min-w-0 flex-1 truncate" data-explorer-root="true">
          {baseName(rootPath)}
        </span>
        </Hint>
        {gitMark ? (
          <Hint label={gitMark.text}>
            {gitMark.state === "loading" ? (
              <LoaderCircleIcon role="img" className="size-(--size-icon-sm) shrink-0 animate-spin text-muted-foreground" data-explorer-git="loading" />
            ) : (
              <Badge role="img" variant="outline" className="border-warning text-warning" data-explorer-git={gitMark.state}>
                !
              </Badge>
            )}
          </Hint>
        ) : null}
        {[
          { id: "new-file", icon: <FilePlusIcon />, label: t("explorer.newFile"), disabled: false, run: () => beginCreate(rootPath, "file") },
          { id: "new-folder", icon: <FolderPlusIcon />, label: t("explorer.newFolder"), disabled: false, run: () => beginCreate(rootPath, "folder") },
          {
            id: "collapse-all",
            icon: <ChevronsDownUpIcon />,
            label: t("explorer.collapseAll"),
            disabled: !expandedUnderRoot(rootPath, expandedPaths).some((path) => path !== rootPath),
            run: () => actions.setExpandedPaths(expandedPaths.includes(rootPath) ? [rootPath] : []),
          },
        ].map((control) => (
          <Hint key={control.id} label={control.label}>
            <Button
              variant="ghost"
              size="icon-sm"
              className={`${ROOT_CONTROL} text-muted-foreground hover:bg-transparent hover:text-foreground`}
              aria-label={control.label}
              disabled={control.disabled}
              data-explorer-root-action={control.id}
              onClick={control.run}
            >
              {control.icon}
            </Button>
          </Hint>
        ))}
        <Hint label={t("common.refresh")}>
          <Button
            variant="ghost"
            size="icon-sm"
            className="text-muted-foreground hover:bg-transparent hover:text-foreground"
            aria-label={t("explorer.refreshTree")}
            onClick={() => {
              // A cached listing turns stale too, so the button re-reads what it
              // is showing rather than only the folders it never listed.
              pending.current.clear();
              refreshListings(watchedFolders(rootPath, expandedPaths));
              setRefreshTick((tick) => tick + 1);
            }}
          >
            <RefreshCwIcon />
          </Button>
        </Hint>
      </div>
      {failure && !failure.onRow ? (
        <div className="break-words border-b border-border px-md py-xs text-caption text-destructive" data-explorer-failure={failure.path}>
          {failure.message}
        </div>
      ) : null}
      {refused ? (
        <div className="border-b border-border px-md py-sm text-caption text-destructive" data-explorer-refusal={refused}>
          {refused}
        </div>
      ) : null}
      {unavailable ? (
        <div className="flex items-center gap-sm border-b border-border px-md py-sm text-caption text-warning" data-explorer-unavailable={unavailable.code}>
          <span className="min-w-0 flex-1 break-words">
            {t("explorer.listingFailed", { name: unavailable.root_path === rootPath ? t("explorer.thisCheckout") : baseName(unavailable.root_path), reason: unavailable.message })}
          </span>
          {needsSettings ? (
            <Button
              variant="ghost"
              size="sm"
              className="h-auto shrink-0 px-none text-subtle-foreground hover:bg-transparent hover:text-foreground"
              onClick={() => actions.openSettings("devices")}
            >
              {t("explorer.openSettings")}
            </Button>
          ) : (
            <Button
              variant="ghost"
              size="sm"
              className="h-auto shrink-0 px-none text-subtle-foreground hover:bg-transparent hover:text-foreground"
              onClick={() => {
                pending.current.clear();
                setRefreshTick((tick) => tick + 1);
              }}
            >
              {t("common.retry")}
            </Button>
          )}
        </div>
      ) : null}
      {!filtering && !rootListing && !refused && !rootUnavailable ? (
        <div className="px-md py-sm text-caption text-muted-foreground" data-explorer-state="loading">
          {t("common.loading")}
        </div>
      ) : filterNote ? (
        <div className="px-md py-sm text-caption text-muted-foreground" data-explorer-state="filter">
          {filterNote}
        </div>
      ) : (
        <div
          ref={scrollRef}
          role="tree"
          tabIndex={0}
          aria-label={t("explorer.aria")}
          className="min-h-0 flex-1 overflow-auto outline-none"
          data-explorer-tree="true"
          onPointerDown={(event) => event.currentTarget.focus()}
          onKeyDown={onKeyDown}
          onCopy={onClipboardEvent("copy")}
          onCut={onClipboardEvent("cut")}
          onPaste={onClipboardEvent("paste")}
          {...osDrag(rootPath)}
          onDragLeave={leaveTree}
          onContextMenu={(event) => {
            if (event.target !== event.currentTarget && (event.target as HTMLElement).closest("[data-explorer-row]")) return;
            event.preventDefault();
            setMenu({ path: rootPath, isDirectory: true, x: event.clientX, y: event.clientY });
          }}
        >
          <div className="relative w-full" style={{ height: `calc(${virtualizer.getTotalSize()}px + var(--size-pane-child-row))` }} data-explorer-tail="true">
            {virtualizer.getVirtualItems().map((virtualRow) => {
              const entry = shown[virtualRow.index];
              if (!entry) return null;
              if (entry.kind === "draft") {
                return (
                  <DraftRowView
                    key="draft"
                    depth={entry.depth}
                    initial={entry.initial}
                    rename={entry.draftKind === "rename"}
                    top={virtualRow.start}
                    onCommit={(name) => {
                      pendingName.current = { kind: entry.draftKind, parent: entry.parent, path: entry.path, initial: name };
                      setDraft(null);
                      if (entry.draftKind === "rename") actions.renameEntry(entry.path, name);
                      else actions.createEntry(entry.parent, name, entry.draftKind === "folder");
                    }}
                    onCancel={() => setDraft(null)}
                  />
                );
              }
              const row = entry.row;
              const needsRefresh = !filtering && row.isDirectory && row.expanded && !watchedNow.has(row.path);
              return (
                <ExplorerRowView
                  key={row.path}
                  row={row}
                  rootPath={rootPath}
                  selected={row.path === selection}
                  top={virtualRow.start}
                  needsRefresh={needsRefresh}
                  onOpen={openRow}
                  onRefresh={() => {
                    pending.current.delete(row.path);
                    actions.setExpandedPaths([...expandedPaths.filter((path) => path !== row.path), row.path]);
                  }}
                  onContextMenu={(event) => {
                    event.preventDefault();
                    setMenu({ path: row.path, isDirectory: row.isDirectory, x: event.clientX, y: event.clientY });
                  }}
                  cut={clipboard?.mode === "cut" && clipboard.root === rootPath && clipboard.path === row.path}
                  osOver={osTarget === row.path}
                  osDrag={osDrag(row.isDirectory ? row.path : null)}
                  onDragStart={(source) => beginDrag(row.path, source)}
                  onDrop={(from) => {
                    if (from && from !== row.path && !row.path.startsWith(`${from}/`)) actions.moveEntry(from, row.path);
                  }}
                />
              );
            })}
            {failure?.onRow && failureTop !== null ? (
              <div
                className="absolute inset-x-0 flex items-center gap-xs pr-md text-caption text-destructive"
                data-explorer-failure={failure.path}
                style={{ top: failureTop + rowHeight(), height: "var(--size-pane-child-row)" }}
              >
                <span className="truncate pl-md">{failure.message}</span>
              </div>
            ) : null}
          </div>
        </div>
      )}
      {menu ? (
        <ExplorerContextMenu
          menu={menu}
          row={rowForPath(rows, menu.path)}
          onClose={() => setMenu(null)}
          onOpenBeside={(path) => {
            setMenu(null);
            setSelection(path);
            actions.openFileBeside(path);
          }}
          onOpenBrowser={(path) => {
            setMenu(null);
            actions.openInBrowser(path);
          }}
          onOpenSource={(path) => {
            setMenu(null);
            setSelection(path);
            actions.openFile(path, false);
          }}
          onRevealExternal={(path) => {
            setMenu(null);
            actions.revealExternal(path);
          }}
          onCopyPath={copyPathOf}
          canPaste={clipboard !== null}
          onCut={(row) => takeItem(row, "cut")}
          onCopy={(row) => takeItem(row, "copy")}
          onPaste={(path, isDirectory) => pasteInto(isDirectory ? path : parentPath(path))}
          onDuplicate={duplicate}
          onNewFile={beginCreate}
          onRename={beginRename}
          onTrash={requestTrash}
        />
      ) : null}
      {footer ? (
        <p className="shrink-0 truncate border-t border-border px-md py-xs text-micro text-muted-foreground" data-lane-tree-footer="true">
          {footer}
        </p>
      ) : null}
    </div>
  );
}

/** The one path a drag is carrying; a ref because a drag never re-renders. */
const dragPath = { current: null as string | null };

/**
 * Starts carrying `path` from its row. The drag marks the root while it runs,
 * so a page it crosses gives way to its still and never takes the drop. The
 * mark and the carried path go at the drag's end, heard on the row itself: a
 * row the list drew away mid-drag still gets its `dragend`, which never
 * reaches the window or React.
 */
function beginDrag(path: string, source: HTMLElement): void {
  dragPath.current = path;
  const release = holdShellDrag("file");
  source.addEventListener(
    "dragend",
    () => {
      dragPath.current = null;
      release();
    },
    { once: true },
  );
}

function DraftRowView({
  depth,
  initial,
  rename,
  top,
  onCommit,
  onCancel,
}: {
  depth: number;
  initial: string;
  rename: boolean;
  top: number;
  onCommit: (name: string) => void;
  onCancel: () => void;
}) {
  const { t } = useInterfaceTranslation();
  const [value, setValue] = useState(initial);
  const done = useRef(false);
  const finish = (commit: boolean) => {
    if (done.current) return;
    done.current = true;
    const name = value.trim();
    if (commit && name) onCommit(name);
    else onCancel();
  };
  return (
    <div
      className="absolute inset-x-0 flex items-center pr-md text-subhead"
      data-explorer-draft={rename ? "rename" : "create"}
      style={{ top: 0, height: "var(--size-pane-child-row)", paddingLeft: `calc(${rowInset(depth)} + var(--size-tree-mark))`, transform: `translateY(${top}px)` }}
    >
      <IndentGuides depth={depth} />
      <Input
        autoFocus
        value={value}
        aria-label={rename ? t("explorer.newName") : t("explorer.newEntryName")}
        className="h-auto min-w-0 flex-1 rounded-xs border-primary px-xxs py-none"
        onChange={(event) => setValue(event.target.value)}
        onKeyDown={(event) => {
          if (event.key !== "Enter" && event.key !== "Escape") {
            event.stopPropagation();
            return;
          }
          event.preventDefault();
          event.stopPropagation();
          finish(event.key === "Enter");
          // The keyboard goes back to the tree the draft sits in, not to the
          // page, where a floating Explorer could no longer hear its Escape.
          // The draft is already finished, so the blur this causes is ignored.
          event.currentTarget.closest<HTMLElement>("[data-explorer-tree]")?.focus({ preventScroll: true });
        }}
        onBlur={() => finish(true)}
      />
    </div>
  );
}

function ExplorerContextMenu({
  menu,
  row,
  onClose,
  onOpenBeside,
  onOpenBrowser,
  onOpenSource,
  onRevealExternal,
  onCopyPath,
  canPaste,
  onCut,
  onCopy,
  onPaste,
  onDuplicate,
  onNewFile,
  onRename,
  onTrash,
}: {
  menu: { path: string; isDirectory: boolean; x: number; y: number };
  row: ExplorerRow | null;
  onClose: () => void;
  /** A second, pinned display of the file beside the active View area (S7 B4). */
  onOpenBeside: (path: string) => void;
  /** An HTML file as a page in a browser display (issue 155). */
  onOpenBrowser: (path: string) => void;
  /** Open HTML in the editor instead of its default Browser View. */
  onOpenSource: (path: string) => void;
  /** The file or folder selected in the OS file manager (issue 324). */
  onRevealExternal: (path: string) => void;
  /** The path (or the path below the checkout root) put on the clipboard. */
  onCopyPath: (path: string, relative: boolean) => void;
  /** The Explorer's clipboard holds an item, so Paste is offered. */
  canPaste: boolean;
  onCut: (row: ExplorerRow) => void;
  onCopy: (row: ExplorerRow) => void;
  /** Paste into the folder the menu stands for, or the folder a file sits in. */
  onPaste: (path: string, isDirectory: boolean) => void;
  onDuplicate: (row: ExplorerRow) => void;
  onNewFile: (parent: string, kind: "file" | "folder") => void;
  onRename: (row: ExplorerRow) => void;
  onTrash: (row: ExplorerRow) => void;
}) {
  const { t } = useInterfaceTranslation();
  // Read as the menu opens: the room beside the only View area (S7 B9, D-06).
  const besideReason = besideUnavailable(workspaceViewOf(useShellStore.getState().rest)?.layout, drawnViews());
  const items = explorerMenuItems({
    isDirectory: menu.isDirectory,
    row,
    html: row !== null && isHtmlFile(row.path),
    besideReason,
    reveal: revealHost(),
    device: explorerContext(useShellStore.getState().rest).device,
    node: windowDeviceId(useShellStore.getState().rest?.navigator?.devices),
    canPaste,
    chord: (id) => commandLabel(id),
  });
  return (
    <EntryPointMenu
      label={row ? t("history.fileActions", { name: row.name }) : t("explorer.folderActions")}
      items={items}
      at={{ x: menu.x, y: menu.y }}
      onClose={onClose}
      data-explorer-menu={menu.path}
      onSelect={(id) => {
        if (id === "new-file") onNewFile(menu.path, "file");
        else if (id === "new-folder") onNewFile(menu.path, "folder");
        else if (row && id === "open-beside") onOpenBeside(row.path);
        else if (row && id === "open-browser") onOpenBrowser(row.path);
        else if (row && id === "open-source") onOpenSource(row.path);
        else if (id === "reveal_external") onRevealExternal(menu.path);
        else if (row && id === "cut") onCut(row);
        else if (row && id === "copy") onCopy(row);
        else if (id === "paste") onPaste(menu.path, menu.isDirectory);
        else if (row && id === "duplicate") onDuplicate(row);
        else if (id === "copy_path") onCopyPath(menu.path, false);
        else if (id === "copy_relative_path") onCopyPath(menu.path, true);
        else if (row && id === "rename") onRename(row);
        else if (row && id === "trash") onTrash(row);
      }}
    />
  );
}

function ExplorerRowView({
  row,
  rootPath,
  selected,
  top,
  needsRefresh,
  onOpen,
  onRefresh,
  onContextMenu,
  cut,
  osOver,
  osDrag,
  onDragStart,
  onDrop,
}: {
  row: ExplorerRow;
  rootPath: string;
  selected: boolean;
  top: number;
  needsRefresh: boolean;
  onOpen: (row: ExplorerRow, preview: boolean) => void;
  onRefresh: () => void;
  onContextMenu: (event: React.MouseEvent) => void;
  /** The row's item is on the clipboard as a cut: drawn faded until it is pasted. */
  cut: boolean;
  /** Files from the file manager are over this folder. */
  osOver: boolean;
  /** What a drag from the file manager does here: a folder takes it, a file refuses it. */
  osDrag: { onDragOver: (event: React.DragEvent<HTMLElement>) => void; onDrop: (event: React.DragEvent<HTMLElement>) => void };
  onDragStart: (source: HTMLElement) => void;
  onDrop: (from: string | null) => void;
}) {
  const { t } = useInterfaceTranslation();
  const [over, setOver] = useState(false);
  return (
    <Hint label={rowTitle(row, rootPath, t)} reveals>
    <div
      role="treeitem"
      aria-selected={selected}
      aria-level={row.depth + 1}
      aria-expanded={row.isDirectory ? row.expanded : undefined}
      aria-label={rowAccessibilityLabel(row, t)}
      draggable
      data-explorer-row={row.path}
      data-selected={selected ? "true" : "false"}
      data-decoration={row.decoration?.status ?? ""}
      data-drop={over || osOver ? "true" : "false"}
      data-cut={cut ? "true" : "false"}
      className={`absolute inset-x-0 flex cursor-default items-center gap-xs pr-md text-subhead text-foreground ${
        selected ? "bg-secondary" : "hover:bg-accent"
      } ${over || osOver ? "bg-secondary" : ""} ${cut ? "opacity-50" : ""}`}
      style={{
        top: 0,
        height: "var(--size-pane-child-row)",
        paddingLeft: rowInset(row.depth),
        transform: `translateY(${top}px)`,
      }}
      onPointerDown={(event) => {
        event.stopPropagation();
        (event.currentTarget.parentElement?.parentElement as HTMLElement | null)?.focus();
      }}
      onClick={() => onOpen(row, true)}
      onDoubleClick={() => onOpen(row, false)}
      onContextMenu={onContextMenu}
      onDragStart={(event) => {
        event.stopPropagation();
        event.dataTransfer.effectAllowed = "move";
        event.dataTransfer.setData("text/plain", row.path);
        onDragStart(event.currentTarget);
      }}
      onDragOver={(event) => {
        if (!dragPath.current) {
          osDrag.onDragOver(event);
          return;
        }
        if (!row.isDirectory) return;
        event.preventDefault();
        event.dataTransfer.dropEffect = "move";
        setOver(true);
      }}
      onDragLeave={() => setOver(false)}
      onDrop={(event) => {
        if (!dragPath.current && carriesOsFiles(event.dataTransfer)) {
          osDrag.onDrop(event);
          return;
        }
        event.preventDefault();
        setOver(false);
        const from = dragPath.current ?? event.dataTransfer.getData("text/plain");
        dragPath.current = null;
        onDrop(from || null);
      }}
      onDragEnd={() => setOver(false)}
    >
      <IndentGuides depth={row.depth} />
      <span className="flex size-(--size-tree-mark) shrink-0 items-center justify-center text-muted-foreground" aria-hidden="true">
        {row.isDirectory ? <ChevronRightIcon className={`size-(--size-tree-mark) ${row.expanded ? "rotate-90" : ""}`} /> : null}
      </span>
      <FileMark icon={row.icon} className="size-(--size-tree-mark)" />
      <span className={`min-w-0 flex-1 truncate ${!row.isDirectory && row.decoration ? gitBadgeColor(row.decoration.status) : ""}`}>{row.name}</span>
      {needsRefresh ? (
        <Hint label={t("explorer.unwatched")}>
          <Button
            variant="ghost"
            size="icon-sm"
            className="shrink-0 text-warning hover:bg-transparent hover:text-foreground"
            data-explorer-refresh={row.path}
            aria-label={t("explorer.refreshFolder", { name: row.name })}
            onClick={(event) => {
              event.stopPropagation();
              onRefresh();
            }}
            onPointerDown={(event) => event.stopPropagation()}
          >
            <RefreshCwIcon />
          </Button>
        </Hint>
      ) : null}
      {row.decoration ? (
        <Hint label={t(row.decoration.title)}>
          <span className={`shrink-0 text-caption ${gitBadgeColor(row.decoration.status)}`}>
            {row.decoration.badge}
          </span>
        </Hint>
      ) : null}
    </div>
    </Hint>
  );
}
