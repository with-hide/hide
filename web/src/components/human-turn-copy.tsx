// Copy beside a person's turn's place, in a file of its own: `human-turn.tsx`
// reads the desktop store for Done, and the phone draws this part without it.

import { CheckIcon, CopyIcon } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { useInterfaceTranslation } from "../i18n/client";
import { cn } from "../lib/utils";
import { Button } from "./ui/button";
import { Hint } from "./ui/tooltip";

/** Puts text on the clipboard of the screen the operator looks at, answering whether it took it (`Actions.copyText`, the phone's `browserClipboard`). */
export type CopyText = (text: string, what: string) => Promise<boolean>;

/** How long the copy icon stays a check after the place was copied. */
const COPIED_MS = 1500;

/**
 * Copy beside a person's turn's place (operator decision 2026-11-08): one
 * icon on every surface that draws Done, which puts the place on the
 * clipboard exactly as the agent wrote it, a URL's query included, and turns
 * into a check for a moment. It never opens the place, which may be meant
 * for another machine (PRD ask-human B10). A refused write is the log's
 * (design principle 13) and leaves the icon as it was. A row draws it above
 * its open button, as it does Done; the phone takes `touch`, a touch target
 * with no hint.
 */
export function HumanTurnCopy({ turn, copy, touch = false, className }: { turn: { id: string; place: string }; copy: CopyText; touch?: boolean; className?: string }) {
  const { t } = useInterfaceTranslation();
  const [copied, setCopied] = useState(false);
  const timer = useRef<number | null>(null);
  // A write that lands after the line went arms no timer for a part that is gone.
  const mounted = useRef(true);
  useEffect(() => () => {
    mounted.current = false;
    if (timer.current !== null) window.clearTimeout(timer.current);
  }, []);
  const label = copied ? t("common.copied") : t("agents.human.copyPlace");
  const button = (
    <Button
      type="button"
      variant="row-action"
      size={touch ? "icon" : "icon-xs"}
      aria-label={copied ? label : t("agents.human.copyPlaceName", { place: turn.place })}
      data-human-turn-copy={turn.id}
      data-copied={copied ? "true" : undefined}
      className={cn("pointer-events-auto relative z-10 shrink-0 self-center", touch && "size-(--size-touch-target)", className)}
      onMouseDown={(event) => event.preventDefault()}
      onClick={(event) => {
        event.stopPropagation();
        void copy(turn.place, "person's turn place").then((took) => {
          if (!took || !mounted.current) return;
          if (timer.current !== null) window.clearTimeout(timer.current);
          setCopied(true);
          timer.current = window.setTimeout(() => setCopied(false), COPIED_MS);
        });
      }}
    >
      {copied ? <CheckIcon aria-hidden="true" className="text-success" /> : <CopyIcon aria-hidden="true" />}
    </Button>
  );
  return touch ? button : <Hint label={label}>{button}</Hint>;
}
