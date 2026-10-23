// The operator's own Herdr server on this machine (PRD herdr-takeover): the
// wizard's first step, Help > Set up Hide's, the screen that holds all of Hide
// while the server speaks another protocol, and the line a move leaves when
// the operator's own `herdr` could not be brought up. Hide does each step it
// safely can itself (operator decision 2026-10-23): the core starts the move
// and updates that `herdr` (`status.herdr_takeover`), and the desktop app
// takes a Hide update for a newer Herdr; only a restart, which ends running
// commands, waits for the operator's press, and it is the desktop host's (D-13).

import { ArrowRightIcon, CheckIcon, LoaderCircleIcon } from "lucide-react";
import { useEffect, useRef, useState, type KeyboardEvent, type ReactNode } from "react";
import type { Actions } from "./actions";
import { Note, Status } from "./components/settings-rows";
import { Button } from "./components/ui/button";
import { Dialog, DialogBody, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "./components/ui/dialog";
import { Kbd } from "./components/ui/kbd";
import { hostBridge, keySystem, type HerdrRestart } from "./host";
import { coreLogPath, leavesForLater, movedLine, newerWait, pathHerdrUpdate, type HerdrGate as Gate } from "./herdrTakeoverRules";
import { useInterfaceTranslation } from "./i18n/client";
import { UpdateRows } from "./settings/AboutTab";
import { fieldLabel } from "./shortcutLabels";
import { holdsFieldModifier } from "./shortcuts";
import type { HerdrTakeover } from "./snapshot";
import { useShellStore } from "./store";
import { checkForUpdates, readyVersion, restartIntoUpdate, useUpdateStatus } from "./updates";

/** How long a restart that went through holds its button while the core's look takes the offer away. */
export const RESTART_HOLD_MS = 15_000;

/**
 * The host's restart, one at a time from this page: `restarting` while it runs. A stop that did not end the server
 * is the core's to hold, as a failed move is, so the step keeps it and offers Not now (D-29).
 */
function useHerdrRestart(actions: Actions): { restarting: boolean; restart: () => void } {
  const [restarting, setRestarting] = useState(false);
  const [ended, setEnded] = useState<HerdrRestart | null>(null);
  // A restart that went through asks the core to look at the server, since the connection state may not have
  // moved, and holds the button while that look takes the offer away: a second press would stop Hide's own
  // Herdr and end the panes' commands again. The hold is bounded, so an offer that stays never strands the step.
  const held = restarting && ended === "restarted";
  useEffect(() => {
    if (!held) return;
    const timer = setTimeout(() => setRestarting(false), RESTART_HOLD_MS);
    return () => clearTimeout(timer);
  }, [held]);
  const restart = () => {
    const bridge = hostBridge();
    if (!bridge || restarting) return;
    setRestarting(true);
    setEnded(null);
    const settle = (answer: HerdrRestart) => {
      setEnded(answer);
      setRestarting(answer === "restarted");
      if (answer === "restarted") actions.herdrTakeover("look");
      if (answer === "stop_failed") actions.herdrTakeover("stop_failed");
    };
    void bridge.restartHerdr().then(settle, () => settle("stop_failed"));
  };
  return { restarting, restart };
}

/** The operator's Herdr now and the one it runs after the move or restart, with no count (D-22). */
function VersionPair({ takeover, after }: { takeover: HerdrTakeover; after: string }) {
  const { t } = useInterfaceTranslation();
  const side = (version: string, label: string, current: boolean) => (
    <div className={current ? "flex min-w-0 flex-1 flex-col gap-xxs rounded-md border border-border p-sm" : "flex min-w-0 flex-1 flex-col gap-xxs rounded-md border border-primary p-sm"}>
      <span className="font-mono text-body font-semibold text-foreground">{t("herdrTakeover.version", { version })}</span>
      <span className="text-caption text-muted-foreground">{label}</span>
    </div>
  );
  return (
    <div className="flex items-center gap-sm" data-herdr-takeover-versions={`${takeover.server_version}->${takeover.version}`}>
      {side(takeover.server_version, t("herdrTakeover.before"), true)}
      <ArrowRightIcon aria-hidden="true" className="size-(--size-icon) shrink-0 text-muted-foreground" />
      {side(takeover.version, after, false)}
    </div>
  );
}

/** What the step does, as ✓ lines for what stays and ! lines for what it ends. */
function Lines({ rows }: { rows: { label: string; ends?: boolean }[] }) {
  return (
    <ul className="flex flex-col gap-xs">
      {rows.map((row) => (
        <li key={row.label}>
          <Status tone={row.ends ? "warn" : "ok"}>{row.label}</Status>
        </li>
      ))}
    </ul>
  );
}

/** Why the last move or restart did not happen, as its result; the cause's detail is the log's (B8, D-29). */
function Failure({ takeover }: { takeover: HerdrTakeover }) {
  const { t } = useInterfaceTranslation();
  if (takeover.stage !== "failed" || !takeover.failure) return null;
  return (
    <Note tone="error" data-herdr-takeover-failure={takeover.failure}>
      {t(`herdrTakeover.failure.${takeover.failure}`)}
    </Note>
  );
}

/** The operator's own `herdr` after a move: Hide updating it, updated, or the one command left to copy (B5). */
function MovedPath({ path, actions }: { path: NonNullable<HerdrTakeover["path_herdr"]>; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  if (path.stage === "updating") {
    return (
      <span className="flex items-center gap-xs text-body text-subtle-foreground" data-herdr-takeover-path={path.stage}>
        <LoaderCircleIcon aria-hidden="true" className="size-(--size-icon) animate-spin" />
        {t("herdrTakeover.moved.pathUpdating")}
      </span>
    );
  }
  if (path.stage === "updated") {
    return <Status tone="ok" data-herdr-takeover-path={path.stage}>{t("herdrTakeover.moved.pathUpdated", { version: path.version })}</Status>;
  }
  return <PathBehind version={path.version} actions={actions} />;
}

/** The operator's own `herdr` that Hide could not bring up: what fails, and the one command to copy. */
function PathBehind({ version, actions }: { version: string; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const update = pathHerdrUpdate(useShellStore((s) => s.rest?.status?.herdr_versions));
  return (
    <div className="flex flex-col gap-sm rounded-md bg-card px-md py-sm" data-herdr-takeover-path="behind" data-herdr-takeover-path-behind={version}>
      <Status tone="warn">{t("herdrTakeover.moved.pathBehind", { version })}</Status>
      {update ? <UpdateCommand update={update} caption={false} actions={actions} /> : null}
    </div>
  );
}

/**
 * The command that brings the operator's own `herdr` up, with a Copy, and in Settings > About what it does. Hide runs
 * it itself after a move unless it asks for `sudo`, so it is shown where Hide could not, and in About.
 */
export function UpdateCommand({ update, caption = true, actions }: { update: NonNullable<ReturnType<typeof pathHerdrUpdate>>; caption?: boolean; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const [copied, setCopied] = useState(false);
  const copy = () => {
    void actions.copyText(update.command, "herdr update command").then(setCopied);
  };
  return (
    <div className="flex flex-wrap items-center gap-sm" data-herdr-update-command={update.command}>
      <span className="flex min-w-0 flex-1 flex-col gap-xxs">
        <code className="break-all font-mono text-caption text-foreground">{update.command}</code>
        {caption ? (
          <span className="text-caption text-muted-foreground">
            {update.brew ? t("herdrTakeover.pathCommand.brew") : t("herdrTakeover.pathCommand.copy", { version: update.hide })}
          </span>
        ) : null}
      </span>
      <Button variant="secondary" size="sm" onClick={copy} data-herdr-takeover-copy="true">
        {copied ? <CheckIcon aria-hidden="true" /> : null}
        {copied ? t("herdrTakeover.copied") : t("herdrTakeover.copy")}
      </Button>
    </div>
  );
}

/** A step's title, its one-line description when it has one, its body and buttons; `busy` spins beside the title. */
type StepParts = { title: string; description: string | null; busy?: boolean; body: ReactNode; footer: ReactNode; primary: (() => void) | null };

/**
 * The Herdr step (B1-B13): the move Hide makes on its own with what it keeps, its failure and retry, the moved
 * result with the operator's own `herdr`, the restart with what ends, or a look that could not read the server with
 * its retry. `onLater` is Not now where the step offers it (the screen that holds Hide offers none), and `onMoved`
 * the moved step's Continue.
 */
function herdrStepParts({ takeover, actions, restart, t, onLater, onMoved, desktop }: {
  takeover: HerdrTakeover;
  actions: Actions;
  restart: ReturnType<typeof useHerdrRestart>;
  t: ReturnType<typeof useInterfaceTranslation>["t"];
  onLater: (() => void) | null;
  onMoved: () => void;
  desktop: boolean;
}): StepParts {
  const later = onLater && leavesForLater(takeover) ? (
    <Button variant="ghost" onClick={onLater} data-herdr-takeover-later="true">{t("herdrTakeover.later")}</Button>
  ) : null;
  if (takeover.stage === "moved") {
    return {
      title: t("herdrTakeover.moved.title"),
      description: t("herdrTakeover.moved.description"),
      body: takeover.path_herdr ? <MovedPath path={takeover.path_herdr} actions={actions} /> : null,
      footer: <MainButton onClick={onMoved} data-herdr-takeover-continue="true">{t("herdrTakeover.continue")}</MainButton>,
      primary: onMoved,
    };
  }
  if (takeover.kind === "look") {
    const look = () => actions.herdrTakeover("look");
    return {
      title: t("herdrTakeover.look.title"),
      description: t("herdrTakeover.look.description"),
      body: null,
      footer: (
        <>
          {later}
          <MainButton onClick={look} data-herdr-takeover-look="true">{t("herdrTakeover.retry")}</MainButton>
        </>
      ),
      primary: look,
    };
  }
  if (takeover.kind === "restart") {
    const terms = (
      <>
        <VersionPair takeover={takeover} after={t("herdrTakeover.afterRestart")} />
        <Lines rows={[{ label: t("herdrTakeover.restart.restores") }, { label: t("herdrTakeover.restart.ends"), ends: true }]} />
      </>
    );
    if (!desktop) {
      return {
        title: t("herdrTakeover.restart.title"),
        description: null,
        body: (
          <>
            {terms}
            <Note data-herdr-takeover-desktop-only="true">{t("herdrTakeover.restart.desktopOnly")}</Note>
          </>
        ),
        footer: later,
        primary: null,
      };
    }
    return {
      title: t("herdrTakeover.restart.title"),
      description: null,
      body: (
        <>
          {terms}
          {restart.restarting ? null : <Failure takeover={takeover} />}
        </>
      ),
      // The button names what it does and holds no keyboard: Enter presses nothing here (B11).
      footer: (
        <>
          {restart.restarting ? null : later}
          <Button variant="destructive" onClick={restart.restart} disabled={restart.restarting} aria-busy={restart.restarting} data-herdr-takeover-restart="true">
            {restart.restarting ? t("herdrTakeover.restart.restarting") : t("herdrTakeover.restart.button")}
          </Button>
        </>
      ),
      primary: null,
    };
  }
  // A move starts on its own as soon as the core offers it; the button is the retry after a failure.
  const moving = takeover.stage === "moving";
  const move = () => {
    if (!moving) actions.herdrTakeover("move");
  };
  return {
    title: t(moving ? "herdrTakeover.move.titleMoving" : "herdrTakeover.move.title", { version: takeover.version }),
    description: null,
    busy: moving,
    body: (
      <>
        <VersionPair takeover={takeover} after={t("herdrTakeover.afterMove")} />
        <Lines rows={[{ label: t("herdrTakeover.move.keeps") }, { label: t("herdrTakeover.move.terminal"), ends: true }]} />
        <Failure takeover={takeover} />
      </>
    ),
    footer: moving ? null : (
      <>
        {later}
        <MainButton onClick={move} data-herdr-takeover-move={takeover.stage}>
          {takeover.stage === "failed" ? t("herdrTakeover.retry") : t("herdrTakeover.move.button")}
        </MainButton>
      </>
    ),
    primary: moving ? null : move,
  };
}

function MainButton(props: React.ComponentProps<typeof Button>) {
  return (
    <span className="flex items-center gap-sm">
      <Button {...props} />
      <Kbd>{fieldLabel("Enter")}</Kbd>
    </span>
  );
}

/** A step's title, description, body and buttons, as the wizard's dialog and the screen that holds Hide draw them. */
function StepLayout({ parts, step, leading }: { parts: StepParts; step: string; leading?: ReactNode }) {
  return (
    <>
      <DialogHeader>
        <DialogTitle className="flex items-center gap-sm" aria-busy={parts.busy ? true : undefined}>
          {parts.busy ? <LoaderCircleIcon aria-hidden="true" className="size-(--size-icon) shrink-0 animate-spin text-muted-foreground" data-herdr-takeover-busy="true" /> : null}
          {parts.title}
        </DialogTitle>
        {parts.description ? <DialogDescription>{parts.description}</DialogDescription> : null}
      </DialogHeader>
      {parts.body ? (
        <DialogBody className="flex flex-col gap-md" data-herdr-takeover-step={step}>
          {parts.body}
        </DialogBody>
      ) : <div className="h-md" data-herdr-takeover-step={step} />}
      <DialogFooter>
        {leading}
        <span className="flex-1" />
        {parts.footer}
      </DialogFooter>
    </>
  );
}

/** The wizard's Herdr step, drawn into the wizard's dialog (B1). */
export function FirstRunHerdr({ actions, takeover, primary, onLater, onMoved }: {
  actions: Actions;
  takeover: HerdrTakeover;
  primary: { current: (() => void) | null };
  onLater: () => void;
  onMoved: () => void;
}) {
  const { t } = useInterfaceTranslation();
  const restart = useHerdrRestart(actions);
  const parts = herdrStepParts({ takeover, actions, restart, t, onLater, onMoved, desktop: hostBridge() !== null });
  primary.current = parts.primary;
  return <StepLayout parts={parts} step={`${takeover.kind}:${takeover.stage}`} />;
}

/**
 * The screen that holds all of Hide while this machine's Herdr speaks another protocol (`herdrGate`): drawn in place of
 * the shell, with no Not now and nothing behind it to reach, so no key, menu item or window opens a workspace on a
 * server Hide cannot use. It offers the core's move or restart, a newer Herdr's wait for a Hide update, or a look again;
 * View log and Quit stand beside them in the desktop app. Every system draws it; only the restart is the desktop's.
 */
export function HerdrGate({ actions, gate }: { actions: Actions; gate: Gate }) {
  const { t } = useInterfaceTranslation();
  const takeover = useShellStore((s) => s.rest?.status?.herdr_takeover ?? null);
  const herdr = useShellStore((s) => s.rest?.status?.herdr);
  const statePath = useShellStore((s) => s.daemon?.core_state_path ?? null);
  const bridge = hostBridge();
  const restart = useHerdrRestart(actions);
  const updates = useHideUpdateFor(bridge !== null && (gate === "update_hide" || gate === "newer"));
  // A restart stops the old server before Hide's starts, so for a moment the core offers nothing while the connection
  // still reads the old protocol; the screen keeps the restart it is running rather than calling the server incompatible.
  const last = useRef(takeover);
  if (takeover) last.current = takeover;
  const shown = takeover ?? (restart.restarting && last.current?.kind === "restart" ? last.current : null);
  const look = () => actions.herdrTakeover("look");
  const again = <Button variant={gate === "older" ? "default" : "secondary"} onClick={look} data-herdr-gate-look="true">{t("herdrTakeover.retry")}</Button>;
  let parts: StepParts;
  if (shown && (gate === "move" || gate === "restart" || gate === "look" || shown !== takeover)) {
    parts = herdrStepParts({ takeover: shown, actions, restart, t, onLater: null, onMoved: () => {}, desktop: bridge !== null });
  } else if (gate === "update_hide" || gate === "newer") {
    const server = takeover?.server_version || herdr?.received_version || null;
    const wait = newerWait(bridge ? updates : null);
    parts = {
      title: t(wait === "downloading" ? "herdrTakeover.newer.downloading" : "herdrTakeover.newer.title"),
      description: wait === "none" && server ? t("herdrTakeover.newer.none", { server }) : null,
      busy: wait === "downloading" || wait === "checking",
      body: (
        <>
          {/* A check that failed or an app that cannot update itself says why in About's own row. */}
          {wait === "failed" ? <div className="flex flex-col" data-herdr-gate-update="true"><UpdateRows /></div> : null}
          <Lines rows={[{ label: t("herdrTakeover.newer.keeps") }]} />
        </>
      ),
      footer: (
        <>
          {again}
          {bridge ? <Button onClick={checkForUpdates} data-herdr-takeover-check-updates="true">{t("herdrTakeover.checkUpdates")}</Button> : null}
        </>
      ),
      primary: bridge ? checkForUpdates : look,
    };
  } else {
    parts = {
      title: t("herdrTakeover.mismatch.title"),
      description: t("herdrTakeover.mismatch.description"),
      body: null,
      footer: again,
      primary: look,
    };
  }
  const log = statePath ? coreLogPath(statePath) : null;
  const leading = bridge ? (
    <>
      {log ? <Button variant="ghost" onClick={() => bridge.revealPath(log)} data-herdr-gate-log="true">{t("herdrTakeover.viewLog")}</Button> : null}
      <Button variant="ghost" onClick={() => bridge.quit()} data-herdr-gate-quit="true">{t("herdrTakeover.quit")}</Button>
    </>
  ) : null;
  const primary = useRef(parts.primary);
  primary.current = parts.primary;
  // ⌘↵ (Ctrl+Enter off macOS) anywhere on the screen, or Enter on the screen itself, presses its main button, as the wizard's does (B40).
  const onKeyDown = (event: KeyboardEvent) => {
    if (event.key !== "Enter" || event.nativeEvent.isComposing) return;
    const target = event.target as HTMLElement;
    if (!holdsFieldModifier(event, keySystem()) && target.closest("button, a")) return;
    if (!primary.current) return;
    event.preventDefault();
    primary.current();
  };
  return (
    <Dialog open>
      <DialogContent
        initialFocus="container"
        aria-describedby={undefined}
        data-herdr-gate={gate}
        className="w-(--size-onboarding-dialog-w) [&_[data-slot=dialog-title]]:text-headline"
        onKeyDown={onKeyDown}
        // Nothing lies behind it to go back to.
        onEscapeKeyDown={(event) => event.preventDefault()}
        onPointerDownOutside={(event) => event.preventDefault()}
        onInteractOutside={(event) => event.preventDefault()}
      >
        <StepLayout parts={parts} step={`gate:${gate}`} leading={leading} />
      </DialogContent>
    </Dialog>
  );
}

/**
 * A Herdr newer than Hide waits for a Hide made for it, so the desktop app looks for one at once rather than after its
 * launch delay and opens it as soon as it is downloaded, through About's own Restart: the core and its agents keep
 * running, and the reopened app reads that Herdr. Nothing here touches the operator's Herdr.
 */
function useHideUpdateFor(waiting: boolean) {
  const status = useUpdateStatus();
  const ready = readyVersion(status);
  const opened = useRef<string | null>(null);
  useEffect(() => {
    if (waiting) checkForUpdates();
  }, [waiting]);
  useEffect(() => {
    // One Restart per downloaded version: an install that failed keeps it ready, and About still offers it.
    if (!waiting || !ready || opened.current === ready) return;
    opened.current = ready;
    restartIntoUpdate();
  }, [waiting, ready]);
  return status;
}

/** The line a move outside the wizard leaves while the operator's own `herdr` could not be brought up; OK closes it (B5). */
export function HerdrMovedLine({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const behind = useShellStore((s) => movedLine(s.rest?.status?.herdr_takeover));
  const step = useShellStore((s) => s.rest?.ui_state?.first_run?.step);
  // The wizard's own step says it while the wizard stands on it.
  if (!behind || step === "herdr") return null;
  return (
    <div className="flex shrink-0 flex-wrap items-center gap-sm px-lg pt-sm" data-herdr-takeover-moved="true">
      <div className="min-w-0 flex-1"><PathBehind version={behind} actions={actions} /></div>
      <Button variant="ghost" size="sm" onClick={() => actions.herdrTakeover("continue")} data-herdr-takeover-done="true">{t("herdrTakeover.done")}</Button>
    </div>
  );
}
