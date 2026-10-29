import { useEffect, useRef, useState } from "react";
import type { Actions } from "../actions";
import { Button } from "../components/ui/button";
import { Dialog, DialogBody, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "../components/ui/dialog";
import { checkCommand, checkMachine, giveUpOffered, moveCopy, stepMarks, type MoveView, type StepMark } from "../coreMove";
import { useInterfaceTranslation } from "../i18n/client";
import { cn } from "../lib/utils";
import { moveMachines } from "../screenMachine";
import { useShellStore } from "../store";
import { deviceProblemLine, retrySignsIn } from "../settings";
import type { DeviceProblem } from "../snapshot";
import { FixCommand } from "./FixCommand";
import { SignInPrompt } from "./SignInPrompt";

/** What the operator asked from Settings: the core toward a device, or back to this machine from a node's window. */
export type MoveRequest = { direction: "forward"; device: string } | { direction: "back" };

/**
 * The dialog's five forms (PRD core-host-node-move B, approved round 3), the
 * wait before the first answer, and amendment 4's confirm of giving up a
 * move's wait on its other machine and its result.
 */
export type MoveForm = "checking" | "checks_failed" | "confirm" | "moving" | "give_up" | "failed" | "done" | "abandoned";

/**
 * Which form the move's frame puts the dialog in. Until the supervisor
 * answers this request (`asked` is the view the request was sent beside),
 * and for a frame about another move, the dialog waits on its checks.
 */
export function moveForm(view: MoveView | null, asked: MoveView | null, request: MoveRequest): MoveForm {
  if (!view || view === asked) return "checking";
  const direction = view.direction ?? "forward";
  if (direction !== request.direction || (request.direction === "forward" && view.device !== request.device)) return "checking";
  switch (view.state) {
    case "checks_failed":
      return "checks_failed";
    case "ready":
      return "confirm";
    case "stopping":
    case "copying":
    case "starting":
    case "linking":
    case "rolling_back":
    case "waiting":
      return "moving";
    case "rolled_back":
      return "failed";
    case "done":
      return "done";
    case "abandoned":
      return "abandoned";
    default:
      return "checking";
  }
}

/** The request a move's own frame stands for, for a dialog opened from the window's strip rather than a device's menu. */
export function requestOfView(view: MoveView): MoveRequest {
  return view.direction === "back" ? { direction: "back" } : { direction: "forward", device: view.device ?? "" };
}

/** Sends `request` and answers the frame it was sent beside, which the dialog waits past. */
export function askMove(actions: Actions, request: MoveRequest, start: boolean): MoveView | null {
  const asked = useShellStore.getState().coreMove;
  if (request.direction === "back") actions.coreMove({ action: start ? "back" : "check_back" });
  else actions.coreMove({ action: start ? "start" : "check", device: request.device });
  return asked;
}

// A step's line and its mark: only a done step's ✓ is green, its words stay the body's.
const MARK: Record<StepMark, { glyph: string; line: string; mark: string }> = {
  done: { glyph: "✓", line: "text-foreground", mark: "text-success" },
  run: { glyph: "…", line: "font-semibold text-foreground", mark: "text-foreground" },
  todo: { glyph: "-", line: "text-muted-foreground", mark: "" },
  fail: { glyph: "✕", line: "text-destructive", mark: "" },
};

/**
 * The move as one dialog over Settings (PRD core-host-node-move B, the
 * operator's choice): its body turns from the checks to the consequences to
 * the steps to the result, and every form names the reason first. The
 * dialog only asks; the supervisor's frame says where the move is, so a
 * window that closed it and opens Settings again finds the same state.
 */
export function CoreMoveDialog({ request, asked: first, giveUp = false, actions, onClose: close, onRequest }: { request: MoveRequest; asked: MoveView | null; giveUp?: boolean; actions: Actions; onClose: () => void; onRequest: (request: MoveRequest) => void }) {
  const { t } = useInterfaceTranslation();
  const view = useShellStore((s) => s.coreMove);
  // A question nobody sees any more is cancelled, which ends the checks' sign-in rather than leave it waiting
  // out its five minutes (PRD system-ssh-device-link B8).
  const onClose = () => {
    const asking = view?.sign_in;
    if (asking && (asking.kind === "secret" || asking.kind === "host_key" || asking.kind === "confirm")) actions.answerCoreMoveSignIn(asking.question, { reply: "cancel" });
    close();
  };
  const refusal = useShellStore((s) => s.coreMoveRefusal);
  const devices = useShellStore((s) => s.rest?.navigator?.devices);
  // The frame the last request was sent beside: the dialog waits until a newer one answers it.
  const [asked, setAsked] = useState<MoveView | null>(first);
  const ask = (next: MoveRequest, start: boolean) => setAsked(askMove(actions, next, start));
  // Giving up the wait is confirmed first, and only while the move's wait offers it.
  const [givingUp, setGivingUp] = useState(giveUp);
  const offered = giveUpOffered(view);
  // An offer that ended, taken or answered first, leaves no confirm behind for a later wait.
  const wasOffered = useRef(offered);
  useEffect(() => {
    if (wasOffered.current && !offered) setGivingUp(false);
    wasOffered.current = offered;
  }, [offered]);
  const asMoving = moveForm(view, asked, request);
  const form: MoveForm = asMoving === "moving" && offered && givingUp ? "give_up" : asMoving;
  const machines = moveMachines(devices, request.direction === "back" ? { direction: "back", device: null, alias: view?.alias } : { direction: "forward", device: request.device, alias: view?.alias }, t);
  const values = moveCopy(machines);
  const back = request.direction === "back";
  // The machine the move waits on: the core's machine for a move back, the device for a forward move.
  const peer = back ? machines.from : machines.to;
  const why = back ? null : t("coreMove.why", values);
  const title = (() => {
    switch (form) {
      case "checking":
      case "checks_failed":
        return back ? t("coreMove.back.title", values) : t("coreMove.title", values);
      case "confirm":
        return back ? t("coreMove.back.confirmTitle", values) : t("coreMove.confirmTitle", values);
      case "moving":
        if (view?.state === "rolling_back") return t("coreMove.rollingBackTitle");
        return back ? t("coreMove.back.movingTitle", values) : t("coreMove.movingTitle", values);
      case "give_up":
        return back ? t("coreMove.takeHere.title") : t("coreMove.giveUp.title", { machine: peer });
      case "abandoned":
        return t("coreMove.abandonedTitle", { machine: peer });
      case "failed":
        return t("coreMove.failedTitle");
      case "done":
        return back ? t("coreMove.back.doneTitle", values) : t("coreMove.doneTitle", values);
    }
  })();
  const failed = view?.failed ?? [];
  const passed = view?.checked === undefined ? null : Math.max(0, view.checked - failed.length);
  return (
    <Dialog open onOpenChange={(next) => { if (!next) onClose(); }}>
      {/* The move stops the core: no button is the default, whichever form opens (design 6). */}
      <DialogContent showCloseButton initialFocus="container" data-core-move-dialog={form} data-core-move-direction={request.direction}>
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
          {why && (form === "checking" || form === "checks_failed" || form === "confirm") ? <DialogDescription>{why}</DialogDescription> : null}
        </DialogHeader>
        <DialogBody className="space-y-sm">
          {form === "checking" ? (
            refusal ? (
              <p className="text-body text-destructive" data-core-move-refused={refusal}>
                {refusal === "move_busy" || refusal === "core_pending" ? t("coreMove.refused.busy") : refusal === "move_unavailable" || refusal === "moving" ? t("coreMove.refused.unavailable") : t("coreMove.refused.other")}
              </p>
            ) : (
              <>
                <p className="text-body text-muted-foreground" data-core-move-checking="true">{t("coreMove.checking")}</p>
                {/* What ssh asks the checks' sign-in, as a device's row shows it (PRD system-ssh-device-link B33). */}
                {view?.sign_in ? <SignInPrompt id="core-move" wait={view.sign_in} actions={actions} answer={(question, reply) => actions.answerCoreMoveSignIn(question, reply)} /> : null}
              </>
            )
          ) : null}
          {form === "checks_failed" ? (
            <>
              <p className="text-caption text-muted-foreground" data-core-move-count="true">
                {passed === null ? t("coreMove.toFix", { failed: failed.length }) : t("coreMove.toFixPassed", { failed: failed.length, passed })}
              </p>
              <ul className="space-y-xs">
                {failed.map((row) => (
                  <FailedCheck key={row.check} check={row.check} detail={row.detail} machine={checkMachine(row.check, machines)} ssh={row.check === "connection" ? (view?.ssh_problem ?? null) : null} alias={view?.alias ?? null} actions={actions} />
                ))}
              </ul>
            </>
          ) : null}
          {form === "confirm" ? (
            <>
              <p className="text-body text-foreground">{t("coreMove.consequence")}</p>
              <p className="text-body text-muted-foreground">{back ? t("coreMove.back.reversible") : t("coreMove.reversible")}</p>
            </>
          ) : null}
          {form === "moving" && view ? (
            <>
              <ol className="space-y-xs" data-core-move-steps="true">
                {stepMarks(view).map(({ step, mark }) => (
                  <li key={step} className={cn("flex items-baseline gap-sm text-body", MARK[mark].line)} data-core-move-step={step} data-core-move-mark={mark}>
                    <span aria-hidden="true" className={cn("w-(--size-icon) shrink-0 text-center", MARK[mark].mark)}>{MARK[mark].glyph}</span>
                    <span>{t(`coreMove.step.${step}`, values)}</span>
                  </li>
                ))}
              </ol>
              {view.state === "waiting" ? <p className="text-body text-warning" data-core-move-waiting="true">{t("coreMove.waiting", { machine: peer })}</p> : null}
            </>
          ) : null}
          {form === "give_up" ? (
            <>
              <p className="break-keep text-body font-semibold text-foreground">{t("coreMove.final")}</p>
              <p className="break-keep text-body text-foreground">{back ? t("coreMove.takeHere.here") : t("coreMove.giveUp.here")}</p>
              <p className="break-keep text-body text-foreground">{back ? t("coreMove.takeHere.held", { machine: peer }) : t("coreMove.giveUp.back", { machine: peer })}</p>
              {back ? null : <p className="break-keep text-body text-foreground" data-core-move-give-up-risk="true">{t("coreMove.giveUp.risk", { machine: peer })}</p>}
              {refusal ? (
                <p className="break-keep text-body text-destructive" data-core-move-refused={refusal}>
                  {refusal === "move_not_waiting" ? t("coreMove.refused.notWaiting") : refusal === "move_busy" ? t("coreMove.refused.busy") : t("coreMove.refused.other")}
                </p>
              ) : null}
            </>
          ) : null}
          {form === "abandoned" ? <p className="break-keep text-body text-subtle-foreground">{t("coreMove.abandoned", { machine: peer })}</p> : null}
          {form === "failed" && view ? (
            <>
              <p className="text-body text-destructive" data-core-move-failed={view.step ?? "check"}>
                <span aria-hidden="true">× </span>
                {t(`coreMove.failedStep.${view.step ?? "check"}`, values)}
              </p>
              <p className="text-body text-success">
                <span aria-hidden="true">✓ </span>
                {t("coreMove.unchanged", values)}
              </p>
            </>
          ) : null}
          {form === "done" ? <p className="text-body text-subtle-foreground">{t("coreMove.rescan", values)}</p> : null}
        </DialogBody>
        {/* While it moves the dialog only reports, but for giving up a wait the move offers; its header's × closes it. */}
        {form === "moving" && !offered ? null : (
        <DialogFooter>
          {form === "moving" ? (
            <Button variant="secondary" data-core-move-give-up="true" onClick={() => setGivingUp(true)}>
              {back ? t("coreMove.takeHere.take") : t("coreMove.giveUp.stop")}
            </Button>
          ) : null}
          {form === "give_up" ? (
            <>
              <Button variant="secondary" onClick={() => setGivingUp(false)}>{t("common.cancel")}</Button>
              <Button
                variant="destructive"
                data-core-move-give-up-go="true"
                onClick={() => actions.coreMove({ action: "abandon" })}
              >
                {back ? t("coreMove.takeHere.take") : t("coreMove.giveUp.stop")}
              </Button>
            </>
          ) : null}
          {form === "abandoned" ? <Button onClick={onClose}>{t("common.close")}</Button> : null}
          {form === "checking" ? (
            <Button variant="secondary" onClick={onClose}>{t("common.close")}</Button>
          ) : null}
          {form === "checks_failed" ? (
            <>
              <Button variant="secondary" onClick={onClose}>{t("common.close")}</Button>
              {/* A sign-in's check again reads Sign in, as the device row's Retry does: the checks may ask. */}
              <Button data-core-move-recheck="true" onClick={() => ask(request, false)}>{retrySignsIn(view?.ssh_problem) ? t("devices.signIn.start") : t("coreMove.recheck")}</Button>
            </>
          ) : null}
          {form === "confirm" ? (
            <>
              <Button variant="secondary" onClick={onClose}>{t("common.cancel")}</Button>
              <Button data-core-move-start="true" onClick={() => ask(request, true)}>
                {back ? t("coreMove.back.start", values) : t("coreMove.start", values)}
              </Button>
            </>
          ) : null}
          {form === "failed" ? (
            <>
              <Button variant="secondary" onClick={onClose}>{t("common.close")}</Button>
              <Button data-core-move-retry="true" onClick={() => ask(request, true)}>{t("common.retry")}</Button>
            </>
          ) : null}
          {form === "done" ? (
            <>
              {back ? null : (
                <Button
                  variant="secondary"
                  data-core-move-undo="true"
                  onClick={() => {
                    const next: MoveRequest = { direction: "back" };
                    onRequest(next);
                    ask(next, false);
                  }}
                >
                  {t("coreMove.undo")}
                </Button>
              )}
              <Button onClick={onClose}>{t("common.close")}</Button>
            </>
          ) : null}
        </DialogFooter>
        )}
      </DialogContent>
    </Dialog>
  );
}

/**
 * One check that did not pass: its name, and the one command that fixes it where there is one; what the check found
 * is its hint. A connection that stopped on ssh (`ssh`) says what the device row says of it, with the row's terminal
 * command for a changed server key.
 */
function FailedCheck({ check, detail, machine, ssh, alias, actions }: { check: MoveView["failed"][number]["check"]; detail: string; machine: string; ssh: DeviceProblem | null; alias: string | null; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const problem = deviceProblemLine(ssh, alias, t);
  const command = problem ? problem.command : checkCommand(check);
  return (
    <li className="flex min-w-0 items-center justify-between gap-md" data-core-move-check={check} title={detail}>
      <span className="min-w-0 text-body text-destructive">
        <span aria-hidden="true">× </span>
        {t(`coreMove.check.${check}`, { machine })}
      </span>
      {problem ? (
        // The device row's words for the ssh failure, and its terminal command for a changed server key.
        <span className="flex min-w-0 items-center justify-end gap-sm">
          <span className="min-w-0 text-right text-caption text-muted-foreground" data-core-move-ssh-problem={ssh ?? undefined}>
            {problem.note ?? problem.headline}
          </span>
          {command ? <FixCommand command={command} what="ssh command" actions={actions} data-core-move-command={check} /> : null}
        </span>
      ) : command ? (
        <FixCommand command={command} what="core move command" actions={actions} data-core-move-command={check} />
      ) : (
        <span className="min-w-0 text-right text-caption text-muted-foreground">{t(`coreMove.fix.${check}`, { machine })}</span>
      )}
    </li>
  );
}
