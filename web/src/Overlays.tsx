import { ChevronDownIcon, ChevronUpIcon, FolderIcon, HouseIcon, XIcon } from "lucide-react";
import { useEffect, useId, useRef, useState, type RefObject } from "react";
import { createPortal } from "react-dom";
import type { Actions } from "./actions";
import { useInterfaceTranslation } from "./i18n/client";
import { statusText } from "./agentStatus";
import type { MessageKey } from "./i18n/catalogs";
import type { TFunction } from "i18next";
import { AlertDialog, AlertDialogAction, AlertDialogCancel, AlertDialogContent, AlertDialogDescription, AlertDialogFooter, AlertDialogHeader, AlertDialogTitle } from "./components/ui/alert-dialog";
import { Button } from "./components/ui/button";
import { Hint } from "./components/ui/tooltip";
import { Input } from "./components/ui/input";
import { useShellStore } from "./store";
import { focusTerminal } from "./terminals";
import { keyboardOwner } from "./viewFocus";
import { useUiStore, type Cycle } from "./ui";
import { markTone, markWords } from "./agentRow";
import { DeviceChip } from "./components/device-chip";
import { StatusMark } from "./components/status-mark";
import { Kbd } from "./components/ui/kbd";
import { visibleWindow, type CycleDetail, type CycleItem } from "./recent";
import type { SurfaceKind } from "./recent";
import { commandLabel } from "./shortcutLabels";
import { displayMark } from "./ViewAreas";
import { AgentMark } from "./AgentMark";
import { agentProductName } from "./agentAdapters";
import { closeScope, closeSheet, stopWorkCopy, subtreeTitle, type StopWork, type Subtree } from "./close";
import { RowMark, SubtreeList } from "./components/subtree-list";

const CYCLE_TITLE: Record<Cycle["kind"], MessageKey> = {
  area: "shell.recentViewTabs",
  agents: "shell.recentAgentPanes",
  panels: "shell.globalRecentPanels",
  projects: "shell.recentProjects",
};

const SURFACE_LABEL: Record<SurfaceKind, MessageKey> = {
  herdr: "common.terminal",
  file: "shell.surface.file",
  diff: "shell.diff",
  browser: "shell.surface.browser",
  external: "shell.surface.file",
};

function cycleDetail(detail: CycleDetail, t: TFunction<"translation">): string {
  switch (detail.kind) {
    case "surface":
      return `${detail.place ?? t("common.home")} · ${t(SURFACE_LABEL[detail.surface])}`;
    case "projects":
      return t("overview.projects", { count: detail.count });
    case "text":
      return detail.text;
  }
}

/**
 * Recent Panels or Recent Projects while the chord's modifier is held: at
 * most nine rows around the highlight, which is what release commits
 * (docs/UI_BEHAVIOR.md, Recent navigation). It is a layer of the body, like
 * the palette and the menus, so a browser page it meets freezes under it
 * (`web/src/browserViews.ts`).
 */
export function CycleOverlay() {
  const { t } = useInterfaceTranslation();
  const cycle = useUiStore((s) => s.cycle);
  const uiState = useShellStore((s) => s.rest?.ui_state);
  if (!cycle) return null;
  const { start, rows } = visibleWindow(cycle.items, cycle.index);
  const title = t(CYCLE_TITLE[cycle.kind]);
  const chord = commandLabel(cycle.kind === "area" || cycle.kind === "agents" ? "recent_area_tab" : cycle.kind === "panels" ? "recent_panel" : "recent_project", uiState);
  return createPortal(
    <div className="fixed inset-x-0 top-[var(--size-tab-strip)] z-30 flex justify-center" data-cycle={cycle.kind}>
      <div role="listbox" aria-label={title} data-slot="cycle-overlay" className="w-[var(--size-pr-popover)] rounded-md border border-border bg-popover py-xs shadow-lg">
        <div className="flex items-center justify-between px-md pb-xxs">
          <span className="text-caption font-semibold uppercase text-muted-foreground">{title}</span>
          {chord ? <Kbd>{chord}</Kbd> : null}
        </div>
        {rows.map((item, offset) => {
          const selected = start + offset === cycle.index;
          const itemTitle = item.kind === "main" ? t("common.home") : item.title;
          const itemDetail = cycleDetail(item.detail, t);
          return (
            <div
              key={item.key}
              role="option"
              data-cycle-row={item.target.kind === "surface" ? item.target.surface.id : item.target.kind === "pane" ? item.target.paneId : item.key}
              data-cycle-kind={item.kind}
              aria-selected={selected}
              title={item.agent ? agentProductName(item.agent.agent_kind) : undefined}
              aria-label={[itemTitle, item.agent && t("agents.kindAgent", { kind: agentProductName(item.agent.agent_kind) }), item.agent && markWords(t, item.agent), itemDetail, item.chip?.label].filter(Boolean).join(", ")}
              className={`flex items-center gap-sm px-md py-xxs ${selected ? "bg-secondary text-foreground" : "text-subtle-foreground"}`}
            >
              <CycleMarks item={item} title={itemTitle} />
              <span className="flex min-w-0 flex-1 flex-col">
                <span className="truncate text-body">{itemTitle}</span>
                <span className="flex min-w-0 items-center gap-xs text-caption text-muted-foreground">
                  <span className="min-w-0 truncate">{itemDetail}</span>
                  {item.chip ? <DeviceChip label={item.chip.label} local={item.chip.local} className="max-w-2/5" /> : null}
                </span>
              </span>
            </div>
          );
        })}
      </div>
    </div>,
    document.body,
  );
}

