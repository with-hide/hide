// The phone's shared layout parts (docs/DESIGN_GUIDE.md, The phone): a list
// screen's head, a page's one-line head, the two-line row every list draws,
// the state strip under a page's head, the line at the foot of a
// conversation that says what the agent does now, and the field a reply is
// written in. Each screen composes these; none draws its own copy.

import { ArrowUpIcon, ChevronLeftIcon, SquareIcon, TerminalIcon } from "lucide-react";
import { useLayoutEffect, useRef, useState, type ComponentProps, type ReactNode } from "react";
import { Elapsed, useRunningClock } from "../components/elapsed";
import { Button } from "../components/ui/button";
import { Input } from "../components/ui/input";
import { Textarea } from "../components/ui/textarea";
import { useInterfaceTranslation } from "../i18n/translator";
import { cn } from "../lib/utils";
import { WorkingOwl } from "../WorkingOwl";

/** The column a row's second line starts in: under the title, after the lead column. */
export const ROW_INSET = "pl-[calc(var(--size-agent-mark)+var(--spacing-sm))]";

type DataProps = Record<`data-${string}`, string | number | boolean | undefined>;

/** A list screen's head (Doop 01, 13, 15): the big title, one line under it, and its action at the right. */
export function ScreenHead({ title, line, action, ...data }: { title: ReactNode; line?: ReactNode; action?: ReactNode } & DataProps) {
  return (
    <header className="phone-safe-top flex min-w-0 items-start gap-md px-xl pb-sm" {...data}>
      <div className="min-w-0 flex-1 pt-lg">
        <h1 className="min-w-0 break-keep text-headline font-semibold text-foreground wrap-anywhere">{title}</h1>
        {line ? <div className="mt-xxs min-w-0 text-subhead text-muted-foreground">{line}</div> : null}
      </div>
      {action ? <div className="shrink-0 pt-md">{action}</div> : null}
    </header>
  );
}

/**
 * A page's head (Doop 02, 11, 14, 16): back, the marks that lead the title (`lead`), the title and one line under it, and an
 * action at the right; what the page keeps under its head (tabs, cells, buttons) comes as `children`. `back.tag` marks the back
 * press for tests and names the list it returns to.
 */
export function PageHead({ back, lead, title, titleEnd, line, lineEnd, action, children, ...data }: { back: { label: string; onClick: () => void; tag: string }; lead?: ReactNode; title: ReactNode; titleEnd?: ReactNode; line?: ReactNode; lineEnd?: ReactNode; action?: ReactNode; children?: ReactNode } & DataProps) {
  return (
    <header className="phone-safe-top shrink-0 border-b border-border bg-background px-lg pb-sm" {...data}>
      <div className="flex min-w-0 items-center gap-sm pt-sm">
        <Button variant="ghost" size="icon" aria-label={back.label} className="-ml-md size-(--size-touch-target) shrink-0 text-foreground" data-phone-back={back.tag} onClick={back.onClick}>
          <ChevronLeftIcon aria-hidden="true" className="size-(--size-icon-lg)" />
        </Button>
        {/* One title line tall, so the marks sit on the title's first line however the title wraps. */}
        {lead ? <span className="flex h-lh shrink-0 items-center gap-xs self-start text-title">{lead}</span> : null}
        <div className="flex min-w-0 flex-1 flex-col">
          {/* What never shrinks after the title (an agent's chips) shares its line only, so the line under it keeps the full width. */}
          <div className="flex min-w-0 items-start gap-sm">
            <h1 className="min-w-0 flex-1 break-keep text-title font-semibold text-foreground wrap-anywhere">{title}</h1>
            {titleEnd ? <div className="flex min-h-lh shrink-0 items-center text-title">{titleEnd}</div> : null}
          </div>
          {line || lineEnd ? (
            // What sits at the right of the line under the title (an agent's tabs) never shrinks; the line gives up its width first.
            <div className={cn("flex min-w-0 items-center gap-sm", lineEnd && "mt-xs")}>
              <div className={cn("flex min-w-0 flex-1 items-center gap-x-xs text-body text-muted-foreground", !lineEnd && "flex-wrap")}>{line}</div>
              {lineEnd ? <div className="shrink-0">{lineEnd}</div> : null}
            </div>
          ) : null}
        </div>
        {action ? <div className="shrink-0">{action}</div> : null}
      </div>
      {children}
    </header>
  );
}

