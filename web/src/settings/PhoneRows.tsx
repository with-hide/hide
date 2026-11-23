// Settings › Devices' phone group (PRD mobile-companion B1-B7, B10, B14,
// B15, B29, B39; settings-cleanup B57-B59; operator review 2026-11-23): a
// phone is one more device. The group's head carries 폰에서 Hide 열기, each
// paired phone is one row as a computer is (its name, one state, ⋯ with
// 이름 바꾸기, 연결 정보 and 연결 해제), 폰 연결 방식 folds the relay's state
// and the advanced Tailscale choice into one row, and a way that is down or
// refused says why in one line over the rows, only then. A phone pairs only
// from Add device › 폰, which opens the QR window here. A screen another
// machine shows of this one lists the phones with 연결 해제 alone (PRD
// device-connect-iroh D-53, D-70). Every value is the daemon's `mobile`
// frame; every control sends one `mobile_*` event.

import { AlertTriangleIcon, ChevronRightIcon, CircleAlertIcon, ExternalLinkIcon, MoreHorizontalIcon } from "lucide-react";
import qrcode from "qrcode-generator";
import { useEffect, useMemo, useRef, useState } from "react";
import type { Actions } from "../actions";
import { AlertDialog, AlertDialogAction, AlertDialogCancel, AlertDialogContent, AlertDialogDescription, AlertDialogFooter, AlertDialogHeader, AlertDialogTitle } from "../components/ui/alert-dialog";
import { Button } from "../components/ui/button";
import { Dialog, DialogBody, DialogContent, DialogHeader, DialogTitle } from "../components/ui/dialog";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuSeparator, DropdownMenuTrigger } from "../components/ui/dropdown-menu";
import { RowButton } from "../components/ui/row-button";
import { Switch } from "../components/ui/switch";
import { Disclosure, Group, Note, Row, Status, Value } from "../components/settings-rows";
import { useInterfaceTranslation } from "../i18n/client";
import { requireInterfaceLanguage } from "../i18n/locale";
import { APP_STORE_URL, checklistView, codeCountdown, exposureLine, lastSeen, phoneLine, phoneProblem, type FailedStep, type MobileState, type PhoneRow } from "../mobileSettings";
import { coreMachineName } from "../screenMachine";
import { useShellStore } from "../store";
import { RenameDevice } from "./RenameDevice";

/**
 * The phone group. `own`: this machine's own window, the only one that
 * turns phones on, chooses their way and pairs one; `pairing`: the QR
 * window is open, which the Devices tab opens from Add device › 폰.
 */
