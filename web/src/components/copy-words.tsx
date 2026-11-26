// `복사` as words: a message's original Markdown, a code block's code.

import { CheckIcon, CopyIcon } from "lucide-react";
import type { ReactNode } from "react";
import { useInterfaceTranslation } from "../i18n/client";
import { cn } from "../lib/utils";
import { useCopied, type CopyText } from "./human-turn-copy";
import { Button } from "./ui/button";

/**
 * `복사` (Doop E2): puts `text` on the clipboard of the screen the operator
 * looks at and turns into `복사함 ✓` for a moment (`useCopied`). It never
 * takes the keyboard from what the operator was doing. `quiet` draws it as
 * words with no box, as a code block's head row holds it (Doop N 1 나).
 */
export function CopyWords({ text, what, copy, quiet = false, className }: { text: string; what: string; copy: CopyText; quiet?: boolean; className?: string }) {
  const { t } = useInterfaceTranslation();
  const { copied, run } = useCopied(copy);
  return (
    <Button
      type="button"
      variant={quiet ? "text" : "outline"}
      size={quiet ? "inline" : "sm"}
      className={cn(
        "gap-xxs text-caption font-normal",
        quiet ? "px-xxs text-muted-foreground hover:text-foreground" : "h-(--size-control-compact) bg-background px-xs text-subtle-foreground",
        copied && "text-success",
        className,
      )}
      data-copy-words={what}
      data-copied={copied ? "true" : undefined}
      onMouseDown={(event) => event.preventDefault()}
      onClick={(event) => {
        event.stopPropagation();
        run(text, what);
      }}
    >
      {copied ? <CheckIcon aria-hidden="true" /> : <CopyIcon aria-hidden="true" />}
      {copied ? t("common.copied") : t("desk.action.copy")}
    </Button>
  );
}

/**
 * A message with `복사` under it (Doop E2, copy-pos 가): the message's
 * original words, Markdown marks included, go to the clipboard. An answer
 * (`own` false) shows it always, under its end at the left. The operator's
 * own message (`own`) stands at the right and shows it under the bubble at
 * the right while the pointer is over the message, the keyboard is in it or
 * it says it copied, and always on a surface with no pointer; its row is held
 * the whole time, so the message keeps its height when `복사` appears. It is
 * the borderless words a code block's head holds, beside the message and
 * never over its words or its edge.
 */
export function CopyableMessage({ text, copy, own = false, className, children, ...rest }: { text: string; copy: CopyText; own?: boolean; className?: string; children: ReactNode } & Record<`data-${string}`, string | undefined>) {
  return (
    <div className={cn("group/message flex min-w-0 flex-col gap-xs", own ? "items-end" : "items-stretch", className)} {...rest}>
      {children}
      <span
        className={cn(
          "flex",
          own
            ? "self-end pointer-events-none opacity-0 group-hover/message:pointer-events-auto group-hover/message:opacity-100 group-focus-within/message:pointer-events-auto group-focus-within/message:opacity-100 has-data-copied:pointer-events-auto has-data-copied:opacity-100 hoverless:pointer-events-auto hoverless:opacity-100"
            : "self-start",
        )}
        data-message-copy={own ? "own" : "answer"}
      >
        <CopyWords quiet text={text} what="message" copy={copy} />
      </span>
    </div>
  );
}
