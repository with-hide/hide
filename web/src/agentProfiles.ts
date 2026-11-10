// A machine's agent profiles as the screens draw them (PRD agent-profiles
// B1-B16, B26-B28): read from the core's snapshot (`kit.profiles`), never kept
// here. Changes go to the core as one `agent_profile_change` each, and the
// screen reads only the answer that carries its own request id.

import type { TFunction } from "i18next";
import { agentAdapter } from "./agentAdapters";
import { PROVIDER_KINDS, type ProfileItem, type ProviderKind } from "./agentPicker";
import type { AgentProfilesView, ProfileAccountWay, ProfileDraft, ProfileInstall, ProfileRefusal, ProfileView, ProfilesMachine, ProjectProfileChoice } from "./components/agent-profiles";
import { kitPartNeedsReinstall } from "./settings";
import type { AgentUsage, LimitCell } from "./generated/hided-ws";
import { localDeviceId, type AgentProfileAnswer, type AgentProfileWire, type Device, type MachineAgentProfiles, type SnapshotRest } from "./snapshot";
import type { StartTarget, StartTargets } from "./startTargets";
import { limitWindow, windowPercent } from "./usage";

/** The built-in profile's id on every machine (`ProfileId::default_profile`). */
export const BUILT_IN_PROFILE = "default";

/** How the agent keeps one account apart from another (the adapter contract's `account_by`); null for one Hide keeps no profiles for (D-25). */
export function accountWay(agent: string): ProfileAccountWay | null {
  return (agentAdapter(agent)?.account_by as ProfileAccountWay | null | undefined) ?? null;
}

/** A machine's list, when its node keeps profiles: null for a machine never read, and for one whose node predates profiles. */
export function listedProfiles(device: Device | undefined): MachineAgentProfiles | null {
  const profiles = device?.kit?.profiles ?? null;
  return profiles && profiles.state !== "unsupported" ? profiles : null;
}

/** Whether the machine's list can change now (B30): live, read-only while its device is not connected, or absent on a node that predates profiles. */
export function profilesMachine(device: Device | undefined, connected: boolean): ProfilesMachine | null {
  const profiles = device?.kit?.profiles;
  if (!profiles) return null;
  if (profiles.state === "unsupported") return "unsupported";
  return connected ? "live" : "offline";
}

/** The one state of a profile's pieces a row shows (B6-B8): none where it has no account of its own. */
function install(profile: AgentProfileWire): ProfileInstall | null {
  switch (profile.install) {
    case "removing":
      return "removing";
    case "remove_failed":
      return "remove_failed";
    case "refused":
      return "failed";
    case "own":
      return [profile.hook, profile.skill].some((piece) => piece && kitPartNeedsReinstall(piece)) ? "failed" : "installed";
    case "default":
      return null;
    // Not judged since this version: an account of its own is being put in place.
    default:
      return profile.account ? "installing" : null;
  }
}

/** The weekly percent left of an account's limits, rounded; null with no open weekly window. */
function weeklyLeft(cell: LimitCell | null | undefined, now: number): number | null {
  const used = windowPercent(limitWindow(cell ?? undefined, "weekly"), now);
  return used === null ? null : Math.max(0, Math.round(100 - used));
}

/** What a profile's row says of its account's usage (B23): the weekly percent left, or a dash where another row already shows that account. */
function profileUsageLeft(usage: AgentUsage | null, id: string, now: number): ProfileView["usageLeft"] {
  const row = usage?.profiles?.find((profile) => profile.id === id);
  if (!row) return null;
  return row.shared ? "shared" : weeklyLeft(row.limits, now);
}

/** The weekly percent left of the account an item runs in (D-11): a profile of the agent's default account reads the agent's own row, a profile of another account its own, which every profile of that account carries. */
function itemLeft(usage: AgentUsage | null, id: string, now: number): number | null {
  if (!usage) return null;
  if (id === BUILT_IN_PROFILE) return weeklyLeft(usage.limits, now);
  const row = usage.profiles?.find((profile) => profile.id === id);
  if (!row) return null;
  return weeklyLeft(row.shared ? (row.limits ?? usage.limits) : row.limits, now);
}

