import type { TFunction } from "i18next";
import type { BranchLoss, DeletionBlock, DeletionDiscard, DeletionGate, DeletionWarning } from "./snapshot";

// The Delete worktree confirmation's words for what the core's deletion gate
// decided. The gate sends codes and counts; a code this build does not know
// gets its group's general line, and the code stays on the element's `data-`.

const UNLOCK = "git worktree unlock";

export function blockedText(blocked: DeletionBlock, t: TFunction<"translation">): string {
  switch (blocked.code) {
    case "main":
      return t("deletionGate.blocked.main");
    case "locked":
      return blocked.reason
        ? t("deletionGate.blocked.locked", { name: blocked.name ?? "", reason: blocked.reason, command: UNLOCK })
        : t("deletionGate.blocked.lockedNoReason", { name: blocked.name ?? "", command: UNLOCK });
    case "ignored_scan_unavailable":
      return t("deletionGate.blocked.ignoredScanUnavailable");
    default:
      return t("deletionGate.blocked.other");
  }
}

export function warningText(warning: DeletionWarning, t: TFunction<"translation">): string {
  switch (warning.code) {
    case "uncommitted":
      return warning.files ? t("deletionGate.warning.uncommitted", { count: warning.files }) : t("deletionGate.warning.uncommittedUncounted");
    case "nested":
      return t("deletionGate.warning.nested");
    case "base":
      return t("deletionGate.warning.base");
    case "status_unknown":
      return t("deletionGate.warning.statusUnknown");
    case "unmerged":
      return t("deletionGate.warning.unmerged", { count: warning.ahead ?? 0 });
    case "not_pushed":
      return t("deletionGate.warning.notPushed");
    case "ignored_repositories":
      return t("deletionGate.warning.ignoredRepositories", { count: warning.count ?? 0 });
    default:
      return t("deletionGate.warning.other");
  }
}

export function branchLossText(loss: BranchLoss, t: TFunction<"translation">): string {
  switch (loss.code) {
    case "commits":
      return loss.base ? t("deletionGate.branch.commits", { count: loss.count ?? 0, base: loss.base }) : t("deletionGate.branch.commitsNoBase", { count: loss.count ?? 0 });
    case "not_merged":
      return t("deletionGate.branch.notMerged");
    case "unknown":
      return t("deletionGate.branch.unknown");
    default:
      return t("deletionGate.branch.other");
  }
}

/** The Discard checkbox's label: every loss the folder would take, in one line. */
export function discardText(discard: DeletionDiscard, t: TFunction<"translation">): string {
  const items: string[] = [];
  if (discard.changed_files !== null) items.push(discard.changed_files > 0 ? t("deletionGate.discard.files", { count: discard.changed_files }) : t("deletionGate.discard.filesUncounted"));
  if (discard.nested) items.push(t("deletionGate.discard.nested"));
  if (discard.status_unknown) items.push(t("deletionGate.discard.statusUnknown"));
  if (discard.ignored_repositories.length > 0) items.push(t("deletionGate.discard.ignored", { names: discard.ignored_repositories.join(", ") }));
  return t("deletionGate.discard", { items: items.join(t("deletionGate.discard.separator")) });
}

/** The confirm button: the panes it closes first, or the plain Delete worktree. */
export function deleteButtonText(gate: DeletionGate, t: TFunction<"translation">): string {
  return gate.pane_count > 0 ? t("deletionGate.button.closePanes", { count: gate.pane_count }) : t("workspace.deleteWorktree");
}
