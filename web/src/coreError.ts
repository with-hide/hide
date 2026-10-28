// The one wording of a failure the core published as `last_error`. The core
// names what went wrong by `kind`; its `message` is diagnostic detail (Herdr's
// protocol answer, a path, an io error) that the store's diagnostics and the
// core's `error.reported` log keep, and the screen never draws it (design
// principle 13). A kind worded here gets its catalog sentence; any other kind,
// a daemon of another build included, gets the surface's own general sentence,
// and the kind stays in the log and on the element's `data-core-error`.

import type { TFunction } from "i18next";
import type { MessageKey } from "./i18n/catalogs";
import type { SnapshotRest } from "./snapshot";

export type CoreError = NonNullable<NonNullable<NonNullable<SnapshotRest["status"]>["last_error"]>>;

const KEYS = new Map<string, MessageKey>([
  ["workspace.create_failed", "coreError.project.addFailed"],
  ["workspace.create_worker_failed", "coreError.project.addFailed"],
  ["workspace.create_refused", "coreError.project.addRefused"],
  ["workspace.create_refused.not_found", "addProject.refusal.notFound"],
  ["workspace.create_refused.not_a_directory", "addProject.refusal.notADirectory"],
  ["workspace.create_refused.already_exists", "addProject.refusal.alreadyExists"],
  ["workspace.create_refused.outside_root", "coreError.project.outsideRoot"],
  ["workspace.create_refused.project_holds_home", "addProject.refusal.projectHoldsHome"],
  ["workspace.pin_refused.needs_confirmation", "coreError.project.pinNeedsConfirmation"],
  ["workspace.create_local_only", "coreError.project.localOnly"],
  ["workspace.create_unconfirmed", "coreError.project.addUnconfirmed"],
  ["workspace.create_unavailable", "coreError.deviceNotConnected"],
  ["workspace.create_in_flight", "coreError.project.addInFlight"],
  ["workspace.remove_in_flight", "coreError.project.removeInFlight"],
  ["workspace.remove_failed", "coreError.project.removeFailed"],
  ["workspace.remove_cancelled", "coreError.project.removeCancelled"],
  ["workspace.control_unavailable", "coreError.herdrNotConnected"],

  ["repository.clone_busy", "coreError.clone.busy"],
  ["repository.clone_invalid", "coreError.clone.invalid"],

  ["task_operation.busy", "coreError.start.busy"],
  ["task_operation.agent_retry_offline", "coreError.deviceNotConnected"],
  ["task_operation.agent_retry_pane_gone", "coreError.start.paneGone"],
  ["task_operation.agent_retry_unavailable", "coreError.start.retryGone"],
  ["task_operation.worker_unavailable", "taskReason.workerUnavailable"],
  ["task_operation.settings_not_ready", "taskReason.settingsNotReady"],
  ["agent_start.folder_missing", "taskReason.folderMissing"],
  // Why a checkout's purpose was not saved (`task_operation.failure`).
  ["checkout_purpose.git_failed_saved", "taskReason.purpose.gitFailedSaved"],
  ["checkout_purpose.git_failed", "taskReason.purpose.gitFailed"],
  ["checkout_purpose.token_failed", "taskReason.purpose.tokenFailed"],
  ["checkout_purpose.unknown_checkout", "taskReason.purpose.unknownCheckout"],
  ["checkout_purpose.invalid", "taskReason.purpose.invalid"],
  ["checkout_purpose.remote_unsupported", "taskReason.purpose.remoteUnsupported"],
  ["checkout_purpose.no_target", "taskReason.purpose.noTarget"],
  ["agent_start.unknown_model", "coreError.start.unknownModel"],
  ["agent_start.invalid_prompt", "coreError.start.invalidPrompt"],
  ["agent_start.invalid_resume", "coreError.start.invalidResume"],
  ["agent_start.confirmation_unavailable", "coreError.start.receiptUnreadable"],
  ["agent_start.device_unavailable", "coreError.deviceNotConnected"],
  // Why a task's agent did not start (`task_operation.agent_reason`).
  ["agent_start.not_installed", "coreError.agent.notInstalled"],
  ["agent_start.install_check_failed", "coreError.agent.installUnchecked"],
  ["agent_start.shell_starting", "coreError.agent.shellStarting"],
  ["agent_start.shell_not_reading", "coreError.agent.shellNotReading"],
  ["agent_start.shell_no_prompt", "coreError.agent.shellNoPrompt"],
  ["agent_start.shell_unreadable", "coreError.agent.shellUnreadable"],
  ["agent_start.line_unchecked", "coreError.agent.lineUnchecked"],
  ["agent_start.refused_by_herdr", "coreError.agent.refusedByHerdr"],
  ["agent_start.unconfirmed", "coreError.agent.unconfirmed"],
  ["agent_start.not_saved", "coreError.agent.notSaved"],
  ["agent_start.no_start_dialect", "coreError.agent.noStartDialect"],
  ["agent_start.terminal_device_unavailable", "coreError.agent.terminalDeviceUnavailable"],
  ["agent_start.terminal_unchecked", "coreError.agent.terminalUnchecked"],
  ["agent_start.terminal_changed", "coreError.agent.terminalChanged"],
  ["agent_start.terminal_unconfirmed", "coreError.agent.terminalUnconfirmed"],
  ["agent_start.pane_restored", "coreError.agent.paneRestored"],
  ["agent_start.pane_restored_unconfirmed", "coreError.agent.paneRestoredUnconfirmed"],
  ["agent_start.worker_unavailable", "coreError.agent.workerUnavailable"],
  ["agent_trust_unavailable", "coreError.agent.trustUnavailable"],
  ["agent_trust_failed", "coreError.agent.trustFailed"],
  ["overview.unknown_checkout", "coreError.start.checkoutGone"],
  ["remote.control.checkout_unconfirmed", "coreError.start.checkoutUnconfirmed"],

  ["worktree.create_invalid_branch", "coreError.worktree.branchRequired"],
  ["worktree.create_unread", "coreError.worktree.unread"],
  ["worktree.create_without_branches", "coreError.worktree.noBranches"],
  ["worktree.create_unknown_model", "coreError.start.unknownModel"],
  ["worktree.create_invalid_prompt", "coreError.start.invalidPrompt"],
  ["worktree.remove_busy", "coreError.worktree.removeBusy"],
  ["worktree.remove_unknown", "coreError.worktree.gone"],
  ["worktree.remove_unavailable", "coreError.worktree.removeUnavailable"],
  ["worktree.remove_ignored_changed", "coreError.worktree.ignoredChanged"],
  ["worktree.remove_discard_unconfirmed", "coreError.worktree.discardUnconfirmed"],
  ["worktree.remove_blocked", "coreError.worktree.blocked"],
  ["home.conflict", "coreError.home.conflict"],
  ["home.unavailable", "coreError.home.unavailable"],

  ["pr_link.busy", "coreError.prLink.busy"],

  ["agent_sleep.invalid_setting", "coreError.settings.notSaved"],
  ["interface_language.invalid", "coreError.settings.notSaved"],
  ["ui_state.save_failed", "coreError.settings.notSaved"],
  ["ui_state.shortcuts_invalid", "coreError.settings.shortcutsInvalid"],
  ["ui_state.browser_shortcuts_invalid", "coreError.settings.shortcutsInvalid"],
  ["ai_settings.provider_not_selectable", "coreError.settings.aiNotSelectable"],

  ["device.invalid", "coreError.device.invalid"],
  ["device.duplicate", "coreError.device.duplicate"],
  ["device.test_inbound", "coreError.device.inbound"],
  ["remote.retry_inbound", "coreError.device.inbound"],
  ["device.host.inbound", "coreError.device.inboundConsent"],
  ["device.not_connected", "coreError.device.noConnection"],
  ["device.test_in_flight", "coreError.device.testInFlight"],
  ["remote.retry_connected", "coreError.device.alreadyConnected"],
  ["device.profile_unknown", "coreError.device.profileGone"],
  ["kit.unavailable", "coreError.kit.unavailable"],

  ["agent_group.refused", "coreError.group.changed"],
  ["pane.move_pending", "coreError.group.movePending"],
  ["agent_group.start_pending", "coreError.group.startPending"],
  ["agent_group.checkout_missing", "coreError.group.checkoutGone"],
  ["agent_group.workspace_unavailable", "coreError.group.checkoutGone"],
  ["agent_group.reopen_failed", "coreError.group.reopenFailed"],

  ["terminal.input_hold_failed", "coreError.notice.inputLost"],
  ["remote.control.close_status_unknown", "coreError.notice.closeStatusUnknown"],
  ["remote.control.close_confirmation_required", "coreError.notice.closeNeedsConfirm"],
  ["remote.control.not_connected", "coreError.deviceNotConnected"],
  ["remote.control.unavailable", "coreError.deviceNotConnected"],
  ["pane_reopen.not_restarted", "coreError.notice.reopenNotRestarted"],
  // Why a close of an agent with its children closed nothing, or not all of it.
  ["tree_close.capacity", "coreError.close.capacity"],
  ["tree_close.busy", "coreError.close.busy"],
  ["tree_close.too_many", "coreError.close.tooMany"],
  ["tree_close.unresolved_close", "coreError.close.unresolved"],
  ["tree_close.incomplete", "coreError.close.incomplete"],
  ["tree_close.status_unknown", "coreError.notice.closeStatusUnknown"],
  ["tree_close.confirmation_required", "coreError.notice.closeNeedsConfirm"],

  ["view_layout.limit", "coreError.view.limit"],
  ["view_layout.unsaved", "coreError.view.unsaved"],
  ["view_layout.waiting", "coreError.view.waiting"],
  ["view_layout.display_limit", "coreError.view.displayLimit"],
  ["view_layout.nothing_to_split", "coreError.view.nothingToSplit"],
  ["agent_layout.display_limit", "coreError.view.agentTabLimit"],
]);

