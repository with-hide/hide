// The agents tab of a phone paired with two computers or more (PRD
// device-connect-iroh D-56, D-69, B55, B62): the row of computers, 모두 and
// each one by name, and under 모두 every computer's 내 차례 and 읽을 것 in
// one list. Another computer is watched only while 모두 is on screen, each
// on its own connection, so one that is slow or out of reach shows its own
// line and never holds the others back. `gather.ts` holds the rules.

import { useEffect, useState } from "react";
import { ChevronRightIcon, Loader2Icon, RepeatIcon } from "lucide-react";
import { RowOverlayButton } from "../components/ui/row-button";
import { ToggleGroup, ToggleGroupItem } from "../components/ui/toggle-group";
import { useInterfaceTranslation } from "../i18n/translator";
import { AskLine, AutomationAskRows } from "./AutomationsScreen";
import { MissionAskRows, OtherMissionAskRow } from "./MissionsScreen";
import { gatherGroups, hereLine, hostLine, type GatheredAsk, type GatheredGroup, type GatheredHost, type HostLine } from "./gather";
import { automationLink, dropHostPush, hostLink, hostStore, missionLink, type PairedHost } from "./hosts";
import { FoldButton, GatheredRowItem } from "./ListRows";
import { DeviceTag } from "./parts";
import { watchHost, type Presence } from "./presence";
import { GROUP_TITLE, type PhoneMission } from "./protocol";
import { setListScope, usePhone } from "./store";

/** A saved computer this phone can connect to: one that refused this phone's credential is not listed. */
export type ConnectableHost = PairedHost & { credential: string };

/** The computers this phone keeps that can connect, in the order it keeps them; null while they are read or when they cannot be. */
export function useConnectableHosts(): ConnectableHost[] | null {
  const [hosts, setHosts] = useState<ConnectableHost[] | null>(null);
  useEffect(() => {
    let live = true;
    void hostStore().list().then(
      (saved) => { if (live) setHosts(saved.filter((host): host is ConnectableHost => host.credential !== null)); },
      () => { if (live) setHosts(null); },
    );
    return () => { live = false; };
  }, []);
  return hosts;
}

/** The room of the computer this page is open on, which its address names. */
export function openRoom(): string | null {
  return location.pathname.startsWith("/p/") ? new URLSearchParams(location.search).get("host") : null;
}

/**
 * 모두 and each computer by name. 모두 and this computer stay on this page; another computer opens its own list through the start
 * page, which loads the bundle its protocol needs (B55).
 */
export function ComputerChoice({ hosts, activeRoom }: { hosts: readonly GatheredHost[]; activeRoom: string }) {
  const { t } = useInterfaceTranslation();
  const scope = usePhone((s) => s.listScope);
  return (
    // One line that scrolls sideways: a phone paired with many computers keeps its list where it was.
    <div className="-mx-xl mt-sm overflow-x-auto px-xl pb-xs [scrollbar-width:none]">
    <ToggleGroup
      type="single"
      variant="pill"
      aria-label={t("mobile.gather.computers")}
      className="flex-nowrap"
      value={scope === "all" ? "all" : activeRoom}
      data-phone-computers={scope}
      onValueChange={(value) => {
        // Pressing the chosen chip again unselects it in a toggle group; the list stays where it is.
        if (!value) return;
        if (value === "all") setListScope("all");
        else if (value === activeRoom) setListScope("one");
        else window.location.assign(hostLink({ room: value }, undefined, true));
      }}
    >
      <ToggleGroupItem value="all" variant="pill" className="min-h-(--size-touch-target) shrink-0 px-md" data-phone-computer-choice="all">{t("mobile.gather.all")}</ToggleGroupItem>
      {hosts.map((host) => (
        <ToggleGroupItem key={host.room} value={host.room} variant="pill" className="min-h-(--size-touch-target) max-w-(--size-mobile-copy) shrink-0 px-md" data-phone-computer-choice={host.room}>
          <span className="truncate">{host.name}</span>
        </ToggleGroupItem>
      ))}
    </ToggleGroup>
    </div>
  );
}

/** How the view watches another computer; the gallery passes values in place of sockets. */
export type Watch = (host: ConnectableHost, changed: (presence: Presence) => void) => () => void;

/**
 * Watches every other computer while the view is on screen and the page is visible, each on its own connection; a computer that
 * refused this phone is dropped from the phone as the device list does, and is not watched again.
 */
