import type { MessageKey } from "./i18n/catalogs";
import type { CodexDaemonOff, PaneConnection, PaneConnectionReason, PaneReopenFailure } from "./snapshot";

// The pane header's "Not connected" chip and its popover (PRD settings-cleanup
// D-11, D-12, B26 to B31). The core judges the connection and says why with a
// code (docs/status-model.md, Not connected, and what fixes it); this module
// only names what each code asks of the operator, so the popover can never
// offer a Reopen the core would refuse.

/** The connection of a pane that wears the chip, or null: connected panes and panes with nothing to judge carry none. */
export function notConnected(connection: PaneConnection | null | undefined): (PaneConnection & { reason: PaneConnectionReason }) | null {
  if (!connection || connection.connected || connection.reason === null) return null;
  return { ...connection, reason: connection.reason };
}

export type ConnectionCopy = {
  title: MessageKey;
  reason: MessageKey;
  /** The Reopen button's label; null where Reopen would change nothing. */
  reopen: MessageKey | null;
};

const COPY: Record<PaneConnectionReason, ConnectionCopy> = {
  codex_shared_server: {
    title: "panes.connection.codexSharedServer.title",
    reason: "panes.connection.codexSharedServer.reason",
    reopen: "panes.connection.codexSharedServer.reopen",
  },
  started_before_hide: {
    title: "panes.connection.startedBeforeHide.title",
    reason: "panes.connection.startedBeforeHide.reason",
    reopen: "panes.connection.startedBeforeHide.reopen",
  },
  setup_needed: {
    title: "panes.connection.setupNeeded.title",
    reason: "panes.connection.setupNeeded.reason",
    reopen: null,
  },
};

export function connectionCopy(reason: PaneConnectionReason): ConnectionCopy {
  return COPY[reason];
}

/** Whether the popover offers Reopen: the core says the pane can be reopened and the reason is one Reopen fixes. */
export function offersReopen(connection: PaneConnection & { reason: PaneConnectionReason }): boolean {
  return connection.can_reopen && COPY[connection.reason].reopen !== null;
}

/** Whether the popover offers the shared-server link: only the reason it fixes. */
export function offersSharedServerOff(connection: PaneConnection & { reason: PaneConnectionReason }): boolean {
  return connection.reason === "codex_shared_server";
}

const REOPEN_FAILURE: Record<PaneReopenFailure, MessageKey> = {
  agent_busy: "panes.connection.reopenFailed.agentBusy",
  session_gone: "panes.connection.reopenFailed.sessionGone",
  codex_unread: "agents.codexUnread",
  start_refused: "panes.connection.reopenFailed.startRefused",
  end_refused: "panes.connection.reopenFailed.endRefused",
};

/** The one line a refused Reopen leaves in the popover, or null while none is. A code this build does not know reads as the generic refusal. */
export function reopenFailureKey(connection: PaneConnection): MessageKey | null {
  const reopen = connection.reopen;
  if (reopen?.state !== "failed") return null;
  return REOPEN_FAILURE[reopen.reason] ?? "panes.connection.reopenFailed.generic";
}

export function reopenPending(connection: PaneConnection): boolean {
  return connection.reopen?.state === "pending";
}

const DAEMON_OFF_FAILURE: Record<string, MessageKey> = {
  codex_missing: "panes.connection.sharedServerFailed.codexMissing",
  codex_refused: "panes.connection.sharedServerFailed.codexRefused",
  timed_out: "panes.connection.sharedServerFailed.timedOut",
  unreachable: "panes.connection.sharedServerFailed.unreachable",
  stop_failed: "panes.connection.sharedServerFailed.stopFailed",
};

export type SharedServerOutcome = { phase: "pending" } | { phase: "done" } | { phase: "failed"; key: MessageKey };

/**
 * What the popover says about the last turn-off of the machine's shared
 * server. A finished answer is shown only to the operator who asked in this
 * popover (`asked`): the core keeps the last answer, and a pane that reads
 * not connected later must not open on an old "done".
 */
export function sharedServerOutcome(off: CodexDaemonOff | null | undefined, asked: boolean): SharedServerOutcome | null {
  if (!off) return null;
  if (off.state === "pending") return { phase: "pending" };
  if (!asked) return null;
  if (off.state === "done") return { phase: "done" };
  return { phase: "failed", key: DAEMON_OFF_FAILURE[off.reason] ?? "panes.connection.sharedServerFailed.generic" };
}
