import { receiveAgentNotice } from "./agentNotices";
import { configureAttachments, receiveAttachmentRefusal } from "./attachments";
import type { AgentNotice } from "./generated/hided-ws";
import { BACKOFF_START_MS, connectionAfterHealthFails, nextBackoff } from "./connection";
import { clearPending, receiveBytes, receiveBytesError, receiveBytesRefusal } from "./fileBytes";
import { receiveAccessResult, refuseWaitingAccessCalls } from "./accessCall";
import { receiveAutomationResult, refuseWaitingAutomationCalls } from "./automationCall";
import { receiveGuestCloseResult, refuseWaitingGuestCloses } from "./guestCloseCall";
import { movesOperatorFocus, numbered } from "./operatorFocus";
import { showProjectOpened } from "./opening";
import { noteArrival, probeEnabled } from "./probe";
import { useShellStore, type TerminalChunk } from "./store";
import type { ProgramCopy } from "./clipboard";
import { hostBridge, hostKind } from "./host";
import type { ViewCommentAnchor } from "./snapshot";
import { browserCommentKey, closeBrowserNote, noteBrowserComment, openBrowserNote, resetBrowserComments, type PagePlace } from "./viewFeedback";
import { BridgeSocket, HIDDEN_CLOSE, remoteBridge } from "./remoteView";
import { refuseCallsOf } from "./windowCall";
import { loseThreadInputConnection, receiveThreadInputResult } from "./sessionDraftSends";

export type DispatchFn = (event: {
  schema_version: number;
  kind: string;
  payload: Record<string, unknown>;
}) => unknown;

type Handlers = {
  onChunks: (chunks: TerminalChunk[], full: boolean) => void;
  /** What a pane's program wrote to the clipboard, handed over once and never with the screen's output. */
  onCopy: (copy: ProgramCopy) => void;
};

