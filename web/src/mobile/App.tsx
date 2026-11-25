import { OfflineText } from "./OfflineText";
// The phone app's screens (PRD mobile-companion D-08, B11-B16, B19-B23,
// B30, B38): pairing, the refused and unpaired guidance, and the list; the
// detail is `Detail.tsx`. Every value comes from the store the socket writes.

import { Loader2Icon, PlusIcon, XIcon } from "lucide-react";
import { useEffect, useState } from "react";
import { useInterfaceTranslation } from "../i18n/translator";
import { Button } from "../components/ui/button";
import { closeDetail, openStartSheet, pairNow } from "./connection";
import { AutomationDetail } from "./AutomationDetail";
import { AutomationAskRows, AutomationsScreen } from "./AutomationsScreen";
import { Detail } from "./Detail";
import { GroupRows } from "./ListRows";
import { MissionDetail } from "./MissionDetail";
import { MissionAskRows, MissionsScreen } from "./MissionsScreen";
import { PrDetail } from "./PrDetail";
import { PrsScreen } from "./PrsScreen";
import { SettingsScreen } from "./SettingsScreen";
import { StartSheet } from "./StartSheet";
import { TabBar } from "./TabBar";
import { ScreenHead } from "./layout";
import {
  GROUP_TITLE,
  type GroupId,
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
import { ADD_DEVICE_LINK, HOST_LIST_LINK, type PairedHost } from "./hosts";
import { hintedMachine, markHinted, readHinted } from "./addHint";
import { DemoBar } from "./Demo";
import { ComputerChoice, GatheredList, openRoom, useConnectableHosts, type Watch } from "./Gathered";

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
  const automationOpen = usePhone((s) => s.automationOpen);
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
  if (tab === "automations") return automationOpen ? <AutomationDetail /> : <AutomationsScreen />;
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
export function ListScreen({ hosted = hostedPhone(), picker, gathered }: { hosted?: boolean; picker?: { room: string; open: boolean }; /** The gallery's: the computer the page is open on and how other computers are watched, in place of the address and sockets. */ gathered?: { room: string; watch: Watch } }) {
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
  // The automations that stopped for a person and the missions that are their own item are lines of 내 차례 too (PRD automations D-12, B23).
  const asks = usePhone((s) => s.automationAsks.length + s.missionAsks.length);
  const scope = usePhone((s) => s.listScope);
  // A hosted phone paired with two computers or more chooses between all of them and this one (PRD device-connect-iroh B55).
  const hosts = useConnectableHosts();
  const activeRoom = gathered?.room ?? openRoom();
  const deviceConnect = usePhone((s) => s.deviceConnect);
  const open = hosts?.find((host) => host.room === activeRoom);
  // Before this computer's hello on this page, its saved answer stands, so a computer that is off still gathers the others (B62).
  const offered = deviceConnect ?? open?.deviceConnect === true;
  const computers = offered && hosted && open && hosts && hosts.length > 1 ? hosts : null;
  useEffect(() => {
    if (groups && connected) void closeStaleNotifications(groups);
  }, [groups, connected]);
  const dim = unreachable && !connected;
  return (
    <main className="flex min-h-full flex-col">
      {/* The computer's name is the screen's title (Doop 01); on the hosted phone it opens the device sheet. */}
      <ScreenHead
        data-phone-connected={connected ? "true" : "false"}
        data-phone-header={!hosted && macName ? headerLine(t, macName, otherPhones, devices) : undefined}
        title={computers && activeRoom ? "hide" : hosted ? <HostPicker name={macName} activeRoom={picker?.room} defaultOpen={picker?.open} /> : macName ? machinesName(macName, devices) : "hide"}
        line={computers && activeRoom ? <ComputerChoice hosts={computers} activeRoom={activeRoom} /> : otherPhones > 0 ? t("mobile.otherPhonesLine", { count: otherPhones }) : null}
        action={
          <Button variant="ghost" size="icon" aria-label={t("mobile.start.title")} data-phone-start="true" className="-mr-sm size-(--size-touch-target) text-foreground" onClick={openStartSheet}>
            <PlusIcon aria-hidden="true" className="size-(--size-icon-lg)" />
          </Button>
        }
      />
      {dim ? (
        <div role="status" className="mt-md flex items-center gap-sm px-xl py-sm text-body text-subtle-foreground" data-phone-unreachable="true">
          <Loader2Icon aria-hidden="true" className="size-(--size-icon-lg) shrink-0 animate-spin" />
          <span><OfflineText /></span>
        </div>
      ) : null}
      {relayError === "storage" || relayError === "loadError" ? <p role="alert" className="px-lg text-warning">{t(`mobile.relay.${relayError}`)}</p> : null}
      {installHint && !standalone() ? <InstallHint /> : null}
      {hosted && hosts ? <AddMachineHint hosts={hosts} /> : null}
      {computers && activeRoom && scope === "all" ? <GatheredList hosts={computers} activeRoom={activeRoom} dim={dim} watch={gathered?.watch} /> : <div className={dim ? "opacity-(--opacity-dimmed)" : undefined} aria-busy={dim}>
        {groups && groups.length === 0 ? (
          <p className="px-lg py-xxl text-center text-title text-muted-foreground" data-phone-empty="true">
            {t("mobile.noAgents")}
          </p>
        ) : null}
        {groups && asks > 0 && !groups.some((group) => group.group === "needs_you") ? (
          <section className="px-xl pt-lg" data-phone-group="needs_you">
            <GroupHead id="needs_you" count={asks} />
            <AutomationAskRows />
            <MissionAskRows />
          </section>
        ) : null}
        {groups?.map((group) => {
          const id = group.group;
          // An automation that stopped for a person, or a mission that is its own item, is one more line of 내 차례 (PRD automations D-12, B23).
          const count = group.count + (id === "needs_you" ? asks : 0);
          if (count === 0) return null;
          return (
            <section key={id} className="px-xl pt-lg" data-phone-group={id}>
              <GroupHead id={id} count={count} />
              {id === "needs_you" ? (
                <>
                  <AutomationAskRows />
                  <MissionAskRows />
                </>
              ) : null}
              {group.count > 0 ? <GroupRows group={group} /> : null}
            </section>
          );
        })}
        {groups && omitted > 0 ? (
          <p className="px-xl py-md text-body text-muted-foreground" data-phone-agents-omitted={omitted}>{t("mobile.omitted", { count: omitted })}</p>
        ) : null}
      </div>}
      <TabBar active="agents" />
      <StartSheet />
    </main>
  );
}

function GroupHead({ id, count }: { id: GroupId; count: number }) {
  const { t } = useInterfaceTranslation();
  return (
    <h2 className={`flex gap-sm pb-xs text-subhead ${id === "needs_you" ? "font-semibold text-warning" : "text-muted-foreground"}`}>
      <span>{t(GROUP_TITLE[id])}</span>
      <span className="font-mono" data-phone-group-count={count}>
        {count}
      </span>
    </h2>
  );
}

/** Shown once after pairing, in a browser tab: keep hide on the Home Screen (B11). */
/**
 * One line, once: "<machine>도 폰에 추가할 수 있어요" for a machine the open computer opens that runs work of its own and that this
 * phone keeps under no saved computer (PRD device-connect-iroh B56). [추가] starts adding a computer; either it or the close ends
 * that machine's hint for good.
 */
function AddMachineHint({ hosts }: { hosts: readonly PairedHost[] }) {
  const { t } = useInterfaceTranslation();
  const others = usePhone((s) => s.otherMachines);
  const [hinted, setHinted] = useState<readonly string[] | null>(() => {
    try { return readHinted(localStorage); } catch { return null; }
  });
  useEffect(() => { if (hinted === null) patch({ relayError: "storage" }); }, [hinted]);
  const machine = hinted ? hintedMachine(others, hosts, hinted) : null;
  if (!machine) return null;
  const done = () => {
    try { markHinted(localStorage, machine.machine); setHinted(readHinted(localStorage)); return true; }
    catch { patch({ relayError: "storage" }); return false; }
  };
  return (
    <div className="mx-xl mt-md flex items-center gap-sm text-body leading-relaxed text-subtle-foreground" data-phone-add-hint={machine.machine}>
      <span className="min-w-0 flex-1 break-keep wrap-anywhere">{t("mobile.addHint", { name: machine.name })}</span>
      <Button variant="text" className="min-h-(--size-touch-target) shrink-0 px-none text-body" data-phone-add-hint-add="true" onClick={() => { if (done()) window.location.assign(ADD_DEVICE_LINK); }}>
        {t("mobile.addHintAdd")}
      </Button>
      <Button variant="ghost" size="icon" aria-label={t("common.close")} className="size-(--size-touch-target) text-muted-foreground" data-phone-add-hint-close="true" onClick={() => { done(); }}>
        <XIcon aria-hidden="true" className="size-(--size-icon)" />
      </Button>
    </div>
  );
}

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