function usePresences(hosts: readonly ConnectableHost[], activeRoom: string, watch: Watch): Record<string, Presence> {
  const [presence, setPresence] = useState<Record<string, Presence>>({});
  useEffect(() => {
    let stops: (() => void)[] = [];
    const stop = () => { for (const close of stops) close(); stops = []; };
    const follow = () => {
      stop();
      if (document.hidden) return;
      stops = hosts.filter((host) => host.room !== activeRoom).map((host) => watch(host, (next) => {
        if (next.revoked) {
          void hostStore().revoke(host.room).catch((error: unknown) => console.warn("phone.revoke_failed", { room: host.room, error: String(error) }));
          void dropHostPush(host.room);
        }
        setPresence((all) => ({ ...all, [host.room]: next }));
      }));
    };
    follow();
    document.addEventListener("visibilitychange", follow);
    return () => { stop(); document.removeEventListener("visibilitychange", follow); };
  }, [hosts, activeRoom, watch]);
  return presence;
}

/** Another computer's automation that stopped for a person, in the gathered view's 내 차례 with that computer's mark. */
type OtherAsk = { host: GatheredHost; ask: GatheredAsk };

/** Another computer's mission that is its own 내 차례 item, in the gathered view's 내 차례 with that computer's mark. */
type OtherMission = { host: GatheredHost; mission: PhoneMission };

/**
 * 모두: 내 차례 and 읽을 것 of this computer and of every other one whose list arrived, each row with its computer's mark, then one
 * line for each computer still connecting, out of reach or speaking another phone protocol (B62), this one included, so a page
 * opened on a computer that is off still gathers the others; and one line for each computer whose list left agents out.
 */
export function GatheredList({ hosts, activeRoom, dim, watch = watchHost }: { hosts: readonly ConnectableHost[]; activeRoom: string; dim: boolean; watch?: Watch }) {
  const { t } = useInterfaceTranslation();
  const groups = usePhone((s) => s.groups);
  const asksHere = usePhone((s) => s.automationAsks.length + s.missionAsks.length);
  const omittedHere = usePhone((s) => s.agentsOmitted);
  const unreachable = usePhone((s) => s.unreachable);
  const presence = usePresences(hosts, activeRoom, watch);
  const here = hosts.find((host) => host.room === activeRoom);
  const others = hosts.filter((host) => host.room !== activeRoom && !presence[host.room]?.revoked);
  const listed = others.filter((host) => presence[host.room]?.gathered);
  const gathered = gatherGroups([
    ...(here && groups ? [{ host: here, here: true, groups }] : []),
    ...listed.map((host) => ({ host, here: false, groups: presence[host.room]!.gathered! })),
  ]);
  const otherAsks: OtherAsk[] = listed.flatMap((host) => (presence[host.room]?.asks ?? []).map((ask) => ({ host, ask })));
  const otherMissions: OtherMission[] = listed.flatMap((host) => (presence[host.room]?.missions ?? []).map((mission) => ({ host, mission })));
  const asks = asksHere + otherAsks.length + otherMissions.length;
  const ownLine = here ? hereLine(groups !== null, unreachable) : null;
  const lines = [
    ...(here && ownLine ? [{ host: here, line: ownLine }] : []),
    ...others.flatMap((host) => {
      const line = hostLine(presence[host.room]);
      return line ? [{ host, line }] : [];
    }),
  ];
  const omitted = [
    ...(here && groups && omittedHere > 0 ? [{ host: here, count: omittedHere }] : []),
    ...listed.flatMap((host) => {
      const count = presence[host.room]?.omitted ?? 0;
      return count > 0 ? [{ host, count }] : [];
    }),
  ];
  const needs = gathered.find((group) => group.group === "needs_you");
  const askRows = <AskRows here={here} others={otherAsks} missions={otherMissions} />;
  return (
    <div data-phone-gathered-list="">
      {asks > 0 && !needs ? (
        <section className="px-xl pt-lg" data-phone-group="needs_you">
          <GatheredHead group="needs_you" count={asks} />
          {askRows}
        </section>
      ) : null}
      {gathered.map((group) => <GatheredSection key={group.group} group={group} asks={group.group === "needs_you" ? asks : 0} askRows={askRows} dim={dim} />)}
      {gathered.length === 0 && asks === 0 && (groups || listed.length > 0) ? (
        <p className="px-lg py-xxl text-center text-title text-muted-foreground" data-phone-gathered-empty="">{t("mobile.gather.empty")}</p>
      ) : null}
      {omitted.map(({ host, count }) => (
        <p key={host.room} className="px-xl pt-md text-body text-muted-foreground" data-phone-gathered-omitted={host.room}>{t("mobile.gather.omitted", { name: host.name, count })}</p>
      ))}
      {lines.length > 0 ? (
        <ul className="flex flex-col px-xl pt-lg" data-phone-host-lines="">
          {lines.map(({ host, line }) => <HostLineRow key={host.room} name={host.name} line={line} room={host.room} />)}
        </ul>
      ) : null}
    </div>
  );
}

/**
 * The automations that stopped for a person and the missions that are their own item, on every computer: this one's with their
 * [다시 켜기] and opening here, another's opening on that computer.
 */
