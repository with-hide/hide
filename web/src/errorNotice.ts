import type { SnapshotRest } from "./snapshot";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";
import { viewRefusal } from "./viewLayout";
import { archiveNotice } from "./archive";

type Shell = { rest: SnapshotRest | null };

/** The failures of an open the keyboard may be waiting on. */
const OPEN_FAILURES: ReadonlySet<string> = new Set(["file.open_failed", "browser.open_refused"]);

/** The core's `last_error` when it is new in `state` (a different occurrence than `previous` carried), else null. */
export function freshError(state: Shell, previous: Shell) {
  const error = state.rest?.status?.last_error;
  return error && error.occurred_at !== previous.rest?.status?.last_error?.occurred_at ? error : null;
}

/**
 * What the operator is told from a failure the core published, and kept until they dismiss it or leave the
 * device it was about. The core clears `last_error` at its next event of any kind, so a line that
 * only read it would vanish before it could be read; a notice is the shell's own and stays.
 * Only a failure the operator can act on is one (design principle 13):
 * - a command a remote host refused (a lost connection, a close that needs confirming);
 * - a View action or open the core refused (S7 B19);
 * - a Reopen that ended the agent and could not start it again, which the operator answers by Reopening
 *   again: nothing is left on the pane to carry it, since the Not connected chip is read from the agent's row.
 */
export function errorNotice(error: { kind: string; message: string } | null | undefined): { text: string; refreshable: boolean } | null {
  if (!error) return null;
  if (error.kind === "terminal.input_hold_failed") return { text: error.message, refreshable: false };
  if (error.kind.startsWith("remote.control.")) return { text: error.message, refreshable: error.kind === "remote.control.close_status_unknown" };
  const refusal = viewRefusal(error);
  if (refusal) return { text: refusal, refreshable: false };
  if (error.kind.startsWith("pane_reopen.")) return { text: error.message, refreshable: false };
  // An archive that closed, saved or deleted nothing says why in one line (PRD session-archive B10, B12, B30).
  const archive = archiveNotice(error.kind);
  if (archive) return { text: archive, refreshable: error.kind === "archive.status_unknown" };
  return null;
}

/** Raises the notice for each new failure the core publishes, and drops one that was about another device. */
export function watchErrorNotices(): () => void {
  return useShellStore.subscribe((state, previous) => {
    if (state.rest === previous.rest) return;
    // A notice about one device's command does not outlive the device context it was about.
    if (state.rest?.navigator?.focused_device_id !== previous.rest?.navigator?.focused_device_id) {
      useUiStore.getState().setNotice(null);
    }
    const fresh = freshError(state, previous);
    const notice = errorNotice(fresh);
    if (notice) useUiStore.getState().setNotice(notice);
    // A refused View action, a file whose read failed or a refused page showed nothing, so the keyboard stays
    // where it is rather than waiting for a move that will not land and taking a later, unrelated one.
    if (viewRefusal(fresh) || OPEN_FAILURES.has(fresh?.kind ?? "")) useUiStore.getState().setViewFocusRequest(null);
  });
}
