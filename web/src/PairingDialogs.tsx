// The two windows a pairing opens wherever the operator is (PRD
// device-connect-iroh B2, B4-B6, B50; D-76): on the machine being opened,
// the allow window with the four words and what the other machine would be
// allowed; on the machine that pasted a link, the same four words with
// They match and They differ, then how the pairing ended. hided sends the
// `pairing` frame to this machine's own windows only, so a screen another
// machine shows of this one draws neither.

import type { Actions } from "./actions";
import { Button } from "./components/ui/button";
import { Dialog, DialogBody, DialogContent, DialogFooter, DialogHeader, DialogTitle } from "./components/ui/dialog";
import { Note, Status } from "./components/settings-rows";
import { useInterfaceTranslation } from "./i18n/client";
import { joinFailureKey, timeLeft, type PairAsk, type PairJoin } from "./pairing";
import { askForFirewallOnce } from "./settings/firewall";
import { PairWords, useNow } from "./settings/PairingRows";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";

export function PairingDialogs({ actions }: { actions: Actions }) {
  // One at a time: the oldest machine that waits, then the next.
  const ask = useShellStore((s) => s.pairing?.asks[0] ?? null);
  const join = useShellStore((s) => s.pairing?.join ?? null);
  // The share window answers a pasted link in its own last step.
  const sharing = useUiStore((s) => s.sharing);
  if (ask && !(sharing && ask.kind === "link")) return <AllowDialog key={ask.id} ask={ask} actions={actions} />;
  if (join) return <JoinDialog key={join.id} join={join} actions={actions} />;
  return null;
}

/** The allow window (B2, B50): closing it is a no, the answer that opens nothing. */
export function AllowDialog({ ask, actions }: { ask: PairAsk; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const now = useNow(true);
  const answer = (allow: boolean) => {
    actions.pairing("answer", { ask: ask.id, allow });
    // Allowing a pairing opens this PC to it: Windows is asked once for
    // the rules that let it come in directly (B32).
    if (allow) void askForFirewallOnce();
  };
  return (
    <Dialog open onOpenChange={(next) => { if (!next) answer(false); }}>
      <DialogContent data-pair-allow={ask.kind} data-pair-allow-device={ask.device}>
        <DialogHeader>
          <DialogTitle>{t(ask.kind === "link" ? "pairing.ask.linkTitle" : "pairing.ask.requestTitle", { name: ask.name })}</DialogTitle>
        </DialogHeader>
        <DialogBody className="space-y-md">
          {ask.words ? (
            <>
              <PairWords words={ask.words} />
              <Note>{t("pairing.ask.compare")}</Note>
            </>
          ) : null}
          <p className="break-keep text-body text-foreground" data-pair-grants="true">{t("pairing.ask.grants", { name: ask.name })}</p>
        </DialogBody>
        <DialogFooter>
          <span className="mr-auto text-caption text-muted-foreground tabular-nums">{t("pairing.ask.left", { time: timeLeft(ask.expires_unix_ms, now) })}</span>
          <Button variant="secondary" data-pair-refuse="true" onClick={() => answer(false)}>{t("pairing.ask.refuse")}</Button>
          <Button data-pair-allow-go="true" onClick={() => answer(true)}>{t("pairing.ask.allow")}</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

/** The pasted link's pairing (B2-B6): the words, then the wait, then how it ended. Closing it gives the pairing up. */
export function JoinDialog({ join, actions }: { join: PairJoin; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const dismiss = () => actions.pairing("dismiss_join");
  const over = join.phase === "paired" || join.phase === "waiting" || join.phase === "failed";
  return (
    <Dialog open onOpenChange={(next) => { if (!next) dismiss(); }}>
      <DialogContent showCloseButton data-pair-join={join.phase} data-pair-join-failure={join.failure ?? undefined}>
        <DialogHeader>
          <DialogTitle>{join.name ? t("pairing.join.title", { name: join.name }) : t("pairing.join.titleUnnamed")}</DialogTitle>
        </DialogHeader>
        <DialogBody className="space-y-md">
          {join.phase === "connecting" ? <Status tone="pending">{t("pairing.join.connecting")}</Status> : null}
          {join.phase === "comparing" && join.words ? (
            <>
              <Note>{t("pairing.join.compare")}</Note>
              <PairWords words={join.words} />
            </>
          ) : null}
          {join.phase === "settling" ? (
            <>
              {join.words ? <PairWords words={join.words} /> : null}
              <Status tone="pending">{t("pairing.join.settling")}</Status>
            </>
          ) : null}
          {join.phase === "paired" ? <Status tone="ok">{t("pairing.join.paired")}</Status> : null}
          {join.phase === "waiting" ? <Status tone="pending">{t("pairing.join.waiting")}</Status> : null}
          {join.phase === "failed" ? <Status tone="warn">{t(joinFailureKey(join.failure))}</Status> : null}
        </DialogBody>
        <DialogFooter>
          {join.phase === "comparing" ? (
            <>
              <Button variant="secondary" data-pair-different="true" onClick={() => actions.pairing("compare", { same: false })}>{t("pairing.join.different")}</Button>
              <Button data-pair-same="true" onClick={() => actions.pairing("compare", { same: true })}>{t("pairing.join.same")}</Button>
            </>
          ) : over ? (
            <Button variant="secondary" data-pair-join-close="true" onClick={dismiss}>{t("pairing.join.close")}</Button>
          ) : null}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
