// `복사` as words: a code block's code.

import { CheckIcon, CopyIcon } from "lucide-react";
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
