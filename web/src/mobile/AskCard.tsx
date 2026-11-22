// The agent's own ask at the foot of its conversation (docs/UI_BEHAVIOR.md, The phone app; Doop 03, 04, 06): its question with
// each choice's note, its plan, or the command a permission waits on, with the choices its computer can pick for the operator.
// Picking only marks a choice; 답하기 sends it, and the computer types the keys the agent's own screen takes for it, never the
// choice's words. An agent whose screen Hide has no keys for shows its choices and sends the operator to the terminal.
// Drawn only, with no connection, so a What's new scene and a practice chapter draw the same card (Detail.tsx connects it).
import { CheckIcon } from "lucide-react";
import type { ReactNode } from "react";
import { Button } from "../components/ui/button";
import { RadioGroup, RadioGroupItem } from "../components/ui/radio-group";
import { useInterfaceTranslation } from "../i18n/translator";
import { cn } from "../lib/utils";
import { renderMarkdown } from "./markdown";
import type { TFunction } from "i18next";
import type { MessageKey } from "../i18n/catalogs";
import type { PhoneAsk } from "./protocol";

/** The words a computer's choice ids stand for, with the line under each where the operator needs one. */
const CHOICE_WORDS: Record<string, { label: MessageKey; note?: MessageKey }> = {
  allow: { label: "mobile.ask.allow" },
  allow_always: { label: "mobile.ask.allowAlways", note: "mobile.ask.allowAlwaysNote" },
  deny: { label: "mobile.ask.deny", note: "mobile.ask.denyNote" },
  approve: { label: "mobile.ask.approve" },
  keep_planning: { label: "mobile.ask.keepPlanning", note: "mobile.ask.keepPlanningNote" },
};

/** The suffix Claude Code's question tool asks the agent to put on the choice it recommends, read off the label it wrote. */
const RECOMMENDED = /\s*\(recommended\)\s*$/i;

type Shown = { id: string; label: string; note: string | null; recommended: boolean };

function shownChoices(ask: PhoneAsk, t: TFunction<"translation">): Shown[] {
  return ask.choices.map((choice) => {
    if (choice.label !== undefined) {
      const recommended = RECOMMENDED.test(choice.label);
      return { id: choice.id, label: choice.label.replace(RECOMMENDED, ""), note: choice.note ?? null, recommended };
    }
    const words = CHOICE_WORDS[choice.id];
    return { id: choice.id, label: words ? t(words.label) : choice.id, note: words?.note ? t(words.note) : null, recommended: false };
  });
}

/** The ask's head word and what it is about: `질문`, or `승인` with `명령 실행` or `계획`. */
function headOf(ask: PhoneAsk): { word: MessageKey; about: MessageKey | null; title: MessageKey | null } {
  switch (ask.kind) {
    case "question":
      return { word: "mobile.ask.question", about: null, title: null };
    case "plan_approval":
      return { word: "mobile.ask.approval", about: "mobile.ask.plan", title: "mobile.ask.planTitle" };
    case "permission":
      return ask.command !== undefined
        ? { word: "mobile.ask.approval", about: "mobile.ask.command", title: "mobile.ask.commandTitle" }
        : { word: "mobile.ask.approval", about: null, title: "mobile.ask.permissionTitle" };
  }
}

/**
 * The card as drawn, from what is marked and where its answer stands: the phone's own card, a What's new scene and a practice
 * chapter draw this one, each holding the mark and the answer its own way.
 */
