// Agent notifications in the desktop app (PRD first-run-onboarding D-09 to
// D-11, B31-B39). hided judges which root agents newly need the operator or
// finished, with the phone push's rules, and sends this window an
// `agent_notice` frame while it asks for them; this window words each one in
// the interface language and hands it to the desktop host, which drops it
// while a Hide window has the keyboard. A click opens that agent's pane, and
// the newest View it has to look at, as pressing its row does; an agent the
// operator has seen takes its notification back. A notice a machine this
// computer may open handed over, because someone is here and no one is
// there (PRD device-connect-iroh D-60), is worded the same way and a click
// selects that machine's tile, as does the notice that a phone was paired
// with such a machine (D-70). The app icon counts root agents in each
// notification group whose switch is on. A once automation whose run ended
// is a Needs You line no agent row carries (mission-104 S-4), so this window
// notifies it itself, once, when the line appears; a click opens that run.

import { useEffect, useRef } from "react";
import type { Actions } from "./actions";
import { statusText } from "./agentStatus";
import { lookText } from "./automationWords";
import { requireInterfaceLanguage } from "./i18n/locale";
import { everyDeviceLooks, type AutomationLook } from "./ownAsks";
import { askSubjectLead, askSubjectNotice } from "./askSubject";
import { AGENT_NOTICE_PREFIX } from "./firstRun";
import type { AgentNotice } from "./generated/hided-ws";
import { hostBridge } from "./host";
import { useInterfaceTranslation } from "./i18n/client";
import type { SnapshotRest } from "./snapshot";
import { agentsByPane, allAgents as allPaneAgents } from "./paneAgents";
import { useShellStore } from "./store";

/** How this window tags a once automation's ended run: `automation-look:<automation id>|<run number>`. */
const LOOK_TAG_PREFIX = "automation-look:";

function lookTag(look: AutomationLook): string {
  return `${AGENT_NOTICE_PREFIX}${LOOK_TAG_PREFIX}${look.automation.id}|${look.automation.look_run ?? ""}`;
}

/** Every device's once automations whose run waits to be opened, which a notification announces and a click opens. */

/** How hided tags a notice another machine handed over: `peer:<connection id>|<that machine's tag>`. */
const PEER_TAG_PREFIX = "peer:";

type Listener = (notice: AgentNotice) => void;
const listeners = new Set<Listener>();

/** One `agent_notice` frame from hided; nothing listening is no notification, which is the answer. */
export function receiveAgentNotice(payload: AgentNotice): void {
  for (const listener of listeners) listener(payload);
}

/** What the host may show at most (desktop `NOTIFY_LIMITS`); longer text is cut here, since the host refuses it. */
const TITLE_LIMIT = 120;
const BODY_LIMIT = 400;

function cut(text: string, limit: number): string {
  return text.length <= limit ? text : `${text.slice(0, limit - 1)}…`;
}

/**
 * What the app icon counts: 내 차례 alone, the root agents of every device the
 * window shows that are Needs You and the missions and automations that are
 * their own Needs You item, while the Needs You notifications are on; unread
 * Done is never counted, whatever its own switch says, so the icon says the
 * number 내 차례 says (operator decisions 2026-11-23, replacing 2026-10-18's
 * Done in the icon). Each device's scope carries those rows as its
 * `needs_you` `raised` entry (unresolved, not delegated, in a pane the device
 * draws), so adding the devices up counts the whole desktop whichever device
 * the sidebar shows; the navigator's own scope has none.
 */
export function notificationBadgeCount(rest: SnapshotRest | null): number {
  if (rest?.ui_state?.desktop_notifications !== true) return 0;
  let count = 0;
  for (const device of rest.navigator?.devices ?? []) {
    const scope = device.agent_scope;
    for (const group of scope.raised) {
      if (group.group === "needs_you") count += group.shown.length + group.more.length;
    }
    count += (scope.mission_asks?.length ?? 0) + (scope.automation_asks?.length ?? 0) + (scope.automation_looks?.length ?? 0);
  }
  return count;
}

/**
 * Shows each group through the desktop host while its own switch is on, and
 * a new phone of a machine this computer may open whichever switch is on;
 * opens what a click names, and keeps the enabled groups' app icon count.
 */