/**
 * A row's one mark, in the icon slot every row shares so every title starts in
 * one column: the agent's status mark, never its colour alone, and no provider
 * logo (the name is in the row's label and tooltip); a tab with no single agent
 * wears the neutral mark the tab strip draws.
 */
function CycleMarks({ item, title }: { item: CycleItem; title: string }) {
  return (
    <span className="flex w-(--size-agent-badge-compact) shrink-0 items-center justify-center" data-cycle-marks={item.agent ? "status" : item.kind}>
      {item.agent ? <StatusMark mark={item.agent.mark} className={markTone(item.agent)} data-cycle-status={item.agent.mark} /> : <KindMark item={item} title={title} />}
    </span>
  );
}

/** The Overview and a Project's Overview wear the marks their sidebar rows wear; an agent's row has its status mark in this slot instead (above). */
function KindMark({ item, title }: { item: CycleItem; title: string }) {
  // A tab with no single agent keeps the neutral mark the tab strip draws (PRD agent-mark-quiet D-09).
  if (item.kind === "herdr") return <AgentMark kind={null} />;
  if (item.kind === "project") return <FolderIcon aria-hidden="true" className="size-(--size-icon) text-muted-foreground" />;
  if (item.kind === "main") return <HouseIcon aria-hidden="true" className="size-(--size-icon) text-muted-foreground" />;
  return displayMark({ kind: item.kind, label: title });
}

/**
 * The close confirmation: the Stop-work sheet (Keep open, or stop the work
 * and close), or, for a target with live descendants outside what closes,
 * the subtree sheet (PRD close-agent-subtree B2). Both are live (B13, B28):
 * every snapshot re-derives which of the two it is, who it lists and in what
 * state, so a sheet turns into the other in place, a press sends what is on
 * screen, and the sheet closes by itself only when its target is gone (D-40).
 */
export function ConfirmClose({ actions }: { actions: Actions }) {
  const pending = useUiStore((s) => s.pendingClose);
  // The sheet re-derives from the snapshot only while it is open; closed, this always-mounted gate ignores it.
  const rest = useShellStore((s) => (pending ? s.rest : null));
  useShellStore((s) => (pending ? s.agents : null));
  const target = pending ? actions.closeTarget(pending) : null;
  const gone = pending != null && target === null;
  useEffect(() => {
    if (gone) actions.keepOpen();
  }, [gone, actions]);
  const sheet = target ? closeSheet(target.panes, target.agents, actions.everyAgent(), closeScope(rest, target.panes.map((p) => p.id))) : null;
  const blocked = sheet?.sheet === "subtree" ? sheet.subtree.unknown || sheet.subtree.targetUnknown : sheet?.stopWork.unknown != null;
  // A pane or descendant whose status turns unknown while the sheet is open
  // disables a close under the keyboard; the cancel button takes it, as when
  // the subtree sheet opens blocked (D-19, B28). A sheet that turns into the
  // other in place leaves the keyboard on the dialog itself, never on a
  // destructive button an Enter meant for the old one would press (D-40).
  const footer = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!blocked) return;
    const active = document.activeElement;
    if (active && active !== document.body && !active.matches(":disabled")) return;
    footer.current?.querySelector<HTMLElement>("[data-close-cancel]")?.focus();
  }, [blocked]);
  return (
    <AlertDialog open={pending != null && target != null} onOpenChange={(open) => { if (!open) actions.keepOpen(); }}>
      {pending && sheet ? (
        <AlertDialogContent
          data-confirm-close={pending.kind}
          data-confirm-subtree={sheet.sheet === "subtree" ? "true" : undefined}
          initialFocus={sheet.sheet === "subtree" ? (blocked ? "cancel" : "action") : "container"}
          {...(sheet.sheet === "subtree" ? { "aria-describedby": undefined } : {})}
        >
          {sheet.sheet === "subtree" ? (
            <SubtreeClose actions={actions} kind={pending.kind} targetId={pending.targetId} subtree={sheet.subtree} footer={footer} />
          ) : (
            <StopWorkClose actions={actions} kind={pending.kind} stopWork={sheet.stopWork} footer={footer} />
          )}
        </AlertDialogContent>
      ) : null}
    </AlertDialog>
  );
}

