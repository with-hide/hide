import type { TFunction } from "i18next";
import { blockedText } from "./deletionGate";
import { fileFailureText } from "./fileFailure";
import type { RemovalNotice, RemovalStop } from "./snapshot";

// The words for how a worktree removal ended or why it stopped. The core and
// the repository's host send codes (`WorktreeRemovalNotice`, `RemovalStop`)
// and keep their sentences in the `worktree_removal` diagnostics; a code this
// build does not know gets the general line.

/** The removal's answer in the viewer's language; `null` before it has one. */
export function removalNoticeText(notice: RemovalNotice | null, phase: string, t: TFunction<"translation">): string {
  if (!notice) return phase === "finished" ? t("workspace.worktreeRemoved") : t("workspace.deleteFailed");
  switch (notice.code) {
    case "removed":
      return notice.branch === "deleted" ? t("removalReason.removed.deleted") : notice.branch === "remains" ? t("removalReason.removed.remains") : t("removalReason.removed.kept");
    case "stopped":
      // A lock's sentence ends on the command to run, so the rest goes first.
      return withTail(notice.stop ? stopText(notice.stop, t) : t("removalReason.other"), notice.panes_closed === true, t, notice.stop?.code === "locked");
    case "host_failed":
      return withTail(notice.reason ? t("removalReason.hostFailed", { reason: fileFailureText({ code: notice.reason }, t) }) : t("removalReason.hostStopped"), notice.panes_closed === true, t);
    case "still_running":
      return t("removalReason.stillRunning", { programs: (notice.programs ?? []).map((program) => `${program.name} (pid ${program.pid})`).join(", ") });
    case "pane_appeared":
      return t("removalReason.paneAppeared");
    case "changed_while_closing":
      return t("removalReason.changedWhileClosing");
    case "close_failed":
      return t("removalReason.closeFailed");
    case "result_unknown":
      return t("removalReason.resultUnknown");
    case "worker_unavailable":
      return t("removalReason.workerUnavailable");
    default:
      return phase === "finished" ? t("workspace.worktreeRemoved") : t("workspace.deleteFailed");
  }
}

function withTail(text: string, panesClosed: boolean, t: TFunction<"translation">, tailFirst = false): string {
  const tail = t(panesClosed ? "removalReason.tail.panesClosed" : "removalReason.tail.nothingClosed");
  return tailFirst ? `${tail} ${text}` : `${text} ${tail}`;
}

function stopText(stop: RemovalStop, t: TFunction<"translation">): string {
  switch (stop.code) {
    case "registration_unreadable":
      return t("removalReason.stop.registrationUnreadable");
    case "not_registered":
      return t("removalReason.stop.notRegistered");
    case "main":
    case "locked":
      return blockedText(stop, t);
    case "identity_changed":
      return t("removalReason.stop.identityChanged");
    case "holds_base":
      return t("removalReason.stop.holdsBase");
    case "nested":
      return t("removalReason.stop.nested");
    case "state_unreadable":
      return t("removalReason.stop.stateUnreadable");
    case "changed":
      return t("removalReason.stop.changed", { count: stop.files ?? 0 });
    case "ignored_repositories":
      return t("removalReason.stop.ignoredRepositories", { names: (stop.names ?? []).join(", ") });
    case "ignored_list_changed":
      return t("removalReason.stop.ignoredListChanged");
    case "not_moved_back":
      return t("removalReason.stop.notMovedBack", { path: stop.path ?? "" });
    case "git_refused":
      return t("removalReason.stop.gitRefused");
    case "still_there":
      return t("removalReason.stop.stillThere");
    default:
      return t("removalReason.other");
  }
}
