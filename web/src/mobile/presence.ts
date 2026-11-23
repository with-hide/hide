import type { PairedHost } from "./hosts";
import { asksOf, gatheredOf, missionAsksOf, type GatheredAsk } from "./gather";
import { refusalOf, type AgentGroup, type PhoneMission, type ServerFrame } from "./protocol";
import { RelaySocket } from "./relaySocket";

export type Presence = {
  online: boolean;
  needs?: number;
  working?: number;
  lastSeen: number | null;
  protocolMismatch?: number;
  revoked?: boolean;
  /** Its last list's 내 차례 and 읽을 것 for the gathered view (`gather.ts`); absent until a list arrived and once the socket closed. */
  gathered?: AgentGroup[];
  /** The agents its last list left out to fit one relay message, which the gathered view says it does not show. */
  omitted?: number;
  /** Its automations that stopped for a person, from its last automations frame: more of 내 차례 in the gathered view. */
  asks?: GatheredAsk[];
  /** Its missions that are their own 내 차례 item, from its last missions frame, as the computer counts them: more of 내 차례 in the gathered view. */
  missions?: PhoneMission[];
};

/**
 * One list observer, owned by a visible picker or the visible gathered view; it sends no command and keeps only the groups the
 * gathered view draws. A revoked host is never watched.
 */
export function watchHost(host: PairedHost & { credential: string }, changed: (presence: Presence) => void): () => void {
  let ended = false;
  let socket: RelaySocket | null = null;
  let timer: ReturnType<typeof setTimeout> | null = null;
  let lastSeen = host.lastSeen;
  let attempt = 0;
  const clear = () => { if (timer) clearTimeout(timer); timer = null; };
  const open = () => {
    if (ended) return;
    let heard = Date.now();
    let denied = false;
    let protocolMismatch: number | undefined;
    let revoked = false;
    // The list, the automations and the missions arrive in separate frames; each keeps the others' last value.
    let listed: Pick<Presence, "needs" | "working" | "gathered" | "omitted"> = {};
    let asks: GatheredAsk[] | undefined;
    let missions: PhoneMission[] | undefined;
    const tell = () => changed({ online: true, lastSeen, ...listed, ...(asks ? { asks } : {}), ...(missions ? { missions } : {}) });
    const ws = new RelaySocket(host, (protocol) => { denied = true; protocolMismatch = protocol; }, () => undefined);
    socket = ws;
    ws.onopen = () => ws.send(JSON.stringify({ client_kind: "phone", token: host.credential }));
    ws.onmessage = (event) => {
      heard = Date.now();
      try {
        const frame = JSON.parse(String(event.data)) as ServerFrame;
        if (frame.type === "hello") { attempt = 0; lastSeen = Date.now(); changed({ online: true, lastSeen }); }
        if (frame.type === "agents") {
          listed = {
            needs: frame.groups.find((g) => g.group === "needs_you")?.count ?? 0,
            working: frame.groups.find((g) => g.group === "working")?.count ?? 0,
            gathered: gatheredOf(frame.groups),
            omitted: frame.omitted ?? 0,
          };
          tell();
        }
        // Before its list, a computer is still connecting: its automations' and missions' asks wait for the list rather than stand alone.
        if (frame.type === "automations") {
          asks = asksOf(frame.items, frame.asks ?? []);
          if (listed.gathered) tell();
        }
        if (frame.type === "missions") {
          missions = missionAsksOf(frame.items, frame.asks ?? []);
          if (listed.gathered) tell();
        }
        if (frame.type === "refused") { denied = frame.reason !== "mobile_off"; revoked = refusalOf(frame.reason) === "revoked"; ws.close(); }
      } catch { ws.close(); }
    };
    const health = () => {
      if (Date.now() - heard >= 30_000) { ws.close(); return; }
      if (ws.readyState === WebSocket.OPEN) {
        try { ws.send(JSON.stringify({ type: "ping", id: 0 })); } catch { ws.close(); return; }
      }
      timer = setTimeout(health, 10_000);
    };
    timer = setTimeout(health, 10_000);
    ws.onclose = () => {
      clear(); socket = null;
      changed({ online: false, lastSeen, protocolMismatch, ...(revoked ? { revoked } : {}) });
      if (!ended && !denied) timer = setTimeout(open, Math.min(10_000, 1000 * 2 ** Math.min(attempt++, 4)) * (0.75 + Math.random() * 0.5));
    };
  };
  open();
  return () => {
    ended = true; clear();
    if (socket) { socket.onclose = socket.onmessage = socket.onopen = null; socket.close(); socket = null; }
  };
}
