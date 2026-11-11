import { useEffect, useState } from "react";
import type { TFunction } from "i18next";
import { LaptopIcon, PlusIcon, ServerIcon, Trash2 } from "lucide-react";
import { Button } from "../components/ui/button";
import { leaveHostPush } from "./appPush";
import { hostStore, hostLink, removeHost, dropHostPush, HOST_LIST_LINK, type PairedHost } from "./hosts";
import { watchHost, type Presence } from "./presence";
import { Landing, DeviceIdentity } from "./Landing";
import { PHONE_PROTOCOL } from "./phoneVersion";
import { MovingOwl } from "../components/moving-owl";
import blink from "../../../site/public/assets/owls/owl-blink.svg";
import blinkStill from "../../../site/public/assets/owls/owl-blink-still.svg";
import type { PhoneDevice } from "./protocol";

/** Observe lists only while the picker is open. Never opens a detail or sends an action. */
export function HostList({ t, active, machines = [], activeRoom: named, choose = (host: PairedHost) => window.location.assign(hostLink(host)) }: { t: TFunction; active?: Presence; /** The Mac being viewed when the page address does not name it (the gallery). */ activeRoom?: string; /** The devices the Mac being viewed reaches, drawn under its row (PRD mobile-companion-ui B9). */ machines?: readonly PhoneDevice[]; choose?: (host: PairedHost) => void }) {
  // Undefined while the list is read (the app reads each computer's secret from the Keychain), null when it cannot be read.
  const [hosts, setHosts] = useState<PairedHost[] | null | undefined>(undefined);
  const [error, setError] = useState(false);
  const reload = () => hostStore().list().then(setHosts, () => setHosts(null));
  useEffect(() => { void reload(); }, []);
  const [removing, setRemoving] = useState<PairedHost | null>(null);
  const activeRoom = named ?? (location.pathname.startsWith("/p/") ? new URLSearchParams(location.search).get("host") : null);
  const [presence, setPresence] = useState<Record<string, Presence>>({});
  useEffect(() => {
    let stops: (() => void)[] = [];
    const stop = () => { for (const close of stops) close(); stops = []; };
    // The device refused this phone: its row stays, without the credential, and is watched no more.
    const revoked = (room: string) => {
      void hostStore().revoke(room).then(reload).catch(() => setError(true));
      void dropHostPush(room);
    };
    const followVisibility = () => {
      stop();
      if (!document.hidden) stops = hosts?.filter((host): host is PairedHost & { credential: string } => host.room !== activeRoom && host.credential !== null).map((host) =>
        watchHost(host, (next) => { if (next.revoked) revoked(host.room); else setPresence((all) => ({ ...all, [host.room]: next })); })) ?? [];
    };
    followVisibility();
    document.addEventListener("visibilitychange", followVisibility);
    return () => { stop(); document.removeEventListener("visibilitychange", followVisibility); };
  }, [hosts, activeRoom]);
  if (hosts === undefined) return null;
  if (hosts === null) return <p role="alert">{t("mobile.relay.storage")}</p>;
  return <div className="flex min-w-0 flex-col gap-xs">
    {hosts.map((host) => {
      const state = host.room === activeRoom ? active : presence[host.room];
      const seen = state?.lastSeen ?? host.lastSeen;
      const mismatch = state?.protocolMismatch !== undefined
        ? t("mobile.relay.versions", { app: PHONE_PROTOCOL, device: state.protocolMismatch }) : null;
      const trash = <Button variant="ghost" size="icon" className="mt-md min-h-(--size-touch-target) min-w-(--size-touch-target) text-muted-foreground" aria-label={t("mobile.relay.remove")} onClick={() => setRemoving(host)}><Trash2 aria-hidden="true" /></Button>;
      if (host.credential === null) return <div key={host.room} className="flex min-w-0 items-start" data-phone-host-revoked={host.room}>
        <div className="flex min-w-0 flex-1 items-start gap-md px-sm py-md">
          <LaptopIcon aria-hidden="true" className="mt-xs size-(--size-icon-lg) shrink-0 text-muted-foreground opacity-(--opacity-dimmed)" />
          <span className="flex min-w-0 flex-1 flex-col gap-xs">
            <span className="flex min-w-0 flex-col gap-xs opacity-(--opacity-dimmed)">
              <span className="whitespace-normal break-keep text-subhead font-medium wrap-anywhere">{host.name}</span>
              <span className="flex items-center gap-xs text-body text-muted-foreground">
                <span aria-hidden="true" className="size-(--size-status-mark) shrink-0 rounded-full bg-muted-foreground" />
                {t("mobile.relay.revoked")}
              </span>
              <span className="whitespace-normal break-keep text-body text-muted-foreground">{t("mobile.relay.revokedReason")}</span>
            </span>
            <Button variant="text" className="min-h-(--size-touch-target) self-start px-none text-body" onClick={() => window.location.assign("/?add=1")}>{t("mobile.relay.reconnect")}</Button>
          </span>
        </div>
        {trash}
      </div>;
      const viewing = host.room === activeRoom && state?.online === true;
      return <div key={host.room} className="flex min-w-0 flex-col">
        <div className="flex min-w-0 items-start">
        <Button variant="ghost" className="h-auto min-h-(--size-touch-target) min-w-0 flex-1 items-start justify-start gap-md px-sm py-md text-left" data-phone-host="true" onClick={() => choose(host)}>
          <LaptopIcon aria-hidden="true" className="mt-xs size-(--size-icon-lg) shrink-0 text-muted-foreground" />
          <span className="flex min-w-0 flex-1 flex-col gap-xs">
            <span className="whitespace-normal break-keep text-subhead font-medium wrap-anywhere">{host.name}</span>
            <span className="flex items-center gap-xs text-body text-muted-foreground">
              <span aria-hidden="true" className={`size-(--size-status-mark) shrink-0 rounded-full ${mismatch ? "bg-warning" : state?.online ? "bg-success" : "bg-muted-foreground"}`} />
              {mismatch ? t("mobile.relay.updateTitle") : viewing ? t("mobile.devices.viewing") : t(state?.online ? "mobile.connected" : "mobile.disconnected")}
            </span>
            <span className="whitespace-normal text-body font-normal text-muted-foreground">{mismatch ?? (state?.online
              ? state.needs !== undefined ? t("mobile.relay.counts", { needs: state.needs, working: state.working ?? 0 }) : t("mobile.relay.connecting")
              : seen ? t("mobile.relay.lastSeen", { time: new Date(seen).toLocaleString() }) : t("mobile.relay.neverSeen"))}</span>
          </span>
        </Button>
        {trash}
        </div>
        {host.room === activeRoom ? <ReachedDevices t={t} devices={machines} /> : null}
      </div>;
    })}
    {removing ? <div className="flex flex-col gap-sm px-sm py-md">
      <p className="text-subhead">{t("mobile.relay.removeConfirm", { name: removing.name })}</p>
      <div className="flex gap-sm">
        <Button variant="secondary" className="min-h-(--size-touch-target)" onClick={() => setRemoving(null)}>{t("common.cancel")}</Button>
        <Button variant="destructive" className="min-h-(--size-touch-target)" onClick={() => {
          void leaveHostPush(removing.room).then(() => removeHost(removing.room)).then(() => {
            if (removing.room === activeRoom) location.assign("/");
            else { void reload(); setRemoving(null); }
          }).catch(() => setError(true));
        }}>{t("mobile.relay.remove")}</Button>
      </div>
    </div> : null}
    <Button variant="ghost" className="mt-sm min-h-(--size-touch-target) justify-start gap-sm border-t border-border px-sm text-body" data-phone-add="true" onClick={() => window.location.assign("/?add=1")}><PlusIcon aria-hidden="true" className="size-(--size-icon)" />{t("mobile.relay.add")}</Button>
    {error ? <p role="alert">{t("mobile.relay.storage")}</p> : null}
  </div>;
}

