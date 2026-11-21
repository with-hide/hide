// Settings › Devices' rows for the Hides this computer is paired with (PRD
// device-connect-iroh B1, B21, B24-B27, B50, B51): the row that shares this
// computer by a one-time link, and one row per paired machine with who opens
// whom, how the two are connected, and each direction stopped or removed on
// its own. hided owns every value (`pairing.ts`); a press sends one
// `pairing_*` event.

import { MoreHorizontalIcon } from "lucide-react";
import { useEffect, useState } from "react";
import type { Actions } from "../actions";
import { AlertDialog, AlertDialogAction, AlertDialogCancel, AlertDialogContent, AlertDialogDescription, AlertDialogFooter, AlertDialogHeader, AlertDialogTitle } from "../components/ui/alert-dialog";
import { Button } from "../components/ui/button";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuSeparator, DropdownMenuTrigger } from "../components/ui/dropdown-menu";
import { RadioGroup, RadioGroupItem } from "../components/ui/radio-group";
import { Note, Row, Status, Value } from "../components/settings-rows";
import { Hint } from "../components/ui/tooltip";
import { useInterfaceTranslation } from "../i18n/client";
import { directionKey, guestPaneCount, mayRequest, ownWindow, pairedMachines, peerState, refusalKey, requestLine, timeLeft, type Direction, type PairRequest, type Peer } from "../pairing";
import { useShellStore } from "../store";
import { DeviceNeedsLine, DeviceNotCarriedLine } from "./DeviceSwitch";
import { askForFirewallOnce, useFirewall } from "./firewall";

/** The time now, read again each second while `ticking`. */
export function useNow(ticking: boolean): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!ticking) return;
    setNow(Date.now());
    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, [ticking]);
  return now;
}

/** The four words two people compare, each with its place, so "the same order" can be checked. */
export function PairWords({ words }: { words: readonly string[] }) {
  const { t } = useInterfaceTranslation();
  return (
    <ol aria-label={t("pairing.words.aria")} className="flex flex-wrap gap-sm" data-pair-words={words.join(" ")}>
      {words.map((word, index) => (
        <li key={`${index}-${word}`} className="flex items-baseline gap-xs rounded-md border border-border bg-card px-md py-sm">
          <span className="text-title font-semibold text-foreground">{word}</span>
          <span className="text-caption text-muted-foreground tabular-nums">{index + 1}</span>
        </li>
      ))}
    </ol>
  );
}

/** The row that shares this computer: Make a link, then the link with how long it has left (B1). */
export function ShareRow({ actions }: { actions: Actions }) {
  const { t, i18n } = useInterfaceTranslation();
  const offer = useShellStore((s) => s.pairing?.offer ?? null);
  const refused = useShellStore((s) => (s.pairingResult?.verb === "share" && !s.pairingResult.ok ? refusalKey(s.pairingResult.reason) : null));
  const now = useNow(offer !== null);
  const [copied, setCopied] = useState(false);
  useEffect(() => setCopied(false), [offer?.link]);
  return (
    <Row
      data-pair-share={offer ? "shared" : "idle"}
      label={
        <>
          <div>{t("pairing.share.title")}</div>
          <Note>{t("pairing.share.caption")}</Note>
        </>
      }
      detail={
        offer ? (
          <div className="space-y-xs">
            <div aria-label={t("pairing.share.linkAria")} data-pair-link="true">
              <Value>{offer.link}</Value>
            </div>
            <div className="flex flex-wrap items-center gap-sm">
              <Button
                variant="secondary"
                size="sm"
                data-pair-copy="true"
                onClick={() => void actions.copyText(offer.link, "pairing link").then((done) => setCopied(done))}
              >
                {copied ? t("pairing.share.copied") : t("pairing.share.copy")}
              </Button>
              <Note>{t("pairing.share.left", { time: timeLeft(offer.expires_unix_ms, now) })}</Note>
              <Button variant="text" size="inline" className="ml-auto text-caption" data-pair-share-end="true" onClick={() => actions.pairing("end_share")}>
                {t("pairing.share.end")}
              </Button>
            </div>
          </div>
        ) : refused ? (
          <Note tone="warn" data-pair-share-refused="true">{t(refused)}</Note>
        ) : null
      }
    >
      {offer ? null : (
        <Button variant="secondary" size="sm" data-pair-share-make="true" onClick={() => {
          actions.pairing("share", { language: i18n.language });
          void askForFirewallOnce();
        }}>
          {t("pairing.share.make")}
        </Button>
      )}
    </Row>
  );
}

/**
 * One paired machine: its name, its one state line, who opens whom, and,
 * on this computer's own window, what can be done about it. An ask names
 * the machine by its connection id: a device id is what a machine says of
 * itself, and two rows can say the same one.
 */