export function PhoneGroup({ actions, own, pairing, onPairingClose }: { actions: Actions; own: boolean; pairing: boolean; onPairingClose: () => void }) {
  const { t, i18n } = useInterfaceTranslation();
  const language = requireInterfaceLanguage(i18n.language);
  const mobile = useShellStore((s) => s.mobile);
  const machine = useShellStore((s) => coreMachineName(s.rest?.navigator?.devices, t));
  const [method, setMethod] = useState(false);
  const [details, setDetails] = useState<string | null>(null);
  const [renaming, setRenaming] = useState<string | null>(null);
  const [revoking, setRevoking] = useState<string | null>(null);
  if (!mobile) {
    return (
      <Group title={t("mobileSetup.group")} data-device-section="phones">
        <Row label={<Note tone="pending">{t("mobileSetup.connecting")}</Note>} />
      </Group>
    );
  }
  const problem = phoneProblem(mobile, machine, t);
  const phone = (id: string | null) => (id === null ? null : (mobile.phones.find((row) => row.id === id) ?? null));
  const renamed = phone(renaming);
  const shown = phone(details);
  const revoked = phone(revoking);
  return (
    <>
      <Group
        title={t("mobileSetup.group")}
        data-device-section="phones"
        data-mobile-tab="true"
        data-mobile-exposure={mobile.exposure}
        {...(own ? {} : { "data-mobile-remote": "true" })}
        data-mobile-phones={String(mobile.phones.length)}
        action={own ? (
          <label className="flex items-center gap-sm text-body text-subtle-foreground">
            {t("mobileSetup.enable")}
            <Switch checked={mobile.enabled} onCheckedChange={(next) => actions.setMobileEnabled(next)} aria-label={t("mobileSetup.enable")} data-mobile-switch="true" />
          </label>
        ) : undefined}
      >
        {/* Turning phones on and pairing one happen on the machine itself (PRD device-connect-iroh D-53, D-70). */}
        {own ? null : <Row label={<Note data-mobile-remote-line="true">{t("mobileSetup.remoteScreen", { machine })}</Note>} />}
        {problem ? (
          <Row data-mobile-problem={mobile.method} label={<Note tone="warn">{problem.text}</Note>}>
            {problem.action === "update" ? (
              <Button variant="secondary" size="sm" onClick={() => actions.openSettings("about")}>{t("mobileSetup.relay.update")}</Button>
            ) : problem.action === "retry" ? (
              <Button variant="secondary" size="sm" onClick={() => actions.retryMobile()} data-mobile-retry="true">{t("mobileSetup.relay.retry")}</Button>
            ) : null}
          </Row>
        ) : null}
        {mobile.phones.map((row) => (
          <PhoneRowView key={row.id} phone={row} now={mobile.now_ms} own={own} language={language} onRename={setRenaming} onDetails={setDetails} onRevoke={setRevoking} />
        ))}
        {mobile.phones.length === 0 ? <Row data-mobile-empty="true" label={<Note>{t(own ? "mobileSetup.empty" : "mobileSetup.emptyRemote")}</Note>} /> : null}
        {own ? (
          <RowButton className="gap-md px-md py-sm text-subhead" onClick={() => setMethod(true)} data-mobile-method-open={mobile.method}>
            <span className="min-w-0 flex-1 text-foreground">{t("mobileSetup.method")}</span>
            <span className="min-w-0 truncate text-body text-muted-foreground">{t(mobile.method === "relay" ? "mobileSetup.methodRelay" : "mobileSetup.methodTailscale")}</span>
            <ChevronRightIcon aria-hidden="true" className="size-(--size-icon) shrink-0 text-muted-foreground" />
          </RowButton>
        ) : null}
      </Group>
      {method && own ? <MethodDialog state={mobile} actions={actions} onClose={() => setMethod(false)} /> : null}
      {pairing && own ? <PairDialog state={mobile} actions={actions} onClose={onPairingClose} /> : null}
      {shown ? <PhoneDetails phone={shown} state={mobile} language={language} onClose={() => setDetails(null)} /> : null}
      {renamed && own ? (
        <RenameDevice
          id={renamed.id}
          current={renamed.name}
          chosen={renamed.own_name != null}
          description={t("mobileSetup.renameDescription")}
          onSave={(name) => actions.renamePhone(renamed.id, name)}
          onClose={() => setRenaming(null)}
        />
      ) : null}
      {revoked ? (
        <AlertDialog open onOpenChange={(next) => { if (!next) setRevoking(null); }}>
          <AlertDialogContent data-mobile-revoke-confirm={revoked.id}>
            <AlertDialogHeader>
              <AlertDialogTitle>{t("mobileSetup.revokeTitle", { name: revoked.name })}</AlertDialogTitle>
              <AlertDialogDescription>{t("mobileSetup.revokeDescription")}</AlertDialogDescription>
            </AlertDialogHeader>
            <AlertDialogFooter>
              <AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel>
              <AlertDialogAction variant="destructive" onClick={() => actions.revokePhone(revoked.id)} data-mobile-revoke={revoked.id}>
                {t("mobileSetup.revoke")}
              </AlertDialogAction>
            </AlertDialogFooter>
          </AlertDialogContent>
        </AlertDialog>
      ) : null}
    </>
  );
}

/**
 * One phone, drawn as a computer's row is: its name, what the operator
 * would read about it (when it was last seen while away, its notifications,
 * the days until the seven-day revoke), one state, and ⋯. The pairing
 * record carries no system of its own, so none is shown (design 10).
 */