/**
 * The Stop-work sheet: every pane that closes with its mark, a status word
 * on the ones that need the operator and a quiet one dimmed (D-42). While a pane's activity is unknown, Stop work and close
 * waits for the status check the sheet offers (B28, D-39).
 */
function StopWorkClose({ actions, kind, stopWork, footer }: { actions: Actions; kind: "pane" | "tab"; stopWork: StopWork; footer: RefObject<HTMLDivElement | null> }) {
  const { t } = useInterfaceTranslation();
  const copy = stopWorkCopy(kind, t);
  return (
    <>
      <AlertDialogHeader>
        <AlertDialogTitle>{copy.title}</AlertDialogTitle>
        <AlertDialogDescription>{copy.consequence}</AlertDialogDescription>
      </AlertDialogHeader>
      <ul className="flex min-w-0 flex-col gap-xxs text-caption" data-stop-work-list="true">
        {stopWork.rows.map((row) => (
          <li
            key={row.pane.id}
            aria-label={`${row.label}, ${statusText(t, (row.agent ?? row.pane).status_code)}`}
            data-stop-work-row={row.pane.id}
            data-stop-work-state={row.state}
            className={`flex min-w-0 items-start gap-xs ${row.state === "quiet" ? "opacity-(--opacity-read-status)" : ""}`}
          >
            <span className="flex w-(--size-agent-mark) shrink-0 justify-center">
              {row.agent ? <RowMark mark={row.agent.mark} tone={markTone(row.agent)} status={row.state === "quiet" ? markWords(t, row.agent) : null} /> : null}
            </span>
            <span className="flex min-w-0 flex-1 flex-wrap items-baseline gap-x-xs" aria-hidden="true">
              <span className="min-w-0 break-words text-foreground">{row.label}</span>
              {row.state === "quiet" ? null : <span className="shrink-0 text-subtle-foreground" data-stop-work-status="true">{statusText(t, (row.agent ?? row.pane).status_code)}</span>}
            </span>
          </li>
        ))}
      </ul>
      {stopWork.unknown ? (
        <p className="flex flex-wrap items-center gap-xs text-caption text-subtle-foreground" data-stop-work-blocked="true">
          <span className="min-w-0 break-words">{t("shell.paneStatusUnknown", { label: stopWork.unknown.label })}</span>
          <Button size="sm" variant="secondary" onClick={() => actions.refreshStatus()} data-stop-work-check-status="true">
            {t("workspace.checkStatus")}
          </Button>
        </p>
      ) : null}
      <AlertDialogFooter ref={footer}>
        <AlertDialogCancel data-close-cancel="true">{t("commands.keep_open")}</AlertDialogCancel>
        <AlertDialogAction disabled={stopWork.unknown != null} onClick={() => actions.confirmClose()}>
          {t("shell.stopWorkClose")}
        </AlertDialogAction>
      </AlertDialogFooter>
    </>
  );
}

/**
 * The close of an agent with descendants (D-07, D-17, D-18, D-19): one list
 * of what closes with it, and Cancel / Close only this / Close all with Close
 * all the Enter default. While a listed descendant's activity is unknown, Close
 * all waits for a status check the sheet itself offers, and the default is Cancel.
 * What Close only this leaves is said by its tooltip and its accessible
 * description (B5, D-42).
 */