/** This machine's usage row of `agent`, which carries its profiles' accounts (B23); a device's profiles have none. */
export function ownAgentUsage(usage: { nodes: { agents: AgentUsage[] }[] } | null | undefined, agent: string): AgentUsage | null {
  const kind = agentAdapter(agent)?.herdr_kind ?? agent;
  return usage?.nodes[0]?.agents.find((row) => row.agent === kind) ?? null;
}

/**
 * An agent's profiles on one machine, its built-in profile first (B2, B28),
 * each with its sign-in (B10), its account's usage left (B23, `usage`, this
 * machine's row; none for a device) and its program's folder warning (B25).
 */
export function agentProfilesView(profiles: MachineAgentProfiles | null, agent: string, label: string, usage: AgentUsage | null = null, now = Date.now()): AgentProfilesView | null {
  const way = accountWay(agent);
  if (!way) return null;
  const chosen = profiles?.defaults[agent] ?? BUILT_IN_PROFILE;
  const builtIn: ProfileView = { id: BUILT_IN_PROFILE, builtIn: true, name: "", isDefault: chosen === BUILT_IN_PROFILE, account: null, program: null, envKeys: [], install: null, login: null, usageLeft: usage ? weeklyLeft(usage.limits, now) : null, wrapperFolder: false };
  const own = (profiles?.profiles ?? [])
    .filter((profile) => profile.kind === agent)
    .map((profile): ProfileView => ({
      id: profile.id,
      builtIn: false,
      name: profile.name,
      isDefault: profile.id === chosen,
      account: profile.account,
      program: profile.program,
      envKeys: profile.env_keys,
      install: install(profile),
      login: profile.login ?? null,
      usageLeft: profileUsageLeft(usage, profile.id, now),
      wrapperFolder: profile.wrapper_folder === true,
    }));
  return { agent, label, way, program: agentAdapter(agent)?.program ?? agent, profiles: [builtIn, ...own] };
}

/** The answer to the request `requestId`, once the core gave one. */
export function profileAnswer(rest: SnapshotRest | null, requestId: string | null): AgentProfileAnswer | null {
  const answer = rest?.status?.agent_profile_answer;
  return requestId && answer?.request_id === requestId ? answer : null;
}

/** 1-64 characters of `[A-Za-z0-9_-]`, the core's request id format. */
export function profileRequestId(): string {
  const random = globalThis.crypto?.randomUUID?.() ?? `${Date.now().toString(36)}${Math.random().toString(36).slice(2)}`;
  return `profile-${random}`.slice(0, 64);
}

/** What `agent_profile_change` carries. */
export type ProfileChange =
  | { op: "add"; kind: string; name: string; account: string | null; program: string | null; env: [string, string][]; default?: boolean }
  | { op: "edit"; id: string; name: string; program: string | null; env: [string, string][] }
  | { op: "default"; kind: string; id: string }
  | { op: "remove_check"; id: string }
  | { op: "remove"; id: string }
  | { op: "reinstall"; id: string }
  | { op: "values"; id: string };

/** The words for a refusal the core names by code; a field the machine refused carries its own. */
export function profileRefusalKey(code: string | undefined): string {
  switch (code) {
    case "profile_gone":
      return "agentProfiles.error.gone";
    case "profile_busy":
      return "agentProfiles.error.busy";
    case "profiles_unsupported":
      return "agentProfiles.unsupported";
    case "profile_builtin":
      return "agentProfiles.builtInNote";
    case "profile_in_use":
      return "agentProfiles.inUse";
    case "machine_unavailable":
    case "machine_unknown":
    case "profiles_unread":
    case "profile_unanswered":
      return "agentProfiles.error.unanswered";
    default:
      return "agentProfiles.error.other";
  }
}

