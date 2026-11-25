import { useEffect, useState, type ReactNode } from "react";
import type { TFunction } from "i18next";
import { LaptopIcon, PlusIcon, ServerIcon, type LucideIcon } from "lucide-react";
import { Badge } from "../components/ui/badge";
import { Button } from "../components/ui/button";
import { ContextMenu, ContextMenuContent, ContextMenuItem, ContextMenuTrigger } from "../components/ui/context-menu";
import { cn } from "../lib/utils";
import { leaveHostPush } from "./appPush";
import { hostStore, hostLink, removeHost, dropHostPush, ADD_COMPUTER_LINK, ADD_DEVICE_LINK, HOST_LIST_LINK, type PairedHost } from "./hosts";
import { watchHost, type Presence } from "./presence";
import { Landing, DeviceIdentity } from "./Landing";
import { PHONE_PROTOCOL } from "./phoneVersion";
import { MovingOwl } from "../components/moving-owl";
import blink from "../../../site/public/assets/owls/owl-blink.svg";
import blinkStill from "../../../site/public/assets/owls/owl-blink-still.svg";
import { linkMark, type PhoneDevice } from "./protocol";

/**
 * Observe lists only while the picker is open. Never opens a detail or sends an action. Each computer is one row of two lines (the
 * name, then its state and counts), the one being viewed outlined; a long press offers 기기 지우기 (docs/UI_BEHAVIOR.md, The phone
 * app). `card` draws the rows split by hairlines, as a Settings group's card does.
 */
export function HostList({ t, active, machines = [], activeRoom: named, card = false, choose = (host: PairedHost) => window.location.assign(hostLink(host)) }: { t: TFunction; active?: Presence; /** The Mac being viewed when the page address does not name it (the gallery). */ activeRoom?: string; /** The devices the Mac being viewed reaches, drawn under its row (PRD mobile-companion-ui B9). */ machines?: readonly PhoneDevice[]; card?: boolean; choose?: (host: PairedHost) => void }) {
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
  // 기기 지우기 is the row's long press (or right-click, or the menu key), so the row itself stays one press.
  const withMenu = (host: PairedHost, row: ReactNode) => <ContextMenu>
    <ContextMenuTrigger className="select-none [-webkit-touch-callout:none]">{row}</ContextMenuTrigger>
    <ContextMenuContent><ContextMenuItem variant="destructive" data-phone-host-remove={host.room} onSelect={() => setRemoving(host)}>{t("mobile.relay.remove")}</ContextMenuItem></ContextMenuContent>
  </ContextMenu>;
  // A Settings card's rows meet its edges and its hairlines, as its other rows do; the sheet's are rounded and spaced.
  const rowBox = card ? "rounded-none px-md" : "rounded-lg px-sm";
  return <div className={cn("flex min-w-0 flex-col", card ? "divide-y divide-border" : "gap-xxs")}>
    {hosts.map((host) => {
      const state = host.room === activeRoom ? active : presence[host.room];
      const seen = state?.lastSeen ?? host.lastSeen;
      if (host.credential === null) return <div key={host.room} className="min-w-0" data-phone-host-revoked={host.room}>{withMenu(host, <div className={cn("flex min-w-0 flex-col gap-xs py-sm", rowBox)}>
        <div className="flex min-w-0 items-center gap-md opacity-(--opacity-dimmed)">
          <DeviceTile icon={LaptopIcon} />
          <TwoLines name={host.name} dot="off" word={t("mobile.relay.revoked")} />
        </div>
        <span className="flex min-w-0 flex-col items-start pl-[calc(var(--size-control-lg)+var(--spacing-md))]">
          <span className="whitespace-normal break-keep text-body text-muted-foreground opacity-(--opacity-dimmed)">{t("mobile.relay.revokedReason")}</span>
          <Button variant="text" className="min-h-(--size-touch-target) px-none text-body" onClick={() => window.location.assign(ADD_COMPUTER_LINK)}>{t("mobile.relay.reconnect")}</Button>
        </span>
      </div>)}</div>;
      const viewing = host.room === activeRoom && state?.online === true;
      // A computer whose phone protocol differs says both versions in its state's place (the warning dot says it needs updating).
      const mismatch = state?.protocolMismatch !== undefined ? t("mobile.relay.versions", { app: PHONE_PROTOCOL, device: state.protocolMismatch }) : null;
      const word = mismatch ?? (state?.online
        ? state.needs === undefined ? t("mobile.relay.connecting") : t(viewing ? "mobile.devices.viewing" : "mobile.connected")
        : seen ? t("mobile.relay.lastSeen", { time: new Date(seen).toLocaleString() }) : t("mobile.relay.neverSeen"));
      return <div key={host.room} className="flex min-w-0 flex-col">
        {withMenu(host, <Button variant="ghost" className={cn("h-auto min-h-(--size-touch-target) w-full min-w-0 justify-start gap-md py-sm text-left font-normal text-foreground", rowBox, viewing && "ring-2 ring-inset ring-primary")}
          data-phone-host="true" data-phone-host-viewing={viewing ? "true" : undefined} onClick={() => choose(host)}>
          <DeviceTile icon={LaptopIcon} />
          <TwoLines name={host.name} dot={mismatch ? "warning" : state?.online ? "on" : "off"} word={word} hint={mismatch ? t("mobile.relay.updateTitle") : undefined}
            counts={!mismatch && state?.online && state.needs !== undefined ? <Counts t={t} needs={state.needs} working={state.working ?? 0} /> : null} />
        </Button>)}
        {host.room === activeRoom ? <ReachedDevices t={t} devices={machines} /> : null}
      </div>;
    })}
    {removing ? <div className="flex flex-col gap-sm px-md py-md">
      <p className="text-subhead">{t("mobile.relay.removeConfirm", { name: removing.name })}</p>
      <div className="flex gap-sm">
        <Button variant="secondary" className="min-h-(--size-touch-target)" onClick={() => setRemoving(null)}>{t("common.cancel")}</Button>
        <Button variant="destructive" className="min-h-(--size-touch-target)" data-phone-host-remove-confirm="true" onClick={() => {
          void leaveHostPush(removing.room).then(() => removeHost(removing.room)).then(() => {
            if (removing.room === activeRoom) location.assign("/");
            else { void reload(); setRemoving(null); }
          }).catch(() => setError(true));
        }}>{t("mobile.relay.remove")}</Button>
      </div>
    </div> : null}
    <Button variant="ghost" className={cn("min-h-(--size-touch-target) justify-start gap-md text-body text-foreground", rowBox)} data-phone-add="true" onClick={() => window.location.assign(ADD_DEVICE_LINK)}>
      <span className="flex size-(--size-control-lg) shrink-0 items-center justify-center"><PlusIcon aria-hidden="true" className="size-(--size-icon-lg)" /></span>{t("mobile.relay.add")}
    </Button>
    {error ? <p role="alert">{t("mobile.relay.storage")}</p> : null}
  </div>;
}

