import { ArrowRightIcon, CircleHelpIcon, RotateCcwIcon } from "lucide-react";
import { useEffect, useState, type ReactNode } from "react";
import { AgentMark } from "../AgentMark";
import { useInterfaceTranslation } from "../i18n/client";
import { cn } from "../lib/utils";
import { useReducedMotion } from "../lib/useReducedMotion";
import { KindMark, type WorkKind } from "./kind-mark";
import { StepNumber } from "./step-number";
import { Button } from "./ui/button";
import { Hint } from "./ui/tooltip";
import { Dialog, DialogBody, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "./ui/dialog";

// The one window that says how a mission differs from a task, opened by the
// `?` beside the kind mark on a mission's page, a plan's page and Settings ›
// Mission. Two stages side by side, each a short picture that plays once when
// the window opens and again on `다시 보기`: a Conductor sends work to three
// workers at the same moment, one agent finishes four steps in turn. The side
// the press came from is lit a little. With the reduced-motion setting on,
// each picture is drawn finished and nothing moves.

const TICK_MS = 450;
/** The picture's last moment: every step done, every worker lit. */
const END = 8;

/** Which moment of the picture it is, 0 to `END`; reduced motion is at the end from the start. */
function useMoment(run: number, still: boolean): number {
  const [moment, setMoment] = useState(still ? END : 0);
  useEffect(() => {
    if (still) {
      setMoment(END);
      return undefined;
    }
    let at = 0;
    setMoment(0);
    const timer = window.setInterval(() => {
      at += 1;
      setMoment(at);
      if (at >= END) window.clearInterval(timer);
    }, TICK_MS);
    return () => window.clearInterval(timer);
  }, [run, still]);
  return moment;
}

/** The `?` that opens the one window on how a mission differs from a task; `from` is the side the press came from, which the window lights. */
export function KindHelpButton({ from, className }: { from: WorkKind; className?: string }) {
  const { t } = useInterfaceTranslation();
  const [open, setOpen] = useState(false);
  return (
    <>
      <Hint label={t("kind.help.open")}>
        <Button variant="ghost" size="icon-xs" className={cn("shrink-0 text-muted-foreground", className)} aria-label={t("kind.help.open")} aria-haspopup="dialog" onClick={() => setOpen(true)} data-kind-help={from}>
          <CircleHelpIcon />
        </Button>
      </Hint>
      <KindHelpDialog from={from} open={open} onOpenChange={setOpen} />
    </>
  );
}

export function KindHelpDialog({ from, open, onOpenChange }: { from: WorkKind; open: boolean; onOpenChange: (open: boolean) => void }) {
  const { t } = useInterfaceTranslation();
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="w-(--size-kind-help-dialog)" data-kind-help-dialog={from}>
        <DialogHeader>
          <DialogTitle>{t("kind.help.title")}</DialogTitle>
          <DialogDescription>{t("kind.help.lead")}</DialogDescription>
        </DialogHeader>
        <Stages from={from} />
      </DialogContent>
    </Dialog>
  );
}

/** Mounted only while the window is open, so the pictures start at their beginning each time. */
function Stages({ from }: { from: WorkKind }) {
  const { t } = useInterfaceTranslation();
  const still = useReducedMotion();
  const [run, setRun] = useState(0);
  const moment = useMoment(run, still);
  return (
    <>
      <DialogBody className="flex flex-wrap gap-md">
        <Stage kind="mission" lit={from === "mission"} picture={t("kind.help.mission.picture")} head={t("kind.help.mission.head")} body={t("kind.help.mission.body")}>
          <MissionPicture moment={moment} still={still} run={run} />
        </Stage>
        <Stage kind="task" lit={from === "task"} picture={t("kind.help.task.picture")} head={t("kind.help.task.head")} body={t("kind.help.task.body")}>
          <TaskPicture moment={moment} />
        </Stage>
      </DialogBody>
      <DialogFooter className="justify-between">
        {still ? <span /> : (
          <Button variant="text" size="inline" className="text-caption text-muted-foreground" onClick={() => setRun((n) => n + 1)} data-kind-help-replay="true">
            <RotateCcwIcon aria-hidden="true" />
            {t("kind.help.replay")}
          </Button>
        )}
      </DialogFooter>
    </>
  );
}