function tokenFromLocation(): string | null {
  const hash = new URLSearchParams(window.location.hash.replace(/^#/, ""));
  return hash.get("token");
}

/** What the daemon's `/health` says of the role it runs: `core`, or `node` while this machine's core is on another one. */
type Health = { role?: string };

/**
 * The page's address once its daemon answers as `health`: the address names
 * this machine as a node only while the daemon runs the node role, so a
 * daemon that became its machine's own core (a core moved before 1.2.0 whose
 * machines were split, PRD device-connect-iroh B45) drops the name, and a
 * reload or the host's next attach opens the machine's own screen. Null when
 * the address stays: only a daemon that says it runs the core changes it.
 */
export function addressForRole(hash: string, health: Health): string | null {
  const params = new URLSearchParams(hash.replace(/^#/, ""));
  if (health.role !== "core" || !params.has("node")) return null;
  params.delete("node");
  return `#${params.toString()}`;
}

/** The daemon's health, or null when it does not answer; a health that cannot be read as one says nothing of its role. */
async function readHealth(): Promise<Health | null> {
  try {
    const response = await fetch("/health", { cache: "no-store" });
    if (!response.ok) return null;
    try {
      return ((await response.json()) ?? {}) as Health;
    } catch {
      return {};
    }
  } catch {
    return null;
  }
}

export function connectShell(handlers: Handlers): { dispatch: DispatchFn; sendBinary: (bytes: Uint8Array) => void; open: () => boolean; close: () => void; drop: () => void } {
  let socket: WebSocket | null = null;
  let closed = false;
  let backoff = BACKOFF_START_MS;
  let healthFails = 0;
  let accessReported = false;
  let revision = 0;
  // The core that numbered `revision`, and the one the last daemon frame
  // named: a revision counts only on the core that gave it, so the second
  // becomes the first only when a snapshot or delta from it is applied.
  let revisionCore: string | null = null;
  let announcedCore: string | null = null;
  let terminalSequence: number | null = null;
  let terminalEpoch: string | null = null;
  let operatorFocusSequence = 0;
  let reconnectTimer: number | undefined;
  // The token the current socket was opened with: the address is the page's
  // credential, and a different one in it names a different daemon run.
  let openedToken: string | null = null;

  // Decided once: the probe is a measurement seam, not a per-frame branch.
  const probing = probeEnabled();
  // Another machine's view: its socket is the bridge's, its page holds no
  // token of the host, and the desktop owns its reconnect (PRD
  // device-connect-iroh D-67).
  const bridge = remoteBridge();
  let hidden = false;

  const dispatch: DispatchFn = (event) => {
    if (!socket || socket.readyState !== WebSocket.OPEN) {
      // A key typed while reconnecting is lost, not queued; the badge shows
      // the state and the log keeps the fact.
      useShellStore.getState().noteDiagnostic(`dispatch dropped: ${event.kind} while socket not open`);
      return false;
    }
    // Each operator focus and card open carries this page's next number, so the snapshot
    // can say whether it includes the last one sent (`operatorFocus.ts`).
    // The number counts only once the frame has left.
    if (movesOperatorFocus(event)) {
      const sequence = operatorFocusSequence + 1;
      socket.send(JSON.stringify(numbered(event, sequence)));
      operatorFocusSequence = sequence;
      useShellStore.getState().noteOperatorFocusSent(sequence);
      return true;
    }
    socket.send(JSON.stringify(event));
    return true;
  };

  const reportAccess = (drawn: boolean) => {
    // Another machine's view is counted on the machine that shows it (D-67).
    if (bridge) return;
    if (!accessReported && drawn && useShellStore.getState().connection === "live") {
      // Once per page lifetime, after the first accepted state is usable.
      accessReported = dispatch({ schema_version: 2, kind: "product_analytics_connected", payload: {} }) === true;
    }
  };

  /** Attachment bytes ride the same socket as binary frames (PRD B14). */
  const sendBinary = (bytes: Uint8Array) => {
    if (!socket || socket.readyState !== WebSocket.OPEN) {
      useShellStore.getState().noteDiagnostic("attachment bytes dropped while socket not open");
      return;
    }
    socket.send(bytes);
  };

  const clearSocketState = () => {
    loseThreadInputConnection();
    clearPending("socket_closed");
    refuseWaitingAutomationCalls();
    refuseWaitingAccessCalls();
    refuseWaitingGuestCloses();
    resetBrowserComments();
  };

  const open = () => {
    if (closed) return;
    const token = bridge ? "" : tokenFromLocation();
    if (token === null) {
      useShellStore.getState().setConnection("gone");
      return;
    }
    useShellStore.getState().setConnection(socket ? "reconnecting" : "connecting");
    openedToken = token;
    // Every connection says again whether it may see the pairings under
    // way: a window of this machine is sent them first, any other screen
    // never, so what an earlier connection showed does not stay.
    useShellStore.getState().clearPairing();
    const ws = bridge
      ? (new BridgeSocket(bridge) as unknown as WebSocket)
      : new WebSocket(`${location.protocol === "https:" ? "wss" : "ws"}://${location.host}/ws`);
    // File bytes arrive as binary frames on this same socket; the header and
    // payload are split by the file-bytes reader, not by the snapshot store.
    ws.binaryType = "arraybuffer";
    socket = ws;
    // Whether this socket has drawn the core's state: a node's held screen
    // sends only where its link stands.
    let drawn = false;
    ws.addEventListener("open", () => {
      useShellStore.getState().releaseOperatorFocus();
      ws.send(
        JSON.stringify({
          token,
          schema_version: 2,
          snapshot_format: "semantic-rest-v1",
          client_kind: hostKind() === "electron" ? "desktop" : "web",
          // Another computer's screen whose window draws this machine's
          // browser displays says so, so an agent here may open one with no
          // desktop window of this machine (docs/BROWSER_DISPLAYS.md).
          ...(bridge?.browser ? { draws_pages: true } : {}),
          have_revision: revision,
          have_core: revisionCore ?? undefined,
          // A client with no cursor names none and has its kept terminals
          // drawn again; a cursor resumes only on the hub its epoch names.
          have_terminal_sequence: terminalSequence ?? undefined,
          have_terminal_epoch: terminalEpoch ?? undefined,
        }),
      );
    });
    ws.addEventListener("message", (event) => {
      if (probing) noteArrival();
      if (event.data instanceof ArrayBuffer) {
        receiveBytes(event.data);
        return;
      }
      const frame = JSON.parse(String(event.data)) as {
        type: string;
        payload?: { revision?: number; terminal_sequence?: number; terminal_epoch?: string };
      };
      // The daemon describes itself before the first snapshot. It is not
      // state: the connection turns live only with the snapshot, because
      // everything keyed to "live" (buffer reconciliation, terminal views)
      // reads the snapshot that has not arrived yet.
      if (frame.type === "daemon") {
        announcedCore = (frame.payload as { core_instance?: string } | undefined)?.core_instance ?? null;
      }
      // A refused kind a window waits on answers that wait; the store notes it either way.
      if (frame.type === "refused") {
        const refused = frame.payload as { kind?: string; reason?: string } | undefined;
        if (refused?.kind) refuseCallsOf(refused.kind, refused.reason ?? "refused");
      }
      if (frame.type === "daemon" || frame.type === "mobile" || frame.type === "pairing" || frame.type === "core_link" || frame.type === "refused") {
        useShellStore.getState().applyFrame(frame);
        return;
      }
      // `hide <path>` brought a project in front: the core already moved
      // there, so the desktop window comes forward and the page shows it.
      if (frame.type === "window_call") {
        hostBridge()?.comeForward?.();
        showProjectOpened(frame.payload as Record<string, unknown> | undefined);
        return;
      }
      if (frame.type === "clipboard") {
        handlers.onCopy(frame.payload as unknown as ProgramCopy);
        return;
      }
      if (frame.type === "agent_notice") {
        receiveAgentNotice(frame.payload as unknown as AgentNotice);
        return;
      }
      if (frame.type === "automation_result") {
        receiveAutomationResult(frame.payload as unknown as { request_id: string; ok: boolean; value?: unknown; code?: string });
        return;
      }
      if (frame.type === "access_result") {
        receiveAccessResult(frame.payload as unknown as { request_id: string; ok: boolean; value?: unknown; code?: string });
        return;
      }
      if (frame.type === "guest_pane_close_result") {
        receiveGuestCloseResult(frame.payload as unknown as { request_id: string; ok: boolean; code?: string });
        return;
      }
      if (frame.type === "file_bytes_error") {
        receiveBytesError(frame.payload as unknown as { request_id: string; reason: string });
        return;
      }
      if (frame.type === "browser_comment") {
        const answer = frame.payload as unknown as {
          workspace: { device_id: string; path: string };
          display_id: string;
          on: boolean;
          reason: string | null;
          pick?: { id: number; anchor: ViewCommentAnchor; at: PagePlace };
          open?: { id: string; at: PagePlace };
          dropped?: boolean;
        };
        const key = browserCommentKey(answer.workspace, answer.display_id);
        noteBrowserComment(key, { on: answer.on, reason: answer.reason });
        if (answer.on && answer.pick) openBrowserNote(key, { kind: "pick", pick: answer.pick.id, anchor: answer.pick.anchor, at: answer.pick.at });
        else if (answer.on && answer.open) openBrowserNote(key, { kind: "open", commentId: answer.open.id, at: answer.open.at });
        // The page left the document its place was picked on.
        else if (answer.dropped) closeBrowserNote(key);
        return;
      }
      if (frame.type === "thread_input_result") {
        receiveThreadInputResult(frame.payload as unknown as NonNullable<import("./snapshot").SessionThreadSection["input"]>);
        return;
      }
      if (frame.type === "attachment_refused") {
        receiveAttachmentRefusal(frame.payload as unknown as { request_id: string; reason: string });
        return;
      }
      // A refused file_bytes path is answered to the viewer that asked, not
      // the Explorer's refusal line.
      if (frame.type === "path_refused" && (frame.payload as { kind?: string } | undefined)?.kind === "file_bytes") {
        const refusal = frame.payload as unknown as { path: string; reason: string };
        receiveBytesRefusal(refusal.path, refusal.reason);
        return;
      }
      const chunks = useShellStore.getState().applyFrame(frame);
      revision = useShellStore.getState().revision;
      if (frame.type === "snapshot" || frame.type === "delta") revisionCore = announcedCore;
      terminalSequence = useShellStore.getState().terminalSequence;
      terminalEpoch = useShellStore.getState().terminalEpoch;
      handlers.onChunks(chunks, frame.type === "snapshot");
      if (frame.type === "snapshot" || frame.type === "delta") {
        drawn = true;
      }
      // A refusal before anything was drawn (a node's held screen refusing
      // an event) leaves the window where it is.
      if (frame.type === "error" && !drawn) return;
      useShellStore.getState().setConnection("live");
      if (frame.type === "snapshot" || frame.type === "delta") reportAccess(drawn);
      backoff = 500;
      healthFails = 0;
    });
    ws.addEventListener("close", (event) => {
      // Only the socket that died fails its own reads; a stale socket's close
      // must not reject a read in flight on its successor.
      if (socket === ws) {
        clearSocketState();
      }
      // A socket a newer one replaced (a new token in the address) ends
      // without a say in the window's state: its refusal is the old token's.
      if (closed || socket !== ws) return;
      if (event.code >= 4001 && event.code <= 4004) {
        useShellStore.getState().setConnection("gone", true);
        return;
      }
      // A tile the desktop hid opens again when it shows.
      if (bridge && event.code === HIDDEN_CLOSE) return;
      void scheduleReconnect(ws);
    });
    ws.addEventListener("error", () => {
      ws.close();
    });
  };

  const scheduleReconnect = async (ended: WebSocket) => {
    useShellStore.getState().setConnection("reconnecting");
    // Another machine's view asks no `/health`: the host is reached only
    // through the bridge, which opens again on the next try or when the
    // tile shows.
    if (bridge) {
      backoff = nextBackoff(backoff);
      reconnectTimer = window.setTimeout(() => {
        if (!hidden && socket === ended) open();
      }, backoff);
      return;
    }
    const health = await readHealth();
    // A new token in the address opened a connection while the health read
    // was out: that one is the window's now, and a second would race it.
    if (closed || socket !== ended) return;
    const ok = health !== null;
    // The daemon may have changed its role while the socket was away.
    const address = health ? addressForRole(window.location.hash, health) : null;
    if (address !== null) history.replaceState(history.state, "", address);
    healthFails = ok ? 0 : healthFails + 1;
    const next = connectionAfterHealthFails(healthFails);
    if (next === "gone") {
      useShellStore.getState().setConnection("gone");
      return;
    }
    backoff = nextBackoff(backoff);
    reconnectTimer = window.setTimeout(open, backoff);
  };

  // A daemon that restarts gives its next run a new token, and the address
  // that carries it is changed under a page that is already open (the host's
  // next attach, a link followed in the same window). The new token is a new
  // connection, from any state the old one ended in: a page the old token's
  // refusal or a long outage left gone is drawn again, without a reload.
  const followToken = () => {
    const token = tokenFromLocation();
    if (closed || !token || token === openedToken) return;
    if (reconnectTimer) window.clearTimeout(reconnectTimer);
    backoff = BACKOFF_START_MS;
    healthFails = 0;
    const replaced = socket;
    // Retire reads owned by the old socket before a successor can start
    // any. Its eventual close must leave the successor's reads alone.
    clearSocketState();
    open();
    replaced?.close();
  };
  window.addEventListener("hashchange", followToken);
  // A tile shown again opens its view at once; a hidden one stays closed.
  bridge?.onVisible((visible) => {
    hidden = !visible;
    if (!visible || closed) return;
    if (socket && socket.readyState !== WebSocket.CLOSED && socket.readyState !== WebSocket.CLOSING) return;
    if (reconnectTimer) window.clearTimeout(reconnectTimer);
    backoff = BACKOFF_START_MS;
    open();
  });

  open();
  configureAttachments({ dispatch, sendBinary });
  return {
    dispatch,
    sendBinary,
    /**
     * Whether the socket is open now, so an event dispatched is sent. Another
     * computer's view whose tile the desktop hid reads `live` with its socket
     * closed until the tile shows again, and an event sent before it reopens
     * is dropped (`dispatch`).
     */
    open: () => socket?.readyState === WebSocket.OPEN,
    /** Closes the socket as a server drop would, so the reconnect path can be exercised (probe seam). */
    drop: () => socket?.close(),
    close: () => {
      closed = true;
      window.removeEventListener("hashchange", followToken);
      if (reconnectTimer) window.clearTimeout(reconnectTimer);
      socket?.close();
    },
  };
}
