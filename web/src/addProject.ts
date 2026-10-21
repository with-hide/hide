// Add a project's own rules: which hosts the dialog offers, the one refusal
// the shell can tell from the snapshot before an event goes out, the line a
// refusal is shown as, and Clone from URL's reading of a URL and its default
// parent. Everything else about a path hided (this Mac) or the device's
// helper decides and answers, and the core parses a URL again before Git
// sees it (`hide_host::clone`).

import type { TFunction } from "i18next";
import type { MessageKey } from "./i18n/catalogs";
import { isCoreMachine, machineName, thisMachineKey, windowDeviceId, windowFirst } from "./screenMachine";
import type { Device, WorkspaceRegistration } from "./snapshot";

export type Host = { id: string; label: string };

/** This window's machine first as This Mac, then every other device, as the Host selector lists them (PRD core-host-node-move N1). */
export function addProjectHosts(devices: readonly Device[] | undefined, t: TFunction<"translation">): Host[] {
  const own = windowDeviceId(devices);
  const rows = windowFirst(devices);
  const hosts = rows.map((device) => ({ id: device.id, label: machineName(devices, device.id, t) }));
  // Before the snapshot names this machine, it is the core's own node by the empty id.
  return rows.some((device) => device.id === own) ? hosts : [{ id: own, label: t(thisMachineKey(null)) }, ...hosts];
}

/**
 * What the dialog offers on a host: Browse folder only on the machine this
 * window runs on, whose folders the native picker shows, and a typed `~/`
 * path that machine's node judges anywhere else; the other ways to add only on
 * the core's machine, the one a new project can be made on (the core refuses
 * a new folder on any other), and their parent picker only when the window
 * runs there too.
 */
export type HostWays = { browse: boolean; otherWays: boolean; pickParent: boolean };

export function hostWays(host: string, devices: readonly Device[] | undefined): HostWays {
  const own = host === windowDeviceId(devices);
  const core = isCoreMachine(devices, host);
  return { browse: own, otherWays: core, pickParent: own && core };
}

/** The host the dialog opens on: the focused device while it is still listed, this Mac otherwise. */
export function initialHost(hosts: readonly Host[], focused: string | null | undefined): string {
  return focused && hosts.some((host) => host.id === focused) ? focused : (hosts[0]?.id ?? "");
}

/** A path without its trailing slashes, so `~/a/` and `~/a` are one folder. */
export function trimFolder(raw: string): string {
  const trimmed = raw.trim();
  return trimmed.length > 1 ? trimmed.replace(/\/+$/, "") : trimmed;
}

/** The folder's last name, which the project is labelled with. */
export function folderLabel(path: string): string {
  return path.slice(path.lastIndexOf("/") + 1);
}

/** True when `path` is already a project on `device`; no event goes out for it. */
export function alreadyRegistered(path: string, device: string, registrations: readonly WorkspaceRegistration[]): boolean {
  return registrations.some((row) => row.device_id === device && trimFolder(row.path) === path);
}

/** A refusal as one line, for a reason code hided answered or the shell's own `already_registered`. */
export function refusalText(reason: string, t: TFunction<"translation">): string {
  switch (reason) {
    case "already_registered":
      return t("addProject.refusal.alreadyRegistered");
    case "outside_home":
      return t("addProject.refusal.outsideHome");
    case "home_root":
      return t("addProject.refusal.homeRoot");
    case "invalid_path":
      return t("addProject.refusal.invalidPath");
    case "not_found":
      return t("addProject.refusal.notFound");
    case "not_a_directory":
      return t("addProject.refusal.notADirectory");
    case "already_exists":
      return t("addProject.refusal.alreadyExists");
    default:
      return t("addProject.refusal.other", { reason });
  }
}

/**
 * A URL Git can clone and the folder it lands in, read the way the core reads
 * it (`hide_host::clone::CloneSource`): https, ssh, the scp form
 * `[user@]host:path`, and `file:///path`; the folder is the last segment
 * without `.git`. The shell reads it to name the folder and enable Clone; the
 * core decides.
 */
export type CloneUrl = { ok: true; host: string; name: string } | { ok: false; reason: MessageKey };

