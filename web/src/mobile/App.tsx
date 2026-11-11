import { OfflineText } from "./OfflineText";
// The phone app's screens (PRD mobile-companion D-08, B11-B16, B19-B23,
// B30, B38): pairing, the refused and unpaired guidance, and the list; the
// detail is `Detail.tsx`. Every value comes from the store the socket writes.

import { LaptopIcon, Loader2Icon, PlusIcon, XIcon } from "lucide-react";
import { useEffect, useState } from "react";
import { useInterfaceTranslation } from "../i18n/translator";
import { Button } from "../components/ui/button";
import { closeDetail, openStartSheet, pairNow } from "./connection";
import { Detail } from "./Detail";
import { GroupRows } from "./ListRows";
import { MissionDetail } from "./MissionDetail";
import { MissionsScreen } from "./MissionsScreen";
import { PrDetail } from "./PrDetail";
import { PrsScreen } from "./PrsScreen";
import { SettingsScreen } from "./SettingsScreen";
import { StartSheet } from "./StartSheet";
import { TabBar } from "./TabBar";
import {
  GROUP_TITLE,
  REFUSAL_NOTICE,
  headerLine,
  machinesName,
  noticeText,
} from "./protocol";
import { closeStaleNotifications } from "./push";
import { patch, usePhone } from "./store";
import { appHost } from "./appHost";
import { hostedPhone, PHONE_PROTOCOL } from "./relayRuntime";
import { HostPicker } from "./HostPicker";
import { Landing, DeviceIdentity } from "./Landing";
import { VersionScreen } from "./HostList";
import { HOST_LIST_LINK } from "./hosts";
import { DemoBar } from "./Demo";

export function App() {
  const demo = usePhone((s) => s.demo);
  if (!demo) return <Screens />;
  // The bar sits over the screen the sample host draws, which scrolls beneath it as it does alone.
  return (
    <div className="flex h-full flex-col">
      <DemoBar />
      <div className="min-h-0 flex-1 overflow-y-auto"><Screens /></div>
    </div>
  );
}

function Screens() {
  const { t } = useInterfaceTranslation();
  const screen = usePhone((s) => s.screen);
  const detail = usePhone((s) => s.detail);
  const name = usePhone((s) => s.macName);
  const deviceProtocol = usePhone((s) => s.deviceProtocol);
  const tab = usePhone((s) => s.tab);
  const missionOpen = usePhone((s) => s.missionOpen);
  const prOpen = usePhone((s) => s.prOpen);
  const pendingOpen = usePhone((s) => s.pendingOpen);
  const unreachable = usePhone((s) => s.unreachable);
  const relayError = usePhone((s) => s.relayError);
  if (screen === "starting") return <main className="min-h-full" />;
  if (screen === "version") return <VersionScreen t={t} name={name} protocol={deviceProtocol} app={String(PHONE_PROTOCOL)} />;
  if (screen === "pair") return <PairScreen />;
  if (screen === "refused") return <Guidance refused />;
  if (screen === "unpaired") return <Guidance refused={false} />;
  // A notification opens the detail, not the list that happens to arrive before it. An offline or refused
  // connection still exposes its normal recovery controls rather than leaving a steady surface forever.
  if (appHost() && pendingOpen && !detail && !unreachable && !relayError) return <main className="min-h-full" />;
  // A worker or a Conductor opened from a mission step is the agent's own detail, on either tab (B36).
  if (detail) return <Detail onBack={closeDetail} />;
  if (tab === "missions") return missionOpen ? <MissionDetail /> : <MissionsScreen />;
  if (tab === "prs") return prOpen ? <PrDetail /> : <PrsScreen />;
  if (tab === "settings") return <SettingsScreen />;
  return <ListScreen />;
}

/** The page the QR opens (B11). */
function PairScreen() {
  const { t } = useInterfaceTranslation();
  const macName = usePhone((s) => s.macName);
  const [pairing, setPairing] = useState(false);
  const unreachable = usePhone((s) => s.unreachable);
  return (
    <Landing note={<>{hostedPhone() ? <>{t("mobile.relay.encrypted")}<br /></> : null}{t("mobile.pair.expiry")}</>}>
      <DeviceIdentity name={macName || t("mobile.mac")} large />
      <p className="max-w-(--size-mobile-copy) text-subhead leading-relaxed text-subtle-foreground">{t("mobile.pair.description")}</p>
      <Button
        variant="secondary"
        className="h-(--size-touch-target) rounded-lg px-xl text-subhead"
        disabled={pairing}
        data-phone-pair="true"
        onClick={() => { setPairing(true); pairNow(); }}
      >
        {t("mobile.pair.connect")}
      </Button>
      {unreachable && hostedPhone() ? <p role="status" className="text-body text-muted-foreground"><OfflineText /></p> : null}
    </Landing>
  );
}

/** No credential, or one hided refused for good (B13, B14, B16). */
function Guidance({ refused }: { refused: boolean }) {
  const { t } = useInterfaceTranslation();
  const refusal = usePhone((s) => s.refusal);
  const relayError = usePhone((s) => s.relayError);
  const text = noticeText(t, refused && refusal ? REFUSAL_NOTICE[refusal] : REFUSAL_NOTICE.no_credential);
  return (
    <Landing>
      <p role={refused ? "alert" : undefined} className="max-w-(--size-mobile-copy) break-keep text-title leading-relaxed text-foreground wrap-anywhere" data-phone-guidance={refused ? (refusal ?? "revoked") : "unpaired"}>
        {relayError ? t(`mobile.relay.${relayError}`) : text}
      </p>
      {refused && refusal !== "phone_limit" && refusal !== "no_credential" ? <p className="text-subhead text-muted-foreground">{noticeText(t, REFUSAL_NOTICE.no_credential)}</p> : null}
      {hostedPhone() ? <Button variant="secondary" className="min-h-(--size-touch-target) px-xl text-subhead" onClick={() => location.assign(HOST_LIST_LINK)}>{t("mobile.relay.choose")}</Button> : null}
    </Landing>
  );
}

