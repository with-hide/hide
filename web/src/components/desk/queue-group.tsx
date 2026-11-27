import { ChevronDownIcon, ChevronUpIcon, PlayIcon, SendIcon, XIcon } from "lucide-react";
import { useEffect, useState } from "react";
// The translator alone, which reads no shell state: the phone's group draws with these words too.
import { useInterfaceTranslation } from "../../i18n/translator";
import type { MessageKey } from "../../i18n/catalogs";
import { cn } from "../../lib/utils";
import type { InputQueueCleared, InputQueueFailure, QueueAction } from "../../snapshot";
import { Button } from "../ui/button";
import { Hint } from "../ui/tooltip";

/** One waiting line as the group draws it: its words, the names of its files, and why it did not go. */
export type QueueGroupLine = { id: number; text: string; files: readonly string[]; sending?: boolean; failure?: InputQueueFailure };

/**
 * `waiting` sends one at each rest; `paused` waits for `보내기` or `▶ 이어 보내기` (■, a turn the operator stopped, an
 * agent that left the pane, a line that did not go); `held` is Desk's line its agent refused, which the notice's
 * `다시 시작` lets go (PRD desk-queue B31).
 */
export type QueueGroupState = "waiting" | "paused" | "held";

export const QUEUE_FAILURE: Record<InputQueueFailure, MessageKey> = {
  uncertain: "desk.queue.failure.uncertain",
  device_unreachable: "desk.queue.failure.device_unreachable",
  file_missing: "desk.queue.failure.file_missing",
  refused: "desk.queue.failure.refused",
};

/** Why a `되돌리기` could not bring the cleared lines back, the words a send past the same cap says. */
export const QUEUE_REFUSED: Record<NonNullable<InputQueueCleared["refused"]>, MessageKey> = {
  queue_full: "desk.queued.full",
  queue_files_full: "panes.attachment.deskFull",
};

export const QUEUE_HEAD: Record<QueueGroupState, MessageKey> = {
  waiting: "desk.queue.waiting",
  paused: "desk.queue.paused",
  held: "desk.queue.held",
};

/**
 * The operator's words that wait for the agent to rest, standing over the input box of a session and of Desk alike
 * (PRD desk-queue D-05, B1-B7, B14-B17, B30, B31): `대기 중 · n ⌃`, which folds, and each line's words, files and ✕;
 * once paused, `모두 지우기` and `▶ 이어 보내기` on its head and `보내기` on each line, which waits while the agent
 * works (`sendable` false). A waiting line whose last typed words opened no turn (`stalled`) offers `▶ 이어 보내기`
 * too, for the next to go. `모두 지우기` leaves `n개 지움 · 되돌리기` until `cleared` runs out, with why a `되돌리기`
 * could not bring them back. The lines and their state are the core's; every move is one `onAction`. `dimmed` draws a
 * line whose device is not connected (B25).
 */
