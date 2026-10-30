// What the operator sees of View openers and their own comments (PRD
// view-opener-feedback): the opener's mark on a View tab and the tab's count,
// a pane's `N views`, the note form and a comment's card, the line under a
// View with its Send, and the regions of an image or a PDF. The rules they
// draw from are in `viewFeedback.ts`; the core owns every comment.

import { AlertCircleIcon, CheckIcon, ChevronDownIcon, CircleHelpIcon, FilesIcon, MessageSquarePlusIcon, PencilIcon, RotateCcwIcon, SendIcon, SquareIcon, Trash2Icon, UndoIcon, ArrowUpLeftIcon } from "lucide-react";
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type PointerEvent as ReactPointerEvent, type ReactNode } from "react";
import { Trans } from "react-i18next";
import type { Actions } from "./actions";
import { agentProductName } from "./agentAdapters";
import { noteShellLayer } from "./browserViews";
import { agentMarkOf } from "./agentLogos";
import { useAgentLogosShown } from "./agentLogoMode";
import { markTone, rowAccessibleName } from "./agentRow";
import { Button } from "./components/ui/button";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuLabel, DropdownMenuSeparator, DropdownMenuTrigger } from "./components/ui/dropdown-menu";
import { Hint } from "./components/ui/tooltip";
import { AgentRecordTitle } from "./components/agent-record-title";
import { StatusMark } from "./components/status-mark";
import { useInterfaceTranslation } from "./i18n/client";
import { allAgents } from "./paneAgents";
import { areasOf } from "./areaLayout";
import { frontCheckout, type AgentRow, type ViewLayoutSnapshot, type ViewComment, type ViewCommentAnchor, type ViewDisplaySnapshot, type ViewFeedback, type ViewFeedbackGroup, type ViewFeedbackSend, type ViewOpener, type ViewRect } from "./snapshot";
import { useShellStore } from "./store";
import { cn } from "./lib/utils";
import { displayMark } from "./ViewAreas";
import { workspaceViewOf } from "./workspace";
import type { ViewWorkspace } from "./viewLayout";
import {
  closeBrowserNote,
  closeNote,
  notePlace,
  pageNotePlace,
  commentsOn,
  COMMENTS_PER_VIEW,
  deletedHere,
  dragRect,
  fractionRect,
  groupOn,
  MIN_DRAG,
  NOTE_CHARS,
  noteDeletedFrom,
  noteSentFrom,
  openNote,
  sendDisplay,
  SEND_REASON_KEYS,
  sendShown,
  setRegionMode,
  SHOWN_MS,
  useFeedbackStore,
  viewOnlyCount,
  type NoteForm,
} from "./viewFeedback";
import { holdsFieldModifier } from "./shortcuts";
import { keySystem } from "./host";
import { composingKey } from "./keys";

/** The front Workspace's feedback, or none. */
export function useViewFeedback(): ViewFeedback | undefined {
  return useShellStore((s) => workspaceViewOf(s.rest)?.feedback);
}

// --- the opener's mark (B1-B7) ----------------------------------------------------

