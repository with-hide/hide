// A machine's agent profiles as the screens draw them (PRD agent-profiles
// B1-B16, B26-B28): read from the core's snapshot (`kit.profiles`), never kept
// here. Changes go to the core as one `agent_profile_change` each, and the
// screen reads only the answer that carries its own request id.

import { agentAdapter } from "./agentAdapters";
import type { AgentProfilesView, ProfileAccountWay, ProfileDraft, ProfileInstall, ProfileRefusal, ProfileView, ProfilesMachine, ProjectProfileChoice } from "./components/agent-profiles";
import { projectSettings, settingsTarget } from "./projectSettings";
import { kitPartNeedsReinstall } from "./settings";
import { catalogWorkspaces, type AgentProfileAnswer, type AgentProfileWire, type Device, type MachineAgentProfiles, type SnapshotRest } from "./snapshot";

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

/**
 * An agent's profiles on one machine, its built-in profile first (B2, B28).
 * Sign-in, usage and the program warning are another change's (B10, B23,
 * B25): until then each is left out rather than drawn unknown.
 */
export function agentProfilesView(profiles: MachineAgentProfiles | null, agent: string, label: string): AgentProfilesView | null {
  const way = accountWay(agent);
  if (!way) return null;
  const chosen = profiles?.defaults[agent] ?? BUILT_IN_PROFILE;
  const builtIn: ProfileView = { id: BUILT_IN_PROFILE, builtIn: true, name: "", isDefault: chosen === BUILT_IN_PROFILE, account: null, program: null, envKeys: [], install: null, login: null, usageLeft: null, wrapperFolder: false };
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
      login: null,
      usageLeft: null,
      wrapperFolder: false,
    }));
  return { agent, label, way, program: agentAdapter(agent)?.program ?? agent, profiles: [builtIn, ...own] };
}

/** The profile a start of `agent` picks with nothing chosen (D-32): the project's default while its machine lists it, else the agent's default there. The core's `launch::pick` decides the start; this only says which one the menu shows chosen. */
export function pickedProfile(profiles: MachineAgentProfiles | null, agent: string, projectDefault: string | null): string {
  // A project that names the built-in profile starts it; only no choice falls through to the agent's default.
  if (projectDefault === BUILT_IN_PROFILE) return BUILT_IN_PROFILE;
  const listed = (id: string | null | undefined) => (id && profiles?.profiles.some((profile) => profile.kind === agent && profile.id === id && profile.install !== "removing" && profile.install !== "remove_failed") ? id : null);
  return listed(projectDefault) ?? listed(profiles?.defaults[agent]) ?? BUILT_IN_PROFILE;
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

/**
 * What a refused answer shows: a field the machine refused under that field,
 * in the machine's words (B4, B5, B13), else one line under the form.
 */
export function answerRefusal(answer: AgentProfileAnswer | null): { refusals: ProfileRefusal[]; text: string | null; key: string | null } {
  if (answer?.state !== "refused") return { refusals: [], text: null, key: null };
  const field = FORM_FIELDS.find((name) => name === answer.field);
  if (answer.code === "profile_refused" && field && answer.text) return { refusals: [{ field, text: answer.text, ...(answer.env_key ? { envKey: answer.env_key } : {}) }], text: null, key: null };
  if (answer.code === "profile_refused" && answer.text) return { refusals: [], text: answer.text, key: null };
  return { refusals: [], text: null, key: profileRefusalKey(answer.code) };
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

/** The project's chosen profile for `agent` (`project_settings.agent_profiles`), by the catalog project a start target names. */
export function projectProfile(rest: SnapshotRest | null, deviceId: string, workspaceId: string, agent: string): string | null {
  const workspace = catalogWorkspaces(rest).find((row) => row.id === workspaceId && row.device_id === deviceId);
  return workspace ? (projectSettings(rest, settingsTarget(workspace)).record?.agent_profiles?.[agent] ?? null) : null;
}

/**
 * The start panel's profile menu for a start of `kind` on a target (B14):
 * absent unless the machine lists another profile of that agent, else its
 * profiles with the one D-32 would pick.
 */
export function startProfiles(rest: SnapshotRest | null, deviceId: string, workspaceId: string, kind: string): { view: AgentProfilesView; projectDefault: string | null; picked: string } | null {
  const adapter = agentAdapter(kind);
  const device = rest?.navigator?.devices?.find((row) => row.id === deviceId);
  const listed = listedProfiles(device);
  if (!adapter || !listed) return null;
  const view = agentProfilesView(listed, adapter.id, adapter.label);
  if (!view || view.profiles.length < 2) return null;
  const projectDefault = projectProfile(rest, deviceId, workspaceId, adapter.id);
  return { view, projectDefault, picked: pickedProfile(listed, adapter.id, projectDefault) };
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