export function PeerRow({ peer, request, actions, own, onRemove }: { peer: Peer; request: PairRequest | undefined; actions: Actions; own: boolean; onRemove: (peer: Peer, direction: Direction) => void }) {
  const { t } = useInterfaceTranslation();
  const state = peerState(peer);
  const asked = requestLine(request);
  const device = peer.device_id;
  const named = peer.key;
  const act = (verb: "pause" | "resume" | "retry", direction?: Direction) => actions.pairing(verb, direction ? { device: named, direction } : { device: named });
  const ask = () => {
    if (request) actions.pairing("dismiss_request", { device: named });
    actions.pairing("request", { device: named });
  };
  return (
    <Row
      data-peer-row={device}
      data-peer-state={state.key}
      label={
        <>
          <div className="truncate">{peer.name}</div>
          <div className="flex flex-wrap items-baseline gap-x-sm">
            <Status tone={state.tone}>{state.reason ? `${t(state.key)} · ${t(state.reason)}` : t(state.key)}</Status>
            <span className="text-body text-muted-foreground" data-peer-direction="true">{t(directionKey(peer))}</span>
          </div>
          {asked ? <Note tone={request?.state === "waiting" ? "pending" : "warn"} data-peer-request={request?.state}>{t(asked)}</Note> : null}
          <DeviceNotCarriedLine deviceId={device} />
          <DeviceNeedsLine deviceId={device} actions={actions} />
        </>
      }
    >
      {own ? (
      <>
      {state.action === "retry" ? (
        <Button variant="secondary" size="sm" data-peer-retry={device} onClick={() => act("retry")}>{t("pairing.action.retry")}</Button>
      ) : null}
      {state.action === "resume_outgoing" || state.action === "resume_incoming" ? (
        <Button variant="secondary" size="sm" data-peer-resume={device} onClick={() => act("resume", state.action === "resume_outgoing" ? "outgoing" : "incoming")}>
          {t("pairing.action.resume")}
        </Button>
      ) : null}
      {state.action === "remove_outgoing" ? (
        <Button variant="secondary" size="sm" data-peer-forget={device} onClick={() => actions.pairing("remove", { device: named, direction: "outgoing" })}>
          {t("pairing.action.forget")}
        </Button>
      ) : null}
      {mayRequest(peer) && request?.state !== "waiting" ? (
        <Button variant="secondary" size="sm" data-peer-request-go={device} onClick={ask}>
          {request ? t("pairing.action.requestAgain") : t("pairing.action.request", { name: peer.name })}
        </Button>
      ) : null}
      <DropdownMenu>
        <Hint label={t("pairing.menu.aria", { name: peer.name })}>
          <DropdownMenuTrigger asChild>
            <Button variant="ghost" size="icon-sm" aria-label={t("pairing.menu.aria", { name: peer.name })} data-peer-menu={device}>
              <MoreHorizontalIcon />
            </Button>
          </DropdownMenuTrigger>
        </Hint>
        <DropdownMenuContent align="end">
          {peer.opens != null && peer.opens !== "unpaired" && peer.opens !== "removing" ? (
            <>
              {peer.opens === "paused" ? (
                <DropdownMenuItem onSelect={() => act("resume", "outgoing")} data-peer-resume-outgoing={device}>{t("pairing.menu.resumeOutgoing")}</DropdownMenuItem>
              ) : (
                <DropdownMenuItem onSelect={() => act("pause", "outgoing")} data-peer-pause-outgoing={device}>{t("pairing.menu.pauseOutgoing")}</DropdownMenuItem>
              )}
              <DropdownMenuItem variant="destructive" onSelect={() => onRemove(peer, "outgoing")} data-peer-remove-outgoing={device}>{t("pairing.menu.removeOutgoing")}</DropdownMenuItem>
            </>
          ) : null}
          {peer.opens != null && peer.opened_by != null ? <DropdownMenuSeparator /> : null}
          {peer.opened_by != null ? (
            <>
              {peer.opened_by === "paused" ? (
                <DropdownMenuItem onSelect={() => act("resume", "incoming")} data-peer-resume-incoming={device}>{t("pairing.menu.resumeIncoming")}</DropdownMenuItem>
              ) : (
                <DropdownMenuItem onSelect={() => act("pause", "incoming")} data-peer-pause-incoming={device}>{t("pairing.menu.pauseIncoming")}</DropdownMenuItem>
              )}
              <DropdownMenuItem variant="destructive" onSelect={() => onRemove(peer, "incoming")} data-peer-remove-incoming={device}>{t("pairing.menu.removeIncoming")}</DropdownMenuItem>
            </>
          ) : null}
          {peer.opens === "unpaired" || peer.opens === "removing" ? (
            <DropdownMenuItem variant="destructive" onSelect={() => actions.pairing("remove", { device: named, direction: "outgoing" })} data-peer-forget-menu={device}>{t("pairing.action.forget")}</DropdownMenuItem>
          ) : null}
        </DropdownMenuContent>
      </DropdownMenu>
      </>
      ) : null}
    </Row>
  );
}

/**
 * Windows lets same-network machines reach this PC directly only through
 * its firewall rules; while they are missing the line says the connection
 * goes through the relay, and [Allow] asks Windows again (B32).
 */