const FORM_FIELDS = ["name", "account", "program", "env"] as const;

/** What the screen says for a node's refusal, in its own words (`profile_reason`). */
const REASON_KEYS: Record<NonNullable<AgentProfileAnswer["reason"]>, string> = {
  kind_unknown: "agentProfiles.refuse.kindUnknown",
  profiles_full: "agentProfiles.refuse.profilesFull",
  name_empty: "agentProfiles.refuse.nameEmpty",
  name_long: "agentProfiles.refuse.nameLong",
  name_control: "agentProfiles.refuse.nameControl",
  name_builtin: "agentProfiles.refuse.nameBuiltin",
  name_taken: "agentProfiles.refuse.nameTaken",
  program_long: "agentProfiles.refuse.programLong",
  profile_builtin: "agentProfiles.builtInNote",
  profile_missing: "agentProfiles.error.gone",
  profile_removing: "agentProfiles.refuse.removing",
  profile_other_kind: "agentProfiles.refuse.otherKind",
  node_variable: "agentProfiles.refuse.nodeVariable",
  account_home_missing: "agentProfiles.refuse.noHome",
  account_unsupported: "agentProfiles.noAccount",
  account_needs_folder: "agentProfiles.refuse.needsFolder",
  account_needs_name: "agentProfiles.refuse.needsName",
  account_empty: "agentProfiles.refuse.empty",
  account_relative: "agentProfiles.refuse.relative",
  account_is_default: "agentProfiles.refuse.defaultLocation",
  account_name: "agentProfiles.refuse.accountName",
  account_unprepared: "agentProfiles.refuse.unprepared",
  env_too_many: "agentProfiles.refuse.envTooMany",
  env_name: "agentProfiles.refuse.envName",
  env_reserved: "agentProfiles.refuse.accountVar",
  env_repeated: "agentProfiles.refuse.envRepeated",
  env_value: "agentProfiles.refuse.envValue",
  other: "agentProfiles.refuse.unknown",
};

/** A message to word: its catalog key and what it names. */
export type Worded = { key: string; params?: Record<string, string | number> };

/** The words for a node's refusal; one from a node that sends no reason reads generically. */
function reasonWords(answer: AgentProfileAnswer): Worded {
  const key = answer.env_key;
  // The home and Hide's own variables are named apart from the account's.
  if (answer.reason === "env_reserved" && key && /^(HOME|USERPROFILE|HIDE_.*)$/i.test(key)) return { key: "agentProfiles.refuse.reserved", params: { key } };
  return { key: REASON_KEYS[answer.reason ?? "other"] ?? REASON_KEYS.other, params: { ...(answer.limit !== undefined ? { limit: answer.limit } : {}), ...(key ? { key } : {}) } };
}

/**
 * What a refused answer shows: a field the machine refused under that field
 * (B4, B5, B13), else one line under the form, each in the screen's language.
 */
export function answerRefusal(answer: AgentProfileAnswer | null): { refusals: (Worded & { field: ProfileRefusal["field"]; envKey?: string })[]; line: Worded | null } {
  if (answer?.state !== "refused") return { refusals: [], line: null };
  if (answer.code !== "profile_refused") return { refusals: [], line: { key: profileRefusalKey(answer.code) } };
  const field = FORM_FIELDS.find((name) => name === answer.field);
  const words = reasonWords(answer);
  if (field) return { refusals: [{ field, ...words, ...(answer.env_key ? { envKey: answer.env_key } : {}) }], line: null };
  return { refusals: [], line: words };
}

/** A profile's fields as the form starts them; the variables' values come from a `values` answer, never the snapshot. */
export function profileDraft(profile: ProfileView | null, values?: readonly [string, string][]): ProfileDraft {
  return {
    name: profile?.name ?? "",
    account: profile?.account ?? "",
    program: profile?.program ?? "",
    env: values ? values.map(([key, value]) => ({ key, value })) : (profile?.envKeys ?? []).map((key) => ({ key, value: "" })),
  };
}

