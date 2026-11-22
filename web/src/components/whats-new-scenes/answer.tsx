// What's new scene for answering from the phone: an agent at work on the Mac
// asks which way to go, and the same question stands on the phone at the foot
// of its conversation, each choice with what the agent wrote under it. The
// operator marks the second choice and presses 답하기; on the Mac the agent's
// own screen moves down one row and takes Enter, the keys the computer types
// for that choice, never its words. The card leaves, the agent answers, and
// the phone says it runs a command, by the clock. It is drawn from the phone's
// own parts and words (`AskCardView`, `ActivityLine`, `mobile.*`), and the
// work is an example, never the operator's.

import { agentProductName } from "../../agentAdapters";
import { useInterfaceTranslation } from "../../i18n/translator";
import { cn } from "../../lib/utils";
import { AskCardView } from "../../mobile/AskCard";
import { ActivityLine } from "../../mobile/layout";
import type { PhoneAsk } from "../../mobile/protocol";
import { AgentLogo } from "../agent-logo";
import { formatRunning } from "../elapsed";
import { Kbd } from "../ui/kbd";
import { ScenePointer } from "./pointer";
import { SCENE_CSS, type SceneProps } from "./scene";

/** One loop, in ms: the beats below fall inside it. */
export const ANSWER_LOOP_MS = 11_000;
/** The pointer comes to the second choice and marks it. */
const POINT = 900;
const PICK = 1_600;
/** It moves to 답하기 and presses it; the phone says it is sending, then sent. */
const SEND_POINT = 2_400;
const SEND = 3_100;
const SENT = 3_400;
/** On the Mac the agent's screen takes ↓, then Enter. */
const DOWN = 3_700;
const ENTER = 4_300;
/** The moment a still picture shows: the choice sent on the phone and taken on the agent's own screen. */
export const ANSWER_STILL_MS = 4_900;
/** The agent goes on: its answer, and the command it runs. */
const MOVED = 5_600;

const CHOICES = ["1", "2"] as const;
/** The command the agent runs once answered, as its screen names it: code, in no language. */
const COMMAND = "pnpm test";

export function AnswerScene({ at }: SceneProps) {
  const { t } = useInterfaceTranslation();
  const moved = at >= MOVED;
  const ask: PhoneAsk = {
    kind: "question",
    turn: "scene",
    since_ms: 0,
    text: t("whatsNewScene.answer.question"),
    truncated: false,
    choices: CHOICES.map((id) => ({ id, label: t(`whatsNewScene.answer.choice${id}`), note: t(`whatsNewScene.answer.note${id}`) })),
    answerable: true,
  };
  const pointer = at < SEND_POINT
    ? { left: 66, top: 66, shown: at >= POINT, pressed: at >= PICK }
    : { left: 90, top: 88, shown: at < SENT + 500, pressed: at >= SEND };
  return (
    <div className="relative flex aspect-[2/1] gap-md overflow-hidden bg-muted p-md text-foreground" data-whats-new-scene="answer" data-scene-beat={moved ? "moved" : at >= SEND ? "sent" : "asked"} aria-hidden>
      <style>{SCENE_CSS}</style>
      <Terminal at={at} />
      <div className="relative min-w-0 flex-1">
        {/* The phone rises from the scene's foot: the bottom of its screen, where the conversation ends and the ask stands. */}
        <div className="absolute inset-x-0 top-0 -bottom-md flex flex-col overflow-hidden rounded-t-3xl border-4 border-b-0 border-subtle-foreground bg-background">
          <div className="flex items-center gap-xs border-b border-border px-md py-xs text-caption">
            <AgentLogo agent="claude" label={agentProductName("claude")} size="sm" />
            <span className="min-w-0 flex-1 truncate font-semibold">{t("whatsNewScene.answer.agent")}</span>
            <span className="rounded-full bg-secondary px-sm text-micro">{t("mobile.tab.conversation")}</span>
          </div>
          <div className="flex min-h-0 flex-1 flex-col justify-end overflow-hidden px-md pb-md">
            {moved ? (
              <div className="wns-rise">
                <p className="text-body">{t("whatsNewScene.answer.read")}</p>
                <p className="mt-md flex justify-end"><span className="rounded-xl bg-secondary px-md py-xs text-body">{t("whatsNewScene.answer.choice2")}</span></p>
                <p className="mt-md text-body">{t("whatsNewScene.answer.next")}</p>
                <ActivityLine word={t("mobile.runningCommand")} since={0} time={formatRunning(at - MOVED + 1_000)} />
              </div>
            ) : (
              <AskCardView
                ask={ask}
                disabled={false}
                waited={t("whatsNewScene.answer.waited")}
                picked={at >= PICK ? "2" : null}
                answer={at >= SENT ? "sent" : at >= SEND ? "sending" : null}
                error={null}
                onPick={() => {}}
                onAnswer={() => {}}
                onTerminal={() => {}}
              />
            )}
          </div>
        </div>
      </div>
      <ScenePointer {...pointer} />
    </div>
  );
}

/** The agent's own screen on the Mac: its question with the highlighted row, which the keys the computer types move and take. */
function Terminal({ at }: { at: number }) {
  const { t } = useInterfaceTranslation();
  const row = at >= DOWN ? 2 : 1;
  const answered = at >= ENTER;
  const key = at >= ENTER ? "⏎" : at >= DOWN ? "↓" : null;
  return (
    <div className="flex w-1/2 shrink-0 flex-col overflow-hidden rounded-md border border-border bg-background">
      <div className="flex items-center gap-xs border-b border-border px-sm py-xxs text-caption">
        <AgentLogo agent="claude" label={agentProductName("claude")} size="sm" />
        <span className="min-w-0 flex-1 truncate">{t("whatsNewScene.answer.agent")}</span>
        {key && at < MOVED ? <Kbd key={key} className="wns-pop">{key}</Kbd> : null}
      </div>
      <div className="flex flex-col gap-xxs px-sm py-xs font-mono text-micro text-subtle-foreground">
        <span className="truncate">● {t("whatsNewScene.answer.read")}</span>
        {answered ? (
          <>
            <span className="wns-rise truncate text-foreground">✔ {t("whatsNewScene.answer.choice2")}</span>
            {at >= MOVED ? <span className="wns-rise truncate">● {t("whatsNewScene.answer.next")}</span> : null}
            {at >= MOVED ? <span className="wns-rise truncate text-foreground">{`⏺ Bash(${COMMAND})`}</span> : null}
            {at >= MOVED ? <span className="wns-rise truncate">  ⎿ {formatRunning(at - MOVED + 1_000)}</span> : null}
          </>
        ) : (
          <div className="flex flex-col gap-xxs rounded-sm border border-border px-xs py-xxs">
            <span className="truncate font-semibold text-foreground">{t("whatsNewScene.answer.question")}</span>
            {CHOICES.map((id, index) => (
              <span key={id} className={cn("truncate", row === index + 1 && "text-primary")}>
                {row === index + 1 ? "❯" : " "} {index + 1}. {t(`whatsNewScene.answer.choice${id}`)}
              </span>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
