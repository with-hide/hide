// The input of a session opened beside Desk (mission-106): what its `@`
// menu lists, what a pick writes into its plain box, which of its agent's
// model and effort its bar can change, and its project's files read for
// `@`. The thread panel draws; these decide. Desk's composer and a session's
// share the menus themselves (`components/desk/composer-menus.tsx`).

import type { TFunction } from "i18next";
import { useEffect, useRef } from "react";
import { agentAdapter } from "./agentAdapters";
import { modelLabel, type ModelCatalog } from "./agentPicker";
import { referenceLands, referenceText } from "./agentReference";
import type { SessionBarSlot } from "./components/desk/desk-composer";
import type { MentionFile, MentionSession } from "./deskComposer";
import type { DeskChoice, DeskMentions, SessionThreadSection, SnapshotRest } from "./snapshot";
import { useShellStore } from "./store";

/** The most of the project's files the `@` menu lists above the sessions. */
export const MENTION_FILES = 5;

/**
 * The sessions a session's `@` can name: Desk's list without the session
 * itself and without a session whose pane its agent could not address (a
 * device's pane named from another device's, `referenceLands`). A child
 * whose parent is left out stands at the top of the tree.
 */
export function sessionMentions(section: DeskMentions | null, rest: SnapshotRest | null, paneId: string | null): DeskMentions | null {
  if (!section || !paneId) return section;
  const kept = section.sessions.filter((session) => session.id !== paneId && referenceLands(rest, session.id, paneId));
  const ids = new Set(kept.map((session) => session.id));
  return {
    sessions: kept.map((session) => (session.parent !== null && !ids.has(session.parent) ? { ...session, parent: null } : session)),
    projects: section.projects,
  };
}

/** How a session's box names a picked session: its reference, as a sidebar row dropped on a terminal types it. */
export function sessionWords(session: Pick<MentionSession, "id" | "name">): string {
  return referenceText(session.name, session.id);
}

/** The command a switch line starts with (`/model`), which the bar's hint names. */
export function switchCommand(line: string | null | undefined): string | null {
  return line?.split(" ")[0] ?? null;
}

/**
 * What a session's agent changes in a running session (`switch` of its
 * adapter, from `[switch]` of its definition): none for an agent Hide has
 * no definition of.
 */
export function agentSwitch(kind: string | null) {
  return (kind ? agentAdapter(kind)?.switch : null) ?? null;
}

/**
 * The model and effort slots of a session's bar: the value the session runs
 * with, the choices a change can make (the agent's own model list, as the
 * start surfaces read it, and the efforts its line takes), and, for a slot
 * the agent changes only in a menu of its own, why it cannot change here.
 */
export function sessionBarSlots(kind: string, choice: SessionThreadSection["choice"], catalog: ModelCatalog | null, t: TFunction<"translation">): { model: SessionBarSlot; effort: SessionBarSlot } {
  const agent = agentAdapter(kind)?.picker_label ?? kind;
  const can = agentSwitch(kind);
  const model = choice.model ?? null;
  const effort = choice.effort ?? null;
  const listed = catalog?.state === "ready" ? catalog.models : [];
  const models: DeskChoice[] = [...listed, ...(model && !listed.includes(model) ? [model] : [])].map((id) => ({ id, label: modelLabel(catalog, id) }));
  const efforts = can?.efforts ?? [];
  return {
    model: {
      value: model,
      label: model ? modelLabel(catalog, model) : null,
      choices: models,
      fixed: can?.model ? null : t("desk.bar.modelFixed", { agent }),
    },
    effort: {
      value: effort,
      label: effort,
      choices: efforts.map((id) => ({ id, label: id })),
      fixed: efforts.length === 0 ? t("desk.bar.effortFixed", { agent }) : can?.effort?.includes("{model}") && model === null ? t("desk.bar.effortNeedsModel", { agent }) : null,
    },
  };
}

/**
 * The files of the checkout `place` names that match `query`, while the
 * `@` menu is open (`query` not null): hided's index of the checkout, which
 * ⌘P reads too, asked once the typing rests and again while it is still
 * walking, at most `MENTION_FILES`.
 */
export function useMentionFiles(requestFileIndex: (root: string, query: string, device: string) => void, place: { device: string; root: string } | null, query: string | null): readonly MentionFile[] {
  const answer = useShellStore((s) => (place && query !== null && s.fileIndex && s.fileIndex.device_id === place.device && s.fileIndex.root_path === place.root && s.fileIndex.query === query ? s.fileIndex : null));
  const ask = useRef(requestFileIndex);
  ask.current = requestFileIndex;
  const device = place?.device ?? null;
  const root = place?.root ?? null;
  useEffect(() => {
    if (device === null || root === null || query === null) return undefined;
    // One request once the typing rests, as ⌘P's palette asks.
    const timer = window.setTimeout(() => ask.current(root, query, device), 120);
    return () => window.clearTimeout(timer);
  }, [device, root, query]);
  const indexing = answer?.indexing ?? false;
  useEffect(() => {
    if (device === null || root === null || query === null || !indexing) return undefined;
    const poll = window.setTimeout(() => ask.current(root, query, device), 250);
    return () => window.clearTimeout(poll);
  }, [device, root, query, indexing, answer]);
  return answer ? answer.files.slice(0, MENTION_FILES).map((file) => ({ relative_path: file.relative_path })) : NO_FILES;
}

const NO_FILES: readonly MentionFile[] = [];