/** The variables a draft sets: rows left wholly empty are not sent; a row with a value and no name is, so the machine says why. */
function draftEnv(draft: ProfileDraft): [string, string][] {
  return draft.env.filter((pair) => pair.key.trim() || pair.value).map((pair) => [pair.key.trim(), pair.value]);
}

const blank = (text: string) => text.trim() || null;

export function addChange(kind: string, way: ProfileAccountWay, draft: ProfileDraft): ProfileChange {
  return { op: "add", kind, name: draft.name.trim(), account: way === "none" ? null : blank(draft.account), program: blank(draft.program), env: draftEnv(draft) };
}

export function editChange(id: string, draft: ProfileDraft): ProfileChange {
  return { op: "edit", id, name: draft.name.trim(), program: blank(draft.program), env: draftEnv(draft) };
}

/** Whether two drafts would save the same profile. */
export function sameDraft(a: ProfileDraft, b: ProfileDraft): boolean {
  return JSON.stringify(editChange("", a)) === JSON.stringify(editChange("", b));
}

/**
 * An item's name on every screen that picks one (PRD agent-profiles-phase2
 * D-01): its agent's name, then `· <profile>` for a profile other than the
 * built-in default, which is the agent's name alone.
 */
export function itemName(t: TFunction<"translation">, agentLabel: string, profile?: string | null): string {
  return profile ? t("agentProfiles.agentWithProfile", { agent: agentLabel, profile }) : agentLabel;
}

/** What an item whose profile the machine no longer lists is called: the agent and `Missing profile`, never a guessed name (D-12). */
export function missingItemName(t: TFunction<"translation">, agentLabel: string): string {
  return itemName(t, agentLabel, t("agentProfiles.itemGone"));
}

/** One item of an agent's list on a machine (D-01): `profile` is its id, `default` for the built-in one, and `name` the profile's name, null for the built-in one. */
export type AgentItem = { kind: ProviderKind; profile: string; name: string | null };

/** The items of `kind` in a machine's profile list: the built-in one first, then the profiles it holds, in order; the built-in one alone where there is no list. */
function itemsOf(listed: MachineAgentProfiles | null, kind: ProviderKind): AgentItem[] {
  const adapter = agentAdapter(kind);
  const view = adapter && listed ? agentProfilesView(listed, adapter.id, adapter.label) : null;
  if (!view) return [{ kind, profile: BUILT_IN_PROFILE, name: null }];
  return view.profiles.map((row) => ({ kind, profile: row.id, name: row.builtIn ? null : row.name }));
}

/** The items of `kind` on `device`. */
export function agentItems(device: Device | undefined, kind: ProviderKind): AgentItem[] {
  return itemsOf(listedProfiles(device), kind);
}

/** One item of the start panel's agent menu (D-01): an agent with one of its machine's profiles, `default` for the built-in one. */
export type StartItem = AgentItem & {
  /** The profile's version now, which its model list must have been read at; null for the built-in profile. */
  version: number | null;
  /** The weekly percent left of the item's account; null when it is not known, which the menu draws `—` (D-11). */
  left: number | null;
};

/** Why a machine's menu lists the built-in items alone, in the words of the panel's existing lines: it is not connected (B6), or its Hide does not keep profiles. */
export type StartItemsNote = "offline" | "unsupported";

/**
 * The start panel's agent menu on a machine (D-01, B2, B3, B6): each agent
 * with its built-in item first, then the profiles the machine lists for it,
 * in its order. `showLeft` is false while the menu lists no profile of any
 * agent, when it is the agent names alone as before (B1). `listed` says the
 * machine keeps profiles (its list was read, or kept from an earlier read),
 * so a start names the item it shows, the built-in one too, whatever the
 * menu draws (D-02). A machine that is not `connected` lists the built-in
 * items alone and says so; a device whose Hide predates profiles does too,
 * which is not a machine whose list could not be read.
 */