/** The selected View toolbar's opener mark; a press goes to the agent's pane. */
export function OpenerMark({ opener, actions }: { opener: ViewOpener; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const product = agentProductName(opener.provider);
  const closed = !opener.pane_id;
  const name = closed ? t("feedback.mark.closed", { name: opener.name }) : opener.name;
  const lines = [name, t("feedback.mark.opened", { product }), ...(closed ? [] : [t("feedback.mark.go")])].join("\n");
  const label = closed ? t("feedback.mark.labelClosed", { name: opener.name, product }) : t("feedback.mark.label", { name: opener.name, product });
  const mark = agentMarkOf(opener.provider);
  return (
    <Hint label={lines} side="top">
      <button
        type="button"
        aria-label={label}
        aria-disabled={closed}
        data-view-opener={opener.pane_id ?? "closed"}
        data-view-opener-kind={opener.provider}
        className={cn(
          "flex size-(--size-view-opener-mark) shrink-0 items-center justify-center rounded-xs outline-none focus-visible:ring-1 focus-visible:ring-ring",
          closed ? "cursor-default opacity-[var(--opacity-dimmed)] grayscale" : "cursor-pointer",
        )}
        onPointerDown={(event) => event.stopPropagation()}
        onPointerMove={(event) => event.stopPropagation()}
        onFocus={(event) => event.stopPropagation()}
        onDoubleClick={(event) => event.stopPropagation()}
        onKeyDown={(event) => event.stopPropagation()}
        onClick={(event) => {
          event.stopPropagation();
          if (opener.pane_id) actions.openAgent(opener.pane_id);
        }}
      >
        {mark?.logo ? (
          <img src={mark.logo} alt="" aria-hidden="true" className={cn("size-full rounded-xs object-contain", mark.plated ? "bg-(--logo-plate)" : "")} />
        ) : (
          <span aria-hidden="true" className="size-full rounded-xs bg-muted-foreground" />
        )}
      </button>
    </Hint>
  );
}

/** A tab's count of unsent comments, orange while some came back undelivered (B14, B29). */
export function CommentCount({ display }: { display: Pick<ViewDisplaySnapshot, "comments" | "not_delivered"> }) {
  const { t } = useInterfaceTranslation();
  const count = display.comments ?? 0;
  if (count === 0) return null;
  const late = (display.not_delivered ?? 0) > 0;
  return (
    <span
      aria-label={[t("feedback.tab.comments", { count }), late ? t("feedback.tab.notDelivered") : null].filter(Boolean).join(" · ")}
      data-view-comments={count}
      data-view-comments-late={late ? "true" : "false"}
      className={cn(
        "flex h-(--size-badge-height) min-w-(--size-badge-height) shrink-0 items-center justify-center rounded-full px-xxs font-mono text-micro",
        late ? "bg-warning text-status-foreground" : "bg-primary text-primary-foreground",
      )}
    >
      {count}
    </span>
  );
}

// --- a pane's views (B8) ---------------------------------------------------------

/** `N views` after a pane's PR chip: the Views its agent opened that the layout shows, those the operator has not looked at marked. */
export function PaneViews({ paneId, name, actions }: { paneId: string; name: string; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const opened = useShellStore((s) => workspaceViewOf(s.rest)?.feedback?.opened);
  const rows = useMemo(() => (opened ?? []).filter((row) => row.pane_id === paneId), [opened, paneId]);
  if (rows.length === 0) return null;
  const label = t("feedback.views", { count: rows.length });
  return (
    <DropdownMenu>
      <Hint label={t("feedback.views.hint")}>
        <DropdownMenuTrigger asChild>
          <button
            type="button"
            data-pane-views={paneId}
            className="flex h-(--size-sidebar-line-detail) shrink-0 items-center gap-xxs rounded-xs px-xs outline-none hover:bg-popover focus-visible:ring-1 focus-visible:ring-ring data-[state=open]:bg-secondary"
          >
            <FilesIcon aria-hidden="true" className="size-(--size-icon-sm)" />
            <span>{label}</span>
          </button>
        </DropdownMenuTrigger>
      </Hint>
      <DropdownMenuContent align="start" className="max-w-(--size-view-note)" data-pane-views-list={paneId}>
        <DropdownMenuLabel>{t("feedback.views.header", { name })}</DropdownMenuLabel>
        {rows.map((row) => (
          <DropdownMenuItem key={row.view_id} data-pane-view={row.view_id} onSelect={() => actions.revealView(row.view_id)} className="items-start">
            <span className="flex size-(--size-icon) shrink-0 items-center justify-center">{displayMark({ kind: row.kind, label: row.label })}</span>
            <span className="flex min-w-0 flex-1 flex-col">
              <span className="truncate text-foreground">{row.label}</span>
              <span className="truncate font-mono text-micro text-muted-foreground">{row.url ?? row.path}</span>
            </span>
            {row.comments > 0 ? <span className="shrink-0 text-micro text-muted-foreground">{t("feedback.views.comments", { count: row.comments })}</span> : null}
            {/* A View the operator has not looked at yet wears the row's 볼 것 mark (PRD unseen-agent-views B10). */}
            {row.unseen ? <StatusMark mark="look" label={t("agents.mark.look")} data-pane-view-unseen={row.view_id} /> : null}
          </DropdownMenuItem>
        ))}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

// --- notes and cards ---------------------------------------------------------------

function basename(path: string): string {
  return path.split("/").filter(Boolean).pop() ?? path;
}

/** What a comment points at, as its card's meta says it. */
export function anchorMeta(t: ReturnType<typeof useInterfaceTranslation>["t"], anchor: ViewCommentAnchor, path: string): string {
  if (anchor.kind === "lines") return `${basename(path)}:${anchor.start === anchor.end ? anchor.start : `${anchor.start}-${anchor.end}`}`;
  if (anchor.kind === "element") return anchor.selector;
  const size = t("feedback.note.area", { width: Math.round(anchor.rect.width), height: Math.round(anchor.rect.height) });
  if (anchor.kind === "area") return size;
  return [anchor.page ? t("feedback.note.page", { page: anchor.page }) : basename(path)].join(" · ");
}

/** The chord a note takes to add, on this system. */
function addChord(): string {
  return keySystem() === "mac" ? "⌘↩" : "Ctrl+Enter";
}

/**
 * The note form under a commented place (B10, B12, B13, B15): ⌘↩ or Add
 * adds, Esc or Cancel cancels, and an empty note adds nothing. A View at its
 * 50 comments takes no new one and says why in place. A page's note adds
 * through `add`, which names the place hided holds; `close` is where the
 * form goes, its display's open note unless said.
 */
export function NoteFormCard({ form, meta, full, workspace, actions, add, close = () => closeNote(form.displayId) }: { form: NoteForm; meta: string; full: boolean; workspace: ViewWorkspace; actions: Actions; add?: (note: string) => void; close?: () => void }) {
  const { t } = useInterfaceTranslation();
  const [note, setNote] = useState(form.note);
  const [busy, setBusy] = useState(false);
  const field = useRef<HTMLTextAreaElement>(null);
  useEffect(() => {
    field.current?.focus({ preventScroll: true });
  }, []);
  const editing = form.commentId !== null;
  const refused = full && !editing;
  const text = note.trim();
  const submit = async () => {
    if (!text || refused || busy) return;
    if (editing) {
      actions.commentView(workspace, { action: "edit", comment_id: form.commentId, note: text });
    } else if (add) {
      add(text);
    } else {
      setBusy(true);
      const shot = form.shot ? await form.shot().catch(() => null) : null;
      actions.commentView(workspace, {
        action: "add",
        display_id: form.displayId,
        anchor: form.anchor,
        note: text,
        ...(form.revision ? { revision: form.revision } : {}),
        ...(shot ? { shot_png: shot } : {}),
      });
    }
    close();
  };
  return (
    <div
      className="flex w-full flex-col gap-xs rounded-md border border-border bg-popover p-sm text-caption shadow-md"
      data-view-note-form={form.displayId}
      onPointerDown={(event) => event.stopPropagation()}
      onMouseDown={(event) => event.stopPropagation()}
      onKeyDown={(event) => {
        event.stopPropagation();
        if (composingKey(event)) return;
        if (event.key === "Escape") {
          event.preventDefault();
          close();
        } else if (event.key === "Enter" && holdsFieldModifier(event, keySystem())) {
          event.preventDefault();
          void submit();
        }
      }}
    >
      <span className="truncate font-mono text-micro text-muted-foreground">{meta}</span>
      {refused ? (
        <p className="rounded-sm bg-secondary p-xs text-muted-foreground" data-view-note-full="true">{t("feedback.note.full", { count: COMMENTS_PER_VIEW })}</p>
      ) : (
        <textarea
          ref={field}
          value={note}
          maxLength={NOTE_CHARS}
          rows={3}
          placeholder={t("feedback.note.placeholder")}
          aria-label={t("feedback.note.placeholder")}
          className="min-h-(--size-control-lg) w-full resize-y rounded-sm border border-input bg-background p-xs text-body text-foreground outline-none focus-visible:border-primary"
          onChange={(event) => setNote(event.currentTarget.value)}
          data-view-note-field="true"
        />
      )}
      <div className="flex items-center gap-xs">
        {refused ? (
          <span className="flex-1 font-mono text-micro text-warning">{`${COMMENTS_PER_VIEW} / ${COMMENTS_PER_VIEW}`}</span>
        ) : (
          <span className="flex-1 text-micro text-muted-foreground">{t("feedback.note.shortcut", { chord: addChord() })}</span>
        )}
        <Button variant="ghost" size="sm" onClick={close} data-view-note-cancel="true">
          {t("common.cancel")}
        </Button>
        <Button size="sm" disabled={!text || refused || busy} onClick={() => void submit()} data-view-note-submit="true">
          {editing ? t("feedback.note.save") : t("feedback.note.add")}
        </Button>
      </div>
    </div>
  );
}

/** A comment's number, orange when its letter expired (B11, B29). */
export function CommentNumber({ comment }: { comment: Pick<ViewComment, "number" | "not_delivered"> }) {
  return (
    <span
      aria-hidden="true"
      className={cn(
        "flex size-(--size-badge-height) shrink-0 items-center justify-center rounded-full font-mono text-micro",
        comment.not_delivered ? "bg-warning text-status-foreground" : "bg-primary text-primary-foreground",
      )}
    >
      {comment.number}
    </span>
  );
}

/** A comment under its line or beside its region: its note, Changed and what it quoted, edit and delete (B12, B31). */
export function CommentCard({ comment, workspace, displayId, actions }: { comment: ViewComment; workspace: ViewWorkspace; displayId: string; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const meta = anchorMeta(t, comment.anchor, comment.path);
  const quoted = comment.changed && comment.anchor.kind === "lines" ? comment.anchor.lines.join(" ") : null;
  return (
    <div
      className="flex w-full items-start gap-xs rounded-md border border-border bg-popover p-sm text-caption shadow-sm"
      data-view-comment={comment.id}
      data-view-comment-number={comment.number}
      data-view-comment-changed={comment.changed ? "true" : "false"}
      onPointerDown={(event) => event.stopPropagation()}
      onMouseDown={(event) => event.stopPropagation()}
    >
      <CommentNumber comment={comment} />
      <div className="flex min-w-0 flex-1 flex-col gap-xxs">
        <div className="flex min-w-0 items-center gap-xs">
          <span className="min-w-0 truncate font-mono text-micro text-muted-foreground">{meta}</span>
          {comment.changed ? <span className="shrink-0 rounded-sm border border-warning px-xxs text-micro text-warning" data-view-comment-changed-badge="true">{t("feedback.changed")}</span> : null}
          {comment.not_delivered ? <span className="shrink-0 text-micro text-warning">{t("feedback.notDelivered")}</span> : null}
        </div>
        <p className="whitespace-pre-wrap break-words text-foreground">{comment.note}</p>
        {quoted ? <p className="truncate border-l border-border pl-xs font-mono text-micro text-muted-foreground" data-view-comment-quote="true">{t("feedback.quoted", { lines: quoted })}</p> : null}
      </div>
      <Hint label={t("feedback.note.edit", { number: comment.number })}>
        <Button
          variant="ghost"
          size="icon-sm"
          data-view-comment-edit={comment.id}
          onClick={() => openNote({ displayId, anchor: comment.anchor, commentId: comment.id, note: comment.note })}
        >
          <PencilIcon />
        </Button>
      </Hint>
      <Hint label={t("feedback.note.delete")}>
        <Button
          variant="ghost"
          size="icon-sm"
          data-view-comment-delete={comment.id}
          onClick={() => {
            noteDeletedFrom(comment.id, displayId, Date.now());
            actions.commentView(workspace, { action: "delete", comment_id: comment.id });
          }}
        >
          <Trash2Icon />
        </Button>
      </Hint>
    </div>
  );
}

// --- the line under a View and its Send (B14-B21, B27-B29, B43) ---------------------

/** The time now, rendered again once `until` passes, for a line that shows for a moment. */
function useNow(until: number | null): number {
  const [, setTick] = useState(0);
  useEffect(() => {
    if (until === null) return undefined;
    const wait = until - Date.now();
    if (wait <= 0) return undefined;
    const timer = window.setTimeout(() => setTick((tick) => tick + 1), wait + 50);
    return () => window.clearTimeout(timer);
  }, [until]);
  return Date.now();
}

/** The agents of the checkout in front, for `Other agents in …` (B17, B18). */
function useCheckoutAgents(): { agents: AgentRow[]; checkout: string } {
  const checkout = useShellStore((s) => frontCheckout(s.rest));
  const local = useShellStore((s) => s.agents);
  const remote = useShellStore((s) => s.rest?.status?.remote);
  return useMemo(() => {
    const panes = new Set(checkout?.tabs.flatMap((tab) => tab.panes.map((pane) => pane.id)) ?? []);
    return { agents: allAgents(local, remote).filter((agent) => panes.has(agent.pane_id)), checkout: checkout?.label ?? "" };
  }, [checkout, local, remote]);
}

/** A new Send's id: a Retry sends the same one (principle 11). */
function sendRequestId(): string {
  return crypto.randomUUID();
}

/**
 * The line under a View with unsent comments: what its recipient gets and
 * Send, or where the last Send stands, or the delete that Undo takes back.
 * It sits under the View, so it never covers the page (B14).
 */
export function FeedbackRow({ display, workspace, actions }: { display: ViewDisplaySnapshot; workspace: ViewWorkspace; actions: Actions }) {
  const { t, i18n } = useInterfaceTranslation();
  const feedback = useViewFeedback();
  const sentFrom = useFeedbackStore((s) => s.sentFrom);
  const deletedFrom = useFeedbackStore((s) => s.deletedFrom);
  const [lastRetry, setLastRetry] = useState<string | null>(null);
  const group = groupOn(feedback, display.id);
  const send = feedback?.send;
  const layoutRoot = useShellStore((s) => workspaceViewOf(s.rest)?.layout?.root ?? null);
  const displayIds = useMemo(() => (layoutRoot ? allDisplayIds(layoutRoot) : []), [layoutRoot]);
  const firstOfRecipient = send ? (feedback?.groups.find((row) => row.recipient?.name === send.to)?.display_ids[0] ?? null) : null;
  const sendHere = sendDisplay(send, sentFrom, displayIds, firstOfRecipient ?? (group ? display.id : null)) === display.id;
  const until = send?.state === "sent" ? send.at_unix_ms + SHOWN_MS : deletedFrom ? deletedFrom.at + SHOWN_MS : null;
  const now = useNow(until);
  // A failed Send's line lasts while its View still holds comments to retry.
  const showSend = sendHere && sendShown(send, now) && send!.reason !== "reopen_failed" && send!.reason !== "closed" && (send!.state !== "failed" || (group?.comments ?? 0) > 0);
  const deleted = deletedHere(feedback, deletedFrom, display.id, now);
  const late = group && group.comments > 0 && group.not_delivered === group.comments;

  let body: ReactNode = null;
  let state = "idle";
  if (deleted && feedback?.deleted) {
    state = "deleted";
    const id = feedback.deleted;
    body = (
      <>
        <span className="min-w-0 flex-1 truncate text-muted-foreground">{t("feedback.deleted")}</span>
        <Button variant="ghost" size="sm" data-view-feedback-undo={id} onClick={() => actions.commentView(workspace, { action: "undo", comment_id: id })}>
          <UndoIcon />
          {t("feedback.undo")}
        </Button>
      </>
    );
  } else if (showSend && send) {
    state = send.state;
    const device = send.device ?? "";
    if (send.state === "failed" && send.reason) {
      body = (
        <>
          <AlertCircleIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0 text-destructive" />
          <span className="min-w-0 flex-1 truncate">
            <Trans i18n={i18n} t={t} i18nKey="feedback.state.failed" values={{ reason: reasonText(t, send.reason, device, send.to) }} components={{ bad: <span className="text-destructive" /> }} />
          </span>
          {/* The core sends again what this Send asked: its recipient and its Views (B27). */}
          <Button
            variant="secondary"
            size="sm"
            data-view-feedback-retry={send.request_id}
            disabled={lastRetry === send.request_id + send.at_unix_ms}
            onClick={() => {
              setLastRetry(send.request_id + send.at_unix_ms);
              actions.sendViewFeedback(workspace, send.request_id, "", { kind: "retry" }, null);
            }}
          >
            {t("common.retry")}
          </Button>
        </>
      );
    } else if (send.state === "sending" || send.state === "reopening") {
      body = <span className="min-w-0 flex-1 truncate text-muted-foreground">{send.state === "sending" ? t("feedback.state.sending", { count: send.count, name: send.to }) : t("feedback.state.reopening", { name: send.to })}</span>;
    } else {
      const waiting = send.state === "waiting";
      body = (
        <>
          {waiting ? <CircleHelpIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0 text-muted-foreground" /> : <CheckIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0 text-success" />}
          <span className="min-w-0 flex-1 truncate">
            <Trans
              i18n={i18n}
              t={t}
              i18nKey={waiting ? "feedback.state.waiting" : "feedback.state.sent"}
              values={{ count: send.count, name: send.to, device }}
              components={{ name: <span className="font-medium text-foreground" />, muted: <span className="text-muted-foreground" /> }}
            />
          </span>
          {send.pane_id ? (
            <Button variant="ghost" size="sm" data-view-feedback-go={send.pane_id} onClick={() => actions.openAgent(send.pane_id!)}>
              <ArrowUpLeftIcon />
              {t("feedback.goToPane")}
            </Button>
          ) : null}
        </>
      );
    }
  } else if (group && late && group.recipient) {
    state = "not_delivered";
    body = (
      <>
        <span aria-hidden="true" className="flex size-(--size-badge-height) shrink-0 items-center justify-center rounded-full bg-warning font-mono text-micro text-status-foreground">{group.not_delivered}</span>
        <span className="min-w-0 flex-1 truncate">
          <Trans i18n={i18n} t={t} i18nKey="feedback.notDeliveredLine" values={{ name: group.recipient.name }} components={{ bad: <span className="text-warning" /> }} />
        </span>
        <SendButton group={group} display={display} workspace={workspace} actions={actions} again />
      </>
    );
  } else if (group && group.comments > 0) {
    state = "unsent";
    body = (
      <>
        <span className="min-w-0 flex-1 truncate text-muted-foreground" data-view-feedback-summary="true">
          <Trans i18n={i18n} t={t} i18nKey="feedback.row.comments" count={group.comments} values={{ count: group.comments }} components={{ n: <span className="text-foreground" /> }} />
          {" · "}
          <Trans i18n={i18n} t={t} i18nKey="feedback.row.views" count={group.views} values={{ count: group.views }} components={{ n: <span className="text-foreground" /> }} />
        </span>
        <SendButton group={group} display={display} workspace={workspace} actions={actions} />
      </>
    );
  }
  if (!body) return null;
  return (
    <div
      role="status"
      className="flex h-(--size-tab-strip) shrink-0 items-center gap-sm border-t border-border bg-background px-md text-caption"
      data-view-feedback-row={display.id}
      data-view-feedback-state={state}
    >
      {body}
    </div>
  );
}

/** Why a Send did not go, as the line says it (B27). */
function reasonText(t: ReturnType<typeof useInterfaceTranslation>["t"], reason: NonNullable<ViewFeedbackSend["reason"]>, device: string, name: string): string {
  return t(SEND_REASON_KEYS[reason], { device, name });
}

function allDisplayIds(root: ViewLayoutSnapshot["root"]): string[] {
  return areasOf(root).flatMap((area) => area.displays.map((display) => display.id));
}

/** An agent's row in the Send menu: its status mark, then its logo while logos show (D-24, B17). */
function AgentLine({ agent, logos }: { agent: AgentRow; logos: boolean }) {
  const mark = agentMarkOf(agent.agent_kind);
  return (
    <>
      <StatusMark mark={agent.mark} className={markTone(agent)} />
      {logos && mark?.logo ? <img src={mark.logo} alt="" aria-hidden="true" className="size-(--size-icon-sm) shrink-0 rounded-xs object-contain" /> : null}
      <AgentRecordTitle agent={agent} />
    </>
  );
}

/**
 * Send, a split button: the recipient's logo and name, and ▾ for `This view
 * only` and the checkout's other agents (B16-B20). A View no agent opened,
 * or one whose opener is closed, opens straight on the choice.
 */
function SendButton({ group, display, workspace, actions, again = false }: { group: ViewFeedbackGroup; display: ViewDisplaySnapshot; workspace: ViewWorkspace; actions: Actions; again?: boolean }) {
  const { t } = useInterfaceTranslation();
  const feedback = useViewFeedback();
  const logos = useAgentLogosShown();
  const { agents, checkout } = useCheckoutAgents();
  const [open, setOpen] = useState(false);
  const recipient = group.recipient ?? null;
  const live = !!recipient?.pane_id;
  const count = group.comments;
  const viewOnly = viewOnlyCount(feedback, display.id, group.key);
  const send = feedback?.send;
  const reopenFailed = send?.reason === "reopen_failed" && send.state === "failed" && send.to === recipient?.name;
  const others = agents.filter((agent) => agent.pane_id !== recipient?.pane_id);
  const go = (to: Parameters<Actions["sendViewFeedback"]>[3], only: string | null) => {
    const requestId = sendRequestId();
    noteSentFrom(requestId, display.id);
    actions.sendViewFeedback(workspace, requestId, group.key, to, only);
  };
  const mark = recipient ? agentMarkOf(recipient.provider) : null;
  const main = again ? t("feedback.sendAgain") : live ? null : t("feedback.sendChoose", { count });
  return (
    <DropdownMenu open={open} onOpenChange={setOpen}>
      <div className="flex shrink-0 items-center" data-view-feedback-send={group.key || "choose"}>
        {live && recipient ? (
          <Button
            size="sm"
            className="min-w-0 max-w-(--size-view-note) rounded-r-none"
            data-view-feedback-send-main="default"
            aria-label={t("feedback.sendTo", { count, name: recipient.name })}
            onClick={() => go({ kind: "default" }, null)}
          >
            <SendIcon />
            {again ? (
              <span>{main}</span>
            ) : (
              <>
                <span className="shrink-0">{t("feedback.send", { count })} →</span>
                {mark?.logo ? <img src={mark.logo} alt="" aria-hidden="true" className="size-(--size-icon-sm) shrink-0 rounded-xs object-contain" /> : null}
                <span className="min-w-0 truncate">{recipient.name}</span>
              </>
            )}
          </Button>
        ) : (
          <DropdownMenuTrigger asChild>
            <Button size="sm" className="rounded-r-none" data-view-feedback-send-main="choose">
              <SendIcon />
              <span>{main}</span>
            </Button>
          </DropdownMenuTrigger>
        )}
        <DropdownMenuTrigger asChild>
          <Button size="sm" className="rounded-l-none border-l border-primary-foreground/20 px-xs" aria-label={t("feedback.sendMenu")} data-view-feedback-send-menu="true">
            <ChevronDownIcon />
          </Button>
        </DropdownMenuTrigger>
      </div>
      <DropdownMenuContent align="end" side="top" className="w-(--size-view-note)" data-view-feedback-menu={group.key || "choose"}>
        {reopenFailed && recipient ? (
          <>
            <div className="flex items-start gap-xs px-sm py-xs text-caption" role="alert" data-view-feedback-reopen-failed="true">
              <AlertCircleIcon aria-hidden="true" className="mt-xxs size-(--size-icon-sm) shrink-0 text-destructive" />
              <span>{t("feedback.state.reopenFailed", { name: recipient.name })}</span>
            </div>
            <DropdownMenuSeparator />
          </>
        ) : null}
        {recipient && live ? (
          <>
            <DropdownMenuLabel>{t("feedback.menu.sendTo")}</DropdownMenuLabel>
            <DropdownMenuItem data-view-feedback-to="default" onSelect={() => go({ kind: "default" }, null)}>
              {(() => {
                const row = agents.find((agent) => agent.pane_id === recipient.pane_id);
                return row ? <AgentLine agent={row} logos={logos} /> : <span className="min-w-0 flex-1 truncate">{recipient.name}</span>;
              })()}
              <span className="shrink-0 text-micro text-muted-foreground">{t("feedback.menu.openedThese", { count })}</span>
            </DropdownMenuItem>
            {viewOnly > 0 && viewOnly < count ? (
              <DropdownMenuItem data-view-feedback-to="view" onSelect={() => go({ kind: "default" }, display.id)}>
                <SquareIcon aria-hidden="true" />
                <span className="min-w-0 flex-1 truncate">{t("feedback.menu.viewOnly")}</span>
                <span className="shrink-0 text-micro text-muted-foreground">{viewOnly}</span>
              </DropdownMenuItem>
            ) : null}
            <DropdownMenuSeparator />
            <DropdownMenuLabel>{t("feedback.menu.others", { checkout })}</DropdownMenuLabel>
          </>
        ) : recipient && !reopenFailed ? (
          <>
            <DropdownMenuLabel>{t("feedback.menu.openedBy")}</DropdownMenuLabel>
            <DropdownMenuItem disabled data-view-feedback-to="closed">
              <StatusMark mark={null} />
              <span className="min-w-0 flex-1 truncate text-muted-foreground">{t("feedback.mark.closed", { name: recipient.name })}</span>
            </DropdownMenuItem>
            {recipient.resumable ? (
              <DropdownMenuItem data-view-feedback-to="reopen" onSelect={() => go({ kind: "reopen" }, null)}>
                <RotateCcwIcon aria-hidden="true" />
                <span>{t("feedback.menu.reopen")}</span>
              </DropdownMenuItem>
            ) : null}
            <DropdownMenuSeparator />
            <DropdownMenuLabel>{t("feedback.menu.orSendTo")}</DropdownMenuLabel>
          </>
        ) : (
          <DropdownMenuLabel>{recipient ? t("feedback.menu.sendTo") : t("feedback.sendChoose", { count })}</DropdownMenuLabel>
        )}
        {(recipient ? others : agents).map((agent) => (
          <DropdownMenuItem key={agent.pane_id} aria-label={rowAccessibleName(t, agent, null)} data-view-feedback-to={agent.pane_id} onSelect={() => go({ kind: "pane", pane_id: agent.pane_id }, null)}>
            <AgentLine agent={agent} logos={logos} />
          </DropdownMenuItem>
        ))}
        {(recipient ? others : agents).length === 0 ? (
          <div className="px-sm py-xs text-caption text-muted-foreground">{t("feedback.menu.none", { checkout })}</div>
        ) : null}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

// --- image and PDF regions (B13, B31) ---------------------------------------------

/** One page an image or PDF viewer lays out: its box in the content and the page it is (none for an image). */
export type RegionPage = { left: number; top: number; width: number; height: number; page: number | null };

/** What a region comment needs of its viewer: the pages, a crop of one, and the file's revision. */
export type RegionSource = {
  pages: RegionPage[];
  /** A PNG (base64, no prefix) of `rect` (fractions) of `page`, or null when the viewer cannot give one. */
  crop: (page: number | null, rect: ViewRect) => Promise<string | null>;
  revision: string | null;
};

/** The longest side a region's screenshot keeps, in pixels per its area. */
const MAX_SHOT_PIXELS = 2_000_000;

/** A PNG of `rect` (fractions) of `source`, scaled down past `MAX_SHOT_PIXELS`. */
export function cropToPng(source: CanvasImageSource, size: { width: number; height: number }, rect: ViewRect): Promise<string | null> {
  const sx = rect.x * size.width;
  const sy = rect.y * size.height;
  const sw = Math.max(1, rect.width * size.width);
  const sh = Math.max(1, rect.height * size.height);
  const scale = Math.min(1, Math.sqrt(MAX_SHOT_PIXELS / (sw * sh)));
  const canvas = document.createElement("canvas");
  canvas.width = Math.max(1, Math.round(sw * scale));
  canvas.height = Math.max(1, Math.round(sh * scale));
  const context = canvas.getContext("2d");
  if (!context) return Promise.resolve(null);
  context.drawImage(source, sx, sy, sw, sh, 0, 0, canvas.width, canvas.height);
  return new Promise((resolve) => {
    canvas.toBlob((blob) => {
      if (!blob) return resolve(null);
      const reader = new FileReader();
      reader.onload = () => resolve(String(reader.result).replace(/^data:image\/png;base64,/, ""));
      reader.onerror = () => resolve(null);
      reader.readAsDataURL(blob);
    }, "image/png");
  });
}

/** `sha256:<hex>` of a file's bytes: an image or PDF comment's revision, which finds a changed file (B31). */
export async function bytesRevision(bytes: Uint8Array): Promise<string | null> {
  if (!crypto.subtle) return null;
  const digest = await crypto.subtle.digest("SHA-256", bytes as unknown as ArrayBuffer);
  return `sha256:${[...new Uint8Array(digest)].map((byte) => byte.toString(16).padStart(2, "0")).join("")}`;
}

/**
 * The regions of an image or a PDF, drawn over its laid-out pages while
 * Comment is on (D-24): a drag makes one and opens its note, a pin opens its
 * comment. Regions are stored as fractions of their page, so a zoom keeps
 * them on the same content (B13).
 */
export function RegionLayer({ displayId, path, source, workspace: view, actions }: { displayId: string; path: string; source: RegionSource; workspace: ViewWorkspace; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const on = useFeedbackStore((s) => s.regionMode[displayId] === true);
  const form = useFeedbackStore((s) => s.forms[displayId] ?? null);
  const feedback = useViewFeedback();
  const comments = useMemo(() => commentsOn(feedback, displayId).filter((comment) => comment.anchor.kind === "region"), [feedback, displayId]);
  const [drag, setDrag] = useState<{ page: number; from: { x: number; y: number }; to: { x: number; y: number }; pointer: number } | null>(null);
  const [shown, setShown] = useState<string | null>(null);
  if (!on) return null;
  const full = commentsOn(feedback, displayId).length >= COMMENTS_PER_VIEW;
  const pointIn = (event: ReactPointerEvent<HTMLElement>) => {
    const box = event.currentTarget.getBoundingClientRect();
    return { x: event.clientX - box.left, y: event.clientY - box.top };
  };
  return (
    <>
      {source.pages.map((page, index) => {
        const pageNumber = page.page;
        const here = comments.filter((comment) => comment.anchor.kind === "region" && (comment.anchor.page ?? null) === pageNumber);
        const dragging = drag?.page === index ? dragRect(drag.from, drag.to, page) : null;
        const formHere = form && form.anchor.kind === "region" && (form.anchor.page ?? null) === pageNumber ? form : null;
        return (
          <div
            key={index}
            className="absolute cursor-crosshair"
            style={{ left: page.left, top: page.top, width: page.width, height: page.height }}
            data-view-region-page={pageNumber ?? 0}
            onPointerDown={(event) => {
              if (event.button !== 0 || event.target !== event.currentTarget) return;
              event.stopPropagation();
              event.currentTarget.setPointerCapture(event.pointerId);
              const point = pointIn(event);
              setShown(null);
              setDrag({ page: index, from: point, to: point, pointer: event.pointerId });
            }}
            onPointerMove={(event) => {
              if (drag?.pointer !== event.pointerId) return;
              event.stopPropagation();
              setDrag({ ...drag, to: pointIn(event) });
            }}
            onPointerUp={(event) => {
              if (drag?.pointer !== event.pointerId) return;
              event.stopPropagation();
              const rect = dragRect(drag.from, pointIn(event), page);
              setDrag(null);
              if (rect.width < MIN_DRAG || rect.height < MIN_DRAG) return;
              const fraction = fractionRect(rect, page);
              const anchor: ViewCommentAnchor = pageNumber === null ? { kind: "region", rect: fraction } : { kind: "region", page: pageNumber, rect: fraction };
              openNote({ displayId, anchor, commentId: null, note: "", shot: () => source.crop(pageNumber, fraction), revision: source.revision });
            }}
            onPointerCancel={() => setDrag(null)}
          >
            {dragging ? <AreaOutline rect={dragging} /> : null}
            {!dragging && formHere && formHere.commentId === null && formHere.anchor.kind === "region" ? <AreaOutline rect={scaledRect(formHere.anchor.rect, page)} /> : null}
            {here.map((comment) => {
              const rect = scaledRect(comment.anchor.kind === "region" ? comment.anchor.rect : { x: 0, y: 0, width: 0, height: 0 }, page);
              return (
                <AreaOutline key={comment.id} rect={rect} notDelivered={comment.not_delivered} id={comment.id}>
                  <button
                    type="button"
                    className="pointer-events-auto absolute -left-(--size-badge-height) -top-(--size-badge-height) rounded-full outline-none focus-visible:ring-1 focus-visible:ring-ring"
                    aria-label={`${comment.number}: ${comment.note}`}
                    data-view-region-pin={comment.number}
                    onPointerDown={(event) => event.stopPropagation()}
                    onClick={() => setShown(shown === comment.id ? null : comment.id)}
                  >
                    <CommentNumber comment={comment} />
                  </button>
                  {shown === comment.id && form?.commentId !== comment.id ? (
                    <div className="pointer-events-auto absolute left-full top-0 z-10 ml-sm w-(--size-view-note)">
                      <CommentCard comment={comment} workspace={view} displayId={displayId} actions={actions} />
                    </div>
                  ) : null}
                </AreaOutline>
              );
            })}
            {formHere ? (
              <div className="absolute z-10 w-(--size-view-note)" style={notePlace(formHere.anchor.kind === "region" ? scaledRect(formHere.anchor.rect, page) : null, page)}>
                <NoteFormCard
                  key={formHere.commentId ?? "new"}
                  form={formHere}
                  meta={[pageNumber ? t("feedback.note.page", { page: pageNumber }) : basename(path), t("feedback.note.area", { width: Math.round(formHere.anchor.kind === "region" ? formHere.anchor.rect.width * page.width : 0), height: Math.round(formHere.anchor.kind === "region" ? formHere.anchor.rect.height * page.height : 0) })].join(" · ")}
                  full={full}
                  workspace={view}
                  actions={actions}
                />
              </div>
            ) : null}
          </div>
        );
      })}
    </>
  );
}

/** The outline of a picked or saved area; a picked area keeps it for as long as its note is open (docs/UI_BEHAVIOR.md, View openers and comments). */
function AreaOutline({ rect, notDelivered = false, id, children }: { rect: ViewRect; notDelivered?: boolean; id?: string; children?: ReactNode }) {
  return (
    <div className={cn("pointer-events-none absolute border-2", notDelivered ? "border-warning bg-warning/10" : "border-primary bg-primary/10")} style={rectStyle(rect)} data-view-region={id} data-view-area-outline={id ? undefined : ""}>
      {children}
    </div>
  );
}

function scaledRect(rect: ViewRect, page: { width: number; height: number }): ViewRect {
  return { x: rect.x * page.width, y: rect.y * page.height, width: rect.width * page.width, height: rect.height * page.height };
}

function rectStyle(rect: ViewRect) {
  return { left: rect.x, top: rect.y, width: rect.width, height: rect.height };
}

/**
 * The note over a page in Comment mode (B10, B12, D-17): the place the
 * operator just picked in the page, or the comment whose pin they pressed,
 * drawn in the window and never in the page, so the page's scripts can
 * neither read nor change what is written. The layer covers the page's
 * slot, which shows the page's still meanwhile; a press beside the note,
 * Esc, Cancel and Add each close it, and `done` hands the page its keyboard
 * back. A new note adds through `add`, which names the place hided holds.
 */
export function PageNoteLayer({ commentKey, displayId, workspace, actions, add, done }: { commentKey: string; displayId: string; workspace: ViewWorkspace; actions: Actions; add: (pick: number, note: string) => void; done: () => void }) {
  const { t } = useInterfaceTranslation();
  const note = useFeedbackStore((s) => s.browserNotes[commentKey] ?? null);
  const edit = useFeedbackStore((s) => s.forms[displayId] ?? null);
  const feedback = useViewFeedback();
  const layer = useRef<HTMLDivElement>(null);
  const [cardHeight, setCardHeight] = useState(0);
  // The card's height, read whenever a card is drawn: the first one only
  // after the slot has been measured.
  const cardObserver = useRef<ResizeObserver | null>(null);
  const card = useCallback((element: HTMLDivElement | null) => {
    cardObserver.current?.disconnect();
    cardObserver.current = null;
    if (!element) return;
    setCardHeight(element.offsetHeight);
    if (typeof ResizeObserver === "undefined") return;
    cardObserver.current = new ResizeObserver(() => setCardHeight(element.offsetHeight));
    cardObserver.current.observe(element);
  }, []);
  const doneRef = useRef(done);
  doneRef.current = done;
  const [size, setSize] = useState<{ width: number; height: number } | null>(null);
  const comment = note?.kind === "open" ? (commentsOn(feedback, displayId).find((each) => each.id === note.commentId) ?? null) : null;
  const shown = note !== null && (note.kind === "pick" || comment !== null);
  useLayoutEffect(() => {
    const element = layer.current;
    if (!shown || !element) return undefined;
    const measure = () => setSize({ width: element.clientWidth, height: element.clientHeight });
    measure();
    // The layer covers the page: the page shows its still until it goes.
    noteShellLayer();
    const observer = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(measure);
    observer?.observe(element);
    return () => {
      observer?.disconnect();
      noteShellLayer();
    };
  }, [shown]);
  // A comment deleted, or gone from another window, takes its card with it.
  useEffect(() => {
    if (note?.kind !== "open" || comment !== null) return;
    closeBrowserNote(commentKey);
    doneRef.current();
  }, [note, comment, commentKey]);
  // A card opened from a pin takes the keyboard, so Esc closes it.
  const openId = note?.kind === "open" && comment ? comment.id : null;
  useEffect(() => {
    if (openId) layer.current?.focus({ preventScroll: true });
  }, [openId]);
  // An edit begun in a card goes with the note.
  useEffect(() => {
    if (!note && useFeedbackStore.getState().forms[displayId]) closeNote(displayId);
  }, [note, displayId]);
  if (!shown || !note) return null;
  const close = () => {
    closeBrowserNote(commentKey);
    closeNote(displayId);
    done();
  };
  const at = size ? { x: note.at.left * size.width, y: note.at.top * size.height, width: note.at.width * size.width, height: note.at.height * size.height } : null;
  const style = at && size ? pageNotePlace(at, size, cardHeight) : { left: 0, top: 0 };
  const full = commentsOn(feedback, displayId).length >= COMMENTS_PER_VIEW;
  let body: ReactNode = null;
  if (note.kind === "pick") {
    const form: NoteForm = { displayId, anchor: note.anchor, commentId: null, note: "" };
    body = <NoteFormCard key={`pick-${note.pick}`} form={form} meta={anchorMeta(t, note.anchor, "")} full={full} workspace={workspace} actions={actions} add={(text) => add(note.pick, text)} close={close} />;
  } else if (comment && edit?.commentId === comment.id) {
    body = <NoteFormCard key={`edit-${comment.id}`} form={edit} meta={anchorMeta(t, comment.anchor, comment.path)} full={full} workspace={workspace} actions={actions} close={close} />;
  } else if (comment) {
    body = <CommentCard comment={comment} workspace={workspace} displayId={displayId} actions={actions} />;
  }
  return (
    <div
      ref={layer}
      tabIndex={-1}
      className="absolute inset-0 z-10 outline-none"
      data-slot="view-note-overlay"
      data-page-note={note.kind}
      onPointerDown={(event) => {
        if (event.target === event.currentTarget) close();
      }}
      onKeyDown={(event) => {
        if (event.key !== "Escape" || composingKey(event)) return;
        event.preventDefault();
        event.stopPropagation();
        close();
      }}
    >
      {at ? <AreaOutline rect={{ x: at.x, y: at.y, width: at.width, height: at.height }} notDelivered={comment?.not_delivered} /> : null}
      {at ? (
        <div ref={card} className="absolute w-(--size-view-note)" style={style}>
          {body}
        </div>
      ) : null}
    </div>
  );
}

/** The Comment button of an image or PDF View's header (B13): on while its regions show; off a PDF Hide cannot read. */
export function RegionCommentButton({ displayId, kind }: { displayId: string; kind: "image" | "pdf" }) {
  const { t } = useInterfaceTranslation();
  const on = useFeedbackStore((s) => s.regionMode[displayId] === true);
  const readable = useFeedbackStore((s) => s.readable[displayId]);
  const unreadable = readable === false;
  return (
    <Hint label={unreadable && kind === "pdf" ? t("feedback.pdfUnreadable") : on ? t("feedback.commentStop") : t("feedback.commentOn")} shortcut={on ? "Esc" : undefined}>
      <span className="inline-flex">
        <Button
          variant={on ? "default" : "ghost"}
          size="sm"
          className={on ? "" : "text-muted-foreground"}
          aria-pressed={on}
          disabled={readable !== true}
          data-view-comment-mode={on ? "on" : "off"}
          onClick={() => setRegionMode(displayId, !on)}
        >
          <MessageSquarePlusIcon />
          {t("feedback.comment")}
        </Button>
      </span>
    </Hint>
  );
}

/** `Drag an area · Esc`, over an image or PDF in Comment mode; Esc ends the mode once no note is open. */
export function RegionHint({ displayId }: { displayId: string }) {
  const { t } = useInterfaceTranslation();
  const on = useFeedbackStore((s) => s.regionMode[displayId] === true);
  useEffect(() => {
    if (!on) return undefined;
    const escape = (event: KeyboardEvent) => {
      if (event.key !== "Escape" || useFeedbackStore.getState().forms[displayId]) return;
      event.preventDefault();
      event.stopPropagation();
      setRegionMode(displayId, false);
    };
    window.addEventListener("keydown", escape, true);
    return () => window.removeEventListener("keydown", escape, true);
  }, [on, displayId]);
  if (!on) return null;
  return (
    <div className="pointer-events-none sticky top-sm z-20 flex h-0 justify-end pr-md" data-view-comment-hint={displayId}>
      <span className="flex h-(--size-control-sm) items-center gap-xs rounded-full border border-border bg-popover px-sm text-caption text-foreground shadow-sm">
        {t("feedback.hint.region")}
      </span>
    </div>
  );
}
