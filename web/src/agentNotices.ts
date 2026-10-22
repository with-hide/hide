// Agent notifications in the desktop app (PRD first-run-onboarding D-09 to
// D-11, B31-B39). hided judges which root agents newly need the operator or
// finished, with the phone push's rules, and sends this window an
// `agent_notice` frame while it asks for them; this window words each one in
// the interface language and hands it to the desktop host, which drops it
// while a Hide window has the keyboard. A click opens that agent's pane, and
// an agent the operator has seen takes its notification back. The app icon
// counts root agents in each notification group whose switch is on.

import { useEffect } from "react";
import type { Actions } from "./actions";
import { statusText } from "./agentStatus";
import { askSubjectText } from "./askSubject";
import { AGENT_NOTICE_PREFIX } from "./firstRun";
import type { AgentNotice } from "./generated/hided-ws";
import { hostBridge } from "./host";
import { useInterfaceTranslation } from "./i18n/client";
import type { SnapshotRest } from "./snapshot";
import { useShellStore } from "./store";

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
 * The agents the app icon counts: the sidebar's enabled notification groups of every
 * device the window shows, the root agents the notifications announce.
 * Each device's scope carries those groups as its `raised` entries
 * (unresolved, not delegated, in a pane the device draws), so adding the
 * devices up counts the whole desktop whichever device the sidebar shows;
 * the navigator's own scope has no such groups. Reading a Done row removes it.
 */
function notificationBadgeCount(rest: SnapshotRest | null): number {
  const needsYou = rest?.ui_state?.desktop_notifications === true;
  const done = (rest?.ui_state?.desktop_notifications_done ?? rest?.ui_state?.desktop_notifications) === true;
  let count = 0;
  for (const device of rest?.navigator?.devices ?? []) {
    for (const group of device.agent_scope.raised) {
      if (group.group === "needs_you" ? needsYou : group.group === "done" && done) {
        count += group.shown.length + group.more.length;
      }
    }
  }
  return count;
}

/**
 * Shows each group through the desktop host while its own switch is on,
 * opens what a click names, and keeps the enabled groups' app icon count.
 */
export function useAgentNotifications(actions: Actions) {
  const { t } = useInterfaceTranslation();
  const bridge = hostBridge();
  const live = useShellStore((s) => s.connection === "live");
  const needsYouEnabled = useShellStore((s) => s.rest?.ui_state?.desktop_notifications === true);
  const doneEnabled = useShellStore((s) => (s.rest?.ui_state?.desktop_notifications_done ?? s.rest?.ui_state?.desktop_notifications) === true);
  const wanted = bridge !== null && live && (needsYouEnabled || doneEnabled);
  // Null until the page is live: the first count is also the host's sign that a click can be opened here.
  const badge = useShellStore((s) => (s.connection !== "live" ? null : notificationBadgeCount(s.rest)));
  useEffect(() => {
    if (bridge && badge !== null) bridge.reportBadge(badge);
  }, [bridge, badge]);
  // A reconnect is a new connection whose demand starts empty, so it is said again each time the page is live.
  useEffect(() => {
    if (!wanted) return;
    actions.wantAgentNotices(true);
    return () => actions.wantAgentNotices(false);
  }, [actions, wanted]);
  useEffect(() => {
    if (!bridge || !wanted) return;
    const listener: Listener = ({ notices, clear }) => {
      if (clear.length > 0) bridge.withdrawNotifications(clear.map((tag) => `${AGENT_NOTICE_PREFIX}${tag}`));
      for (const notice of notices) {
        if (!(notice.state === "needs_you" ? needsYouEnabled : doneEnabled)) continue;
        // The row's own word leads, as on its line (status-marks B23); the section name only when the core sent none.
        const word = notice.status_code ? statusText(t, notice.status_code) : t(notice.state === "needs_you" ? "firstRun.notice.needsYou" : "firstRun.notice.done");
        const detail = notice.subject ? askSubjectText(t, notice.subject) : notice.detail;
        const state = detail ? `${word} · ${detail}` : word;
        const body = notice.place ? t("firstRun.notice.body", { state, project: notice.place }) : state;
        bridge.notify(`${AGENT_NOTICE_PREFIX}${notice.tag}`, cut(notice.title, TITLE_LIMIT), cut(body, BODY_LIMIT), { unlessFocused: true });
      }
    };
    listeners.add(listener);
    return () => {
      listeners.delete(listener);
    };
  }, [bridge, wanted, needsYouEnabled, doneEnabled, t]);
  useEffect(() => {
    if (!bridge) return undefined;
    return bridge.onNotificationOpen((id) => {
      if (!id.startsWith(AGENT_NOTICE_PREFIX)) return;
      // The tag is `<device id>|<pane id>`; the pane id names the agent wherever it runs.
      const tag = id.slice(AGENT_NOTICE_PREFIX.length);
      const paneId = tag.slice(tag.indexOf("|") + 1);
      if (paneId) actions.openListedAgent(paneId);
    });
  }, [bridge, actions]);
}