export function AskCardView({ ask, disabled, waited, picked, answer, error, inTerminal = false, onPick, onAnswer, onTerminal }: {
  ask: PhoneAsk;
  disabled: boolean;
  /** How long the agent has waited, in the phone's words (`3분`), or null before it is known. */
  waited: string | null;
  picked: string | null;
  answer: "sending" | "sent" | "failed" | null;
  /** Nothing more is sent from here: the keys may have landed, or the terminal's highlight no longer stood where they count from. */
  inTerminal?: boolean;
  error: ReactNode;
  onPick: (id: string) => void;
  onAnswer: (id: string) => void;
  onTerminal: () => void;
}) {
  const { t } = useInterfaceTranslation();
  const head = headOf(ask);
  const choices = shownChoices(ask, t);
  const sending = answer === "sending";
  const sent = answer === "sent";
  const plan = ask.kind === "plan_approval" && ask.text ? renderMarkdown(ask.text) : null;
  return (
    <section
      aria-label={t(head.word)}
      className="mt-md flex min-w-0 flex-col gap-sm rounded-xl border border-warning/50 bg-card px-lg py-md"
      data-phone-ask={ask.kind}
    >
      <p className="flex min-w-0 flex-wrap items-baseline gap-x-sm text-body">
        <span className="font-semibold text-warning">{t(head.word)}</span>
        <span className="min-w-0 text-muted-foreground">
          {[head.about ? t(head.about) : null, waited ? t("mobile.ask.waiting", { time: waited }) : null].filter(Boolean).join(" · ")}
        </span>
      </p>
      {ask.kind === "question" ? (
        <p className="whitespace-pre-wrap break-keep text-headline font-semibold wrap-anywhere" data-phone-ask-text="true">{ask.text}</p>
      ) : head.title ? (
        <p className="break-keep text-headline font-semibold wrap-anywhere">{t(head.title)}</p>
      ) : null}
      {plan ? (
        <div className="max-h-(--size-mobile-ask-plan) space-y-sm overflow-auto rounded-md bg-background px-md py-sm text-body break-keep wrap-anywhere" data-phone-ask-text="true">
          {plan}
        </div>
      ) : null}
      {ask.command !== undefined ? (
        <pre className="overflow-x-auto rounded-md bg-background px-md py-sm font-mono text-body whitespace-pre-wrap wrap-anywhere" data-phone-ask-command="true">
          {ask.command}
          {ask.command_truncated ? "…" : ""}
        </pre>
      ) : null}
      {ask.truncated ? <p className="text-caption text-muted-foreground">{t("mobile.truncated")}</p> : null}
      {choices.length ? (
        <RadioGroup
          value={picked ?? ""}
          onValueChange={onPick}
          disabled={disabled || !ask.answerable || sending || sent || inTerminal}
          className="gap-xs"
          aria-label={ask.text ?? t(head.word)}
        >
          {choices.map((shown) => (
            <label
              key={shown.id}
              className={cn(
                "flex min-h-(--size-touch-target) min-w-0 items-start gap-sm rounded-md border px-md py-sm text-body",
                ask.answerable ? "cursor-pointer" : "opacity-(--opacity-secondary)",
                picked === shown.id ? "border-primary bg-accent" : "border-border",
              )}
            >
              <RadioGroupItem value={shown.id} className="mt-xxs shrink-0" data-phone-ask-choice={shown.id} />
              <span className="flex min-w-0 flex-1 flex-col">
                <span className="break-keep wrap-anywhere">{shown.label}</span>
                {shown.note ? <span className="break-keep text-subhead text-muted-foreground wrap-anywhere">{shown.note}</span> : null}
              </span>
              {shown.recommended ? (
                <span className="shrink-0 rounded-full bg-secondary px-sm text-caption font-semibold text-subtle-foreground" data-phone-ask-recommended="true">
                  {t("mobile.ask.recommended")}
                </span>
              ) : null}
            </label>
          ))}
        </RadioGroup>
      ) : null}
      {!ask.answerable ? (
        // Hide has no keys for this agent's screen, so the choice is the operator's to make in its terminal.
        <div className="flex min-w-0 flex-wrap items-center justify-end gap-sm">
          <span className="break-keep text-caption text-muted-foreground wrap-anywhere">{t("mobile.ask.inTerminalHint")}</span>
          <Button variant="secondary" className="min-h-(--size-touch-target)" data-phone-ask-terminal="true" onClick={onTerminal}>
            {t("mobile.ask.inTerminal")}
          </Button>
        </div>
      ) : inTerminal ? (
        // The keys may have landed, or would pick another row than the one marked here, so the choice is made in the terminal.
        <div className="flex min-w-0 justify-end">
          <Button variant="default" className="min-h-(--size-touch-target)" data-phone-ask-terminal="refused" onClick={onTerminal}>
            {t("mobile.ask.inTerminal")}
          </Button>
        </div>
      ) : sent ? (
        <p className="flex min-w-0 items-center justify-end gap-xs text-body text-muted-foreground" data-phone-ask-sent={picked ?? "true"}>
          <CheckIcon aria-hidden="true" className="size-(--size-icon) shrink-0 text-success" />
          {t("mobile.ask.sent")}
        </p>
      ) : (
        <div className="flex min-w-0 flex-wrap items-center justify-end gap-sm">
          {picked !== null ? <span className="break-keep text-caption text-muted-foreground wrap-anywhere" data-phone-ask-hint="true">{t("mobile.ask.marked")}</span> : null}
          <Button
            variant={picked !== null ? "default" : "secondary"}
            className="min-h-(--size-touch-target)"
            disabled={disabled || picked === null || sending}
            data-phone-ask-send="true"
            onClick={() => picked !== null && onAnswer(picked)}
          >
            {t("mobile.ask.answer")}
          </Button>
        </div>
      )}
      {error ? (
        <p role="alert" className="break-keep text-body text-destructive wrap-anywhere" data-phone-ask-error="true">
          {error}
        </p>
      ) : null}
    </section>
  );
}

/**
 * The ask folded to one line between the conversation and the reply being written (Doop 06), so it never hides the keyboard;
 * 선택지 보기 leaves the field, which unfolds the card where it stands.
 */
export function AskStrip({ ask }: { ask: PhoneAsk }) {
  const { t } = useInterfaceTranslation();
  const head = headOf(ask);
  const line = ask.kind === "question" ? (ask.text ?? "") : ask.command ?? (head.title ? t(head.title) : "");
  return (
    <div className="flex min-h-(--size-touch-target) min-w-0 items-center gap-sm border-y border-border border-l-warning bg-warning/5 px-lg text-body" data-phone-ask-strip={ask.kind}>
      <span className="shrink-0 font-semibold text-warning">{t(head.word)}</span>
      <span className="min-w-0 flex-1 truncate">{line}</span>
      <Button
        variant="link"
        className="h-(--size-touch-target) shrink-0 px-0 text-body text-muted-foreground"
        data-phone-ask-open="true"
        // Pressed before the field loses focus, so the press is not lost to the keyboard closing.
        onPointerDown={(event) => event.preventDefault()}
        onClick={() => (document.activeElement as HTMLElement | null)?.blur()}
      >
        {t("mobile.ask.open")}
      </Button>
    </div>
  );
}