function PhoneRowView({ phone, now, own, language, onRename, onDetails, onRevoke }: { phone: PhoneRow; now: number; own: boolean; language: ReturnType<typeof requireInterfaceLanguage>; onRename: (id: string) => void; onDetails: (id: string) => void; onRevoke: (id: string) => void }) {
  const { t } = useInterfaceTranslation();
  // The daemon's clock decides "just now" and the revoke date; this only adds the time since its frame.
  const line = phoneLine(phone, now + useSinceFrame(phone, 30_000, true), t, language);
  return (
    <Row
      data-mobile-phone-row={phone.id}
      label={
        <span className="flex min-w-0 flex-col" data-mobile-phone={phone.id}>
          <span className="break-words font-semibold">{phone.name}</span>
          {line ? <span className="break-words text-caption text-muted-foreground" data-mobile-phone-line={phone.id}>{line}</span> : null}
        </span>
      }
    >
      <Status tone={phone.connected ? "live" : "muted"} data-mobile-phone-state={`${phone.id}:${phone.connected ? "connected" : "away"}`}>
        {t(phone.connected ? "devices.line.ready" : "devices.state.unavailable")}
      </Status>
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button variant="ghost" size="icon-sm" aria-label={t("mobileSetup.moreAria", { name: phone.name })} data-mobile-phone-menu={phone.id}>
            <MoreHorizontalIcon aria-hidden="true" />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end" data-mobile-phone-menu-content={phone.id}>
          {/* A screen another machine shows of this one removes a phone and nothing else (D-53, D-70). */}
          {own ? (
            <>
              <DropdownMenuItem onSelect={() => onRename(phone.id)} data-mobile-rename={phone.id}>{t("devices.renameMenu")}</DropdownMenuItem>
              <DropdownMenuItem onSelect={() => onDetails(phone.id)} data-mobile-details={phone.id}>{t("devices.detailsMenu")}</DropdownMenuItem>
              <DropdownMenuSeparator />
            </>
          ) : null}
          <DropdownMenuItem variant="destructive" onSelect={() => onRevoke(phone.id)} data-mobile-revoke-menu={phone.id}>{t("mobileSetup.revokeMenu")}</DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
    </Row>
  );
}

/** ⋯ › 연결 정보: what the row leaves out, each from the pairing record or the frame. */
function PhoneDetails({ phone, state, language, onClose }: { phone: PhoneRow; state: MobileState; language: ReturnType<typeof requireInterfaceLanguage>; onClose: () => void }) {
  const { t } = useInterfaceTranslation();
  const notifications = phone.notifications === "on" ? t("mobileSetup.notificationsOn") : phone.notifications === "off" ? t("mobileSetup.notificationsOff") : t("mobileSetup.details.unasked");
  return (
    <Dialog open onOpenChange={(next) => { if (!next) onClose(); }}>
      <DialogContent data-mobile-details-dialog={phone.id}>
        <DialogHeader>
          <DialogTitle>{t("mobileSetup.details.title", { name: phone.name })}</DialogTitle>
        </DialogHeader>
        <DialogBody className="space-y-md">
          <div className="divide-y divide-border rounded-md border border-border bg-card">
            {/* Shown only once the operator renamed it, beside the name it paired with. */}
            {phone.own_name ? <Row label={t("mobileSetup.details.ownName")}><Value mono={false} data-mobile-own-name={phone.id}>{phone.own_name}</Value></Row> : null}
            <Row label={t("mobileSetup.details.method")}><Value mono={false}>{t(state.method === "relay" ? "mobileSetup.methodRelay" : "mobileSetup.methodTailscale")}</Value></Row>
            <Row label={t("mobileSetup.details.seen")}><Value mono={false}>{lastSeen(phone.last_seen_ms, state.now_ms, phone.connected, t, language)}</Value></Row>
            <Row label={t("mobileSetup.details.notifications")}><Value mono={false}>{notifications}</Value></Row>
          </div>
          {state.method === "relay" ? <Note>{t("mobileSetup.relay.encrypted")}</Note> : null}
        </DialogBody>
      </DialogContent>
    </Dialog>
  );
}

