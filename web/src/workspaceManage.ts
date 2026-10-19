import type { MenuPresentation } from "./components/menu-symbols";
// The pure rules behind the sidebar's row menus: which actions a project,
// checkout or agent row offers and why one is disabled, the purpose field's
// limits, and what the worktree dialogs read out of a task or removal the
// core reports. The core re-checks every one of these; they decide what the
// screen offers, not what is allowed.

import type { TFunction } from "i18next";
import { statusText } from "./agentStatus";
import type { MessageKey } from "./i18n/catalogs";
import { shownPullRequest } from "./projects";
import { supportsRemotePurpose } from "./remote";
import { revealExternalEntry, type RevealHost } from "./revealExternal";
import { issueSourceChoices } from "./settings";
import type { AgentRow, Checkout, RemoteStatus, TaskOperation, Workspace, WorktreeRemoval } from "./snapshot";

/** The native purpose field's limits (`PurposeInputPresentation`). */
export const PURPOSE_RECOMMENDED = 40;
export const PURPOSE_HARD_LIMIT = 80;

/** Unicode scalars, which is what the core counts (`chars()`), not UTF-16 units. */
export function scalarCount(text: string): number {
  return [...text].length;
}

/** One line, at most the hard limit of scalars; the field shows what will be saved. */
export function normalizePurpose(text: string): string {
  return [...text.replace(/[\r\n]/g, " ")].slice(0, PURPOSE_HARD_LIMIT).join("");
}

export function purposeCountLabel(text: string, t: TFunction<"translation">): string {
  return t("workspace.purposeCount", { count: scalarCount(text), limit: PURPOSE_RECOMMENDED });
}

/**
 * Where a saved purpose is kept (PRD S5.5 B30). A device's purpose is its
 * Herdr's workspace metadata only; on this machine a branch also keeps it as
 * its Git description, which outlives the Herdr workspace.
 */
export function purposeScope(deviceLabel: string | null, branch: string | null, t: TFunction<"translation">): string {
  if (deviceLabel) return t("workspace.purposeScopeRemote", { device: deviceLabel });
  if (branch) return t("workspace.purposeScopeBranch", { branch });
  return t("workspace.purposeScopeLocal");
}

export function purposeIsLong(text: string): boolean {
  return scalarCount(text) > PURPOSE_RECOMMENDED;
}

/**
 * A branch name the dialog can send. Git's own `check-ref-format` runs on the
 * daemon before anything is created; this catches the obvious cases while
 * the operator types, so the field says why before a round trip.
 */
