// The measurement and e2e seam over the terminal pane.
//
// xterm.js draws through WebGL, so neither Playwright nor a CDP driver can
// read the screen from the DOM. When the page URL carries `probe=1` this
// installs `window.__hideProbe`, which reads the parsed buffer and resolves a
// promise at the xterm write completion that first shows an armed marker
// (the S0 echo definition: t1 = write callback with the marker in the parsed
// buffer). Without the query nothing is installed and the writer path is the
// plain `term.write(data)`.

import type { Terminal } from "@xterm/xterm";
import type { InputRequest } from "./snapshot";
import { useShellStore } from "./store";
import { keyboardCommandOwner, type KeyboardOwner } from "./viewFocus";

export type EchoSample = { arrival_ms: number; write_ms: number };

export type Probe = {
  /** Bounded existing diagnostic log, including opt-in terminal path probe counts. */
  diagnostics: () => string[];
  /** The recorded command target, including native pages outside the shell DOM. */
  commandOwner: () => KeyboardOwner;
  paneId: () => string | null;
  screenText: () => string;
  /** The colors the focused terminal draws with (background, foreground and the ANSI 16), null without one. */
  screenColors: () => Record<string, string> | null;
  /** Arms one marker; `waitArmed` resolves at the first write completion whose screen contains it. */
  arm: (marker: string, timeoutMs?: number) => void;
  waitArmed: () => Promise<EchoSample>;
  /** WebSocket frames received since the page loaded. */
  arrivals: () => number;
  /** Closes the live socket the way a server drop would; the shell reconnects on its own. */
  dropSocket: () => void;
  /** The key-holding requests the core lists now (`terminal.input_requests`). */
  inputRequests: () => InputRequest[];
  /** Pane ids whose terminal session the core reports as not released (the attach window). */
  attachedPanes: () => string[];
  /** Pane ids that hold an xterm instance right now, shown or parked (D-05). */
  liveTerminals: () => string[];
  /** The parsed buffer of any pane's instance, empty when it has none. */
  paneText: (paneId: string) => string;
  /** The grid an instance currently has, or null when the pane has none. */
  paneGrid: (paneId: string) => { cols: number; rows: number } | null;
  /** The text a copy of the pane's drag selection puts on the clipboard, null without one. */
  paneSelection: (paneId: string) => string | null;
};

declare global {
  interface Window {
    __hideProbe?: Probe;
  }
}

export const epochMs = () => performance.timeOrigin + performance.now();

let armed: {
  marker: string;
  resolve: (sample: EchoSample) => void;
  timer: ReturnType<typeof setTimeout>;
} | null = null;
let armedPromise: Promise<EchoSample> | null = null;
let lastArrivalMs = 0;
let arrivals = 0;

export function probeEnabled(search: string = window.location.search): boolean {
  return new URLSearchParams(search).get("probe") === "1";
}

export function screenText(term: Terminal): string {
  const buffer = term.buffer.active;
  const lines: string[] = [];
  for (let i = 0; i < buffer.length; i += 1) {
    lines.push(buffer.getLine(i)?.translateToString(true) ?? "");
  }
  return lines.join("\n");
}

/** Called at WebSocket message entry, before JSON parsing. */
export function noteArrival(): void {
  lastArrivalMs = epochMs();
  arrivals += 1;
}

/** Called from the terminal writer's write-completion callback. */
export function noteWriteComplete(term: Terminal): void {
  if (!armed) return;
  const write_ms = epochMs();
  if (!screenText(term).includes(armed.marker)) return;
  clearTimeout(armed.timer);
  armed.resolve({ arrival_ms: lastArrivalMs, write_ms });
  armed = null;
}

export function installProbe(
  term: () => Terminal | null,
  paneId: () => string | null,
  dropSocket: () => void = () => {},
  attachedPanes: () => string[] = () => [],
  liveTerminals: () => string[] = () => [],
  terminalOf: (paneId: string) => Terminal | null = () => null,
  selectionOf: (paneId: string) => string | null = () => null,
): void {
  window.__hideProbe = {
    diagnostics: () => useShellStore.getState().diagnostics,
    commandOwner: keyboardCommandOwner,
    inputRequests: () => useShellStore.getState().rest?.terminal?.input_requests ?? [],
    paneId,
    dropSocket,
    attachedPanes,
    liveTerminals,
    paneText: (id) => {
      const current = terminalOf(id);
      return current ? screenText(current) : "";
    },
    paneGrid: (id) => {
      const current = terminalOf(id);
      return current ? { cols: current.cols, rows: current.rows } : null;
    },
    paneSelection: selectionOf,
    screenText: () => {
      const current = term();
      return current ? screenText(current) : "";
    },
    screenColors: () => {
      const theme = term()?.options.theme;
      return theme ? Object.fromEntries(Object.entries(theme).filter((entry): entry is [string, string] => typeof entry[1] === "string")) : null;
    },
    arm: (marker, timeoutMs = 3000) => {
      if (armed) throw new Error("echo sample already armed");
      armedPromise = new Promise<EchoSample>((resolve, reject) => {
        const timer = setTimeout(() => {
          armed = null;
          reject(new Error(`echo timeout: ${marker}`));
        }, timeoutMs);
        armed = { marker, resolve, timer };
      });
      // A timeout is still retrieved through waitArmed when the driver is late.
      void armedPromise.catch(() => {});
    },
    waitArmed: () => {
      if (!armedPromise) return Promise.reject(new Error("echo sample not armed"));
      return armedPromise;
    },
    arrivals: () => arrivals,
  };
}