function AskRows({ here, others, missions }: { here: GatheredHost | undefined; others: readonly OtherAsk[]; missions: readonly OtherMission[] }) {
  return (
    <>
      <AutomationAskRows computer={here?.name} />
      <MissionAskRows computer={here?.name} />
      {others.length > 0 ? (
        <ul className="divide-y divide-border border-b border-border" data-phone-gathered-asks={others.length}>
          {others.map(({ host, ask }) => <OtherAskRow key={`${host.room}|${ask.id}`} host={host} ask={ask} />)}
        </ul>
      ) : null}
      {missions.length > 0 ? (
        <ul data-phone-gathered-missions={missions.length}>
          {missions.map(({ host, mission }) => (
            <li key={`${host.room}|${mission.id}`} data-phone-gathered-mission={`${host.room}|${mission.id}`}>
              <OtherMissionAskRow mission={mission} computer={host.name} onOpen={() => window.location.assign(missionLink(host, mission.id))} />
            </li>
          ))}
        </ul>
      ) : null}
    </>
  );
}

function OtherAskRow({ host, ask }: OtherAsk) {
  const { t } = useInterfaceTranslation();
  return (
    <li className="relative" data-phone-gathered-ask={`${host.room}|${ask.id}`}>
      <RowOverlayButton aria-label={`${t("mobile.automations.open", { name: ask.name })}, ${host.name}`} hover={false} className="active:bg-accent" onClick={() => window.location.assign(automationLink(host, ask.id))} />
      <div className="pointer-events-none relative flex min-h-(--size-touch-target) items-center gap-sm py-md">
        <RepeatIcon aria-hidden="true" className="size-(--size-icon) shrink-0 text-warning" />
        <span className="flex min-w-0 flex-1 flex-col items-start gap-xs">
          <AskLine name={ask.name} reason={ask.reason} />
          <DeviceTag label={host.name} computer />
        </span>
        <ChevronRightIcon aria-hidden="true" className="size-(--size-icon) shrink-0 text-muted-foreground" />
      </div>
    </li>
  );
}

function GatheredHead({ group, count }: { group: "needs_you" | "done"; count: number }) {
  const { t } = useInterfaceTranslation();
  return (
    <h2 className="flex gap-sm pb-xs text-body text-muted-foreground">
      <span>{t(GROUP_TITLE[group])}</span>
      <span className="font-mono" data-phone-group-count={count}>{count}</span>
    </h2>
  );
}

function GatheredSection({ group, asks, askRows, dim }: { group: GatheredGroup; asks: number; askRows: React.ReactNode; dim: boolean }) {
  const { t } = useInterfaceTranslation();
  const [opened, setOpened] = useState(false);
  const id = group.group === "needs_you" ? "needs_you" : "done";
  return (
    <section className="px-xl pt-lg" data-phone-group={id}>
      <GatheredHead group={id} count={group.count + asks} />
      {id === "needs_you" ? askRows : null}
      <ul className="divide-y divide-border border-b border-border">
        {[...group.rows, ...(opened ? group.more : [])].map((item) => <GatheredRowItem key={`${item.host.room}|${item.row.kind === "agent" ? `${item.row.device_id}|${item.row.pane_id}` : item.row.sleep_id}`} item={item} dim={dim} />)}
        {group.more.length > 0 && !opened ? (
          <li><FoldButton label={t("mobile.more", { count: group.more.length })} note={null} data-phone-fold={`gather:${id}`} onOpen={() => setOpened(true)} /></li>
        ) : null}
      </ul>
    </section>
  );
}

/** One other computer whose rows are not here: connecting with the spinner, out of reach, or "<computer> 업데이트 필요" (D-69). */
function HostLineRow({ name, line, room }: { name: string; line: HostLine; room: string }) {
  const { t } = useInterfaceTranslation();
  return (
    <li className="flex min-h-(--size-touch-target) min-w-0 items-center gap-sm text-body text-subtle-foreground" data-phone-host-line={line} data-phone-host-room={room}>
      {/* Every line keeps the spinner's place, so the names stand in one column. */}
      <span className="flex size-(--size-icon) shrink-0 items-center justify-center">{line === "connecting" ? <Loader2Icon aria-hidden="true" className="size-(--size-icon) animate-spin text-muted-foreground" /> : null}</span>
      {line === "update" ? (
        <span className="min-w-0 flex-1 break-keep wrap-anywhere">{t("mobile.gather.update", { name })}</span>
      ) : (
        <>
          <span className="min-w-0 flex-1 truncate">{name}</span>
          <span className="shrink-0 text-muted-foreground">{t(line === "connecting" ? "mobile.relay.connecting" : "mobile.disconnected")}</span>
        </>
      )}
    </li>
  );
}
