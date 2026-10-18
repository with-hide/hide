import { useVirtualizer } from "@tanstack/react-virtual";
import { useEffect, useMemo, useRef, useState } from "react";
import type { Actions } from "./actions";
import { fileIcon } from "./fileIcons";
import { EntryContextMenu, type MenuEntry } from "./components/entry-menu";
import { FileMark } from "./components/file-mark";
import { Hint } from "./components/ui/tooltip";
import { windowDeviceId } from "./screenMachine";
import { changesFor, explorerContext, frontCheckout, type ChangedFileSnapshot, type ChangedFileStatus } from "./snapshot";
import { revealHost } from "./host";
import { revealExternalEntry } from "./revealExternal";
import { useShellStore } from "./store";
import { drawnViews } from "./viewFocus";
import { besideUnavailable } from "./viewLayout";
import { workspaceViewOf } from "./workspace";
import type { TFunction } from "i18next";
import { useInterfaceTranslation } from "./i18n/client";
import type { MessageKey } from "./i18n/catalogs";

const STATUS = {
  modified: { mark: "M", label: "history.status.modified", color: "text-warning" },
  added: { mark: "A", label: "history.status.added", color: "text-success" },
  deleted: { mark: "D", label: "history.status.deleted", color: "text-destructive" },
  untracked: { mark: "U", label: "history.status.untracked", color: "text-success" },
  renamed: { mark: "R", label: "history.status.renamed", color: "text-warning" },
  conflict: { mark: "!", label: "history.status.conflict", color: "text-destructive" },
} as const satisfies Record<ChangedFileStatus, { mark: string; label: MessageKey; color: string }>;

function identity(entry: ChangedFileSnapshot, committed: boolean, t: TFunction<"translation">): string {
  const path = entry.previous_relative_path
    ? `${entry.previous_relative_path} → ${entry.relative_path}`
    : entry.relative_path;
  return t(committed ? "history.identity.committed" : "history.identity.uncommitted", { path, status: t(STATUS[entry.status].label) });
}

function ChangeRow({ entry, committed, selected, actions }: {
  entry: ChangedFileSnapshot;
  committed: boolean;
  selected: boolean;
  actions: Actions;
}) {
  const { t } = useInterfaceTranslation();
  const parts = entry.relative_path.split("/");
  const name = parts.pop() ?? entry.relative_path;
  const parent = parts.join("/");
  const icon = fileIcon(name);
  const status = STATUS[entry.status];
  const title = identity(entry, committed, t);
  // The row's diff beside the active View area (S7 B4, contract 4.2), as a
  // click would open it in the active area; a deleted file has a diff too,
  // but nothing on disk for the OS file manager to show (issue 324).
  const menu = (): MenuEntry<"open_beside" | "reveal_external">[] => {
    const rest = useShellStore.getState().rest;
    return [
      { id: "open_beside", label: t("history.openBeside"), unavailable: besideUnavailable(workspaceViewOf(rest)?.layout, drawnViews()) },
      ...revealExternalEntry(revealHost(), explorerContext(rest).device, windowDeviceId(rest?.navigator?.devices), t, entry.status === "deleted" ? t("history.fileDeleted") : null, true),
    ];
  };
  return (
    <EntryContextMenu
      label={t("history.fileActions", { name })}
      items={menu}
      onSelect={(id) => (id === "reveal_external" ? actions.revealExternal(entry.path) : actions.openChangeBeside(entry.path, committed))}
      className="block"
      data-history-menu={entry.relative_path}
    >
      <Hint label={title} reveals>
      <button
        type="button"
        className={`flex h-[var(--size-pane-child-row)] w-full min-w-0 items-center gap-xs px-sm text-left text-caption hover:bg-accent ${selected ? "bg-secondary text-foreground" : "text-subtle-foreground"}`}
        aria-label={title}
        aria-current={selected ? "true" : undefined}
        data-history-path={entry.relative_path}
        data-history-group={committed ? "committed" : "working"}
        onClick={() => actions.selectChange(entry.path, committed, true)}
        onDoubleClick={() => actions.selectChange(entry.path, committed, false)}
      >
        <FileMark icon={icon} />
        <span className="flex min-w-0 flex-1 items-baseline gap-xs overflow-hidden">
          <span className="shrink-0 truncate">{name}</span>
          {parent ? <span className="min-w-0 truncate text-muted-foreground">{parent}</span> : null}
        </span>
        {entry.added_lines !== null ? <span className="shrink-0 text-success" aria-label={t("history.linesAdded", { count: entry.added_lines })}>+{entry.added_lines}</span> : null}
        {entry.removed_lines !== null ? <span className="shrink-0 text-destructive" aria-label={t("history.linesRemoved", { count: entry.removed_lines })}>-{entry.removed_lines}</span> : null}
        <span className={`shrink-0 ${status.color}`} aria-hidden="true">{status.mark}</span>
      </button>
      </Hint>
    </EntryContextMenu>
  );
}

