import { useEffect, useState } from "react";
import type { Actions } from "../../actions";
import type { DeskJob } from "../../desk";
import type { AgentRow } from "../../snapshot";
import { useInterfaceTranslation } from "../../i18n/client";
import { useShellStore } from "../../store";
import { askIdentity, inputFailure, noticeText } from "../../mobile/protocol";
import { useElapsed } from "../elapsed";
import { SessionAskCard } from "./desk-session";

export function SessionThreadAsk({ paneId, deviceId, job, row, actions, onWrite, onTerminal }: { paneId: string; deviceId: string | null; job: DeskJob | null; row: AgentRow | null; actions: Actions; onWrite: () => void; onTerminal?: () => void }) {
  const { t } = useInterfaceTranslation();
  // The daemon does not push an ask: it is read when the panel opens and again whenever the agent's row moves (its change time or group).
  const changed = row?.changed_at_unix_ms ?? null;
  const group = row?.group ?? null;
  useEffect(() => {
    if (deviceId) actions.askPane(deviceId, paneId);
  }, [actions, deviceId, paneId, changed, group, job?.changed_at_unix_ms]);
  const read = useShellStore((s) => (s.paneAsk && s.paneAsk.pane_id === paneId && s.paneAsk.device_id === deviceId ? s.paneAsk : null));
  const [marked, setMarked] = useState<{ ask: string; id: string } | null>(null);
  const [sent, setSent] = useState<{ ask: string; choice: string; request: string | null; dropped?: boolean } | null>(null);
  const result = useShellStore((s) => (sent?.request && s.paneChoice?.request_id === sent.request ? s.paneChoice : null));
  // A connection lost before the answer's result came back leaves no result to wait for: the choice may have landed, so it
  // is answered in the terminal from then on, as a result that says `uncertain` is (review m3).
  const live = useShellStore((s) => s.connection === "live");
  useEffect(() => {
    if (!live) setSent((now) => (now && now.request !== null && !now.dropped ? { ...now, dropped: true } : now));
  }, [live]);
  const ask = read?.ask ?? null;
  const waited = useElapsed(ask?.since_ms ?? null);
  // A choice refused stale means the agent asks something else now: the card is read again.
  const stale = result?.ok === false && result.reason === "stale" ? result.request_id : null;
  useEffect(() => {
    if (stale && deviceId) actions.askPane(deviceId, paneId);
  }, [actions, deviceId, paneId, stale]);
  if (!ask || !deviceId) return null;
  const identity = askIdentity({ device_id: deviceId, pane_id: paneId }, ask);
  const mine = sent?.ask === identity ? sent : null;
  const lost = mine?.dropped === true && !result;
  const answer = !mine ? null : mine.request === null || lost || (result && !result.ok) ? "failed" : result ? "sent" : "sending";
  const reason = mine?.request === null ? "offline" : lost ? "uncertain" : result && !result.ok ? result.reason : null;
  const inTerminal = answer === "failed" && (reason === "uncertain" || reason === "screen_differs" || reason === "ask_not_shown");
  return (
    <SessionAskCard
      ask={ask}
      disabled={false}
      waited={waited}
      picked={marked?.ask === identity ? marked.id : (mine?.choice ?? null)}
      answer={answer}
      error={answer === "failed" ? noticeText(t, inputFailure(reason)) : null}
      inTerminal={inTerminal}
      onPick={(id) => setMarked({ ask: identity, id })}
      onAnswer={(choice) => {
        // An answer sent, or one that may have landed, is never sent again from here: a second send would be a second press.
        if (answer === "sending" || answer === "sent" || inTerminal) return;
        setSent({ ask: identity, choice, request: actions.answerPane(deviceId, paneId, ask, choice) });
      }}
      onTerminal={onTerminal ?? (() => actions.deskTerminal())}
      onWrite={onWrite}
    />
  );
}