/** The devices the Mac being viewed reaches: each with its state and counts, dimmed while not connected, and why no pairing is needed (B9). */
function ReachedDevices({ t, devices }: { t: TFunction; devices: readonly PhoneDevice[] }) {
  const reached = devices.filter((device) => !device.local);
  if (reached.length === 0) return null;
  const names = reached.map((device) => device.label ?? device.device_id).join(", ");
  return <div className="ml-xl flex min-w-0 flex-col gap-sm border-l border-border pb-sm pl-md" data-phone-devices="true">
    <p className="text-body text-muted-foreground">{t("mobile.devices.via")}</p>
    {reached.map((device) => <div key={device.device_id} className={`flex min-w-0 flex-col gap-xs ${device.connected ? "" : "opacity-(--opacity-dimmed)"}`} data-phone-device-row={device.device_id} data-connected={device.connected ? "true" : "false"}>
      <span className="flex min-w-0 items-center gap-sm">
        <ServerIcon aria-hidden="true" className="size-(--size-icon-lg) shrink-0 text-muted-foreground" />
        <span className="min-w-0 whitespace-normal break-keep text-subhead font-medium wrap-anywhere">{device.label ?? device.device_id}</span>
      </span>
      <span className="flex items-center gap-xs text-body text-muted-foreground">
        <span aria-hidden="true" className={`size-(--size-status-mark) shrink-0 rounded-full ${device.connected ? "bg-success" : "bg-muted-foreground"}`} />
        {t(device.connected ? "mobile.connected" : "mobile.disconnected")}
      </span>
      {device.connected ? <span className="text-body text-muted-foreground">{t("mobile.relay.counts", { needs: device.needs_you, working: device.working })}</span> : null}
    </div>)}
    <p className="whitespace-normal break-keep text-body text-muted-foreground wrap-anywhere">{t("mobile.devices.mixed", { names })}</p>
  </div>;
}