export function SubtreeClose({ actions, kind, targetId, subtree, footer }: { actions: Actions; kind: "pane" | "tab"; targetId: string | null; subtree: Subtree; footer: RefObject<HTMLDivElement | null> }) {
  const count = subtree.rows.filter((row) => !row.target).length;
  const targetDevice = subtree.rows.find((row) => row.target)?.agent.device_id;
  const closeAllBlocked = subtree.unknown || subtree.targetUnknown;
  const closeOnlyResult = useId();
  const { t } = useInterfaceTranslation();
  const closeOnlyWords = t("shell.closeOnlyResult");
  return (
    <>
      <AlertDialogHeader>
        <AlertDialogTitle>{subtreeTitle(kind, count, t)}</AlertDialogTitle>
      </AlertDialogHeader>
      <SubtreeList subtree={subtree} targetDevice={targetDevice ?? (targetId ?? undefined)} />
      {closeAllBlocked ? (
        <p className="flex flex-wrap items-center gap-xs text-caption text-subtle-foreground" data-subtree-blocked="true">
          <span className="min-w-0 break-words">
            {subtree.targetUnknown ? t("shell.agentStatusUnknown") : t("shell.childStatusUnknown")}
          </span>
          <Button size="sm" variant="secondary" onClick={() => actions.refreshStatus()} data-subtree-check-status="true">
            {t("workspace.checkStatus")}
          </Button>
        </p>
      ) : null}
      <AlertDialogFooter ref={footer}>
        <AlertDialogCancel data-subtree-cancel="true" data-close-cancel="true">
          {t("common.cancel")}
        </AlertDialogCancel>
        <Hint label={closeOnlyWords} reveals>
          <Button variant="secondary" disabled={subtree.targetUnknown} onClick={() => actions.confirmClose()} aria-describedby={closeOnlyResult} data-subtree-close-only="true">
            {t("shell.closeOnly")}
          </Button>
        </Hint>
        <span id={closeOnlyResult} className="sr-only">
          {closeOnlyWords}
        </span>
        <Button
          variant="destructive"
          disabled={closeAllBlocked}
          onClick={() => actions.closeSubtree(subtree.ids)}
          data-subtree-close-all="true"
          data-initial-focus={closeAllBlocked ? undefined : "true"}
        >
          {t("shell.closeAll")}
        </Button>
      </AlertDialogFooter>
    </>
  );
}

