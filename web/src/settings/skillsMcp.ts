// What Settings › 스킬·MCP shows of one machine's store (PRD skill-mcp-manager,
// decisions of 2026-11-19): which columns stand, what each row's summary
// says, what its 전체 switch sends and how a version reads. Every value is
// the store report's; nothing here decides what an agent supports.

import { agentAdapter } from "../agentAdapters";
import type { KitAgent, McpServer, StoreAnswer, StoreCell, StoreCellState, StoreChange, StoreColumn, StoreFoundSkill, StoreItem, StoreItemKind, StorePreview, StoreRefusalCause, StoreReport, StoreSource, StoreVersion } from "../snapshot";
import { agentInstalled } from "./agentRows";

/** The two ways the page lays its rows out: rows that unfold (the default) and the table (scene 1-2 A). */
export type StoreView = "list" | "table";

const VIEW_KEY = "hide.settings.store-view";

/** The view this viewer chose last; a store that cannot be read is the list. */
export function savedStoreView(): StoreView {
  try {
    return globalThis.localStorage?.getItem(VIEW_KEY) === "table" ? "table" : "list";
  } catch {
    return "list";
  }
}

export function saveStoreView(view: StoreView): void {
  try {
    globalThis.localStorage?.setItem(VIEW_KEY, view);
  } catch {
    // A viewer whose storage is refused keeps the choice for this page only.
  }
}

/**
 * The agents the page shows on a machine: installed there and switched on in
 * Settings › Agents (decision 2026-11-19, `agentInstalled` as the Agents page
 * reads it). "모든 에이전트" means every one of these.
 */
export function shownAgents(agents: readonly KitAgent[] | undefined): Set<string> {
  return new Set((agents ?? []).filter((agent) => agent.enabled && agentInstalled(agent)).map((agent) => agent.id));
}

/** An agent's short name, as the picker names it ("Claude", "Codex"); its id when this build has no adapter. */
export function agentShortName(id: string): string {
  return agentAdapter(id)?.picker_label ?? id;
}

/** One column the page draws. */
export type ShownColumn = {
  id: string;
  /** The skill folder every agent of `readers` reads (the shared folder), rather than one agent's. */
  shared: boolean;
  /** The column's own agent, for a column of one agent. */
  agent: string | null;
  /** The shown agents that read what this column holds, its own and those that read its folder as well. */
  readers: string[];
};

/**
 * A kind's columns as the page draws them: a skill folder whose agents are
 * all hidden, or an MCP agent that is, has no column. The shared skill folder
 * (`agents`) is one column for every agent that reads it.
 */
export function shownColumns(report: StoreReport, kind: StoreItemKind, shown: ReadonlySet<string>): ShownColumn[] {
  return (report.columns[kind] ?? []).flatMap((column: StoreColumn) => {
    const own = column.agents.filter((agent) => shown.has(agent));
    if (own.length === 0) return [];
    const also = (column.also ?? []).filter((agent) => shown.has(agent) && !own.includes(agent));
    return [{ id: column.id, shared: column.agents.length > 1, agent: column.agents.length === 1 ? (column.agents[0] ?? null) : null, readers: [...own, ...also] }];
  });
}

/** The column's name in a head or a summary: the agent's short name, or the shared folder's word. */
export function columnName(column: ShownColumn, shared: string): string {
  return column.shared || !column.agent ? shared : agentShortName(column.agent);
}

/** The cell of `item` in `column`; a column the report has no cell for is unsupported there. */
export function cellOf(item: StoreItem, column: ShownColumn): StoreCell {
  return item.cells.find((cell) => cell.column === column.id) ?? { column: column.id, state: "unsupported" };
}

/** The cell states that count toward a row's summary; a column that cannot take the item never makes it differ. */
function counts(state: StoreCellState): boolean {
  return state !== "unsupported" && state !== "agent_off";
}

/** On, or on and waiting for its agent's folder (D-31). */
export function cellOn(state: StoreCellState): boolean {
  return state === "on" || state === "waiting";
}

/** What a row says at its end (decision 2026-11-19): every shown agent has it, none does, or only what differs. */
export type RowSummary =
  | { kind: "all" }
  | { kind: "off" }
  | { kind: "differs"; parts: { column: ShownColumn; state: Exclude<StoreCellState, "on" | "waiting" | "unsupported" | "agent_off"> }[] };