function Stage({ kind, lit, picture, head, body, children }: { kind: WorkKind; lit: boolean; picture: string; head: string; body: string; children: ReactNode }) {
  return (
    <section className={cn("flex min-w-0 flex-1 basis-(--size-kind-help-stage) flex-col gap-sm rounded-md border p-md", lit ? "border-input bg-card" : "border-border")} data-kind-help-stage={kind} data-kind-help-lit={lit || undefined}>
      <KindMark kind={kind} className="self-start" />
      <div role="img" aria-label={picture} className="flex h-(--size-kind-help-picture) min-w-0 flex-col justify-center">
        {children}
      </div>
      <p className="text-caption text-subtle-foreground">
        <b className="font-semibold text-foreground">{head}</b> {body}
      </p>
    </section>
  );
}

const WORKERS = ["claude", "codex", "claude"] as const;
/** The three lines' feet, as thirds of the picture's width, so each lands over its worker. */
const FEET = [43.3, 130, 216.7] as const;

/** A Conductor, and under it three workers it sends work to at once: three dots run down their lines together, then the workers light. */
function MissionPicture({ moment, still, run }: { moment: number; still: boolean; run: number }) {
  const { t } = useInterfaceTranslation();
  const lit = moment >= 2;
  return (
    <div className="flex min-w-0 flex-col items-center" aria-hidden="true" data-kind-help-moment={moment}>
      <span className="inline-flex items-center gap-xs rounded-md border border-kind-mission bg-secondary px-sm py-xs text-caption font-semibold">
        <AgentMark kind="claude" />
        {t("kind.help.conductor")}
      </span>
      <svg viewBox="0 0 260 56" className="h-(--size-kind-help-lines) w-full" fill="none">
        {FEET.map((foot) => (
          <path key={foot} d={`M130 0 C130 28 ${foot} 28 ${foot} 56`} className="stroke-border" strokeWidth="1.5" />
        ))}
        {!still && !lit
          ? FEET.map((foot) => (
              <circle key={`${run}-${foot}`} r="3.5" className="fill-kind-mission">
                <animateMotion path={`M130 0 C130 28 ${foot} 28 ${foot} 56`} dur="0.7s" fill="freeze" />
              </circle>
            ))
          : null}
      </svg>
      <div className="grid w-full grid-cols-3 gap-sm">
        {WORKERS.map((agent, index) => (
          <span key={index} className={cn("flex min-w-0 items-center justify-center gap-xs rounded-md border bg-card px-xs py-xs text-micro motion-safe:transition-colors", lit ? "border-kind-mission text-foreground" : "border-border text-muted-foreground")} data-kind-help-worker={lit ? "lit" : "idle"}>
            <AgentMark kind={agent} />
            <span className="truncate">{t("kind.help.worker")}</span>
          </span>
        ))}
      </div>
    </div>
  );
}

/** One agent, and four steps it finishes one after another: the arrow stands at the step at work. */
function TaskPicture({ moment }: { moment: number }) {
  const { t } = useInterfaceTranslation();
  return (
    <div className="flex min-w-0 items-center gap-md" aria-hidden="true" data-kind-help-moment={moment}>
      <span className="flex shrink-0 items-center rounded-md border border-kind-task bg-secondary p-sm">
        <AgentMark kind="claude" />
      </span>
      <ol className="flex min-w-0 flex-1 flex-col gap-xs">
        {([1, 2, 3, 4] as const).map((number) => {
          const status = moment >= number * 2 ? "done" : moment === number * 2 - 1 ? "working" : "planned";
          return (
            <li key={number} className="flex min-w-0 items-center gap-xs text-caption" data-kind-help-step={status}>
              <span className="flex size-(--size-icon) shrink-0 items-center justify-center text-kind-task">{status === "working" ? <ArrowRightIcon className="size-(--size-icon-sm)" /> : null}</span>
              <StepNumber number={number} status={status} />
              <span className={cn("min-w-0 truncate", status === "planned" ? "text-muted-foreground" : status === "done" ? "text-subtle-foreground" : "font-semibold text-foreground")}>{t(`kind.help.step${number}`)}</span>
            </li>
          );
        })}
      </ol>
    </div>
  );
}
