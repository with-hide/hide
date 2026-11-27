import { ChevronDownIcon, ChevronUpIcon, PlayIcon, XIcon } from "lucide-react";
import { useState } from "react";
import { QUEUE_FAILURE, QUEUE_HEAD, useUndoShown } from "../components/desk/queue-group";
import { Button } from "../components/ui/button";
import { useInterfaceTranslation } from "../i18n/translator";
import { cn } from "../lib/utils";
import type { InputQueue } from "../snapshot";
import { sendQueue } from "./connection";

/**
 * The open detail's waiting words over the reply bar (PRD desk-queue D-11, B32): what the desktop's group shows and does, at the
 * phone's touch size. A paused line's `보내기` waits while the agent works, which a line under the head says, since a phone has no
 * hover to carry the hint.
 */
export function PhoneQueueGroup({ queue, working, disabled }: { queue: InputQueue; working: boolean; disabled: boolean }) {
  const { t } = useInterfaceTranslation();
  const [folded, setFolded] = useState(false);
  const undo = useUndoShown(queue.cleared ?? null);
  const { lines } = queue;
  if (lines.length === 0 && !undo) return null;
  const paused = queue.state === "paused";
  const head = t(QUEUE_HEAD[queue.state], { count: lines.length });
  return (
    <section aria-label={head} className={cn("mb-xs flex min-w-0 flex-col rounded-xl border border-border bg-secondary", disabled && "opacity-(--opacity-dimmed)")} data-phone-queue={queue.state}>
      {undo ? (
        <p className="flex min-w-0 items-center gap-xs px-md text-body text-muted-foreground" data-phone-queue-cleared={undo.count}>
          {t("desk.queue.cleared", { count: undo.count })} ·
          <Button variant="text-link" className="h-(--size-touch-target) px-xs text-body" disabled={disabled} onClick={() => sendQueue("restore")} data-phone-queue-restore="true">{t("desk.queue.restore")}</Button>
        </p>
      ) : null}
      {lines.length > 0 ? (
        <>
          <div className="flex min-w-0 items-center gap-xs pl-md pr-xs">
            <Button variant="text" className="h-(--size-touch-target) min-w-0 gap-xxs px-none text-body text-foreground" aria-expanded={!folded} onClick={() => setFolded(!folded)} data-phone-queue-fold={folded ? "folded" : "open"}>
              <span className="truncate">{head}</span>
              {folded ? <ChevronDownIcon aria-hidden="true" /> : <ChevronUpIcon aria-hidden="true" />}
            </Button>
            <span className="flex-1" />
            {paused ? (
              <>
                <Button variant="text" className="h-(--size-touch-target) px-sm text-body" disabled={disabled} onClick={() => sendQueue("clear")} data-phone-queue-clear="true">{t("desk.queue.clear")}</Button>
                <Button size="sm" className="h-(--size-touch-target) gap-xxs rounded-full px-md" disabled={disabled} onClick={() => sendQueue("resume")} data-phone-queue-resume="true"><PlayIcon aria-hidden="true" className="fill-current" />{t("desk.queue.resume")}</Button>
              </>
            ) : null}
          </div>
          {paused && working && !folded ? <p className="px-md text-caption text-muted-foreground" data-phone-queue-later="true">{t("desk.queue.sendLater")}</p> : null}
          {folded ? null : (
            <ol className="flex max-h-(--size-desk-queue-list) min-w-0 flex-col overflow-y-auto pb-xs" data-phone-queue-lines={lines.length}>
              {lines.map((line) => (
                <li key={line.id} className="flex min-w-0 items-center gap-xs pl-md pr-xs" data-phone-queue-line={line.id}>
                  <span className="flex min-w-0 flex-1 flex-col py-xxs">
                    <span className={cn("line-clamp-2 min-w-0 whitespace-pre-wrap break-words text-body", line.sending ? "text-muted-foreground" : "text-foreground")}>{line.text}</span>
                    {line.files.length > 0 ? <span className="truncate text-caption text-muted-foreground">{line.files.join(" · ")}</span> : null}
                    {line.failure ? <span className="text-caption text-warning" data-phone-queue-failure={line.failure}>{t(QUEUE_FAILURE[line.failure])}</span> : null}
                  </span>
                  {paused && !line.sending ? (
                    <Button variant="ghost" className="h-(--size-touch-target) px-sm text-body" disabled={disabled || working} onClick={() => sendQueue("send", line.id)} data-phone-queue-send={line.id}>{t("desk.queue.send")}</Button>
                  ) : null}
                  {line.sending ? null : (
                    <Button variant="ghost" size="icon" className="size-(--size-touch-target)" aria-label={t("desk.queue.remove")} disabled={disabled} onClick={() => sendQueue("remove", line.id)} data-phone-queue-remove={line.id}><XIcon aria-hidden="true" /></Button>
                  )}
                </li>
              ))}
            </ol>
          )}
        </>
      ) : null}
    </section>
  );
}
