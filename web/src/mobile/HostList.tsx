import { useEffect, useState } from "react";
import type { TFunction } from "i18next";
import { LaptopIcon, PlusIcon, Trash2 } from "lucide-react";
import { Button } from "../components/ui/button";
import { readHosts, hostLink, removeHost, type PairedHost } from "./hosts";
import { watchHost, type Presence } from "./presence";
import { Landing, DeviceIdentity } from "./Landing";
import { PHONE_PROTOCOL } from "./phoneVersion";

/** Observe lists only while the picker is open. Never opens a detail or sends an action. */
export function HostList({ t, active, choose = (host: PairedHost) => window.location.assign(hostLink(host)) }: { t: TFunction; active?: Presence; choose?: (host: PairedHost) => void }) {
  const [hosts, setHosts] = useState(() => {
    try { return readHosts(localStorage); } catch { return null; }
  });
  const [error, setError] = useState(false);
  const [removing, setRemoving] = useState<PairedHost | null>(null);
  const activeRoom = location.pathname.startsWith("/p/") ? new URLSearchParams(location.search).get("host") : null;
  const [presence, setPresence] = useState<Record<string, Presence>>({});
  useEffect(() => {
    let stops: (() => void)[] = [];
    const stop = () => { for (const close of stops) close(); stops = []; };
    const followVisibility = () => {
      stop();
      if (!document.hidden) stops = hosts?.filter((host) => host.room !== activeRoom).map((host) =>
        watchHost(host, (next) => setPresence((all) => ({ ...all, [host.room]: next })))) ?? [];
    };
    followVisibility();
    document.addEventListener("visibilitychange", followVisibility);
    return () => { stop(); document.removeEventListener("visibilitychange", followVisibility); };
  }, [hosts, activeRoom]);
  if (!hosts) return <p role="alert">{t("mobile.relay.storage")}</p>;
  return <div className="flex min-w-0 flex-col gap-xs">
    {hosts.map((host) => {
      const state = host.room === activeRoom ? active : presence[host.room];
      const seen = state?.lastSeen ?? host.lastSeen;
      const mismatch = state?.protocolMismatch !== undefined
        ? t("mobile.relay.versions", { app: PHONE_PROTOCOL, device: state.protocolMismatch }) : null;
      return <div key={host.room} className="flex min-w-0 items-start">
        <Button variant="ghost" className="h-auto min-h-(--size-touch-target) min-w-0 flex-1 items-start justify-start gap-md px-sm py-md text-left" onClick={() => choose(host)}>
          <LaptopIcon aria-hidden="true" className="mt-xs size-(--size-icon-lg) shrink-0 text-muted-foreground" />
          <span className="flex min-w-0 flex-1 flex-col gap-xs">
            <span className="whitespace-normal break-keep text-subhead font-medium wrap-anywhere">{host.name}</span>
            <span className="flex items-center gap-xs text-body text-muted-foreground">
              <span aria-hidden="true" className={`size-(--size-status-mark) shrink-0 rounded-full ${mismatch ? "bg-warning" : state?.online ? "bg-success" : "bg-muted-foreground"}`} />
              {mismatch ? t("mobile.relay.updateTitle") : t(state?.online ? "mobile.connected" : "mobile.disconnected")}
            </span>
            <span className="whitespace-normal text-body font-normal text-muted-foreground">{mismatch ?? (state?.online
              ? state.needs !== undefined ? t("mobile.relay.counts", { needs: state.needs, working: state.working ?? 0 }) : t("mobile.relay.connecting")
              : seen ? t("mobile.relay.lastSeen", { time: new Date(seen).toLocaleString() }) : t("mobile.relay.neverSeen"))}</span>
          </span>
        </Button>
        <Button variant="ghost" size="icon" className="mt-md min-h-(--size-touch-target) min-w-(--size-touch-target) text-muted-foreground" aria-label={t("mobile.relay.remove")} onClick={() => setRemoving(host)}><Trash2 aria-hidden="true" /></Button>
      </div>;
    })}
    {removing ? <div className="flex flex-col gap-sm px-sm py-md">
      <p className="text-subhead">{t("mobile.relay.removeConfirm", { name: removing.name })}</p>
      <div className="flex gap-sm">
        <Button variant="secondary" className="min-h-(--size-touch-target)" onClick={() => setRemoving(null)}>{t("common.cancel")}</Button>
        <Button variant="destructive" className="min-h-(--size-touch-target)" onClick={() => {
          void removeHost(removing.room).then(() => {
            if (removing.room === activeRoom) location.assign("/");
            else { setHosts(readHosts(localStorage)); setRemoving(null); }
          }).catch(() => setError(true));
        }}>{t("mobile.relay.remove")}</Button>
      </div>
    </div> : null}
    <Button variant="ghost" className="mt-sm min-h-(--size-touch-target) justify-start gap-sm border-t border-border px-sm text-body" onClick={() => window.location.assign("/?add=1")}><PlusIcon aria-hidden="true" className="size-(--size-icon)" />{t("mobile.relay.add")}</Button>
    {error ? <p role="alert">{t("mobile.relay.storage")}</p> : null}
  </div>;
}

export function VersionScreen({ t, name, protocol, app }: { t: TFunction; name: string; protocol: number | null; app: string }) {
  return <Landing>
    <DeviceIdentity name={name} />
    <h1 className="max-w-(--size-mobile-copy) whitespace-normal break-keep text-headline font-semibold leading-relaxed wrap-anywhere">{t("mobile.relay.updateTitle")}</h1>
    <dl className="flex gap-xxl text-body text-muted-foreground" aria-label={t("mobile.relay.versions", { app, device: protocol ?? "?" })}>
      <div className="flex flex-col gap-xs"><dt>{t("mobile.relay.phoneVersion")}</dt><dd className="font-mono text-subtle-foreground">{app}</dd></div>
      <div className="flex flex-col gap-xs"><dt>{t("mobile.relay.deviceVersion")}</dt><dd className="font-mono text-subtle-foreground">{protocol ?? "?"}</dd></div>
    </dl>
    <Button variant="secondary" className="min-h-(--size-touch-target) px-xl text-subhead" onClick={() => window.location.assign("/")}>{t("mobile.relay.choose")}</Button>
  </Landing>;
}
