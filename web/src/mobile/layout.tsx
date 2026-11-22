// The phone's shared layout parts (docs/DESIGN_GUIDE.md, The phone): a list
// screen's head, a page's one-line head, the two-line row every list draws,
// the state strip under a page's head, the line at the foot of a
// conversation that says what the agent does now, and the field a reply is
// written in. Each screen composes these; none draws its own copy.

import { ArrowUpIcon, ChevronLeftIcon } from "lucide-react";
import type { ComponentProps, ReactNode } from "react";
import { Elapsed } from "../components/elapsed";
import { Button } from "../components/ui/button";
import { Input } from "../components/ui/input";
import { useInterfaceTranslation } from "../i18n/translator";
import { cn } from "../lib/utils";

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
export function PageHead({ back, lead, title, line, action, children, ...data }: { back: { label: string; onClick: () => void; tag: string }; lead?: ReactNode; title: ReactNode; line?: ReactNode; action?: ReactNode; children?: ReactNode } & DataProps) {
  return (
    <header className="phone-safe-top shrink-0 border-b border-border bg-background px-lg pb-sm" {...data}>
      <div className="flex min-w-0 items-center gap-sm pt-sm">
        <Button variant="ghost" size="icon" aria-label={back.label} className="-ml-md size-(--size-touch-target) shrink-0 text-foreground" data-phone-back={back.tag} onClick={back.onClick}>
          <ChevronLeftIcon aria-hidden="true" className="size-(--size-icon-lg)" />
        </Button>
        {lead ? <span className="flex shrink-0 items-center gap-xs self-start pt-xxs">{lead}</span> : null}
        <div className="flex min-w-0 flex-1 flex-col">
          <h1 className="min-w-0 break-keep text-title font-semibold text-foreground wrap-anywhere">{title}</h1>
          {line ? <div className="flex min-w-0 flex-wrap items-center gap-x-xs text-body text-muted-foreground">{line}</div> : null}
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
 * The state of what a page is about, under its head (Doop 02, 09, 10): the state's word in its colour (`tone`), one line of what
 * it does now, and how long ago it changed at the right.
 */
export function StatusStrip({ word, tone, line, since }: { word: string; tone: string; line: string | null; since: number | null | undefined }) {
  return (
    <div className="flex min-h-(--size-touch-target) shrink-0 items-center gap-sm border-b border-border bg-background px-lg text-body" data-phone-status-strip={word}>
      <span className={cn("shrink-0 font-medium", tone)}>{word}</span>
      <span className="min-w-0 flex-1 truncate text-foreground" data-phone-status-line="true">{line}</span>
      <Elapsed since={since} className="shrink-0 font-mono text-muted-foreground" />
    </div>
  );
}

/** What the agent does now, at the foot of its conversation, its time counted on the phone (Doop 09): `생각 중 · 4초`. */
export function ActivityLine({ word, since }: { word: string; since: number }) {
  return (
    <p className="flex items-center gap-xs pt-xs text-body" data-phone-activity={word}>
      <span className="text-agent-working opacity-(--opacity-secondary)">{word}</span>
      <span aria-hidden="true" className="text-muted-foreground">·</span>
      <Elapsed since={since} className="font-mono text-muted-foreground" />
    </p>
  );
}

/**
 * The field a reply is written in (Doop 02, 11, 14): one rounded line with the round send at its end. `input` and `send` carry
 * each screen's own name, limit and test marks.
 */
export function Composer({ value, onChange, onSubmit, placeholder, disabled, sendDisabled, input, send }: {
  value: string;
  onChange: (value: string) => void;
  onSubmit: () => void;
  placeholder: string;
  disabled: boolean;
  sendDisabled: boolean;
  input?: Omit<ComponentProps<typeof Input>, "value" | "onChange" | "placeholder" | "disabled"> & DataProps;
  send?: DataProps;
}) {
  const { t } = useInterfaceTranslation();
  return (
    <form className={cn("flex min-w-0 items-center gap-xs rounded-full border border-border bg-input py-xxs pl-lg pr-xxs", disabled && "opacity-(--opacity-disabled)")} onSubmit={(event) => { event.preventDefault(); onSubmit(); }}>
      <Input
        {...input}
        value={value}
        onChange={(event) => onChange(event.target.value)}
        placeholder={placeholder}
        disabled={disabled}
        enterKeyHint="send"
        autoComplete="off"
        autoCorrect="off"
        className="h-(--size-touch-target) min-w-0 flex-1 rounded-none border-none bg-transparent px-none text-title text-foreground shadow-none outline-none placeholder:text-muted-foreground focus-visible:ring-0 disabled:opacity-100"
      />
      <Button {...send} variant="secondary" size="icon" type="submit" aria-label={t("mobile.send")} disabled={sendDisabled} className="size-(--size-touch-target) shrink-0 rounded-full">
        <ArrowUpIcon aria-hidden="true" className="size-(--size-icon-lg)" />
      </Button>
    </form>
  );
}