export function branchProblem(name: string): MessageKey | null {
  const branch = name.trim();
  if (!branch) return "workspace.branchRequired";
  if (branch.startsWith("-")) return "workspace.branchDash";
  const control = [...branch].some((character) => character.charCodeAt(0) < 0x20 || character.charCodeAt(0) === 0x7f);
  if (control || /[\s~^:?*[\\]|\.\.|@\{|\/\/|\.lock$|^\/|\/$|\.$/.test(branch)) {
    return "workspace.branchInvalid";
  }
  return null;
}

export type MenuItem = {
  id:
    | "new_worktree"
    | "project_settings"
    | "new_tab_primary"
    | "reveal_external"
    | "open_in_editor"
    | "copy_path"
    | "issue_source"
    | "issue_source_auto"
    | "issue_source_github"
    | "issue_source_local"
    | "pin"
    | "unpin"
    | "remove_project"
    | "open_checkout"
    | "new_tab_here"
    | "new_group"
    | "open_pull_request"
    | "set_purpose"
    | "set_primary"
    | "copy_branch"
    | "delete_worktree";
  label: string;
  /** Why the action is not offered here; the item is drawn disabled with this as its hint. */
  unavailable: string | null;
  /** Drawn after a separator: where a menu turns to another group of actions. */
  separated?: boolean;
  /** The chord that does the same, drawn at the item's end. */
  shortcut?: string;
  /** An action that removes something from disk, drawn in the destructive color. */
  destructive?: boolean;
  /** A choice of one: the item opens a submenu of these, the current one checked. */
  choices?: { id: MenuItem["id"]; label: string; checked: boolean }[];
};

/** What a row's menu reads from where the shell runs, so the rules stay pure. */
export type MenuHost = {
  /** The desktop app can show a folder in the OS file manager, under this label; a browser tab cannot, so it offers no such item. */
  reveal: RevealHost;
  /** The new-tab chord on this host, or "" where it has none. */
  newTabChord: string;
  /** The core's own node id, whose checkouts take a primary. */
  node: string;
  /** The device this screen runs on, whose files the OS file manager shows (`screenMachine.ts`). */
  screen: string;
  /** The desktop app found VS Code on this computer; a browser tab, or a computer without it, offers no Open in VS Code (D-14). */
  editor: boolean;
};

/** The device a receipt names; the core's own node when it names none. */
function receiptDevice(deviceId: string | null | undefined, node: string): string {
  return deviceId ?? node;
}

/**
 * The checkout "New tab in main" opens a tab in: the one the home glyph
 * marks, or a folder project's own checkout, which has no primary.
 */
export function primaryCheckout(workspace: Workspace): Checkout | null {
  return workspace.checkouts.find((checkout) => checkout.is_primary === true) ?? (workspace.is_git ? null : (workspace.checkouts[0] ?? null));
}

function revealItem(workspace: Workspace, host: MenuHost, t: TFunction<"translation">, separated: boolean): MenuItem[] {
  return revealExternalEntry(host.reveal, workspace.device_id, host.screen, t, null, separated);
}

/**
 * Open in VS Code, for a checkout or a plain folder (PRD sidebar-titlebar-refresh
 * D-13, D-14): offered only where the desktop app found VS Code, and disabled
 * with Reveal's reason on another device's checkout, whose folder is not here.
 */
function editorItem(workspace: Workspace, host: MenuHost, t: TFunction<"translation">, separated: boolean): MenuItem[] {
  if (!host.editor) return [];
  const unavailable = workspace.device_id !== host.screen ? t("explorer.revealHereOnly") : null;
  return [{ id: "open_in_editor", label: t("workspace.menu.openInVsCode"), unavailable, ...(separated ? { separated } : {}) }];
}

/**
 * The project row's menu on any device, registered or not (PRD
 * sidebar-context-menus D-02, D-14): open the project, start work in it, find
 * its folder, then keep or drop it. A row Herdr shows without a registration
 * offers the same Pin and Remove: Pin registers it with its device and root
 * and pins it in the same event, and Remove closes its panes, after which the
 * row leaves with Herdr's workspace because there is no registration to keep it.
 */
export function projectMenu(workspace: Workspace, host: MenuHost, t: TFunction<"translation">, storedIssueSource?: string, folder = false): MenuItem[] {
  const primary = primaryCheckout(workspace);
  // A plain folder is its own checkout, so its row opens it in VS Code too (D-13).
  const editor = folder ? editorItem(workspace, host, t, true) : [];
  const reveal = revealItem(workspace, host, t, editor.length === 0);
  return [
    ...(!workspace.is_home ? [{ id: "project_settings" as const, label: t("projectSettings.title"), unavailable: null }] : []),
    { id: "new_worktree", label: t("workspace.menu.newWorktree"), unavailable: workspace.is_git === false ? t("workspace.unavailable.notGit") : null },
    {
      id: "new_tab_primary",
      label: t("workspace.menu.newTabMain"),
      unavailable: !primary ? t("workspace.unavailable.noDefault") : primary.exists ? null : t("workspace.unavailable.defaultMissing"),
      shortcut: host.newTabChord,
    },
    ...editor,
    ...reveal,
    { id: "copy_path", label: t("workspace.menu.copyPath"), unavailable: null, ...(reveal.length || editor.length ? {} : { separated: true }) },
    ...issueSourceItem(workspace, storedIssueSource, t),
    { id: workspace.pinned ? "unpin" : "pin", label: workspace.pinned ? t("workspace.menu.unpin") : t("workspace.menu.pin"), unavailable: null, separated: true },
    { id: "remove_project", label: t("workspace.menu.removeProject"), unavailable: null },
  ];
}

/**
 * Where this project's issues live (PRD settings-cleanup D-04): a submenu of
 * Automatic, GitHub (a Git project only) and Local with the one in force
 * checked. A project the issue boards do not draw for (a device's, the Home)
 * offers none.
 */
function issueSourceItem(workspace: Workspace, stored: string | undefined, t: TFunction<"translation">): MenuItem[] {
  if (workspace.is_home || workspace.tasks?.source == null) return [];
  const { value, options } = issueSourceChoices(workspace, stored, t);
  return [
    {
      id: "issue_source",
      label: t("issueSettings.sourceMenu"),
      unavailable: null,
      choices: options.map((option) => ({ id: `issue_source_${option.id}` as const, label: option.label, checked: option.id === value })),
    },
  ];
}

/**
 * What removing a project does, as the short facts its confirmation shows on
 * one line (PRD close-agent-subtree D-42): the panes that close, the agents
 * that stop, and what stays on disk.
 */
export function projectRemovalFacts(workspace: Workspace, t: TFunction<"translation">): string[] {
  const panes = workspace.removal?.pane_count ?? 0;
  const running = workspace.removal?.running_agent_count ?? 0;
  const facts: string[] = [];
  if (panes > 0) facts.push(t("workspace.facts.panesClose", { count: panes }));
  if (panes > 0 && running > 0) facts.push(t("workspace.facts.agentsStop", { count: running }));
  // A row Herdr shows without a registration leaves once its panes do.
  facts.push(workspace.registered ? t("workspace.facts.registrationOnly") : panes > 0 ? t("workspace.facts.rowWithPanes") : t("workspace.facts.rowWithWorkspace"));
  facts.push(t("workspace.facts.filesStay"));
  return facts;
}

/** The checkout items a plain folder's row adds after its project's; its row routes these to the checkout. */
export const FOLDER_CHECKOUT_ITEMS: ReadonlySet<MenuItem["id"]> = new Set<MenuItem["id"]>(["open_checkout", "open_pull_request", "set_purpose", "new_group"]);

/**
 * A plain folder's one row (`folderCheckout`): the project's items, then its
 * checkout's that the project's do not already cover. The folder is the
 * checkout, so its new tab, path and reveal items are the project's, and it
 * has no other checkout to make the default.
 */
export function folderMenu(
  workspace: Workspace,
  checkout: Checkout,
  host: MenuHost,
  t: TFunction<"translation">,
  purposeProblem: string | null = null,
  storedIssueSource?: string,
): MenuItem[] {
  const project = projectMenu(workspace, host, t, storedIssueSource, true);
  const [first, ...rest] = checkoutMenu(workspace, checkout, host, t, purposeProblem).filter((item) => FOLDER_CHECKOUT_ITEMS.has(item.id));
  return first ? [...project, { ...first, separated: true }, ...rest.map((item) => ({ ...item, separated: false }))] : project;
}

/**
 * Why a remote checkout's purpose cannot be written: the core stores it in
 * that host's Herdr, which takes it from 0.9.1 (the native `purposeUnavailableReason`).
 */
export function remotePurposeProblem(workspace: Workspace, remote: RemoteStatus[] | undefined, t: TFunction<"translation">): string | null {
  const targetId = workspace.remote_target_id;
  if (!targetId) return null;
  const version = remote?.find((row) => row.target_id === targetId)?.herdr_version ?? null;
  if (supportsRemotePurpose(version)) return null;
  return version ? t("workspace.remotePurposeVersion", { minimum: "0.9.1", version }) : t("workspace.remotePurposeUnknownVersion", { minimum: "0.9.1" });
}

/**
 * Why a checkout cannot become its project's default (`set_primary_checkout`
 * refuses the same cases): the choice is stored on this machine's
 * registration of a Git project, for a checkout whose folder exists.
 */
function primaryProblem(workspace: Workspace, checkout: Checkout, node: string, t: TFunction<"translation">): string | null {
  if (workspace.device_id !== node) return t("workspace.unavailable.otherDevice");
  if (workspace.is_git === false) return t("workspace.unavailable.plainFolder");
  if (checkout.is_primary) return t("workspace.unavailable.alreadyDefault");
  if (!workspace.registered) return t("workspace.unavailable.pinFirst");
  if (!checkout.exists) return t("workspace.unavailable.folderMissing");
  return null;
}

/**
 * The checkout row's menu (PRD sidebar-context-menus D-03, D-07, D-09): open
 * it or a tab in it, its pull request while GitHub knows one (PRD
 * checkout-pr-glyph-card D-07), then what describes it, then deleting a
 * linked worktree. Choices stored on this machine and the OS file manager
 * are this computer's only, so a device's checkout lists them disabled.
 */
export function checkoutMenu(workspace: Workspace, checkout: Checkout, host: MenuHost, t: TFunction<"translation">, purposeProblem: string | null = null): MenuItem[] {
  const pr = shownPullRequest(checkout);
  const items: MenuItem[] = [
    { id: "open_checkout", label: t("common.open"), unavailable: null },
    { id: "new_tab_here", label: t("workspace.menu.newTabHere"), unavailable: checkout.exists ? null : t("workspace.unavailable.folderMissing"), shortcut: host.newTabChord },
    { id: "new_group", label: t("groups.create"), unavailable: checkout.exists ? null : t("workspace.unavailable.folderMissing") },
  ];
  if (pr) items.push({ id: "open_pull_request", label: t("workspace.menu.openPr", { number: pr.number }), unavailable: null });
  items.push(
    { id: "set_purpose", label: t("workspace.menu.setPurpose"), unavailable: purposeProblem, separated: true },
    { id: "set_primary", label: t("workspace.menu.setPrimary"), unavailable: primaryProblem(workspace, checkout, host.node, t) },
    { id: "copy_branch", label: t("workspace.menu.copyBranch"), unavailable: checkout.branch ? null : t("workspace.unavailable.detached") },
    { id: "copy_path", label: t("workspace.menu.copyPath"), unavailable: null },
    ...editorItem(workspace, host, t, false),
    ...revealItem(workspace, host, t, false),
  );
  // Never disabled: what deleting would lose is the dialog's to say, with
  // the choice beside it, on any device and before Git has been read.
  if (checkout.is_worktree) items.push({ id: "delete_worktree", label: t("workspace.menu.deleteWorktree"), unavailable: null, separated: true, destructive: true });
  return items;
}

export type AgentMenuItem = MenuPresentation & {
  id: "show_agent" | "copy_title" | "copy_session_id" | "copy_pane_id" | "close_tab";
  label: string;
  unavailable: string | null;
  separated?: boolean;
  shortcut?: string;
};

/**
 * An agent row's menu (PRD sidebar-context-menus D-04, D-06): go to its pane,
 * copy what names it, close the tab it is in through the tab close
 * confirmation. `showChord` is the ⌥n that selects the same row, or "" when
 * it holds no number on this host. Herdr 0.9.1 can neither mark a pane seen
 * nor stop an agent, so neither is offered (D-05).
 */
export function agentMenu(agent: AgentRow, showChord: string, t: TFunction<"translation">): AgentMenuItem[] {
  return [
    { id: "show_agent", icon: "terminal", label: t("workspace.menu.showAgent"), unavailable: null, shortcut: showChord },
    { id: "copy_title", group: "copy", menuLabel: t("menus.sessionTitle"), icon: "copy", label: t("workspace.menu.copyTitle"), unavailable: null, separated: true },
    { id: "copy_session_id", group: "copy", menuLabel: t("menus.sessionId"), icon: "id", label: t("workspace.menu.copySession"), unavailable: agent.session_id ? null : t("workspace.unavailable.noSession") },
    { id: "copy_pane_id", group: "copy", menuLabel: t("menus.terminalId"), icon: "id", label: t("workspace.menu.copyPaneId"), unavailable: null },
    { id: "close_tab", icon: "close", label: t("workspace.menu.closeTab"), unavailable: null, separated: true },
  ];
}

/** The agents deleting a checkout stops: every pane an agent runs in, by the name and state the sidebar shows. */
export function stoppedAgents(checkout: Checkout, t: TFunction<"translation">): string[] {
  return checkout.tabs.flatMap((tab) => tab.panes).flatMap((pane) => (pane.identity_label ? [`${pane.identity_label} (${statusText(t, pane.status_code)})`] : []));
}

/**
 * What deleting a worktree does, as the short facts its confirmation shows
 * on one line under the path (D-42); the core's warnings are its badges.
 */
export function deletionFacts(checkout: Checkout, paneCount: number, t: TFunction<"translation">): string[] {
  const facts: string[] = [t("workspace.facts.folderRemoved")];
  if (paneCount > 0) facts.push(t("workspace.facts.panesClose", { count: paneCount }));
  const agents = stoppedAgents(checkout, t);
  if (agents.length > 0) facts.push(t("workspace.facts.stopsNamed", { count: agents.length, names: agents.join(", ") }));
  return facts;
}

/** A Discard choice belongs only to the measured loss currently displayed. */
export function discardConfirmationKey(checkout: Checkout): string {
  const row = checkout.worktree;
  return JSON.stringify([row?.deletion_gate.discard_label ?? null, row?.ignored_repositories ?? [], row?.ignored_scan_unavailable ?? null, row?.lock_reason ?? null]);
}

/** Facts joined into the one line a confirmation shows, its first word capitalized. */
export function factsLine(facts: readonly string[]): string {
  const line = facts.join(" · ");
  return line.charAt(0).toUpperCase() + line.slice(1);
}

/** The newest task the operator asked for on this page, once it names the request's own target. */
export function taskFor(
  operation: TaskOperation | null | undefined,
  request: { kind: string; afterId: number; deviceId?: string; repositoryRoot?: string; branch?: string; path?: string } | null,
  node: string,
): TaskOperation | null {
  if (!operation || !request) return null;
  if (operation.kind !== request.kind || operation.id <= request.afterId) return null;
  if (request.deviceId !== undefined && receiptDevice(operation.device_id, node) !== request.deviceId) return null;
  if (request.repositoryRoot !== undefined && operation.repository_root !== request.repositoryRoot) return null;
  if (request.branch !== undefined && operation.branch !== request.branch) return null;
  if (request.path !== undefined && operation.path !== request.path) return null;
  return operation;
}

/** The removal this page asked for, by the device and checkout it named; another checkout's removal is not its answer. */
export function removalFor(removal: WorktreeRemoval | null | undefined, deviceId: string, checkoutPath: string, afterId: number, node: string): WorktreeRemoval | null {
  if (!removal || receiptDevice(removal.device_id, node) !== deviceId || removal.checkout_path !== checkoutPath || removal.id <= afterId) return null;
  return removal;
}

/**
 * Whether the checkout is being deleted: from the confirmation until Git
 * answers, whoever confirmed it. A finished removal has taken the row away
 * and a failed one gives it back as it was.
 */
export function checkoutRemoving(removal: WorktreeRemoval | null | undefined, deviceId: string, checkoutPath: string, node: string): boolean {
  const current = removalFor(removal, deviceId, checkoutPath, 0, node);
  return current !== null && (current.phase === "checking" || current.phase === "closing" || current.phase === "removing");
}
