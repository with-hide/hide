// `복사` as words: a message's original Markdown, a code block's code.

import { CheckIcon, CopyIcon } from "lucide-react";
import type { MouseEvent, ReactNode } from "react";
import { useInterfaceTranslation } from "../i18n/client";
import { cn } from "../lib/utils";
import { useCopied, type CopyText } from "./human-turn-copy";
import { Button } from "./ui/button";
import { Hint } from "./ui/tooltip";

/**
 * `복사` (Doop E2): puts `text` on the clipboard of the screen the operator
 * looks at and turns into `복사함 ✓` for a moment (`useCopied`). It never
 * takes the keyboard from what the operator was doing. `icon` draws the icon
 * alone with its words as the hint.
 */
export function CopyWords({ text, what, copy, icon = false, className }: { text: string; what: string; copy: CopyText; icon?: boolean; className?: string }) {
  const { t } = useInterfaceTranslation();
  const { copied, run } = useCopied(copy);
  const press = (event: MouseEvent) => {
    event.stopPropagation();
    run(text, what);
  };
  // A code block's gutter holds only the icon (Doop N 1 가); its words are the hint and the name.
  if (icon) {
    const label = copied ? t("common.copied") : t("desk.action.copy");
    return (
      <Hint label={label}>
        <Button
          type="button"
          variant="row-action"
          size="icon-xs"
          aria-label={label}
          className={cn(copied && "text-success", className)}
          data-copy-words={what}
          data-copied={copied ? "true" : undefined}
          onMouseDown={(event) => event.preventDefault()}
          onClick={press}
        >
          {copied ? <CheckIcon aria-hidden="true" /> : <CopyIcon aria-hidden="true" />}
        </Button>
      </Hint>
    );
  }
  return (
    <Button
      type="button"
      variant="outline"
      size="sm"
      className={cn("h-(--size-control-compact) gap-xxs bg-background px-xs text-caption font-normal text-subtle-foreground", copied && "text-success", className)}
      data-copy-words={what}
      data-copied={copied ? "true" : undefined}
      onMouseDown={(event) => event.preventDefault()}
      onClick={press}
    >
      {copied ? <CheckIcon aria-hidden="true" /> : <CopyIcon aria-hidden="true" />}
      {copied ? t("common.copied") : t("desk.action.copy")}
    </Button>
  );
}

/**
 * A message with `복사` at its top right while the pointer is over it, it
 * holds the keyboard or it says it copied (Doop E2): the message's original
 * words, Markdown marks included, go to the clipboard. A surface with no
 * pointer shows it always.
 */
export function CopyableMessage({ text, copy, align = "start", className, children, ...rest }: { text: string; copy: CopyText; align?: "start" | "end"; className?: string; children: ReactNode } & Record<`data-${string}`, string | undefined>) {
  return (
    <div className={cn("group/message relative flex min-w-0 flex-col", align === "end" ? "items-end" : "items-stretch", className)} {...rest}>
      {children}
      <span className="pointer-events-none absolute right-0 top-0 z-10 -translate-y-1/2 flex opacity-0 group-hover/message:pointer-events-auto group-hover/message:opacity-100 group-focus-within/message:pointer-events-auto group-focus-within/message:opacity-100 has-data-copied:pointer-events-auto has-data-copied:opacity-100 hoverless:pointer-events-auto hoverless:opacity-100" data-message-copy="true">
        <CopyWords text={text} what="message" copy={copy} />
      </span>
    </div>
  );
}
