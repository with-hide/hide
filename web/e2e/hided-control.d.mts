// Types for hided-control.mjs, which stays plain JavaScript because the web
// shell measurement runs it with Node.

/** The snapshot's `rest`, as the frames so far have built it. */
export type Rest = Record<string, unknown>;

export type HidedControl = {
  /** The daemon's `path_refused` answers, oldest first; each also wakes a waiting `until`. */
  refusals: Record<string, unknown>[];
  /** Sends one client event, as `web/src/actions.ts` dispatches it. */
  send: (kind: string, payload: Record<string, unknown>) => void;
  /**
   * Resolves with `read(rest)` once it answers anything but undefined, null
   * or false for the snapshot as the frames have built it, checking each
   * frame as it lands; rejects with the error a `read` throws, or naming
   * `what` after `ms`.
   */
  until: <T>(read: (rest: Rest) => T | undefined | null | false, what: string, ms?: number) => Promise<T>;
  /**
   * Closes the connection and resolves once the daemon has closed it, which
   * it does only after applying every event sent on it; rejects naming the
   * connection and the latest events sent on it when that does not happen
   * within the connection's bound.
   */
  close: () => Promise<void>;
};

export function connectControl(daemon: { origin: string; token: string; clientKind?: "web" }, ms?: number, terminalEvidence?: (chunks: unknown[]) => void): Promise<HidedControl>;
export function connectStateDir(stateDir: string, ms?: number): Promise<HidedControl>;
export function useControl<T>(control: HidedControl, use: (control: HidedControl) => Promise<T>): Promise<T>;
