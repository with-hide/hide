// Settings > Mobile (PRD mobile-companion B1-B7, B10, B14, B15, B29, B39;
// settings-cleanup B57-B59): the switch, one ready line or only the failing
// Tailscale steps with their fix, the QR once hided has confirmed its serve
// entry and Show QR is pressed, and the paired phones with Revoke; the push
// mode is on Settings > Notifications (PRD settings-left-nav B6, B7). Every
// value is the daemon's `mobile` frame; every control sends one `mobile_*`
// event.

import { AlertTriangleIcon, CircleAlertIcon, ExternalLinkIcon } from "lucide-react";
import qrcode from "qrcode-generator";
import { useEffect, useMemo, useState } from "react";
import type { Actions } from "./actions";
import { useInterfaceTranslation } from "./i18n/client";
import { requireInterfaceLanguage } from "./i18n/locale";
import { Button } from "./components/ui/button";
import { Switch } from "./components/ui/switch";
import { Group, Note, Row, Status } from "./components/settings-rows";
import {
  checklistView,
  codeCountdown,
  exposureLine,
  phoneLine,
  phonesTitle,
  type FailedStep,
  type MobileState,
} from "./mobileSettings";
import { coreMachineName } from "./screenMachine";
import { useShellStore } from "./store";

export function MobileTab({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const mobile = useShellStore((s) => s.mobile);
  const live = useShellStore((s) => s.connection === "live");
  // hided rechecks the checklist while this tab is open (B3). Opening it
  // makes no code and voids none: only Show QR does (B58). A restarted daemon
  // has forgotten the tab, so every live connection says so again.
  useEffect(() => {
    if (!live) return;
    actions.observeMobile(true);
    return () => actions.observeMobile(false);
  }, [actions, live]);
  if (!mobile) return <Note tone="pending">{t("mobileSetup.connecting")}</Note>;
  return (
    <div data-mobile-tab="true" data-mobile-exposure={mobile.exposure}>
      <Group title={t("mobileSetup.title")}>
        <Row
          label={
            <span className="flex min-w-0 flex-col gap-xxs">
              <span>{t("mobileSetup.enable")}</span>
              <span className="text-body text-muted-foreground">{t("mobileSetup.enableDescription")}</span>
            </span>
          }
        >
          <Switch checked={mobile.enabled} onCheckedChange={(next) => actions.setMobileEnabled(next)} aria-label={t("mobileSetup.enable")} data-mobile-switch="true" />
        </Row>
        {mobile.enabled ? <Checks state={mobile} /> : null}
        {/* A removal that failed at switch-off stays on screen until it is finished (B7). */}
        {mobile.enabled || mobile.exposure === "failed" ? <Pairing state={mobile} actions={actions} /> : null}
      </Group>
      {mobile.phones.length > 0 ? <Phones state={mobile} actions={actions} /> : null}
    </div>
  );
}

/** One ready line when Tailscale is set up, otherwise only the steps that need the operator (B57). */
function Checks({ state }: { state: MobileState }) {
  const { t } = useInterfaceTranslation();
  // Tailscale runs on the core's machine, which a window on another machine names rather than calling it this Mac.
  const machine = useShellStore((s) => coreMachineName(s.rest?.navigator?.devices, t));
  const view = checklistView(state, machine, t);
  if (view.kind === "ready") {
    return (
      <Row label={<Status tone="ok">{t("mobileSetup.ready")}</Status>} data-mobile-ready="true" />
    );
  }
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

/** The ms since this frame arrived, ticking every `tickMs` while `live`. */
function useSinceFrame(frame: MobileState, tickMs: number, live: boolean): number {
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

function QrCode({ text, expired }: { text: string; expired: boolean }) {
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
  const quiet = 2;
  return (
    <svg
      role="img"
      aria-label={t("mobileSetup.qr")}
      viewBox={`${-quiet} ${-quiet} ${cells.count + quiet * 2} ${cells.count + quiet * 2}`}
      className={`size-(--size-mobile-qr) shrink-0 rounded-lg bg-qr-background${expired ? " opacity-(--opacity-disabled)" : ""}`}
      data-mobile-qr={text}
      data-mobile-qr-expired={expired ? "true" : undefined}
      shapeRendering="crispEdges"
    >
      <path d={cells.path} className="fill-qr-foreground" />
    </svg>
  );
}

function Pairing({ state, actions }: { state: MobileState; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const since = useSinceFrame(state, 1000, state.code_expires_at_ms !== null);
  const machine = useShellStore((s) => coreMachineName(s.rest?.navigator?.devices, t));
  const line = exposureLine(state, machine, t);
  if (line) {
    return (
      <div className="flex items-start gap-md px-md py-sm" data-mobile-pairing="waiting">
        {line.tone === "error" || line.tone === "warn" ? (
          <AlertTriangleIcon aria-hidden="true" className={`size-(--size-icon-lg) shrink-0 ${line.tone === "error" ? "text-destructive" : "text-warning"}`} />
        ) : (
          <CircleAlertIcon aria-hidden="true" className="size-(--size-icon-lg) shrink-0 text-muted-foreground" />
        )}
        <span
          role={line.tone === "error" ? "alert" : undefined}
          className={`min-w-0 break-words text-subhead ${line.tone === "error" ? "text-destructive" : line.tone === "warn" ? "text-warning" : "text-muted-foreground"}`}
          data-mobile-exposure-line={state.exposure}
        >
          {line.text}
        </span>
      </div>
    );
  }
  // Until hided has confirmed its serve entry there is nothing to pair with.
  if (state.exposure !== "exposed") return null;
  const countdown = codeCountdown(state.code_expires_at_ms, state.now_ms, since);
  // An expired code cannot pair: the button offers a fresh one directly instead of Hide QR.
  const expired = state.qr !== null && countdown === null;
  return (
    <>
      <Row
        data-mobile-pairing="idle"
        label={
          <span className="flex min-w-0 flex-col gap-xxs">
            <span>{t("mobileSetup.pair")}</span>
            <span className="text-body text-muted-foreground">{t("mobileSetup.pairDescription")}</span>
          </span>
        }
      >
        {state.qr && !expired ? (
          <Button variant="secondary" size="sm" onClick={() => actions.hideMobileCode()} data-mobile-hide-code="true">
            {t("mobileSetup.hideQr")}
          </Button>
        ) : (
          <Button variant="secondary" size="sm" onClick={() => actions.showMobileCode()} data-mobile-show-code="true">
            {t("mobileSetup.showQr")}
          </Button>
        )}
      </Row>
      {state.qr ? (
        <div className="flex flex-wrap items-center gap-xl px-md py-lg" data-mobile-pairing="ready">
          <QrCode text={state.qr} expired={expired} />
          <div className="flex min-w-0 flex-1 flex-col gap-sm">
            <span className="text-headline font-semibold text-foreground">{t("mobileSetup.scan")}</span>
            <span className="text-subhead text-muted-foreground">{t("mobileSetup.scanDescription")}</span>
            <span className="break-all font-mono text-subhead text-subtle-foreground" data-mobile-url={state.url ?? ""}>
              {state.url}
            </span>
            <span className="font-mono text-subhead text-muted-foreground" data-mobile-countdown={countdown ?? "expired"}>
              {countdown ? t("mobileSetup.codeExpires", { time: countdown }) : t("mobileSetup.codeExpired")}
            </span>
          </div>
        </div>
      ) : null}
    </>
  );
}

function Phones({ state, actions }: { state: MobileState; actions: Actions }) {
  const { t, i18n } = useInterfaceTranslation();
  const language = requireInterfaceLanguage(i18n.language);
  // The daemon's clock decides "just now" and the revoke date; this only adds the time since its frame.
  const daemonNow = state.now_ms + useSinceFrame(state, 30_000, true);
  return (
    <Group title={phonesTitle(state, t)} data-mobile-phones={String(state.phones.length)}>
      {state.phones.map((phone) => (
        <Row
          key={phone.id}
          label={
            <span className="flex min-w-0 flex-col gap-xxs" data-mobile-phone={phone.id}>
              <span className="break-words">{phone.name}</span>
              <span className="text-body text-muted-foreground" data-mobile-phone-line={phone.id}>
                {phoneLine(phone, daemonNow, t, language)}
              </span>
            </span>
          }
        >
          <Button variant="secondary" size="sm" className="text-destructive" onClick={() => actions.revokePhone(phone.id)} data-mobile-revoke={phone.id}>
            {t("mobileSetup.revoke")}
          </Button>
        </Row>
      ))}
    </Group>
  );
}