export function useAgentNotifications(actions: Actions) {
  const { t, i18n } = useInterfaceTranslation();
  const bridge = hostBridge();
  const live = useShellStore((s) => s.connection === "live");
  const needsYouEnabled = useShellStore((s) => s.rest?.ui_state?.desktop_notifications === true);
  const doneEnabled = useShellStore((s) => (s.rest?.ui_state?.desktop_notifications_done ?? s.rest?.ui_state?.desktop_notifications) === true);
  const wanted = bridge !== null && live && (needsYouEnabled || doneEnabled);
  // Null until the page is live: the first count is also the host's sign that a click can be opened here.
  const badge = useShellStore((s) => (s.connection !== "live" ? null : notificationBadgeCount(s.rest)));
  useEffect(() => {
    if (!live) return;
    return bridge?.onComputerUse?.((inUse) => actions.reportComputerUse(inUse));
  }, [bridge, live, actions]);
  useEffect(() => {
    if (bridge && badge !== null) bridge.reportBadge(badge);
  }, [bridge, badge]);
  // A reconnect is a new connection whose demand starts empty, so it is said again each time the page is live.
  useEffect(() => {
    if (!wanted) return;
    actions.wantAgentNotices(true, needsYouEnabled);
    return () => actions.wantAgentNotices(false);
  }, [actions, wanted, needsYouEnabled]);
  // hided sends every desktop window its frames; the window listens while it is live, whichever switch is on, since a new phone
  // shows with both off (D-70) and each group's notice is held to its own switch below.
  useEffect(() => {
    if (!bridge || !live) return;
    const listener: Listener = ({ notices, clear, phones }) => {
      if (clear.length > 0) bridge.withdrawNotifications(clear.map((tag) => `${AGENT_NOTICE_PREFIX}${tag}`));
      // A phone paired with a machine this computer may open shows whichever switch is on (D-70): it is how the operator sees a pairing they did not make.
      for (const paired of phones ?? []) {
        const machine = useShellStore.getState().rest?.navigator?.peers?.find((peer) => peer.key === paired.peer)?.name;
        const body = machine ? `${machine} · ${paired.phone}` : paired.phone;
        bridge.notify(`${AGENT_NOTICE_PREFIX}${paired.tag}`, t("mobile.notice.phonePaired"), cut(body, BODY_LIMIT), { unlessFocused: false });
      }
      for (const notice of notices) {
        if (!(notice.state === "needs_you" ? needsYouEnabled : doneEnabled)) continue;
        // The row's own word leads, as on its line (status-marks B23); the section name only when the core sent none.
        // A View to look at says the eye's word with the View as its sentence (status-model.md, A View to look at).
        const code = notice.status_code;
        const word = askSubjectLead(t, notice.subject) ?? (code === "look" ? t("agents.mark.look") : code ? statusText(t, code) : t(notice.state === "needs_you" ? "firstRun.notice.needsYou" : "firstRun.notice.done"));
        const detail = notice.subject ? askSubjectNotice(t, notice.subject) : notice.detail;
        const state = detail ? `${word} · ${detail}` : word;
        const body = notice.place ? t("firstRun.notice.body", { state, project: notice.place }) : state;
        bridge.notify(`${AGENT_NOTICE_PREFIX}${notice.tag}`, cut(notice.title, TITLE_LIMIT), cut(body, BODY_LIMIT), { unlessFocused: true });
      }
    };
    listeners.add(listener);
    return () => {
      listeners.delete(listener);
    };
  }, [bridge, live, needsYouEnabled, doneEnabled, t]);
  // A once automation's line appearing is notified once, as the Needs You switch says; the lines there when the page went live are
  // not, as hided seeds its own (B33); a line put away takes its notification back. The snapshot is read through a subscription,
  // not a selector, so a snapshot that moves no line renders nothing here.
  const shownLooks = useRef<Set<string> | null>(null);
  useEffect(() => {
    if (!bridge || !live) {
      shownLooks.current = null;
      return undefined;
    }
    const observe = (state: Pick<ReturnType<typeof useShellStore.getState>, "rest" | "automations">) => {
      const looks = everyDeviceLooks(state.rest, state.automations);
      const now = new Set(looks.map(lookTag));
      const before = shownLooks.current;
      shownLooks.current = now;
      if (before === null) return;
      const gone = [...before].filter((tag) => !now.has(tag));
      if (gone.length > 0) bridge.withdrawNotifications(gone);
      if (!needsYouEnabled) return;
      const language = requireInterfaceLanguage(i18n.language);
      for (const look of looks) {
        const tag = lookTag(look);
        if (before.has(tag)) continue;
        const result = look.run ? lookText(look.run, language, t).text : null;
        const body = [t("automationScreen.look.kind"), result, look.run?.answer?.trim()].filter(Boolean).join(" · ");
        bridge.notify(tag, cut(look.automation.name, TITLE_LIMIT), cut(body, BODY_LIMIT), { unlessFocused: true });
      }
    };
    observe(useShellStore.getState());
    return useShellStore.subscribe((state, previous) => {
      if (state.rest !== previous.rest || state.automations !== previous.automations) observe(state);
    });
  }, [bridge, live, needsYouEnabled, t, i18n.language]);
  useEffect(() => {
    if (!bridge) return undefined;
    return bridge.onNotificationOpen((id) => {
      if (!id.startsWith(AGENT_NOTICE_PREFIX)) return;
      const tag = id.slice(AGENT_NOTICE_PREFIX.length);
      if (tag.startsWith(LOOK_TAG_PREFIX)) {
        const [automation, run] = tag.slice(LOOK_TAG_PREFIX.length).split("|");
        if (automation) actions.showScreen(null, () => actions.openAutomation(automation, run ? Number(run) : null));
        return;
      }
      // Another machine's work opens in that machine's tile, as pressing its row does (D-50).
      if (tag.startsWith(PEER_TAG_PREFIX)) {
        const key = tag.slice(PEER_TAG_PREFIX.length, tag.indexOf("|"));
        const machine = useShellStore.getState().rest?.navigator?.peers?.find((peer) => peer.key === key);
        if (machine) actions.focusDevice(machine.device_id);
        return;
      }
      // The tag is `<device id>|<pane id>`; the pane id names the agent wherever it runs.
      const paneId = tag.slice(tag.indexOf("|") + 1);
      if (!paneId) return;
      // A 볼 것 notice names the View, so it opens that View as the row's chip does; every other notice goes to the agent
      // (operator decision 2026-11-23). A row's notice is its newest, so the row's mark now says which one was clicked.
      // It is this machine's work, so a window on another computer's screen shows its own again, told after the open.
      const state = useShellStore.getState();
      const agent = agentsByPane(allPaneAgents(state.agents, state.rest?.status?.remote)).get(paneId);
      actions.showScreen(null, () => {
        if (agent?.mark === "look" && agent.look) actions.openLook(paneId, agent.look);
        else actions.openListedAgent(paneId);
      });
    });
  }, [bridge, actions]);
}