export function parseCloneUrl(raw: string): CloneUrl {
  const url = raw.trim();
  if (!url) return { ok: false, reason: "addProject.url.empty" };
  // `::` is Git's transport-helper form (`ext::`), never a URL this accepts.
  // eslint-disable-next-line no-control-regex
  if (url.startsWith("-") || url.includes("::") || /[\s\u0000-\u001f\u007f]/.test(url)) return { ok: false, reason: "addProject.url.notGit" };
  let host: string;
  let path: string;
  const scheme = /^(https|ssh|file):\/\/(.*)$/.exec(url);
  if (scheme) {
    const [, kind, rest] = scheme as unknown as [string, string, string];
    if (kind === "file") {
      if (!rest.startsWith("/")) return { ok: false, reason: "addProject.url.fileAbsolute" };
      host = "localhost";
      path = rest;
    } else {
      const slash = rest.indexOf("/");
      if (slash < 0) return { ok: false, reason: "addProject.url.noRepository" };
      const authority = rest.slice(0, slash);
      const hostPort = authority.slice(authority.lastIndexOf("@") + 1);
      host = hostPort.replace(/:\d*$/, "");
      path = rest.slice(slash + 1);
      if (!validHost(host)) return { ok: false, reason: "addProject.url.noHost" };
    }
  } else if (url.includes("://")) {
    return { ok: false, reason: "addProject.url.scheme" };
  } else {
    const colon = url.indexOf(":");
    const authority = colon < 0 ? "" : url.slice(0, colon);
    host = authority.slice(authority.lastIndexOf("@") + 1);
    if (colon < 0 || authority.includes("/") || !validHost(host)) return { ok: false, reason: "addProject.url.scheme" };
    path = url.slice(colon + 1);
  }
  const trimmed = path.replace(/\/+$/, "");
  const stem = (trimmed.endsWith(".git") ? trimmed.slice(0, -4) : trimmed).replace(/\/+$/, "");
  const name = stem.split(/[/:]/).pop() ?? "";
  if (!name || name === "." || name === ".." || name.includes("\\")) return { ok: false, reason: "addProject.url.noRepository" };
  return { ok: true, host: host.toLowerCase(), name };
}

function validHost(host: string): boolean {
  return host.length > 0 && /^[A-Za-z0-9._\-[\]:]+$/.test(host);
}

/** Only a complete URL without authentication material may become a shared setup draft. Git still receives the form's original input. */
export function cloneUrlDraft(raw: string): string {
  const value = raw.trim();
  if (value.length > 4096 || /[?#]/.test(value) || !parseCloneUrl(value).ok) return "";
  const authority = value.includes("://") ? value.split("://")[1]!.split("/")[0]! : value.split(":")[0]!;
  return authority.includes("@") ? "" : value;
}

/**
 * Where Create new project and Clone from URL put their folder at first: the
 * parent most of this Mac's projects live in, the most recently added one
 * winning a tie, else the home folder. One project added elsewhere (a
 * worktree folder, a scratch checkout) does not move where the next one goes.
 */
export function defaultProjectParent(registrations: readonly WorkspaceRegistration[], node: string): string {
  // The Home (`~/hide`) is no project, so it is not where the next one is expected to live.
  const parents = registrations
    .filter((row) => row.device_id === node && !row.home)
    .map((row) => {
      const path = trimFolder(row.path);
      const cut = path.lastIndexOf("/");
      return cut > 0 ? path.slice(0, cut) : "~";
    });
  const counts = new Map<string, number>();
  for (const parent of parents) counts.set(parent, (counts.get(parent) ?? 0) + 1);
  let best = "~";
  let bestCount = 0;
  // Registrations are in the order they were added, so `>=` lets a later parent take a tie.
  for (const parent of parents) {
    const count = counts.get(parent)!;
    if (count >= bestCount) {
      best = parent;
      bestCount = count;
    }
  }
  return best;
}

/** Why a typed `name` cannot be a new project's folder name, or null; an empty name has nothing to say yet. */
export function projectNameProblem(name: string): MessageKey | null {
  if (name.includes("/")) return "addProject.name.slash";
  if (name === "." || name === "..") return "addProject.name.dots";
  return null;
}

/** The folder a new project takes: its parent and the name, with a placeholder until one is typed. */
export function projectPath(parent: string, name: string): string {
  return `${parent === "/" ? "" : parent}/${name || "project-name"}`;
}
