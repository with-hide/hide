// Settings › Devices' rows for the Hides this computer is paired with (PRD
// device-connect-iroh B1, B21, B24-B27, B50, B51): the window that shares
// this computer by a one-time link, and one row per paired machine with its
// state, and in its menu its name, how the two are connected, who opens whom
// and each direction stopped or removed on its own. hided owns every value
// (`pairing.ts`); a press sends one `pairing_*` event.

import { CheckIcon, MoreHorizontalIcon } from "lucide-react";
import { useEffect, useState, type ReactNode } from "react";
import type { Actions } from "../actions";
import { AlertDialog, AlertDialogAction, AlertDialogCancel, AlertDialogContent, AlertDialogDescription, AlertDialogFooter, AlertDialogHeader, AlertDialogTitle } from "../components/ui/alert-dialog";
import { Button } from "../components/ui/button";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuSeparator, DropdownMenuTrigger } from "../components/ui/dropdown-menu";
import { RadioGroup, RadioGroupItem } from "../components/ui/radio-group";
import { Note, Row, Status } from "../components/settings-rows";
import { Dialog, DialogBody, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "../components/ui/dialog";
import { cn } from "../lib/utils";
import { useUiStore } from "../ui";
import { Hint } from "../components/ui/tooltip";
import { useInterfaceTranslation } from "../i18n/client";
import { guestPaneCount, mayRequest, ownWindow, pairedMachines, peerState, refusalKey, relayTip, requestLine, timeLeft, type Direction, type PairRequest, type Peer } from "../pairing";
import { useShellStore } from "../store";
import { DeviceNeedsLine, DeviceNotCarriedLine } from "./DeviceSwitch";
import { askForFirewallOnce, useFirewall } from "./firewall";
import { machineSystem } from "./agentRows";
import { PeerDetails } from "./DeviceDetails";
import { RenameDevice } from "./RenameDevice";

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

/**
 * The window that shares this computer (B1, B2, B50; operator review
 * 2026-11-22): the link is made when it opens, and three steps read top to
 * bottom, the one under way marked: copy the link with how long it has left,
 * paste it on the other computer, then compare the four words and allow it
 * here. The link stays until it ends, so closing the window keeps it; a
 * machine that pastes it while the window is closed asks in the allow window.
 */
export function ShareDialog({ actions, onClose }: { actions: Actions; onClose: () => void }) {
  const { t, i18n } = useInterfaceTranslation();
  const offer = useShellStore((s) => s.pairing?.offer ?? null);
  // A machine that pasted this computer's link waits for the words here.
  const ask = useShellStore((s) => s.pairing?.asks.find((row) => row.kind === "link") ?? null);
  const refused = useShellStore((s) => (s.pairingResult?.verb === "share" && !s.pairingResult.ok ? refusalKey(s.pairingResult.reason) : null));
  const now = useNow(offer !== null || ask !== null);
  const [copied, setCopied] = useState(false);
  // Whether this window asked for the link now showing, so one that ends says so rather than waiting.
  const [seen, setSeen] = useState(false);
  const make = () => {
    setSeen(false);
    setCopied(false);
    actions.pairing("share", { language: i18n.language });
    void askForFirewallOnce();
  };
  // Opening the window makes the link, unless one is already shared.
  useEffect(() => {
    if (useShellStore.getState().pairing?.offer == null) make();
  }, []);
  useEffect(() => {
    if (offer) setSeen(true);
  }, [offer]);
  useEffect(() => setCopied(false), [offer?.link]);
  // This window shows the allow step, so the allow window stays out of its way.
  useEffect(() => {
    useUiStore.getState().setSharing(true);
    return () => useUiStore.getState().setSharing(false);
  }, []);
  const answer = (allow: boolean) => {
    if (!ask) return;
    actions.pairing("answer", { ask: ask.id, allow });
    if (allow) void askForFirewallOnce();
    onClose();
  };
  const phase = ask ? "words" : offer ? (copied ? "waiting" : "copy") : refused && !seen ? "refused" : seen ? "ended" : "making";
  const step = (index: number, active: number) => (index < active ? "done" : index === active ? "on" : "later");
  const active = phase === "words" ? 3 : phase === "waiting" ? 2 : 1;
  return (
    <Dialog open onOpenChange={(next) => { if (!next) onClose(); }}>
      <DialogContent showCloseButton data-pair-share={phase}>
        <DialogHeader>
          <DialogTitle>{t("pairing.share.title")}</DialogTitle>
          <DialogDescription>{t("pairing.share.caption")}</DialogDescription>
        </DialogHeader>
        <DialogBody>
          <ol className="flex flex-col gap-md">
            <ShareStep index={1} state={step(1, active)} title={t(active > 1 ? "pairing.share.step.copied" : "pairing.share.step.copy")}>
              {phase === "copy" && offer ? (
                <div className="space-y-xs">
                  <div className="flex items-center gap-sm rounded-md bg-secondary py-xs pr-xs pl-md">
                    <code aria-label={t("pairing.share.linkAria")} className="min-w-0 flex-1 truncate font-mono text-caption text-subtle-foreground" data-pair-link="true" title={offer.link}>{offer.link}</code>
                    <Button size="sm" data-pair-copy="true" onClick={() => void actions.copyText(offer.link, "pairing link").then((done) => setCopied(done))}>
                      {t("pairing.share.copy")}
                    </Button>
                  </div>
                  <Note data-pair-left="true">{t("pairing.share.left", { time: timeLeft(offer.expires_unix_ms, now) })}</Note>
                </div>
              ) : phase === "waiting" && offer ? (
                <Note>
                  {t("pairing.share.left", { time: timeLeft(offer.expires_unix_ms, now) })}
                  {" · "}
                  <Button variant="text" size="inline" data-pair-copy="true" onClick={() => void actions.copyText(offer.link, "pairing link").then((done) => setCopied(done))}>{t("pairing.share.copyAgain")}</Button>
                </Note>
              ) : phase === "making" ? (
                <Status tone="pending">{t("pairing.share.making")}</Status>
              ) : phase === "refused" && refused ? (
                <Note tone="warn" data-pair-share-refused="true">{t(refused)}</Note>
              ) : phase === "ended" ? (
                <div className="flex items-center gap-sm">
                  <Note>{t("pairing.share.ended")}</Note>
                  <Button variant="text" size="inline" className="text-caption" data-pair-share-again="true" onClick={make}>{t("pairing.share.again")}</Button>
                </div>
              ) : null}
            </ShareStep>
            <ShareStep index={2} state={step(2, active)} title={ask ? t("pairing.share.step.pasted", { name: ask.name }) : t("pairing.share.step.paste")}>
              {ask ? null : <Note>{t("pairing.share.step.pasteWhere")}</Note>}
              {phase === "waiting" ? <Status tone="pending" data-pair-share-waiting="true">{t("pairing.share.step.waiting")}</Status> : null}
            </ShareStep>
            <ShareStep index={3} state={step(3, active)} title={ask ? t("pairing.share.step.wordsAsk", { name: ask.name }) : t("pairing.share.step.words")}>
              {ask ? (
                <div className="space-y-sm">
                  {ask.words ? <PairWords words={ask.words} /> : null}
                  <Note>{t("pairing.ask.compare")}</Note>
                  <p className="break-keep text-body text-foreground" data-pair-grants="true">{t("pairing.ask.grants", { name: ask.name })}</p>
                </div>
              ) : null}
            </ShareStep>
          </ol>
        </DialogBody>
        <DialogFooter>
          {ask ? (
            <>
              <span className="mr-auto text-caption text-muted-foreground tabular-nums">{t("pairing.ask.left", { time: timeLeft(ask.expires_unix_ms, now) })}</span>
              <Button variant="secondary" data-pair-refuse="true" onClick={() => answer(false)}>{t("pairing.ask.refuse")}</Button>
              <Button data-pair-allow-go="true" onClick={() => answer(true)}>{t("pairing.ask.allow")}</Button>
            </>
          ) : offer ? (
            <Button variant="secondary" data-pair-share-end="true" onClick={() => { actions.pairing("end_share"); onClose(); }}>{t("pairing.share.end")}</Button>
          ) : null}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

/** One step of the share window: its number (a check once done), its title, and what it holds while under way. */
function ShareStep({ index, state, title, children }: { index: number; state: "done" | "on" | "later"; title: string; children?: ReactNode }) {
  return (
    <li className={cn("flex items-start gap-sm", state === "later" && "opacity-(--opacity-dimmed)")} data-pair-step={index} data-pair-step-state={state}>
      <span
        aria-hidden="true"
        className={cn(
          "flex size-(--size-icon-lg) shrink-0 items-center justify-center rounded-full text-micro font-semibold",
          state === "done" ? "bg-success text-status-foreground" : state === "on" ? "bg-primary text-primary-foreground" : "bg-secondary text-subtle-foreground",
        )}
      >
        {state === "done" ? <CheckIcon className="size-(--size-icon-sm)" /> : index}
      </span>
      <div className="flex min-w-0 flex-1 flex-col gap-xs">
        <div className="text-body font-medium text-foreground">{title}</div>
        {children}
      </div>
    </li>
  );
}

/**
 * One paired machine (operator review 2026-11-23): its name and system, one
 * state (`● 연결됨` while connected, the path being Connection details'),
 * the relay tip on a row whose connection goes through the relay, and, on
 * this computer's own window, the one action its state offers and ⋯ with
 * Rename, Connection details and each direction's pause and removal. An ask
 * names the machine by its connection id: a device id is what a machine says
 * of itself, and two rows can say the same one.
 */
export function PeerRow({ peer, request, actions, own, onRemove, onDetails, onRename }: { peer: Peer; request: PairRequest | undefined; actions: Actions; own: boolean; onRemove: (peer: Peer, direction: Direction) => void; onDetails: (peer: Peer) => void; onRename: (peer: Peer) => void }) {
  const { t } = useInterfaceTranslation();
  const state = peerState(peer);
  const asked = requestLine(request);
  const device = peer.device_id;
  const named = peer.key;
  // The machine's system, as the device the core made of it reports it.
  const system = useShellStore((s) => machineSystem(s.rest?.navigator?.devices?.find((row) => row.peer_key === peer.key)) ?? null);
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
        <span className="flex min-w-0 flex-col">
          <span className="break-words font-semibold">{peer.name}</span>
          {system ? <span className="text-caption text-muted-foreground" data-peer-system={device}>{system}</span> : null}
          {state.reason ? <Note tone="warn" data-peer-reason={device}>{t(state.reason)}</Note> : null}
          {relayTip(peer) ? <Note data-peer-relay-tip={device}>{t("devices.relayTip")}</Note> : null}
          {asked ? <Note tone={request?.state === "waiting" ? "pending" : "warn"} data-peer-request={request?.state}>{t(asked)}</Note> : null}
          <DeviceNotCarriedLine deviceId={device} />
          <DeviceNeedsLine deviceId={device} actions={actions} />
        </span>
      }
    >
      <Status tone={state.tone === "ok" ? "live" : state.tone} data-peer-status={device}>{t(state.key)}</Status>
      {own ? (
      <>
      {state.action === "retry" ? (
        <Button variant="secondary" size="sm" data-peer-retry={device} onClick={() => act("retry")}>{t("pairing.action.retry")}</Button>
      ) : null}
      {state.action === "update_there" ? (
        <Button variant="secondary" size="sm" data-peer-update={device} onClick={() => actions.pairing("update", { device: named })}>{t("pairing.action.updateThere", { name: peer.name })}</Button>
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
          <DropdownMenuItem onSelect={() => onRename(peer)} data-peer-rename={device}>{t("devices.renameMenu")}</DropdownMenuItem>
          <DropdownMenuItem onSelect={() => onDetails(peer)} data-peer-details={device}>{t("devices.detailsMenu")}</DropdownMenuItem>
          <DropdownMenuSeparator />
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
 * The paired machines' rows in Settings › Devices' one group of other
 * devices (B24, B52; operator review 2026-11-23): the firewall row while
 * Windows blocks direct connections, every paired machine with its actions,
 * and the windows its menu opens (details, rename, the confirm a removal
 * asks). The share window opens from this computer's own row.
 */
export function PeerRows({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const own = useShellStore((s) => ownWindow(s.pairing));
  const peers = useShellStore((s) => pairedMachines(s.rest));
  const requests = useShellStore((s) => s.pairing?.requests);
  const sharing = useShellStore((s) => s.pairing?.offer != null);
  const [removing, setRemoving] = useState<{ peer: Peer; direction: Direction; end: boolean } | null>(null);
  const [details, setDetails] = useState<Peer | null>(null);
  const [renaming, setRenaming] = useState<Peer | null>(null);
  // The terminals the machine being removed runs on this computer.
  const removingPanes = useShellStore((s) => (removing ? guestPaneCount(s.rest, removing.peer.key) : 0));
  const opened = sharing || peers.length > 0;
  const firewall = useFirewall(own && opened);
  return (
    <>
      {own && opened && firewall.state === "missing" ? <FirewallRow asking={firewall.asking} onAllow={firewall.allow} /> : null}
      {peers.map((peer) => (
        <PeerRow
          key={peer.key}
          peer={peer}
          request={requests?.find((request) => request.key === peer.key)}
          actions={actions}
          own={own}
          onRemove={(target, direction) => setRemoving({ peer: target, direction, end: false })}
          onDetails={setDetails}
          onRename={setRenaming}
        />
      ))}
      {details ? <PeerDetails peer={peers.find((peer) => peer.key === details.key) ?? details} onClose={() => setDetails(null)} /> : null}
      {renaming ? (
        <RenameDevice
          id={renaming.device_id}
          current={renaming.name}
          onSave={(name) => actions.pairing("rename", { device: renaming.key, name })}
          onClose={() => setRenaming(null)}
        />
      ) : null}
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
