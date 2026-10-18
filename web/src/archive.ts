// 세션 보관 (PRD session-archive): where `세션 보관` is offered, what its
// confirmation lists, and the archived rows the 태스크 list folds under
// `보관됨`. The core decides and refuses the same cases on the event; this
// only words them before the press.

import type { TFunction } from "i18next";
import { deviceConnected } from "./devices";
import { translate } from "./i18n/client";
import { machineName } from "./screenMachine";
import { localDeviceId, type AgentRow, type ArchivedSession, type SnapshotRest, type Workspace } from "./snapshot";

/** `세션 보관` on one row: not offered, offered, or offered disabled with why (B1-B4, B13). */
export type ArchiveOffer = { kind: "hidden" } | { kind: "offered"; unavailable: string | null };

const HIDDEN: ArchiveOffer = { kind: "hidden" };

/**
 * A plain shell has no row; a Hide AI agent and a Factory worker belong to
 * their homes (B2). A delegated child goes with its root (B4), a device that
 * is gone cannot close it (B13), and a conversation Hide has not seen cannot
 * be resumed (B3).
 */
export function archiveOffer(rest: SnapshotRest | null, agent: AgentRow | undefined, t: TFunction<"translation"> = translate): ArchiveOffer {
  if (!agent || agent.hide_ai || rest?.navigator?.factory_worker_panes?.includes(agent.pane_id)) return HIDDEN;
  if (agent.delegated || agent.lineage_parent_pane_id) return { kind: "offered", unavailable: t("archive.unavailable.delegated") };
  const device = agent.device_id ?? localDeviceId(rest);
  if (device !== localDeviceId(rest) && !deviceConnected(rest, device)) {
    return { kind: "offered", unavailable: t("archive.unavailable.disconnected", { device: machineName(rest?.navigator?.devices, device, t) }) };
  }
  if (!agent.session_id) return { kind: "offered", unavailable: t("archive.unavailable.unknownSession") };
  return { kind: "offered", unavailable: null };
}

/** The root and the live descendants that would close with it and are working now: what the confirmation lists (B6). */
export function archiveWorking(root: AgentRow, byPane: ReadonlyMap<string, AgentRow>): AgentRow[] {
  return [root, ...(root.close_descendant_pane_ids ?? []).flatMap((pane) => byPane.get(pane) ?? [])].filter((agent) => agent.state.subtree === "working");
}

/** The archived rows of one device that stand in `보관됨`, newest archived first as the core ordered them (B14, B15). */
export function archivedRows(rest: SnapshotRest | null, deviceId: string): ArchivedSession[] {
  return (rest?.navigator?.archived_sessions ?? []).filter((row) => row.listed && row.device_id === deviceId);
}

/** The project an archived row belongs to on its device, by the checkout or project path it was archived from. */
export function archivedProject(row: ArchivedSession, projects: readonly Workspace[]): Workspace | null {
  return projects.find((project) => project.device_id === row.device_id && (project.path === row.project_root || project.checkouts.some((checkout) => checkout.path === row.checkout_path))) ?? null;
}

/** The archive entry that holds a conversation, and that member, whether or not it stands in `보관됨`. */
export function archivedMemberOf(rest: SnapshotRest | null, session: { device_id: string; agent: string; session_id: string }): { row: ArchivedSession; index: number } | null {
  for (const row of rest?.navigator?.archived_sessions ?? []) {
    const index = row.members.findIndex((member) => member.session.device_id === session.device_id && member.session.session_id === session.session_id && member.agent_kind === session.agent);
    if (index >= 0) return { row, index };
  }
  return null;
}

/** The archived members a live root's record shows as child rows: a partial close's and a failed resume's (B24). */
export function archivedChildrenOf(rest: SnapshotRest | null, session: { device_id: string; agent: string; session_id: string }): { row: ArchivedSession; members: ArchivedSession["members"] } | null {
  const found = archivedMemberOf(rest, session);
  if (!found || found.index !== 0 || found.row.listed) return null;
  const members = found.row.members.slice(1).filter((member) => member.resume !== "resumed");
  return members.length ? { row: found.row, members } : null;
}

/** A shell-side word for a refusal the core published about an archive, or null when the kind is not one (B10, B12, B30). */
export function archiveNotice(kind: string, t: TFunction<"translation"> = translate): string | null {
  switch (kind) {
    case "archive.full":
      return t("archive.notice.full");
    case "archive.incomplete":
      return t("archive.notice.incomplete");
    case "archive.incomplete_timeout":
      return t("archive.notice.incompleteTimeout");
    case "archive.incomplete_disconnected":
      return t("archive.notice.incompleteDisconnected");
    case "archive.confirmation_required":
      return t("archive.notice.confirm");
    case "archive.delegated":
      return t("archive.notice.delegated");
    case "archive.not_offered":
      return t("archive.notice.notOffered");
    case "archive.too_many":
      return t("archive.notice.tooMany");
    case "archive.save_failed":
      return t("archive.notice.saveFailed");
    case "archive.delete_failed":
    case "archive.delete_busy":
      return t("archive.notice.deleteFailed");
    case "archive.device_disconnected":
      return t("archive.notice.disconnected");
    case "archive.session_unknown":
      return t("archive.notice.unknownSession");
    case "archive.status_unknown":
      return t("archive.notice.statusUnknown");
    case "archive.busy":
      return t("archive.notice.busy");
    case "archive.target_unknown":
    case "archive.context_unavailable":
    case "archive.unknown":
    case "archive.identity":
      return t("archive.notice.gone");
    default:
      return null;
  }
}