/**
 * A row of every phone list (Doop 01, 13-16): the lead column (a status mark, a dot; `null` keeps the column empty, `undefined`
 * leaves it out), the head (a title with what never shrinks after it), the time at the right, and under them the second line,
 * from the title's column. The caller makes it a press.
 */
export function TwoLineRow({ lead, head, end, line }: { lead?: ReactNode; head: ReactNode; end?: ReactNode; line?: ReactNode }) {
  const column = lead !== undefined;
  return (
    <span className="flex w-full min-w-0 flex-col gap-xxs">
      <span className="flex w-full min-w-0 items-center gap-sm">
        {column ? <span className="flex size-(--size-agent-mark) shrink-0 items-center justify-center">{lead}</span> : null}
        <span className="flex min-w-0 flex-1 items-center gap-sm">{head}</span>
        {end ? <span className="shrink-0 font-mono text-body text-muted-foreground">{end}</span> : null}
      </span>
      {line ? <span className={cn("flex min-w-0 items-center gap-sm text-body", column && ROW_INSET)}>{line}</span> : null}
    </span>
  );
}

/**
 * The state of what a page is about, under its head (Doop 02, 09, 10): what leads it (`lead`, an agent's children chip), the
 * state's word in its colour (`tone`), one line of what it does now, and how long ago it changed at the right.
 */
export function StatusStrip({ lead, word, tone, line, since }: { lead?: ReactNode; word: string; tone: string; line: string | null; since: number | null | undefined }) {
  return (
    <div className="flex min-h-(--size-touch-target) shrink-0 items-center gap-sm border-b border-border bg-background px-lg text-body" data-phone-status-strip={word}>
      {lead}
      <span className={cn("shrink-0 font-medium", tone)}>{word}</span>
      <span className="min-w-0 flex-1 truncate text-foreground" data-phone-status-line="true">{line}</span>
      <Elapsed since={since} className="shrink-0 font-mono text-muted-foreground" />
    </div>
  );
}

/**
 * What the agent does now, at the foot of its conversation, after Hide's small owl looking about (`WorkingOwl`), its time
 * counted on the phone: `생각 중 · 4초` (Doop 09), or with `clock` a command's line, the terminal mark and a running clock,
 * `>_ 명령 실행 중 · 0:42` (Doop 02).
 */
export function ActivityLine({ word, since, clock = false, time }: { word: string; since: number; clock?: boolean; time?: string }) {
  return (
    <p className="flex items-center gap-xs pt-xs text-body" data-phone-activity={word}>
      <WorkingOwl />
      {clock ? <TerminalIcon aria-hidden="true" className="size-(--size-icon) shrink-0 text-agent-working opacity-(--opacity-secondary)" /> : null}
      <span className="text-agent-working opacity-(--opacity-secondary)">{word}</span>
      <span aria-hidden="true" className="text-muted-foreground">·</span>
      {/* `time` is the time a drawing plays (a What's new scene), in place of the clock. */}
      {time !== undefined ? <span className="text-muted-foreground tabular-nums">{time}</span> : clock ? <RunningClock since={since} /> : <Elapsed since={since} className="font-mono text-muted-foreground" />}
    </p>
  );
}

function RunningClock({ since }: { since: number }) {
  const text = useRunningClock(since);
  return text === null ? null : <span className="text-muted-foreground tabular-nums">{text}</span>;
}

/** The most lines a reply field grows to before it scrolls (Doop 06). */
const COMPOSER_LINES = 4;

/**
 * The field a reply is written in (Doop 02, 06, 11, 14): one rounded line with the round send at its end. With `lines` it takes new
 * lines (the keyboard's return key writes one; only the send sends) and grows to four lines, its corners then those of a box.
 * `input` and `send` carry each screen's own name, limit and test marks.
 */