/** The summary read from the cells, never from the item's own flag (store review note, 2026-11-19). */
export function rowSummary(item: StoreItem, columns: readonly ShownColumn[]): RowSummary {
  const counted = columns.map((column) => ({ column, state: cellOf(item, column).state })).filter((cell) => counts(cell.state));
  if (counted.every((cell) => cellOn(cell.state))) return counted.length > 0 ? { kind: "all" } : { kind: "off" };
  if (counted.every((cell) => cell.state === "off")) return { kind: "off" };
  const rank = (state: StoreCellState) => (state === "failed" ? 0 : state === "hand_written" ? 1 : 2);
  const parts = counted
    .filter((cell) => !cellOn(cell.state))
    .map((cell) => ({ column: cell.column, state: cell.state as "off" | "failed" | "hand_written" }))
    .sort((a, b) => rank(a.state) - rank(b.state));
  return { kind: "differs", parts };
}

/**
 * The row's 전체 switch: drawn on while any shown cell has the item, and a
 * press turns every agent on unless every one already has it, which turns
 * the item off (D-40). `Set` with no column clears the columns switched off.
 */
export function allSwitch(item: StoreItem, columns: readonly ShownColumn[]): { checked: boolean; change: StoreChange } {
  const summary = rowSummary(item, columns);
  const checked = columns.some((column) => cellOn(cellOf(item, column).state));
  return { checked, change: { op: "set", item: item.id, on: summary.kind !== "all" } };
}

/**
 * A column's own switch, and the change that tries a failed cell again: the
 * switch the operator last set, sent again, runs a pass that writes or takes
 * out the copy there once more (B26).
 */
export function columnWants(item: StoreItem, column: string): boolean {
  return item.on && !(item.off_for ?? []).includes(column);
}

/** The short version beside the name (B36); null for a local MCP that pins nothing. */
export function versionText(version: StoreVersion, t: (key: "store.version.folder" | "store.version.copy" | "store.version.remote" | "store.version.npmLatest") => string): string | null {
  switch (version.kind) {
    case "tag":
    case "commit":
    case "pinned":
      return version.value;
    case "folder":
      return t("store.version.folder");
    case "copy":
      return t("store.version.copy");
    case "remote":
      return t("store.version.remote");
    case "npm_latest":
      return t("store.version.npmLatest");
    case "unpinned":
      return null;
  }
}

