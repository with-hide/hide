// Moving the operator's own older Herdr server onto Hide's Herdr (PRD
// herdr-takeover): the wizard's first step, Help > Set up Hide's, Overview's
// Herdr place and the line a move leaves when the operator's own `herdr` is
// behind. The core owns the offer and the move (`status.herdr_takeover`); a
// restart where Herdr cannot hand over is the desktop host's, after the
// operator saw what ends (D-13).

import { ArrowRightIcon, CheckIcon } from "lucide-react";
import { useState, type ReactNode } from "react";
import type { Actions } from "./actions";
import { Note, Status } from "./components/settings-rows";
import { Badge } from "./components/ui/badge";
import { Button } from "./components/ui/button";
import { Dialog, DialogBody, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "./components/ui/dialog";
import { Kbd } from "./components/ui/kbd";
import { hostBridge, type HerdrRestart } from "./host";
import { HERDR_UPDATE_COMMAND, leavesForLater, movedLine, overviewTakeover, stopYourselfShown } from "./herdrTakeoverRules";
import { useInterfaceTranslation } from "./i18n/client";
import { fieldLabel } from "./shortcutLabels";
import type { HerdrStatus, HerdrTakeover } from "./snapshot";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";
import { checkForUpdates } from "./updates";

/** The host's restart, one at a time from this page: `restarting` while it runs, and how the last one ended. */
function useHerdrRestart(): { restarting: boolean; ended: HerdrRestart | null; restart: () => void } {
  const [restarting, setRestarting] = useState(false);
  const [ended, setEnded] = useState<HerdrRestart | null>(null);
  const restart = () => {
    const bridge = hostBridge();
    if (!bridge || restarting) return;
    setRestarting(true);
    setEnded(null);
    const settle = (answer: HerdrRestart) => {
      setEnded(answer);
      setRestarting(false);
    };
    void bridge.restartHerdr().then(settle, () => settle("stop_failed"));
  };
  return { restarting, ended, restart };
}

/** The operator's Herdr and the one Hide moves it to, with no count (D-22). */
function VersionPair({ takeover }: { takeover: HerdrTakeover }) {
  const { t } = useInterfaceTranslation();
  const side = (version: string, label: string, current: boolean) => (
    <div className={current ? "flex min-w-0 flex-1 flex-col gap-xxs rounded-md border border-border p-sm" : "flex min-w-0 flex-1 flex-col gap-xxs rounded-md border border-primary p-sm"}>
      <span className="font-mono text-body font-semibold text-foreground">{t("herdrTakeover.version", { version })}</span>
      <span className="text-caption text-muted-foreground">{label}</span>
    </div>
  );
  return (
    <div className="flex items-center gap-sm" data-herdr-takeover-versions={`${takeover.server_version}->${takeover.version}`}>
      {side(takeover.server_version, t("herdrTakeover.yours"), true)}
      <ArrowRightIcon aria-hidden="true" className="size-(--size-icon) shrink-0 text-muted-foreground" />
      {side(takeover.version, t("herdrTakeover.inHide"), false)}
    </div>
  );
}

function Kept({ rows }: { rows: { label: string; ends?: boolean }[] }) {
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

/** Why the last move did not happen, with the step to take where Hide cannot read the server (B8, D-29). */
function MoveFailure({ takeover }: { takeover: HerdrTakeover }) {
  const { t } = useInterfaceTranslation();
  if (takeover.stage !== "failed" || !takeover.failure) return null;
  return (
    <Note tone="error" data-herdr-takeover-failure={takeover.failure}>
      {t(`herdrTakeover.failure.${takeover.failure}`)}
      {stopYourselfShown(takeover) ? <span className="block text-muted-foreground" data-herdr-takeover-stop-yourself="true">{t("herdrTakeover.stopYourself")}</span> : null}
    </Note>
  );
}

/** The operator's own `herdr` is behind the moved server: what fails and the one command that fixes it (B5). */
function PathBehind({ version }: { version: string }) {
  const { t } = useInterfaceTranslation();
  const [copied, setCopied] = useState(false);
  const copy = () => {
    void navigator.clipboard?.writeText(HERDR_UPDATE_COMMAND).then(() => setCopied(true), () => setCopied(false));
  };
  return (
    <div className="flex flex-wrap items-center gap-sm rounded-md bg-card px-md py-sm" data-herdr-takeover-path-behind={version}>
      <span className="min-w-0 flex-1 text-caption text-subtle-foreground">{t("herdrTakeover.moved.pathBehind", { version })}</span>
      <code className="font-mono text-caption text-foreground">{HERDR_UPDATE_COMMAND}</code>
      <Button variant="secondary" size="sm" onClick={copy} data-herdr-takeover-copy="true">
        {copied ? <CheckIcon aria-hidden="true" /> : null}
        {copied ? t("herdrTakeover.copied") : t("herdrTakeover.copy")}
      </Button>
    </div>
  );
}

/** What a restart ends and brings back (B11): shown before anything changes. */
function RestartTerms() {
  const { t } = useInterfaceTranslation();
  return (
    <>
      <Kept rows={[{ label: t("herdrTakeover.restart.restores") }, { label: t("herdrTakeover.restart.ends"), ends: true }]} />
      <p className="text-caption text-muted-foreground">{t("herdrTakeover.restart.note")}</p>
    </>
  );
}

type StepParts = { title: string; description: string; body: ReactNode; footer: ReactNode; primary: (() => void) | null };

/**
 * The wizard's Herdr step (B1-B13): the move with what it keeps, its
 * failure and retry, the moved server's `herdr update` line, or the
 * restart with what ends. `onLater` is Not now, where the step offers it,
 * and `onMoved` the moved step's Continue.
 */
function herdrStepParts({ takeover, actions, restart, t, onLater, onMoved }: {
  takeover: HerdrTakeover;
  actions: Actions;
  restart: ReturnType<typeof useHerdrRestart>;
  t: ReturnType<typeof useInterfaceTranslation>["t"];
  onLater: () => void;
  onMoved: () => void;
}): StepParts {
  const later = leavesForLater(takeover) ? (
    <Button variant="ghost" onClick={onLater} data-herdr-takeover-later="true">{t("herdrTakeover.later")}</Button>
  ) : null;
  if (takeover.stage === "moved") {
    return {
      title: t("herdrTakeover.moved.title"),
      description: t("herdrTakeover.moved.description", { version: takeover.version }),
      body: takeover.path_herdr_behind ? <PathBehind version={takeover.path_herdr_behind} /> : null,
      footer: <MainButton onClick={onMoved} data-herdr-takeover-continue="true">{t("herdrTakeover.continue")}</MainButton>,
      primary: onMoved,
    };
  }
  if (takeover.kind === "restart") {
    return {
      title: t("herdrTakeover.restart.title"),
      description: t("herdrTakeover.restart.description"),
      body: (
        <>
          <RestartTerms />
          {restart.ended === "stop_failed" ? <Note tone="error" data-herdr-takeover-failure="stop_failed">{t("herdrTakeover.restart.stopFailed")}</Note> : null}
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
  const moving = takeover.stage === "moving";
  const move = () => {
    if (!moving) actions.herdrTakeover("move");
  };
  return {
    title: t("herdrTakeover.move.title"),
    description: t("herdrTakeover.move.description"),
    body: (
      <>
        <VersionPair takeover={takeover} />
        <div className="flex flex-col gap-xs">
          <span className="text-caption text-muted-foreground">{t("herdrTakeover.keeps")}</span>
          <Kept rows={[{ label: t("herdrTakeover.keeps.workspaces") }, { label: t("herdrTakeover.keeps.running") }]} />
        </div>
        <p className="text-caption text-muted-foreground">
          {t("herdrTakeover.move.terminal")}
          {takeover.same_protocol ? null : ` ${t("herdrTakeover.move.unreadable")}`}
        </p>
        <MoveFailure takeover={takeover} />
      </>
    ),
    footer: (
      <>
        {later}
        <MainButton onClick={move} disabled={moving} aria-busy={moving} data-herdr-takeover-move={takeover.stage}>
          {moving ? t("herdrTakeover.move.moving") : takeover.stage === "failed" ? t("herdrTakeover.retry") : t("herdrTakeover.move.button")}
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

/** The wizard's Herdr step, drawn into the wizard's dialog (B1). */
export function FirstRunHerdr({ actions, takeover, primary, onLater, onMoved }: {
  actions: Actions;
  takeover: HerdrTakeover;
  primary: { current: (() => void) | null };
  onLater: () => void;
  onMoved: () => void;
}) {
  const { t } = useInterfaceTranslation();
  const restart = useHerdrRestart();
  const parts = herdrStepParts({ takeover, actions, restart, t, onLater, onMoved });
  primary.current = parts.primary;
  return (
    <>
      <DialogHeader>
        <DialogTitle>{parts.title}</DialogTitle>
        <DialogDescription>{parts.description}</DialogDescription>
      </DialogHeader>
      <DialogBody className="flex flex-col gap-md" data-herdr-takeover-step={`${takeover.kind}:${takeover.stage}`}>
        {parts.body}
      </DialogBody>
      <DialogFooter>
        <span className="flex-1" />
        {parts.footer}
      </DialogFooter>
    </>
  );
}

/**
 * Overview's Herdr place for this machine (B16, B18, B19): a server Hide
 * cannot read gets its version, one line and the move (or the restart, behind
 * the same confirmation as the wizard's); a newer protocol, the update. A
 * mismatch Hide has nothing to offer for is the one line alone. Null when
 * this machine's Herdr is nothing of these.
 */
export function HerdrOverview({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const takeover = useShellStore((s) => s.rest?.status?.herdr_takeover ?? null);
  const herdr = useShellStore((s) => s.rest?.status?.herdr);
  const desktop = hostBridge() !== null;
  const shown = overviewTakeover(takeover);
  const restart = useHerdrRestart();
  const [confirming, setConfirming] = useState(false);
  if (!shown) return herdr?.state === "protocol_mismatch" ? <Mismatch herdr={herdr} /> : null;
  const current = takeover!;
  const badge = <Badge variant="outline" className="font-mono">{t("herdrTakeover.version", { version: current.server_version })}</Badge>;
  if (shown === "update_hide") {
    return (
      <Place kind="update_hide" badge={badge} title={t("herdrTakeover.overview.update")} hint={t("herdrTakeover.overview.updateHint")}>
        {desktop ? (
          <Button onClick={() => { checkForUpdates(); useUiStore.getState().openSettings("about"); }} data-herdr-takeover-check-updates="true">
            {t("herdrTakeover.overview.checkUpdates")}
          </Button>
        ) : null}
      </Place>
    );
  }
  if (shown === "restart") {
    return (
      <Place kind="restart" badge={badge} title={t("herdrTakeover.overview.mismatch")} hint={t("herdrTakeover.overview.restartHint", { version: current.version })}>
        {desktop ? (
          <Button variant="secondary" onClick={() => setConfirming(true)} disabled={restart.restarting} data-herdr-takeover-restart-open="true">
            {restart.restarting ? t("herdrTakeover.restart.restarting") : t("herdrTakeover.restart.button")}
          </Button>
        ) : (
          <Note data-herdr-takeover-desktop-only="true">{t("herdrTakeover.restart.desktopOnly")}</Note>
        )}
        {restart.ended === "stop_failed" ? <Note tone="error" data-herdr-takeover-failure="stop_failed">{t("herdrTakeover.restart.stopFailed")}</Note> : null}
        <Dialog open={confirming} onOpenChange={setConfirming}>
          <DialogContent initialFocus="container" aria-describedby={undefined} data-herdr-takeover-confirm="restart">
            <DialogHeader>
              <DialogTitle>{t("herdrTakeover.restart.title")}</DialogTitle>
              <DialogDescription>{t("herdrTakeover.restart.description")}</DialogDescription>
            </DialogHeader>
            <DialogBody className="flex flex-col gap-md">
              <RestartTerms />
            </DialogBody>
            <DialogFooter>
              <span className="flex-1" />
              <Button variant="ghost" onClick={() => setConfirming(false)}>{t("common.cancel")}</Button>
              <Button variant="destructive" onClick={() => { setConfirming(false); restart.restart(); }} data-herdr-takeover-restart="true">
                {t("herdrTakeover.restart.button")}
              </Button>
            </DialogFooter>
          </DialogContent>
        </Dialog>
      </Place>
    );
  }
  const moving = current.stage === "moving";
  return (
    <Place kind="move" badge={badge} title={t("herdrTakeover.overview.mismatch")} hint={t("herdrTakeover.overview.moveHint", { version: current.version })}>
      <Button onClick={() => actions.herdrTakeover("move")} disabled={moving} aria-busy={moving} data-herdr-takeover-move={current.stage}>
        {moving ? t("herdrTakeover.move.moving") : current.stage === "failed" ? t("herdrTakeover.retry") : t("herdrTakeover.move.button")}
      </Button>
      <MoveFailure takeover={current} />
    </Place>
  );
}

function Place({ kind, badge, title, hint, children }: { kind: string; badge: ReactNode; title: string; hint: string | null; children?: ReactNode }) {
  return (
    <div role="status" className="flex flex-1 flex-col items-center justify-center gap-md p-xl text-center" data-herdr-takeover-overview={kind}>
      {badge}
      <p className="text-subhead break-keep text-foreground">{title}</p>
      {hint ? <p className="max-w-(--size-panel-min) break-keep text-body text-subtle-foreground">{hint}</p> : null}
      {children}
    </div>
  );
}

/** A Herdr on another protocol Hide offers nothing for (no Herdr of its own here, a preview): the version and the one line. */
function Mismatch({ herdr }: { herdr: HerdrStatus }) {
  const { t } = useInterfaceTranslation();
  const badge = herdr.received_version ? <Badge variant="outline" className="font-mono">{t("herdrTakeover.version", { version: herdr.received_version })}</Badge> : null;
  return <Place kind="mismatch" badge={badge} title={t("herdrTakeover.overview.mismatch")} hint={null} />;
}

/** The line a move outside the wizard leaves while the operator's own `herdr` is behind; OK closes it (B5). */
export function HerdrMovedLine({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const behind = useShellStore((s) => movedLine(s.rest?.status?.herdr_takeover));
  const step = useShellStore((s) => s.rest?.ui_state?.first_run?.step);
  // The wizard's own step says it while the wizard stands on it.
  if (!behind || step === "herdr") return null;
  return (
    <div className="flex shrink-0 flex-wrap items-center gap-sm px-lg pt-sm" data-herdr-takeover-moved="true">
      <div className="min-w-0 flex-1"><PathBehind version={behind} /></div>
      <Button variant="ghost" size="sm" onClick={() => actions.herdrTakeover("continue")} data-herdr-takeover-done="true">{t("herdrTakeover.done")}</Button>
    </div>
  );
}