export function Composer({ value, onChange, onSubmit, placeholder, disabled, sendDisabled, lines = false, lead, input, send, onStop }: {
  value: string;
  onChange: (value: string) => void;
  onSubmit: () => void;
  placeholder: string;
  disabled: boolean;
  sendDisabled: boolean;
  lines?: boolean;
  /** What stands before the field inside its frame, such as the reply bar's `+` (Doop 08). */
  lead?: ReactNode;
  input?: Omit<ComponentProps<typeof Input>, "value" | "onChange" | "placeholder" | "disabled"> & DataProps;
  send?: DataProps;
  /** ■ beside send while the agent works (PRD desk-queue B33); send then puts the words in the waiting line (B32). */
  onStop?: () => void;
}) {
  const { t } = useInterfaceTranslation();
  const tall = useGrowingField(lines ? value : null);
  const field = "min-w-0 flex-1 rounded-none border-none bg-transparent px-none text-title text-foreground shadow-none outline-none placeholder:text-muted-foreground focus-visible:ring-0 disabled:opacity-100";
  return (
    // The rounded frame is the field: it takes the field's focus ring and its over-limit mark, as `Input` draws them on its own border.
    <form className={cn("flex min-w-0 items-center gap-xs border border-border bg-input py-xxs pr-xxs focus-within:border-ring focus-within:ring-1 focus-within:ring-ring has-[[aria-invalid=true]]:border-destructive has-[[aria-invalid=true]]:ring-destructive", tall.grown ? "items-end rounded-xl" : "rounded-full", lead ? "pl-xxs" : "pl-lg", disabled && "opacity-(--opacity-disabled)")} onSubmit={(event) => { event.preventDefault(); onSubmit(); }}>
      {lead}
      {lines ? (
        <Textarea
          {...(input as ComponentProps<typeof Textarea>)}
          ref={tall.ref}
          bare
          rows={1}
          value={value}
          onChange={(event) => onChange(event.target.value)}
          placeholder={placeholder}
          disabled={disabled}
          enterKeyHint="enter"
          autoComplete="off"
          className={cn(field, "py-sm leading-normal")}
        />
      ) : (
        <Input
          {...input}
          value={value}
          onChange={(event) => onChange(event.target.value)}
          placeholder={placeholder}
          disabled={disabled}
          enterKeyHint="send"
          autoComplete="off"
          autoCorrect="off"
          className={cn(field, "h-(--size-touch-target)")}
        />
      )}
      {onStop ? (
        <Button variant="secondary" size="icon" type="button" aria-label={t("desk.composer.stop")} disabled={disabled} onClick={onStop} className="size-(--size-touch-target) shrink-0 rounded-full" data-phone-stop="true">
          <SquareIcon aria-hidden="true" className="size-(--size-icon) fill-current" />
        </Button>
      ) : null}
      <Button {...send} variant="secondary" size="icon" type="submit" aria-label={t("mobile.send")} disabled={sendDisabled} className="size-(--size-touch-target) shrink-0 rounded-full">
        <ArrowUpIcon aria-hidden="true" className="size-(--size-icon-lg)" />
      </Button>
    </form>
  );
}

/**
 * Grows a text area with what is written in it, up to `COMPOSER_LINES` lines, and says whether it is past one line; `value` null
 * leaves the field alone.
 */
function useGrowingField(value: string | null) {
  const ref = useRef<HTMLTextAreaElement>(null);
  const [grown, setGrown] = useState(false);
  useLayoutEffect(() => {
    const field = ref.current;
    if (!field || value === null) return;
    const style = getComputedStyle(field);
    const line = Number.parseFloat(style.lineHeight) || 0;
    const padding = Number.parseFloat(style.paddingTop) + Number.parseFloat(style.paddingBottom);
    field.style.height = "auto";
    const content = field.scrollHeight - padding;
    field.style.height = `${Math.min(content, line * COMPOSER_LINES) + padding}px`;
    setGrown(line > 0 && content > line * 1.5);
  }, [value]);
  return { ref, grown };
}