/** A device's icon in its small rounded tile: a paired computer's laptop, a reached device's server (`compact`, a size smaller). */
export function DeviceTile({ icon: Icon, compact = false }: { icon: LucideIcon; compact?: boolean }) {
  return <span aria-hidden="true" className={cn("flex shrink-0 items-center justify-center rounded-sm bg-secondary text-subtle-foreground", compact ? "size-(--size-control)" : "size-(--size-control-lg)")}>
    <Icon className={compact ? "size-(--size-icon)" : "size-(--size-icon-lg)"} />
  </span>;
}

/**
 * A device row's two lines: its name on one line (with `mark` after it), then its state's dot and word and, at the right, its counts.
 * `hint` is what the dot's colour says, read to assistive technology before the word.
 */
function TwoLines({ name, mark, dot, word, hint, counts, compact = false }: { name: string; mark?: ReactNode; dot: "on" | "off" | "warning"; word: ReactNode; hint?: string; counts?: ReactNode; compact?: boolean }) {
  return <span className="flex min-w-0 flex-1 flex-col gap-xxs">
    <span className="flex min-w-0 items-center gap-xs">
      <span className={cn("min-w-0 truncate font-medium", compact ? "text-body" : "text-subhead")}>{name}</span>{mark}
    </span>
    <span className="flex min-w-0 items-center gap-xs text-body text-muted-foreground">
      <span aria-hidden="true" className={cn("size-(--size-status-mark) shrink-0 rounded-full", dot === "on" ? "bg-success" : dot === "warning" ? "bg-warning" : "bg-muted-foreground")} />
      {hint ? <span className="sr-only">{hint} · </span> : null}
      <span className="min-w-0 truncate">{word}</span>
      {counts}
    </span>
  </span>;
}

/** 내 차례 and 작업 중 as two small numbers at the right, the first in the hold colour; the words are for assistive technology. */
function Counts({ t, needs, working }: { t: TFunction; needs: number; working: number }) {
  return <span className="ml-auto flex shrink-0 items-center gap-xs pl-sm" data-phone-counts={`${needs}:${working}`}>
    <span className="sr-only">{t("mobile.relay.counts", { needs, working })}</span>
    <Badge aria-hidden="true" className="font-mono text-caption text-warning">{needs}</Badge>
    <Badge aria-hidden="true" className="font-mono text-caption">{working}</Badge>
  </span>;
}

/** The devices the Mac being viewed reaches, on a rail under its row: compact rows, dimmed while not connected, and one short line (B9). */
function ReachedDevices({ t, devices }: { t: TFunction; devices: readonly PhoneDevice[] }) {
  const reached = devices.filter((device) => !device.local);
  if (reached.length === 0) return null;
  return <div className="ml-[calc(var(--spacing-sm)+var(--size-control-lg)/2)] flex min-w-0 flex-col gap-xs border-l border-border py-xs pl-md" data-phone-devices="true">
    {reached.map((device) => {
      const mark = linkMark(device);
      return <div key={device.device_id} className={cn("flex min-w-0 items-center gap-md py-xs pr-sm", !device.connected && "opacity-(--opacity-dimmed)")} data-phone-device-row={device.device_id} data-connected={device.connected ? "true" : "false"}>
        <DeviceTile icon={ServerIcon} compact />
        <TwoLines compact name={device.label ?? device.device_id} dot={device.connected ? "on" : "off"} word={t(device.connected ? "mobile.connected" : "mobile.disconnected")}
          mark={mark ? <Badge variant="outline" data-phone-device-link={device.link}>{mark}</Badge> : null}
          counts={device.connected ? <Counts t={t} needs={device.needs_you} working={device.working} /> : null} />
      </div>;
    })}
    <p className="whitespace-normal break-keep text-caption text-muted-foreground wrap-anywhere">{t("mobile.devices.mixed")}</p>
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