/** The trash confirmation: an irreversible effect is confirmed first (B10). */
export function ConfirmTrash({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const pending = useUiStore((s) => s.pendingTrash);
  return (
    <AlertDialog open={pending != null} onOpenChange={(open) => { if (!open) actions.cancelTrash(); }}>
      {pending ? (
        <AlertDialogContent data-confirm-trash={pending.path}>
          <AlertDialogHeader>
            <AlertDialogTitle>{t("commands.move_to_trash")}</AlertDialogTitle>
            <AlertDialogDescription>
              {pending.isDirectory ? t("shell.trashDirectory", { name: pending.name }) : t("shell.trashFile", { name: pending.name })}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel data-trash-cancel="true">{t("common.cancel")}</AlertDialogCancel>
            <AlertDialogAction data-trash-confirm="true" onClick={() => actions.confirmTrash()}>
              {t("commands.move_to_trash")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      ) : null}
    </AlertDialog>
  );
}

/**
 * A one-line notice the operator can act on: the refreshable one offers
 * `refresh_status`, and a view whose unsaved work this page cannot save
 * offers Don't save, the only close that drops it (S7 B5). It stays its own
 * row rather than a toast, because its state is one the operator still has to
 * act on (design 13).
 */
export function NoticeBar({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const notice = useUiStore((s) => s.notice);
  const setNotice = useUiStore((s) => s.setNotice);
  if (!notice) return null;
  const dontSave = notice.dontSave;
  return (
    <div role="status" data-notice="true" className="flex items-center gap-md border-b border-border bg-card px-md py-xs text-caption text-subtle-foreground">
      <Hint label={notice.text} reveals>
      <span className="min-w-0 flex-1 truncate">
        {notice.text}
      </span>
      </Hint>
      {notice.refreshable ? (
        <Button variant="link" size="sm" className="h-auto px-none" onClick={() => actions.refreshStatus()}>
          {t("workspace.checkStatus")}
        </Button>
      ) : null}
      {dontSave ? (
        <Button
          variant="link"
          size="sm"
          className="h-auto px-none text-destructive"
          data-notice-dont-save={dontSave.displayId}
          onClick={() => actions.closeViewWithoutSaving(dontSave)}
        >
          {t("shell.dontSave")}
        </Button>
      ) : null}
      <Hint label={t("workspace.dismiss")}>
        <Button variant="ghost" size="icon-sm" aria-label={t("workspace.dismiss")} onClick={() => setNotice(null)}>
          <XIcon />
        </Button>
      </Hint>
    </div>
  );
}

/** ⌘F over the focused pane, through the core's `pane_find`; the count comes back in the snapshot. An agent with its own find gets this bar only when the core answers `bar`. */
export function FindBar({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const open = useUiStore((s) => s.overlay === "find");
  const close = useUiStore((s) => s.closeOverlay);
  const pushEscape = useUiStore((s) => s.pushEscape);
  const find = useShellStore((s) => s.find);
  const paneId = useShellStore((s) => s.focusedPaneId);
  const pending = useUiStore((s) => s.agentFindRequest);
  const [term, setTerm] = useState("");
  const inputRef = useRef<HTMLInputElement>(null);
  useEffect(() => {
    if (open) inputRef.current?.focus();
  }, [open]);
  // The core's answer to ⌘F on an agent with its own find, acted on once:
  // the agent's search takes the typing, or this bar opens. It acts only
  // where ⌘F was pressed: a keyboard the operator moved while the core
  // answered (into a document, onto another pane) stays where it went.
  const opened = find?.opened;
  useEffect(() => {
    if (!pending || !opened || opened.request_id !== pending.id) return;
    useUiStore.getState().setAgentFindRequest(null);
    if (JSON.stringify(keyboardOwner()) !== JSON.stringify(pending.owner)) return;
    if (opened.route === "bar") useUiStore.getState().openOverlay("find");
    else if (find?.pane_id) focusTerminal(find.pane_id);
  }, [pending, opened, find?.pane_id]);
  // Escape and × end the search and give the keyboard back to the pane it
  // searched, so typing continues where it was (B5).
  const dismiss = useRef(() => {});
  dismiss.current = () => {
    if (paneId) actions.find(paneId, "", 0);
    close("find");
    if (paneId) focusTerminal(paneId);
  };
  useEffect(() => (open ? pushEscape(() => dismiss.current(), false, true) : undefined), [open, pushEscape]);
  if (!open) return null;
  const submit = (step: -1 | 0 | 1) => {
    if (paneId) actions.find(paneId, term, step);
  };
  // The core's index is already 1-based, and 0 while nothing matches.
  const count = find && find.pane_id === paneId && find.term === term ? `${find.index}/${find.total}${find.truncated ? "+" : ""}` : "";
  return (
    <div data-find-bar="true" className="flex items-center gap-sm border-b border-border bg-card px-md py-xs text-caption">
      <Input
        ref={inputRef}
        value={term}
        placeholder={t("commands.find_in_pane")}
        className="h-(--size-control-sm) flex-1"
        onChange={(event) => setTerm(event.target.value)}
        onKeyDown={(event) => {
          if (event.nativeEvent.isComposing) return;
          if (event.key === "Enter") {
            event.preventDefault();
            submit(event.shiftKey ? -1 : 1);
          }
        }}
      />
      <span className="text-muted-foreground">{find?.unavailable_reason && find.pane_id === paneId ? find.unavailable_reason : count}</span>
      <Hint label={t("shell.previousMatch")}>
        <Button variant="ghost" size="icon-sm" aria-label={t("shell.previousMatch")} onClick={() => submit(-1)}>
          <ChevronUpIcon />
        </Button>
      </Hint>
      <Hint label={t("shell.nextMatch")}>
        <Button variant="ghost" size="icon-sm" aria-label={t("shell.nextMatch")} onClick={() => submit(1)}>
          <ChevronDownIcon />
        </Button>
      </Hint>
      <Hint label={t("shell.closeFind")}>
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label={t("shell.closeFind")}
          onClick={() => dismiss.current()}
        >
          <XIcon />
        </Button>
      </Hint>
    </div>
  );
}

/** A refused replacement close remains an explicit, retryable operator intent. */
export function AgentCloseNotice({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const pending = useShellStore((s) => s.rest?.recent_closed?.pending);
  const item = pending?.find((item) => item.phase === "failed" || item.phase === "refused");
  if (!item) return null;
  return <div role="status" data-agent-close-notice={item.key} className="flex items-center gap-md border-b border-border bg-card px-md py-xs text-caption text-subtle-foreground">
    <span className="min-w-0 flex-1">{item.message}</span>
    {item.retryable && <Button variant="link" size="sm" onClick={() => actions.retryAgentClose(item.key)}>{t("shell.retryClose")}</Button>}
    <Button variant="ghost" size="icon-sm" aria-label={t("shell.dismissClose")} onClick={() => actions.dismissAgentClose(item.key)}><XIcon /></Button>
  </div>;
}