/**
 * 폰 연결 방식: the relay's state with its retry or update, and under 고급
 * the direct Tailscale way with the checks it needs. Pairing is not here:
 * Add device › 폰 is the one way to pair (operator review 2026-11-23).
 */
function MethodDialog({ state, actions, onClose }: { state: MobileState; actions: Actions; onClose: () => void }) {
  const { t } = useInterfaceTranslation();
  return (
    <Dialog open onOpenChange={(next) => { if (!next) onClose(); }}>
      <DialogContent showCloseButton data-mobile-method-dialog={state.method}>
        <DialogHeader>
          <DialogTitle>{t("mobileSetup.method")}</DialogTitle>
        </DialogHeader>
        <DialogBody>
          <div className="divide-y divide-border rounded-md border border-border bg-card">
            {state.method === "relay" && (state.enabled || state.exposure === "failed") ? <RelayConnection state={state} actions={actions} />
              : <Row label={t(state.method === "relay" ? "mobileSetup.methodRelay" : "mobileSetup.methodTailscale")} detail={state.method === "relay" ? <span className="text-body text-muted-foreground">{t("mobileSetup.relay.encrypted")}</span> : undefined} />}
            <Disclosure title={t("mobileSetup.relay.advanced")} data-mobile-advanced="true">
              <Row label={t("mobileSetup.relay.tailscale")}>
                <Switch checked={state.method === "tailscale"} aria-label={t("mobileSetup.relay.tailscale")} onCheckedChange={(direct) => actions.setMobileMethod(direct ? "tailscale" : "relay")} />
              </Row>
              {state.method === "tailscale" && state.enabled ? <Checks state={state} /> : null}
              {state.method === "tailscale" ? <ExposureLine state={state} /> : null}
            </Disclosure>
          </div>
        </DialogBody>
      </DialogContent>
    </Dialog>
  );
}

/**
 * The QR window, which Add device › 폰 opens: it turns 폰에서 Hide 열기 on
 * when it is off, makes one five-minute code once the way is ready, and
 * shows the QR, where it opens and its countdown. An expired code offers a
 * fresh one; a phone that pairs closes the window, and closing it ends the
 * code (PRD settings-cleanup B58).
 */