/** A chosen device while its bundle opens: the blinking owl holds the wait (the start page has no log, so a failed drawing goes to the console). */
export function DeviceConnecting({ t, name, error, onChoose }: { t: TFunction; name: string; error: "qrError" | "cameraError" | "cameraBusy" | "storage" | "loadError" | "limit" | null; onChoose: () => void }) {
  return <Landing>
    <MovingOwl motion={blink} still={blinkStill} scene="phone_connecting" className="size-(--size-brand-owl)"
      onUnavailable={(asset) => console.error(JSON.stringify({ event: "brand_owl.unavailable", scene: "phone_connecting", asset }))} />
    <DeviceIdentity name={name} />
    <p role="status" className="text-body text-muted-foreground">{t("mobile.relay.connecting")}</p>
    <Button variant="ghost" className="min-h-(--size-touch-target) text-body" onClick={onChoose}>{t("mobile.relay.choose")}</Button>
    {error ? <p role="alert" className="max-w-(--size-mobile-copy) text-body text-warning">{t(`mobile.relay.${error}`)}</p> : null}
  </Landing>;
}

/** `appBehind`: this is the iOS app and the computer speaks a newer protocol than any bundle inside it, so the app is what needs updating. */
export function VersionScreen({ t, name, protocol, app, appBehind = false }: { t: TFunction; name: string; protocol: number | null; app: string; appBehind?: boolean }) {
  return <Landing>
    <DeviceIdentity name={name} />
    <h1 className="max-w-(--size-mobile-copy) whitespace-normal break-keep text-headline font-semibold leading-relaxed wrap-anywhere">{t(appBehind ? "mobile.relay.updateAppTitle" : "mobile.relay.updateTitle")}</h1>
    <dl className="flex gap-xxl text-body text-muted-foreground" aria-label={t("mobile.relay.versions", { app, device: protocol ?? "?" })}>
      <div className="flex flex-col gap-xs"><dt>{t("mobile.relay.phoneVersion")}</dt><dd className="font-mono text-subtle-foreground">{app}</dd></div>
      <div className="flex flex-col gap-xs"><dt>{t("mobile.relay.deviceVersion")}</dt><dd className="font-mono text-subtle-foreground">{protocol ?? "?"}</dd></div>
    </dl>
    <Button variant="secondary" className="min-h-(--size-touch-target) px-xl text-subhead" onClick={() => window.location.assign(HOST_LIST_LINK)}>{t("mobile.relay.choose")}</Button>
  </Landing>;
}
