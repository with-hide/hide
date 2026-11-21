import type { PairedHost } from "./hosts";
import { refusalOf, type ServerFrame } from "./protocol";
import { RelaySocket } from "./relaySocket";

export type Presence = { online: boolean; needs?: number; working?: number; lastSeen: number | null; protocolMismatch?: number; revoked?: boolean };

/** One list observer, owned by a visible picker; no commands or retained payloads. A revoked host is never watched. */
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
    const ws = new RelaySocket(host, (protocol) => { denied = true; protocolMismatch = protocol; }, () => undefined);
    socket = ws;
    ws.onopen = () => ws.send(JSON.stringify({ client_kind: "phone", token: host.credential }));
    ws.onmessage = (event) => {
      heard = Date.now();
      try {
        const frame = JSON.parse(String(event.data)) as ServerFrame;
        if (frame.type === "hello") { attempt = 0; lastSeen = Date.now(); changed({ online: true, lastSeen }); }
        if (frame.type === "agents") changed({ online: true, lastSeen,
          needs: frame.groups.find((g) => g.group === "needs_you")?.count ?? 0,
          working: frame.groups.find((g) => g.group === "working")?.count ?? 0 });
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