function PairDialog({ state, actions, onClose }: { state: MobileState; actions: Actions; onClose: () => void }) {
  const { t } = useInterfaceTranslation();
  const asked = useRef(false);
  const phones = useRef(state.phones.length);
  const ready = state.exposure === "exposed";
  // As the window opens it turns phone access on when it is off, and as it
  // closes it takes the code it showed with it; a window mounted again (React
  // runs a mount twice in development) asks for a code again.
  const opened = useRef({ enabled: state.enabled, actions });
  useEffect(() => {
    const { enabled, actions: atOpen } = opened.current;
    if (!enabled) atOpen.setMobileEnabled(true);
    return () => {
      asked.current = false;
      atOpen.hideMobileCode();
    };
  }, []);
  useEffect(() => {
    if (!ready || asked.current) return;
    asked.current = true;
    actions.showMobileCode();
  }, [actions, ready]);
  // The code was spent: the phone it paired is in the list behind the window.
  useEffect(() => {
    if (state.phones.length > phones.current) onClose();
    phones.current = state.phones.length;
  }, [state.phones.length, onClose]);
  const since = useSinceFrame(state, 1000, state.code_expires_at_ms !== null);
  const countdown = codeCountdown(state.code_expires_at_ms, state.now_ms, since);
  const expired = state.qr !== null && countdown === null;
  return (
    <Dialog open onOpenChange={(next) => { if (!next) onClose(); }}>
      <DialogContent showCloseButton data-mobile-pair-dialog={state.method}>
        <DialogHeader>
          <DialogTitle>{t("mobileSetup.pair")}</DialogTitle>
        </DialogHeader>
        <DialogBody className="space-y-md">
          {!ready ? (
            <div className="divide-y divide-border rounded-md border border-border bg-card">
              {state.method === "relay" ? <RelayConnection state={state} actions={actions} /> : <><Checks state={state} /><ExposureLine state={state} /></>}
            </div>
          ) : null}
          {ready && state.qr && !expired ? (
            <div className="flex flex-wrap items-center gap-xl" data-mobile-pairing="ready">
              <QrCode text={state.qr} />
              <div className="flex min-w-0 flex-1 flex-col gap-sm">
                <span className="text-headline font-semibold text-foreground">{t("mobileSetup.scan")}</span>
                {state.method === "relay" ? (
                  <>
                    <span className="text-subhead text-muted-foreground" data-mobile-scan="iphone">
                      {t("mobileSetup.scanIphone")}{" "}
                      <a href={APP_STORE_URL} target="_blank" rel="noopener noreferrer" className="inline-flex items-center gap-xs text-primary outline-none focus-visible:ring-1 focus-visible:ring-ring" data-mobile-app-store="true">
                        {t("mobileSetup.appStore")}
                        <ExternalLinkIcon aria-hidden="true" className="size-(--size-icon)" />
                      </a>
                    </span>
                    <span className="text-subhead text-muted-foreground" data-mobile-scan="other">{t("mobileSetup.scanOther")}</span>
                  </>
                ) : (
                  <span className="text-subhead text-muted-foreground">{t("mobileSetup.pairDescription")}</span>
                )}
                <span className="break-all font-mono text-subhead text-subtle-foreground" data-mobile-url={state.url ?? ""}>{state.url}</span>
                <span className="font-mono text-subhead text-muted-foreground" data-mobile-countdown={countdown ?? "expired"}>
                  {countdown ? t("mobileSetup.codeExpires", { time: countdown }) : t("mobileSetup.codeExpired")}
                </span>
              </div>
            </div>
          ) : null}
          {ready && expired ? (
            <Row data-mobile-pairing="expired" label={<Note>{t("mobileSetup.codeExpired")}</Note>}>
              <Button variant="secondary" size="sm" onClick={() => actions.showMobileCode()} data-mobile-show-code="true">{t("mobileSetup.showQr")}</Button>
            </Row>
          ) : null}
        </DialogBody>
      </DialogContent>
    </Dialog>
  );
}