/** The catalog sentence for `kind`, or null when this build does not word it. */
export function coreErrorKey(kind: string): MessageKey | null {
  return KEYS.get(kind) ?? null;
}

/** `error` as the screen says it: its kind's sentence, else `fallback`, the surface's general sentence. */
export function coreErrorText(error: { kind: string }, t: TFunction<"translation">, fallback: MessageKey = "coreError.other"): string {
  return t(coreErrorKey(error.kind) ?? fallback);
}

const HERDR_STATES = new Map<string, MessageKey>([
  ["socket_missing", "coreError.herdr.socket_missing"],
  ["unreachable", "coreError.herdr.unreachable"],
  ["stale", "coreError.herdr.stale"],
  ["malformed", "coreError.herdr.malformed"],
  ["unconfigured", "coreError.herdr.unconfigured"],
]);

/** Why this machine's Herdr does not answer, from `status.herdr.state`; its `message` is the About tab's and the log's. */
export function herdrStateText(state: string, t: TFunction<"translation">): string {
  return t(HERDR_STATES.get(state) ?? "coreError.herdr.other");
}

const REMOTE_STATES = new Map<string, MessageKey>([
  ["socket_missing", "coreError.remote.socket_missing"],
  ["unreachable", "coreError.remote.unreachable"],
  ["protocol_mismatch", "coreError.remote.protocol_mismatch"],
  ["malformed", "coreError.remote.malformed"],
  ["disabled", "coreError.remote.disabled"],
]);