export function FirewallRow({ asking, onAllow }: { asking: boolean; onAllow: () => void }) {
  const { t } = useInterfaceTranslation();
  return (
    <Row
      data-pair-firewall="missing"
      label={
        <>
          <Status tone="warn">{t("pairing.firewall.blocked")}</Status>
          <Note>{t("pairing.firewall.caption")}</Note>
        </>
      }
    >
      <Button variant="secondary" size="sm" data-pair-firewall-allow="true" disabled={asking} onClick={onAllow}>
        {t("pairing.firewall.allow")}
      </Button>
    </Row>
  );
}

/**
 * What becomes of the work a removed machine ran on this computer (B26,
 * B60). It stays by default and shows on this computer's screen only; while
 * that machine has terminals here (`navigator.guest_panes`) the operator
 * may end them with the removal instead, which needs no connection to that
 * machine. With none there is nothing to end, and the one answer is said.
 */
function RemovedWork({ panes, end, onEnd }: { panes: number; end: boolean; onEnd: (end: boolean) => void }) {
  const { t } = useInterfaceTranslation();
  if (panes === 0) {
    return (
      <div className="space-y-xs" data-peer-remove-work="keep">
        <div className="text-body text-subtle-foreground">{t("pairing.remove.workTitle")}</div>
        <div className="text-body text-foreground">{t("pairing.remove.workKeep")}</div>
        <Note>{t("pairing.remove.workKeepNote")}</Note>
      </div>
    );
  }
  const choices = [
    { value: "keep", label: t("pairing.remove.workKeep"), note: t("pairing.remove.workKeepNote") },
    { value: "end", label: t("pairing.remove.workEnd"), note: t("pairing.remove.workEndNote", { count: panes }) },
  ];
  return (
    <div className="space-y-xs" data-peer-remove-work={end ? "end" : "keep"}>
      <div className="text-body text-subtle-foreground">{t("pairing.remove.workTitle")}</div>
      <RadioGroup value={end ? "end" : "keep"} onValueChange={(value) => onEnd(value === "end")} aria-label={t("pairing.remove.workTitle")} className="gap-xs">
        {choices.map((choice) => (
          <label key={choice.value} className="flex cursor-pointer items-start gap-sm" data-peer-remove-choice={choice.value}>
            <RadioGroupItem value={choice.value} aria-label={choice.label} className="mt-xxs" />
            <span className="flex min-w-0 flex-1 flex-col">
              <span className="text-body text-foreground">{choice.label}</span>
              <Note>{choice.note}</Note>
            </span>
          </label>
        ))}
      </RadioGroup>
    </div>
  );
}

/**
 * Every paired machine, and on this computer's own window the share row,
 * each machine's actions and the confirm a removal asks (B26, B52).
 */
export function PairedRows({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const own = useShellStore((s) => ownWindow(s.pairing));
  const peers = useShellStore((s) => pairedMachines(s.rest));
  const requests = useShellStore((s) => s.pairing?.requests);
  const sharing = useShellStore((s) => s.pairing?.offer != null);
  const [removing, setRemoving] = useState<{ peer: Peer; direction: Direction; end: boolean } | null>(null);
  // The terminals the machine being removed runs on this computer.
  const removingPanes = useShellStore((s) => (removing ? guestPaneCount(s.rest, removing.peer.key) : 0));
  const opened = sharing || peers.length > 0;
  const firewall = useFirewall(own && opened);
  return (
    <>
      {own ? <ShareRow actions={actions} /> : null}
      {own && opened && firewall.state === "missing" ? <FirewallRow asking={firewall.asking} onAllow={firewall.allow} /> : null}
      {peers.map((peer) => (
        <PeerRow
          key={peer.key}
          peer={peer}
          request={requests?.find((request) => request.key === peer.key)}
          actions={actions}
          own={own}
          onRemove={(target, direction) => setRemoving({ peer: target, direction, end: false })}
        />
      ))}
      {removing ? (
        <AlertDialog open onOpenChange={(next) => { if (!next) setRemoving(null); }}>
          <AlertDialogContent data-peer-remove-confirm={`${removing.peer.device_id}:${removing.direction}`}>
            <AlertDialogHeader>
              <AlertDialogTitle>{t(removing.direction === "incoming" ? "pairing.remove.incomingTitle" : "pairing.remove.outgoingTitle", { name: removing.peer.name })}</AlertDialogTitle>
              <AlertDialogDescription className="break-keep">{t(removing.direction === "incoming" ? "pairing.remove.incomingDescription" : "pairing.remove.outgoingDescription")}</AlertDialogDescription>
            </AlertDialogHeader>
            {removing.direction === "incoming" ? (
              <RemovedWork panes={removingPanes} end={removing.end} onEnd={(end) => setRemoving({ ...removing, end })} />
            ) : null}
            <AlertDialogFooter>
              <AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel>
              <AlertDialogAction
                data-peer-remove-go="true"
                onClick={() =>
                  actions.pairing("remove", {
                    device: removing.peer.key,
                    direction: removing.direction,
                    // Sent only when chosen while there is work to end.
                    ...(removing.direction === "incoming" && removing.end && removingPanes > 0 ? { end_work: true } : {}),
                  })
                }
              >
                {t("pairing.remove.go")}
              </AlertDialogAction>
            </AlertDialogFooter>
          </AlertDialogContent>
        </AlertDialog>
      ) : null}
    </>
  );
}