export function startItems(rest: SnapshotRest | null, deviceId: string, usage: { nodes: { node: string; agents: AgentUsage[] }[] } | null = null, now = Date.now(), connected = true): { items: StartItem[]; showLeft: boolean; listed: boolean; note: StartItemsNote | null } {
  const device = rest?.navigator?.devices?.find((row) => row.id === deviceId);
  const kept = listedProfiles(device);
  const listed = connected ? kept : null;
  const remote = deviceId !== localDeviceId(rest);
  const note: StartItemsNote | null = !connected ? "offline" : remote && kept === null ? "unsupported" : null;
  // The usage read is this machine's alone: a device's items have no figure.
  const own = usage?.nodes[0]?.node === deviceId ? usage : null;
  const items = PROVIDER_KINDS.flatMap((kind): StartItem[] => {
    const row = own ? ownAgentUsage(own, kind) : null;
    return itemsOf(listed, kind).map((item) => ({
      ...item,
      version: listed?.profiles.find((profile) => profile.id === item.profile)?.version ?? null,
      left: itemLeft(row, item.profile, now),
    }));
  });
  return { items, showLeft: items.some((item) => item.name !== null), listed: listed !== null, note };
}

/** The profile of the item `profile` of `kind` as `catalogFor` asks for it: its id and version now; null for the built-in item. */
export function profileItem(items: readonly StartItem[], kind: string, profile: string): ProfileItem | null {
  const item = items.find((row) => row.kind === kind && row.profile === profile);
  return item && item.version !== null ? { id: item.profile, version: item.version } : null;
}

/** The profile a start of `kind` in the project `workspaceId` preselects: the core's one answer (D-03), `default` where it gave none. */
export function preselectedProfile(rest: SnapshotRest | null, deviceId: string, workspaceId: string, kind: string): string {
  const adapter = agentAdapter(kind);
  const selection = rest?.status?.start_profiles?.[workspaceId];
  return (adapter && selection?.device_id === deviceId ? selection.profiles[adapter.id] : undefined) ?? BUILT_IN_PROFILE;
}

/**
 * Project settings › Agent accounts (B15): each agent the machine keeps
 * profiles for, with the project's choice while the machine still lists it;
 * `ProjectAgentAccounts` leaves out an agent with no profile of its own.
 */
export function projectAccountChoices(device: Device | undefined, chosen: Record<string, string> | undefined): ProjectProfileChoice[] {
  const listed = listedProfiles(device);
  if (!listed) return [];
  return (device?.kit?.agents ?? []).flatMap((agent) => {
    const view = agentProfilesView(listed, agent.id, agent.label);
    if (!view) return [];
    const id = chosen?.[agent.id] ?? null;
    return [{ view, chosen: id && view.profiles.some((row) => row.id === id) ? id : null }];
  });
}

/**
 * Where `열어서 로그인` starts a profile's agent on `deviceId` (B10): its start
 * kind and the checkout a start panel opened over Settings would take there,
 * else that device's first main checkout. Null when the agent is not one Hide
 * starts or the device has no checkout to start in; the start panel opens then.
 */
export function signInStart(targets: StartTargets, deviceId: string, agent: string): { provider: ProviderKind; checkoutPath: string } | null {
  const provider = PROVIDER_KINDS.find((kind) => kind === agentAdapter(agent)?.herdr_kind);
  if (!provider) return null;
  const checkouts = (targets.groups.find((group) => group.deviceId === deviceId)?.items ?? []).filter(
    (item): item is Extract<StartTarget, { kind: "checkout" }> => item.kind === "checkout" && item.disabled === null,
  );
  const checkout = checkouts.find((item) => item.key === targets.defaultKey) ?? checkouts.find((item) => item.primary) ?? checkouts[0];
  return checkout ? { provider, checkoutPath: checkout.checkoutPath } : null;
}