type Item = { kind: "group"; committed: boolean; branch: string | null; count: number } | { kind: "row"; committed: boolean; entry: ChangedFileSnapshot };

export function HistoryList({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const checkout = useShellStore((s) => frontCheckout(s.rest));
  const rootPath = useShellStore((s) => s.rest?.navigator?.changes_root_path ?? null);
  const changes = useShellStore((s) => changesFor(s.changes, rootPath));
  const [expanded, setExpanded] = useState({ working: true, committed: true });
  const scrollRef = useRef<HTMLDivElement>(null);
  useEffect(() => setExpanded({ working: true, committed: true }), [rootPath]);
  const items = useMemo(() => {
    const result: Item[] = [];
    if (!changes || changes.unavailable_reason) return result;
    if (changes.entries.length > 0) {
      result.push({ kind: "group", committed: false, branch: null, count: changes.entries.length });
      if (expanded.working) result.push(...changes.entries.map((entry): Item => ({ kind: "row", committed: false, entry })));
    }
    if (changes.base_branch && changes.committed.length > 0) {
      result.push({ kind: "group", committed: true, branch: changes.base_branch, count: changes.committed.length });
      if (expanded.committed) result.push(...changes.committed.map((entry): Item => ({ kind: "row", committed: true, entry })));
    }
    return result;
  }, [changes, expanded]);
  const virtualizer = useVirtualizer({
    count: items.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => Number.parseFloat(getComputedStyle(document.documentElement).getPropertyValue("--size-pane-child-row")) || 24,
    overscan: 12,
  });
  if (!checkout) return <p className="px-md py-sm text-caption text-muted-foreground" data-history-state="no-workspace">{t("history.openWorkspace")}</p>;
  if (!rootPath) return <p className="px-md py-sm text-caption text-muted-foreground" data-history-state="no-folder">{t("history.noFolder")}</p>;
  if (!changes) return <p className="px-md py-sm text-caption text-muted-foreground" data-history-state="loading">{t("history.reading")}</p>;
  // The core's own reason, which names what to do where there is something
  // to do (allow the helper, install Git); History reads again on its own.
  if (changes.unavailable_reason) return <p className="break-words px-md py-sm text-caption text-warning" data-history-state="unavailable">{t("history.unavailable", { reason: changes.unavailable_reason })}</p>;
  const stale = changes.stale_reason ? (
    <p className="break-words px-md py-xs text-caption text-warning" data-history-state="stale">
      {t("history.stale", { reason: changes.stale_reason })}
    </p>
  ) : null;
  if (items.length === 0) {
    return (
      <>
        {stale}
        <p className="px-md py-sm text-caption text-muted-foreground" data-history-state="clean">{t("history.empty")}</p>
      </>
    );
  }
  return (
    <>
    {stale}
    <div ref={scrollRef} className="min-h-0 flex-1 overflow-auto" data-history-root={rootPath} aria-label={t("history.aria")}>
      <div className="relative w-full" style={{ height: `${virtualizer.getTotalSize()}px` }}>
        {virtualizer.getVirtualItems().map((virtualRow) => {
          const item = items[virtualRow.index];
          if (!item) return null;
          // Placed by `top`, not a transform: a transformed row would hold its
          // row menu's fixed position, drawing the menu away from the pointer
          // and clipping it inside this list (S7 B4, B9).
          const groupTitle = item.kind === "group" ? (item.branch === null ? t("history.uncommittedHeading") : t("history.committedHeading", { branch: item.branch })) : "";
          return <div key={item.kind === "group" ? `group:${item.committed}` : `${item.committed}:${item.entry.path}`} className="absolute inset-x-0" style={{ top: virtualRow.start }}>
            {item.kind === "group" ? <button
              type="button"
              className="flex h-[var(--size-pane-child-row)] w-full items-center gap-xs px-sm text-left text-caption text-subtle-foreground hover:bg-accent"
              aria-expanded={item.committed ? expanded.committed : expanded.working}
              aria-label={t("history.groupCount", { title: groupTitle, count: item.count })}
              data-history-group-section={item.committed ? "committed" : "working"}
              onClick={() => setExpanded((current) => item.committed ? { ...current, committed: !current.committed } : { ...current, working: !current.working })}
            >
              <span aria-hidden="true">{(item.committed ? expanded.committed : expanded.working) ? "▾" : "▸"}</span>
              <span className="min-w-0 flex-1 truncate">{groupTitle}</span>
              <span aria-hidden="true">{item.count}</span>
            </button> : <ChangeRow entry={item.entry} committed={item.committed} selected={changes.selected_path === item.entry.path && changes.selected_committed === item.committed} actions={actions} />}
          </div>;
        })}
      </div>
    </div>
    </>
  );
}
