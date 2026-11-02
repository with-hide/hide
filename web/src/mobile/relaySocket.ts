import { NoiseSession, encodeClient, decodeServer } from "../../../hide-relay-protocol/web";
import { keyBytes, relayReady, RELAY_URL } from "./relayRuntime";

/** The application's socket boundary, shared by direct WebSocket and encrypted relay. */
export interface PhoneSocket {
  readyState: number;
  onopen: ((event: Event) => unknown) | null;
  onmessage: ((event: MessageEvent) => unknown) | null;
  onclose: ((event: CloseEvent) => unknown) | null;
  send(text: string): void;
  close(): void;
}

/** One socket, one Noise session, no retained send queue. Closing owns every resource. */
export class RelaySocket implements PhoneSocket {
  readyState: number = WebSocket.CONNECTING;
  onopen: PhoneSocket["onopen"] = null;
  onmessage: PhoneSocket["onmessage"] = null;
  onclose: PhoneSocket["onclose"] = null;
  private ws: WebSocket | null = null;
  private noise: NoiseSession | null = null;
  private handshaking = true;
  private timer: ReturnType<typeof setInterval> | null = null;
  private openedAt = performance.now();

  constructor(private host: { room: string; noiseKey: string }, private version: (protocol: number) => boolean,
    private refused: (reason: string) => void) {
    void relayReady().then(() => this.open()).catch(() => this.fail("crypto_unavailable"));
  }

  private open(): void {
    if (this.readyState === WebSocket.CLOSED) return;
    const ws = new WebSocket(RELAY_URL);
    this.ws = ws;
    ws.binaryType = "arraybuffer";
    ws.onopen = () => ws.send(encodeClient({ type: "phone", version: 1, room: this.host.room }));
    ws.onmessage = (event) => {
      try {
        if (typeof event.data === "string") {
          const control = decodeServer(event.data);
          if (control.type === "refused") return this.fail(control.reason);
          if (control.type === "peer_left") return this.close();
          if (control.type !== "joined" || this.noise) throw new Error("invalid_control");
          if (!this.version(control.phone_protocol)) return this.close();
          this.noise = NoiseSession.initiator(keyBytes(this.host.noiseKey), control.connection);
          ws.send(this.noise.write_handshake());
          return;
        }
        if (!(event.data instanceof ArrayBuffer) || !this.noise) throw new Error("invalid_frame");
        const frame = new Uint8Array(event.data);
        if (this.handshaking) {
          this.noise.read_handshake(frame);
          this.handshaking = false;
          this.readyState = WebSocket.OPEN;
          this.onopen?.(new Event("open"));
        } else {
          const message = this.noise.recv_message(frame, BigInt(Math.floor(performance.now())));
          if (message) this.onmessage?.(new MessageEvent("message", { data: new TextDecoder("utf-8", { fatal: true }).decode(message) }));
        }
      } catch { this.fail("invalid_frame"); }
    };
    ws.onerror = () => this.close();
    ws.onclose = () => this.close();
    // Includes a silent partial chunk; receiving more bytes never extends its crate-owned deadline.
    this.timer = setInterval(() => {
      try {
        if (this.handshaking && performance.now() - this.openedAt >= 15_000) return this.fail("timeout");
        this.noise?.check_timeout(BigInt(Math.floor(performance.now())));
      } catch { this.fail("invalid_frame"); }
    }, 1000);
  }

  send(text: string): void {
    if (!this.noise || !this.ws || this.readyState !== WebSocket.OPEN) throw new Error("relay_closed");
    try {
      if (this.ws.bufferedAmount > 1_048_576) throw new Error("send_limit");
      for (const frame of this.noise.send_message(new TextEncoder().encode(text))) this.ws.send(frame);
    } catch (error) { this.close(); throw error; }
  }

  private fail(reason: string): void {
    try { this.refused(reason); } finally { this.close(); }
  }

  close(): void {
    if (this.readyState === WebSocket.CLOSED) return;
    this.readyState = WebSocket.CLOSED;
    if (this.timer !== null) clearInterval(this.timer);
    this.timer = null;
    if (this.ws) {
      this.ws.onopen = this.ws.onmessage = this.ws.onclose = this.ws.onerror = null;
      this.ws.close();
      this.ws = null;
    }
    this.noise?.close();
    this.noise?.free();
    this.noise = null;
    this.onclose?.(new CloseEvent("close"));
  }
}