/** A git address as people read it: no scheme, no `.git`. */
export function shortUrl(url: string): string {
  return url.replace(/^[a-z+]+:\/\//i, "").replace(/^git@([^:]+):/, "$1/").replace(/\.git$/, "");
}

/** Where an item comes from, one line: the repository and its folder, the followed folder, or the MCP server's command or address. */
export function sourceLine(item: Pick<StoreItem, "source" | "server">): string {
  if (item.server) return serverLine(item.server);
  return sourceText(item.source);
}

export function sourceText(source: StoreSource): string {
  switch (source.kind) {
    case "git":
      return source.path ? `${shortUrl(source.url)}/${source.path}` : shortUrl(source.url);
    case "folder":
      return source.path;
    case "copy":
      return source.from;
    case "pasted":
      return "";
  }
}

/** An MCP server as one line: its command with its arguments, or its address. */
export function serverLine(server: McpServer): string {
  return server.reach.kind === "local" ? [server.reach.command, ...(server.reach.args ?? [])].join(" ") : server.reach.url;
}

/** The names of an MCP server's keys: a local server's variables, a remote one's headers. */
export function keyNames(server: McpServer | null | undefined): string[] {
  if (!server) return [];
  return server.reach.kind === "local" ? (server.reach.env ?? []) : (server.reach.headers ?? []);
}

/** The items of one kind, in the store's order. */
export function itemsOf(report: StoreReport, kind: StoreItemKind): StoreItem[] {
  return report.items.filter((item) => item.kind === kind);
}

/** The items a check found a newer version of (the head's 업데이트 n). */
export function updatable(report: StoreReport): StoreItem[] {
  return report.items.filter((item) => item.update);
}

/** A fresh request id for one press: the screen's own short token. */
export function storeRequestId(): string {
  return `store-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`;
}

/** The newest undo entry for `item`, which a delete just made. */
export function trashFor(report: StoreReport, item: Pick<StoreItem, "kind" | "name">) {
  return report.trash.filter((entry) => entry.kind === item.kind && entry.name === item.name && entry.cause.kind === "deleted").sort((a, b) => b.at_unix_ms - a.at_unix_ms)[0] ?? null;
}

/** What a store request's refusal says: the first change refused, or the request that never ran. */
export function refusalOf(answer: StoreAnswer | null | undefined): StoreRefusalCause | "unavailable" | null {
  if (!answer) return null;
  if (answer.state === "refused") return "unavailable";
  return answer.outcomes.find((outcome) => outcome.refused)?.refused?.cause ?? null;
}

/** Whether a request is still on its way: sent and not yet answered. */
export function answerPending(sent: string | null, answer: StoreAnswer | null | undefined): boolean {
  return sent !== null && (!answer || answer.state === "pending");
}

/**
 * What the add field holds (scene 2, 3): a GitHub address or a git URL is a
 * skill source, a path is a folder, anything else is read as an MCP paste
 * (a command, an address or a README's JSON).
 */
export function addInput(text: string): { kind: "git"; url: string } | { kind: "folder"; path: string } | { kind: "mcp"; text: string } | null {
  const value = text.trim();
  if (!value) return null;
  if (/^(\/|~\/|[a-z]:[\\/])/i.test(value)) return { kind: "folder", path: value };
  if (/^(git@|ssh:\/\/|file:\/\/)/i.test(value) || /^(https?:\/\/)?(www\.)?(github\.com|gitlab\.com|codeberg\.org|bitbucket\.org)\//i.test(value) || /\.git$/i.test(value)) return { kind: "git", url: value };
  if (/^[\w.-]+\/[\w.-]+$/.test(value)) return { kind: "git", url: `github.com/${value}` };
  return { kind: "mcp", text: value };
}

/** The ids a report holds now, so the one a request adds can be told from the rest. */
export function previewIds(report: StoreReport | null | undefined): Set<string> {
  return new Set([...(report?.previews ?? []).map((preview) => preview.id), ...(report?.mcp_previews ?? []).map((preview) => preview.id), ...(report?.diffs ?? []).map((diff) => diff.id)]);
}

/** The newest entry whose id was not there before the request. */
export function newest<T extends { id: string; at_unix_ms: number }>(rows: readonly T[] | undefined, before: ReadonlySet<string>): T | null {
  return [...(rows ?? [])].filter((row) => !before.has(row.id)).sort((a, b) => b.at_unix_ms - a.at_unix_ms)[0] ?? null;
}

/** The files a set of found skills brings, and how many of them run. */
export function filesOf(skills: readonly StoreFoundSkill[]): { files: { path: string; executable: boolean }[]; executables: number } {
  const files = skills.flatMap((skill) => skill.files.map((file) => ({ path: `${skill.name}/${file.path}`, executable: file.executable })));
  return { files, executables: files.filter((file) => file.executable).length };
}

/** A preview's source as one line: the repository at its commit, or the folder. */
export function previewSource(preview: StorePreview): string {
  if (preview.source.kind === "folder") return preview.source.path;
  const at = preview.tag ?? preview.commit?.slice(0, 7);
  return at ? `${shortUrl(preview.url ?? preview.source.url)} @ ${at}` : shortUrl(preview.url ?? preview.source.url);
}

/** A version as the update confirm shows it on each side of the arrow. */
export function versionValue(version: StoreVersion): string {
  return "value" in version ? version.value : version.kind;
}

/** Whether the item asked for came from a place a person can open: a repository's page, or a folder of this machine. */
export function openableSource(item: StoreItem, local: boolean): { kind: "url"; url: string } | { kind: "folder"; path: string } | null {
  if (item.source.kind === "git") {
    const url = item.source.url.replace(/^git@([^:]+):/, "https://$1/").replace(/\.git$/, "");
    return { kind: "url", url: /^https?:\/\//.test(url) ? url : `https://${url}` };
  }
  if (item.source.kind === "folder" && local) return { kind: "folder", path: item.source.path };
  return null;
}