function standalone(): boolean {
  // The app is installed by being the app.
  if (appHost()) return true;
  const iosStandalone = (navigator as Navigator & { standalone?: boolean }).standalone === true;
  return iosStandalone || window.matchMedia("(display-mode: standalone)").matches;
}

/**
 * `hosted` and `picker` are the gallery's: the list as a hosted phone draws it,
 * with the device sheet already open on the Mac it names. A phone passes none.
 */
export function ListScreen({ hosted = hostedPhone(), picker }: { hosted?: boolean; picker?: { room: string; open: boolean } }) {
  const { t } = useInterfaceTranslation();
  const macName = usePhone((s) => s.macName);
  const otherPhones = usePhone((s) => s.otherPhones);
  const connected = usePhone((s) => s.connected);
  const unreachable = usePhone((s) => s.unreachable);
  const groups = usePhone((s) => s.groups);
  const omitted = usePhone((s) => s.agentsOmitted);
  const devices = usePhone((s) => s.devices);
  const installHint = usePhone((s) => s.installHint);
  const relayError = usePhone((s) => s.relayError);
  useEffect(() => {
    if (groups && connected) void closeStaleNotifications(groups);
  }, [groups, connected]);
  const dim = unreachable && !connected;
  return (
    <main className="flex min-h-full flex-col">
      <header className="phone-safe-top px-xl" data-phone-connected={connected ? "true" : "false"}>
        <div className="flex items-center justify-between pt-lg">
          <h1 className="text-headline font-semibold text-foreground">hide</h1>
          <div className="flex items-center">
            <button
              type="button"
              aria-label={t("mobile.start.title")}
              data-phone-start="true"
              className="-mr-sm flex size-(--size-touch-target) items-center justify-center rounded-lg text-foreground active:bg-accent"
              onClick={openStartSheet}
            >
              <PlusIcon aria-hidden="true" className="size-(--size-icon-lg)" />
            </button>
          </div>
        </div>
        {hosted ? <HostPicker name={machinesName(macName, devices)} activeRoom={picker?.room} defaultOpen={picker?.open} /> : macName ? (
          <p className="mt-xs flex items-center gap-xs text-subhead text-muted-foreground" data-phone-header={headerLine(t, macName, otherPhones, devices)}>
            <LaptopIcon aria-hidden="true" className="size-(--size-icon) shrink-0" />
            <span className="min-w-0 truncate">{headerLine(t, macName, otherPhones, devices)}</span>
          </p>
        ) : null}
      </header>
      {dim ? (
        <div role="status" className="mt-md flex items-center gap-sm px-xl py-sm text-body text-subtle-foreground" data-phone-unreachable="true">
          <Loader2Icon aria-hidden="true" className="size-(--size-icon-lg) shrink-0 animate-spin" />
          <span><OfflineText /></span>
        </div>
      ) : null}
      {relayError === "storage" || relayError === "loadError" ? <p role="alert" className="px-lg text-warning">{t(`mobile.relay.${relayError}`)}</p> : null}
      {installHint && !standalone() ? <InstallHint /> : null}
      <div className={dim ? "opacity-(--opacity-dimmed)" : undefined} aria-busy={dim}>
        {groups && groups.length === 0 ? (
          <p className="px-lg py-xxl text-center text-title text-muted-foreground" data-phone-empty="true">
            {t("mobile.noAgents")}
          </p>
        ) : null}
        {groups?.map((group) => {
          const id = group.group;
          if (group.count === 0) return null;
          return (
            <section key={id} className="px-xl pt-lg" data-phone-group={id}>
              <h2 className="flex gap-sm pb-xs text-body text-muted-foreground">
                <span>{t(GROUP_TITLE[id])}</span>
                <span className="font-mono" data-phone-group-count={group.count}>
                  {group.count}
                </span>
              </h2>
              <GroupRows group={group} />
            </section>
          );
        })}
        {groups && omitted > 0 ? (
          <p className="px-xl py-md text-body text-muted-foreground" data-phone-agents-omitted={omitted}>{t("mobile.omitted", { count: omitted })}</p>
        ) : null}
      </div>
      <TabBar active="agents" />
      <StartSheet />
    </main>
  );
}

/** Shown once after pairing, in a browser tab: keep hide on the Home Screen (B11). */
function InstallHint() {
  const { t } = useInterfaceTranslation();
  return (
    <div className="mx-xl mt-lg flex items-start gap-sm text-body leading-relaxed text-subtle-foreground" data-phone-install-hint="true">
      <span className="min-w-0 flex-1 break-keep wrap-anywhere">{t("mobile.installHint")}</span>
      <Button variant="ghost" size="icon" aria-label={t("common.close")} className="size-(--size-touch-target) text-muted-foreground" onClick={() => {
        patch({ installHint: false });
        if (hostedPhone()) { try { localStorage.setItem("hide.phone.installed-hint", "seen"); } catch { patch({ relayError: "storage" }); } }
      }}>
        <XIcon aria-hidden="true" className="size-(--size-icon)" />
      </Button>
    </div>
  );
}