/** The relay's one row, including refusals. It never opens an alert. */
function RelayConnection({ state, actions }: { state: MobileState; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const machine = useShellStore((s) => coreMachineName(s.rest?.navigator?.devices, t));
  const status = state.relay.state;
  const problem = phoneProblem(state, machine, t);
  const text = problem ? problem.text : status === "connected" ? t("mobileSetup.relay.connected") : t("mobileSetup.relay.connecting");
  return (
    <Row data-mobile-connection={status} label={<Status tone={problem ? "warn" : status === "connected" ? "live" : "pending"}>{text}</Status>}
      detail={<span className="text-body text-muted-foreground">{t("mobileSetup.relay.encrypted")}</span>}>
      {problem?.action === "update" ? <Button variant="secondary" size="sm" onClick={() => actions.openSettings("about")}>{t("mobileSetup.relay.update")}</Button>
        : problem?.action === "retry" ? <Button variant="secondary" size="sm" onClick={() => actions.retryMobile()}>{t("mobileSetup.relay.retry")}</Button> : null}
    </Row>
  );
}

/** One ready line when Tailscale is set up, otherwise only the steps that need the operator (B57). */
function Checks({ state }: { state: MobileState }) {
  const { t } = useInterfaceTranslation();
  // Tailscale runs on the core's machine, which a window on another machine names rather than calling it this Mac.
  const machine = useShellStore((s) => coreMachineName(s.rest?.navigator?.devices, t));
  const view = checklistView(state, machine, t);
  if (view.kind === "ready") return <Row label={<Status tone="ok">{t("mobileSetup.ready")}</Status>} data-mobile-ready="true" />;
  return view.kind === "failed" ? view.steps.map((step) => <FailedStepRow key={step.id} step={step} />) : null;
}

function FailedStepRow({ step }: { step: FailedStep }) {
  const { t } = useInterfaceTranslation();
  return (
    <div className="flex items-start gap-md px-md py-sm" data-mobile-step={step.id} data-step-state="failed">
      <AlertTriangleIcon aria-hidden="true" className="size-(--size-icon-lg) shrink-0 text-warning" />
      <span className="sr-only">{t("mobileSetup.step.failed")}: </span>
      <div className="flex min-w-0 flex-1 flex-col gap-xxs">
        <span className="text-subhead text-foreground">{step.title}</span>
        <span className="text-body text-muted-foreground">{step.action}</span>
      </div>
      {step.link ? (
        <a href={step.link.href} target="_blank" rel="noopener noreferrer" className="inline-flex shrink-0 items-center gap-xs text-subhead text-primary outline-none focus-visible:ring-1 focus-visible:ring-ring" data-mobile-step-link={step.id}>
          {step.link.label}
          <ExternalLinkIcon aria-hidden="true" className="size-(--size-icon)" />
        </a>
      ) : null}
    </div>
  );
}

/** What hide itself says about the direct way's serve entry (checking, a foreign entry, a failed step), or nothing. */
function ExposureLine({ state }: { state: MobileState }) {
  const { t } = useInterfaceTranslation();
  const machine = useShellStore((s) => coreMachineName(s.rest?.navigator?.devices, t));
  const line = exposureLine(state, machine, t);
  if (!line) return null;
  return (
    <div className="flex items-start gap-md px-md py-sm" data-mobile-pairing="waiting">
      {line.tone === "error" || line.tone === "warn" ? (
        <AlertTriangleIcon aria-hidden="true" className={`size-(--size-icon-lg) shrink-0 ${line.tone === "error" ? "text-destructive" : "text-warning"}`} />
      ) : (
        <CircleAlertIcon aria-hidden="true" className="size-(--size-icon-lg) shrink-0 text-muted-foreground" />
      )}
      <span role={line.tone === "error" ? "alert" : undefined} className={`min-w-0 break-words text-subhead ${line.tone === "error" ? "text-destructive" : line.tone === "warn" ? "text-warning" : "text-muted-foreground"}`} data-mobile-exposure-line={state.exposure}>
        {line.text}
      </span>
    </div>
  );
}

/** The ms since `frame` arrived, ticking every `tickMs` while `live`. */
function useSinceFrame(frame: object, tickMs: number, live: boolean): number {
  const [arrived, setArrived] = useState(() => Date.now());
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const at = Date.now();
    setArrived(at);
    setNow(at);
  }, [frame]);
  useEffect(() => {
    if (!live) return;
    const timer = window.setInterval(() => setNow(Date.now()), tickMs);
    return () => window.clearInterval(timer);
  }, [tickMs, live]);
  return Math.max(0, now - arrived);
}

function QrCode({ text }: { text: string }) {
  const { t } = useInterfaceTranslation();
  const cells = useMemo(() => {
    const code = qrcode(0, "M");
    code.addData(text, "Byte");
    code.make();
    const count = code.getModuleCount();
    const dark: string[] = [];
    for (let row = 0; row < count; row += 1) {
      for (let col = 0; col < count; col += 1) {
        if (code.isDark(row, col)) dark.push(`M${col} ${row}h1v1h-1z`);
      }
    }
    return { count, path: dark.join("") };
  }, [text]);
  const quiet = 4;
  return (
    <svg
      role="img"
      aria-label={t("mobileSetup.qr")}
      viewBox={`${-quiet} ${-quiet} ${cells.count + quiet * 2} ${cells.count + quiet * 2}`}
      className="size-(--size-mobile-qr) shrink-0 rounded-lg"
      data-mobile-qr={text}
      shapeRendering="crispEdges"
    >
      <rect x={-quiet} y={-quiet} width={cells.count + quiet * 2} height={cells.count + quiet * 2} className="fill-qr-background" />
      <path d={cells.path} className="fill-qr-foreground" />
    </svg>
  );
}
