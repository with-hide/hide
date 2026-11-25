// The phone's start sheet (PRD home-device-rail D-24, B42-B46): what it
// preselects from the catalog hided sends, why a start cannot leave yet, and
// the words for each refusal, named by catalog key. Pure, so every rule is
// tested without a socket.

import type { Notice, StartCatalog, StartKind, StartKindEntry, StartTarget } from "./protocol";
import { PROVIDER_KINDS, providerLabel, takesModel } from "../agentPicker";

/** The longest instruction hided starts an agent with (hided/src/mobile/start.rs MAX_PROMPT_CHARS). */
export const MAX_START_CHARS = 4000;

export const START_KINDS: readonly { id: StartKind; label: string }[] = PROVIDER_KINDS
  .map((id) => ({ id, label: providerLabel(id) }));

/** What the operator changed in the sheet; anything not changed follows the remembered choice. */
export type StartChoice = { target: string | null; kind: StartKind | null; model: string | undefined };

export const NO_CHOICE: StartChoice = { target: null, kind: null, model: undefined };

export type StartSelection = {
  target: StartTarget | null;
  /** The project `target` belongs to, whose checkouts the branch list offers. */
  project: StartProject | null;
  kind: StartKind;
  /** The catalog id sent, or "" for the CLI's own default. */
  model: string;
  /** The catalog for `kind`; null until hided has sent one. */
  entry: StartKindEntry | null;
};

/** Why an instruction cannot be sent yet, checked before it leaves the phone. */
export function startProblem(text: string): "empty" | "too_long" | "control_characters" | null {
  if (text.trim().length === 0) return "empty";
  if ([...text].length > MAX_START_CHARS) return "too_long";
  // eslint-disable-next-line no-control-regex
  if (/[\u0000-\u0008\u000b-\u001f\u007f-\u009f]/.test(text)) return "control_characters";
  return null;
}

/** One project of the sheet's project list: its checkouts, the main one first, then the rest as the catalog lists them. */
export type StartProject = { key: string; label: string; targets: StartTarget[] };

/** One device's projects in the project list; `label` null is the core's own machine, which the phone names from `core_platform`. */
export type StartDevice = { deviceId: string; label: string | null; connected: boolean; projects: StartProject[] };

/**
 * The project list (docs/UI_BEHAVIOR.md, The phone app): the targets gathered
 * into each device's projects, in the order the catalog lists them, which is
 * the core's own machine first. A Mac that predates `project` names none, so
 * each of its targets is a project of its own, named by its place.
 */
export function startDevices(catalog: StartCatalog | null): StartDevice[] {
  const devices: StartDevice[] = [];
  const projects = new Map<string, StartProject>();
  for (const target of catalog?.targets ?? []) {
    let device = devices.find((candidate) => candidate.deviceId === target.device_id);
    if (!device) {
      device = { deviceId: target.device_id, label: target.device_label, connected: target.connected, projects: [] };
      devices.push(device);
    }
    const key = `${target.device_id}\u0000${target.project_id ?? target.id}`;
    let project = projects.get(key);
    if (!project) {
      project = { key, label: target.project ?? target.place, targets: [] };
      projects.set(key, project);
      device.projects.push(project);
    }
    if (target.main) project.targets.unshift(target);
    else project.targets.push(target);
  }
  return devices;
}

/** What the sheet shows selected: the first project's main checkout until the operator picks (none when no device has a project), the remembered kind and model until then. */
export function selectionOf(catalog: StartCatalog | null, choice: StartChoice): StartSelection {
  const projects = startDevices(catalog).flatMap((device) => device.projects);
  const project = projects.find((candidate) => candidate.targets.some((target) => target.id === choice.target)) ?? projects[0] ?? null;
  const target = project?.targets.find((candidate) => candidate.id === choice.target) ?? project?.targets[0] ?? null;
  const kind = choice.kind ?? catalog?.remembered.kind ?? START_KINDS[0]?.id;
  if (!kind) throw new Error("No start adapter declared");
  const entry = catalog?.kinds.find((candidate) => candidate.id === kind) ?? null;
  const wanted = choice.model ?? catalog?.remembered.models[kind] ?? "";
  // A remembered model the catalog does not list is still the one sent: the CLI decides, never a quiet swap (B32).
  // A kind whose start takes no model sends none, as the desktop's panel does.
  return { target, project, kind, model: takesModel(kind) ? wanted : "", entry };
}

/** The model menu is disabled while the catalog has none; it then shows the remembered model or the default. */
export function modelsOf(selection: StartSelection): string[] {
  return selection.entry?.models ?? [];
}

/**
 * Whether a start that did not answer ok may still have started its agent:
 * then a resend keeps its request id and hided answers it once (B43). Any
 * other answer is final, and tapping Start again is a new start with a new id.
 */
export function mayHaveStarted(reason: string | null): boolean {
  return reason === "timeout" || reason === "in_flight" || reason === "offline";
}

/** The line inside the sheet for a refused or failed start. */
export function startFailure(reason: string | null): Notice {
  switch (reason) {
    case "in_flight":
      return { key: "mobile.start.inFlight" };
    case "timeout":
      return { key: "mobile.start.timeout" };
    case "offline":
      return { key: "mobile.start.offline" };
    case "empty":
      return { key: "mobile.start.empty" };
    case "too_long":
      return { key: "mobile.start.tooLong", limit: MAX_START_CHARS };
    case "control_characters":
    case "agent_start.invalid_prompt":
      return { key: "mobile.start.controlCharacters" };
    case "unknown_target":
    case "overview.unknown_checkout":
      return { key: "mobile.start.unknownTarget" };
    case "unknown_kind":
    case "agent_start.unknown_provider":
      return { key: "mobile.start.unknownKind" };
    case "unknown_model":
    case "agent_start.unknown_model":
      return { key: "mobile.start.unknownModel" };
    case "task_operation.busy":
      return { key: "mobile.start.busy" };
    case "revoked":
    case "mobile_off":
      return { key: "mobile.start.revoked" };
    case "unavailable":
      return { key: "mobile.start.unavailable" };
    case "agent_failed":
      return { key: "mobile.start.agentFailed" };
    default:
      return { key: "mobile.start.failed" };
  }
}
