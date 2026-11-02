import { useEffect, useState } from "react";
import type { TFunction } from "i18next";
import { Button } from "../components/ui/button";
import { readHosts, hostLink, removeHost, type PairedHost } from "./hosts";
import { watchHost, type Presence } from "./presence";

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
      return <div key={host.room} className="flex min-w-0 items-center"><Button variant="ghost" className="h-auto min-h-(--size-touch-target) min-w-0 flex-1 justify-start gap-sm px-sm py-md text-left" onClick={() => choose(host)}>
        <span role="img" aria-label={t(state?.online ? "mobile.connected" : "mobile.disconnected")} className={`size-(--size-status-mark) shrink-0 rounded-full ${state?.online ? "bg-success" : "bg-muted-foreground"}`} />
        <span className="min-w-0 flex-1"><span className="block truncate text-title">{host.name}</span>
          <span className="block text-body text-muted-foreground">{state?.online
            ? state.needs !== undefined ? t("mobile.relay.counts", { needs: state.needs, working: state.working ?? 0 }) : t("mobile.relay.connecting")
            : seen ? t("mobile.relay.lastSeen", { time: new Date(seen).toLocaleString() }) : t("mobile.relay.neverSeen")}</span>
        </span>
      </Button><Button variant="ghost" size="sm" className="min-h-(--size-touch-target)" onClick={() => setRemoving(host)}>{t("mobile.relay.remove")}</Button></div>;
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
    <Button variant="ghost" className="min-h-(--size-touch-target) justify-start" onClick={() => window.location.assign("/?add=1")}>{t("mobile.relay.add")}</Button>
    {error ? <p role="alert">{t("mobile.relay.storage")}</p> : null}
  </div>;
}

export function VersionScreen({ t, name, protocol, app }: { t: TFunction; name: string; protocol: number | null; app: string }) {
  return <main className="flex min-h-full flex-col justify-center gap-lg px-lg text-center">
    <h1 className="break-words text-headline font-semibold">{t("mobile.relay.update", { name })}</h1>
    <p className="text-subhead text-muted-foreground">{t("mobile.relay.versions", { app, device: protocol ?? "?" })}</p>
    <Button variant="secondary" className="min-h-(--size-touch-target)" onClick={() => window.location.assign("/")}>{t("mobile.relay.choose")}</Button>
  </main>;
}