/** Why a device is not connected, from `RemoteStatus.state`; `device` is its name as this window calls it. */
export function remoteStateText(state: string, device: string, t: TFunction<"translation">): string {
  return t(REMOTE_STATES.get(state) ?? "coreError.remote.other", { device });
}

const CLONE_FAILURES = new Map<string, MessageKey>([
  ["authentication", "coreError.clone.failure.authentication"],
  ["unknown_host_key", "coreError.clone.failure.unknown_host_key"],
  ["not_found", "coreError.clone.failure.not_found"],
  ["unreachable_host", "coreError.clone.failure.unreachable_host"],
  ["stalled", "coreError.clone.failure.stalled"],
  ["target_exists", "coreError.clone.failure.target_exists"],
  ["io", "coreError.clone.failure.io"],
]);

/** A failed clone's `failure` code as the screen says it; a code this build does not word is the plain "the clone failed". */
export function cloneFailureText(failure: string | null, t: TFunction<"translation">): string {
  return t((failure ? CLONE_FAILURES.get(failure) : undefined) ?? "addProject.clone.failed");
}

/** Git's own stage names, which a clone's progress carries as Git printed them. */
const CLONE_STAGES = new Map<string, MessageKey>([
  ["Enumerating objects", "coreError.clone.stage.enumerating"],
  ["Counting objects", "coreError.clone.stage.counting"],
  ["Compressing objects", "coreError.clone.stage.compressing"],
  ["Receiving objects", "coreError.clone.stage.receiving"],
  ["Resolving deltas", "coreError.clone.stage.resolving"],
  ["Updating files", "coreError.clone.stage.updating"],
]);

/** A clone's Git stage in the viewer's language; a stage Git names that is not listed is the plain "cloning". */
export function cloneStageText(stage: string, t: TFunction<"translation">): string {
  return t(CLONE_STAGES.get(stage) ?? "addProject.clone.stageFallback");
}