export function QueueGroup({ state, lines, cleared = null, stalled = false, sendable, dimmed = false, onAction }: {
  state: QueueGroupState;
  lines: readonly QueueGroupLine[];
  cleared?: InputQueueCleared | null;
  stalled?: boolean;
  sendable: boolean;
  dimmed?: boolean;
  onAction: (action: QueueAction, id?: number) => void;
}) {
  const { t } = useInterfaceTranslation();
  const [folded, setFolded] = useState(false);
  const undo = useUndoShown(cleared);
  if (lines.length === 0 && !undo) return null;
  const paused = state === "paused";
  const resumable = paused || (state === "waiting" && stalled);
  return (
    <section
      aria-label={t(QUEUE_HEAD[state], { count: lines.length })}
      className={cn("flex min-w-0 flex-col rounded-xl border border-input bg-background text-body", dimmed && "opacity-(--opacity-dimmed)")}
      data-queue-group={state}
      data-queue-count={lines.length}
    >
      {undo ? (
        <p className="flex min-w-0 items-center gap-xs px-md py-xs text-caption text-muted-foreground" data-queue-cleared={undo.count}>
          {t("desk.queue.cleared", { count: undo.count })} ·
          <Button variant="text-link" size="inline" onClick={() => onAction("restore")} data-queue-restore="true">{t("desk.queue.restore")}</Button>
          {undo.refused ? <span className="text-warning" data-queue-restore-refused={undo.refused}>· {t(QUEUE_REFUSED[undo.refused])}</span> : null}
        </p>
      ) : null}
      {lines.length > 0 ? (
        <>
          <div className="flex min-w-0 items-center gap-xs px-md pt-xs pb-xxs">
            <Button
              variant="text"
              size="inline"
              className="min-w-0 gap-xxs text-caption text-foreground"
              aria-expanded={!folded}
              aria-label={`${t(QUEUE_HEAD[state], { count: lines.length })}, ${t(folded ? "desk.queue.unfold" : "desk.queue.fold")}`}
              onClick={() => setFolded(!folded)}
              data-queue-fold={folded ? "folded" : "open"}
            >
              <span className="truncate">{t(QUEUE_HEAD[state], { count: lines.length })}</span>
              {folded ? <ChevronDownIcon className="size-(--size-icon-sm)" /> : <ChevronUpIcon className="size-(--size-icon-sm)" />}
            </Button>
            <span className="flex-1" />
            {paused ? <Button variant="text" size="inline" className="text-caption" onClick={() => onAction("clear")} data-queue-clear="true">{t("desk.queue.clear")}</Button> : null}
            {resumable ? <Button size="sm" className="h-auto gap-xxs px-sm py-xxs text-caption" onClick={() => onAction("resume")} data-queue-resume="true"><PlayIcon className="size-(--size-icon-sm) fill-current" />{t("desk.queue.resume")}</Button> : null}
          </div>
          {folded ? <span className="pb-xxs" /> : (
            <ol className="flex max-h-(--size-desk-queue-list) min-w-0 flex-col overflow-y-auto px-xs pb-xs" data-queue-lines={lines.length}>
              {lines.map((line) => <QueueLine key={line.id} line={line} paused={paused} sendable={sendable} onAction={onAction} />)}
            </ol>
          )}
        </>
      ) : null}
    </section>
  );
}

function QueueLine({ line, paused, sendable, onAction }: { line: QueueGroupLine; paused: boolean; sendable: boolean; onAction: (action: QueueAction, id?: number) => void }) {
  const { t } = useInterfaceTranslation();
  return (
    <li className="flex min-w-0 items-start gap-xs rounded-md px-sm py-xxs" data-queue-line={line.id} data-queue-line-sending={line.sending ? "true" : undefined}>
      <span className="flex min-w-0 flex-1 flex-col">
        <span className={cn("line-clamp-2 min-w-0 whitespace-pre-wrap break-words", line.sending ? "text-muted-foreground" : "text-foreground")} title={line.text}>{line.text}</span>
        {line.files.length > 0 ? <span className="truncate text-caption text-muted-foreground" data-queue-line-files={line.files.length}>{line.files.join(" · ")}</span> : null}
        {line.failure ? <span className="text-caption text-warning" data-queue-line-failure={line.failure}>{t(QUEUE_FAILURE[line.failure])}</span> : null}
      </span>
      {paused && !line.sending ? (
        // A line goes only while the agent rests: one sent mid-turn would land in its work (B16).
        <Hint label={sendable ? t("desk.queue.send") : t("desk.queue.sendLater")}>
          <span className="inline-flex">
            <Button variant="ghost" size="sm" className="h-auto gap-xxs px-xs py-xxs text-caption" disabled={!sendable} aria-label={t("desk.queue.send")} onClick={() => onAction("send", line.id)} data-queue-send={line.id}><SendIcon className="size-(--size-icon-sm)" />{t("desk.queue.send")}</Button>
          </span>
        </Hint>
      ) : null}
      {line.sending ? null : (
        <Hint label={t("desk.queue.remove")}>
          <Button variant="ghost" size="icon-xs" aria-label={t("desk.queue.remove")} onClick={() => onAction("remove", line.id)} data-queue-remove={line.id}><XIcon /></Button>
        </Hint>
      )}
    </li>
  );
}

/** `cleared` while `되돌리기` is still offered, and none once its time runs out, read again when it does. */
export function useUndoShown(cleared: InputQueueCleared | null): InputQueueCleared | null {
  const [now, setNow] = useState(() => Date.now());
  const until = cleared?.until_unix_ms ?? 0;
  useEffect(() => {
    if (until <= Date.now()) return;
    const timer = setTimeout(() => setNow(Date.now()), until - Date.now());
    return () => clearTimeout(timer);
  }, [until]);
  return cleared && cleared.until_unix_ms > Math.max(now, Date.now()) ? cleared : null;
}
