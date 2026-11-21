// Add a project's own rules: which hosts the dialog offers, how Browse folder
// opens on each, what a line typed into the folder list asks for, the one
// refusal the shell can tell from the snapshot before an event goes out, the
// line a refusal is shown as, and Clone from URL's reading of a URL and its
// default parent. Everything else about a path hided (this Mac) or the
// machine's node decides and answers, and the core parses a URL again before
// Git sees it (`hide_host::clone`).

import type { TFunction } from "i18next";
import type { MessageKey } from "./i18n/catalogs";
import { machineName, thisMachineKey, windowDeviceId, windowFirst } from "./screenMachine";
import type { Device, WorkspaceRegistration } from "./snapshot";

/** `offline`: a device that is not connected now, listed dimmed and not chosen (PRD device-add-project B31). */
export type Host = { id: string; label: string; offline: boolean };

/** This window's machine first as This Mac, then every other device, as the Host selector lists them (PRD core-host-node-move N1). */
export function addProjectHosts(devices: readonly Device[] | undefined, t: TFunction<"translation">): Host[] {
  const own = windowDeviceId(devices);
  const rows = windowFirst(devices);
  // The machine this window runs on is reachable whatever its row says: the window is drawn from it.
  const hosts = rows.map((device) => ({ id: device.id, label: machineName(devices, device.id, t), offline: device.id !== own && deviceOffline(device) }));
  // Before the snapshot names this machine, it is the core's own node by the empty id.
  return rows.some((device) => device.id === own) ? hosts : [{ id: own, label: t(thisMachineKey(null)), offline: false }, ...hosts];
}

/** A device whose connection is not up now: its node cannot list, clone or create. The core's own row is never offline. */
export function deviceOffline(device: Device | undefined): boolean {
  return device !== undefined && device.kind === "remote" && device.state !== "ready";
}

/**
 * How Browse folder and the parent pickers open on a host (PRD
 * device-add-project D-01, D-02): every host offers the same three ways, and
 * the folders are picked with the system's own folder window (`native`) only
 * on the machine this window runs on when the window can open one (the
 * desktop app can, a browser tab cannot); anywhere else, a connected device
 * or the core's machine seen from a node's window, the dialog's own folder
 * list shows that machine's folders as its node answers them.
 */
export type HostWays = { own: boolean; native: boolean };

export function hostWays(host: string, devices: readonly Device[] | undefined, picker: boolean): HostWays {
  const own = host === windowDeviceId(devices);
  return { own, native: own && picker };
}

/** The host the dialog opens on: the focused device while it is listed and connected, this Mac otherwise. */
export function initialHost(hosts: readonly Host[], focused: string | null | undefined): string {
  return focused && hosts.some((host) => host.id === focused && !host.offline) ? focused : (hosts[0]?.id ?? "");
}

/**
 * Where the folder list starts on `device`: the folder it last showed there,
 * which the core keeps across a restart (PRD device-add-project D-04), else
 * home. `node` is the core's own machine, whose folder is kept apart from
 * the devices'.
 */
export function rememberedFolder(state: { add_project_folder?: string | null; device_add_project_folders?: Record<string, string> } | undefined, device: string, node: string): string | null {
  return (device === node || device === "" ? state?.add_project_folder : state?.device_add_project_folders?.[device]) || null;
}

/** How long typing settles before a machine is asked about what was typed: a clone or project target, or a folder. */
export const TYPED_ASK_DELAY_MS = 150;

/**
 * What a line typed into the folder list asks for (PRD device-add-project
 * D-06): a `~` path moves the list to the folder before its last `/` and
 * filters by the name after it; an absolute path, a Windows one included
 * (Explorer's quoted `Copy as path` too), names a folder the machine's node
 * places against its home, spelled as a WSL device shows the PC's drives
 * when `drives` says where; a drive path on a machine that is neither
 * Windows nor WSL names nothing there and filters, like anything else.
 */
export type TypedPlace = { kind: "filter"; text: string } | { kind: "folder"; folder: string; filter: string } | { kind: "path"; path: string };

export function typedPlace(raw: string, drives: string | null | undefined, windows: boolean): TypedPlace {
  const text = raw.trim().replace(/^"(.*)"$/, "$1");
  if (text === "~" || text.startsWith("~/")) {
    const cut = text.lastIndexOf("/");
    if (cut < 0) return { kind: "folder", folder: "~", filter: "" };
    return { kind: "folder", folder: text.slice(0, cut) || "~", filter: text.slice(cut + 1) };
  }
  if ((windows || drives) && /^[A-Za-z]:([\\/]|$)/.test(text)) {
    const spelled = drives ? wslDrivePath(text, drives) : text.replace(/\\/g, "/");
    return { kind: "path", path: trimFolder(spelled) };
  }
  if (text.startsWith("/")) return { kind: "path", path: trimFolder(text) };
  return { kind: "filter", text: raw };
}

/** The entries whose names hold `filter`, letter case ignored; an empty filter keeps them all. */
export function filterEntries<T extends { name: string }>(entries: readonly T[], filter: string): T[] {
  const wanted = filter.trim().toLocaleLowerCase();
  return wanted ? entries.filter((entry) => entry.name.toLocaleLowerCase().includes(wanted)) : [...entries];
}

/** A child of a folder as the machine spells it: `/` between names, whatever system it runs. */
export function childPath(folder: string, name: string): string {
  return `${folder.replace(/\/+$/, "")}/${name}`;
}

/** The crumbs of a folder's `~` label, each with the label it moves to: `~`, `~/a`, `~/a/b`. */
export function folderCrumbs(label: string): { name: string; label: string }[] {
  const names = label.split("/").filter(Boolean);
  return names.map((name, index) => ({ name, label: names.slice(0, index + 1).join("/") }));
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

/** The project `path` already is on `device`, if any; no event goes out for it, the dialog goes to it. */
export function registeredProject(path: string, device: string, registrations: readonly WorkspaceRegistration[]): WorkspaceRegistration | undefined {
  return registrations.find((row) => row.device_id === device && trimFolder(row.path) === path);
}

/** True when `path` is already a project on `device`. */
export function alreadyRegistered(path: string, device: string, registrations: readonly WorkspaceRegistration[]): boolean {
  return registeredProject(path, device, registrations) !== undefined;
}

/**
 * A path pasted into a WSL device's field, spelled as that device names it: the device reports where it
 * shows the PC's drives (`windows_drives`, `/mnt`), so `C:\work\app`, `C:/work/app` and Explorer's quoted
 * Copy as path turn into `/mnt/c/work/app` (PRD wsl-device B16). Any other text, and any path on a device
 * that reports no drives, an ordinary Linux server's included, is left as written.
 */
export function wslDrivePath(raw: string, drives: string | null | undefined): string {
  if (!drives) return raw;
  const match = /^"?([A-Za-z]):(?:[\\/]([^"]*))?"?$/.exec(raw.trim());
  if (!match) return raw;
  const [, letter, rest = ""] = match as unknown as [string, string, string | undefined];
  const tail = rest.trim().split(/[\\/]+/).filter(Boolean).join("/");
  const root = drives.replace(/\/+$/, "");
  return `${root}/${letter.toLowerCase()}${tail ? `/${tail}` : ""}`;
}

/** Whether `path` is on the PC's C drive as WSL shows it (`/mnt/c`), which a confirmation says may be slow (PRD wsl-device D-15). */
export function onCDrive(path: string, drives: string | null | undefined): boolean {
  const drive = `${(drives || "/mnt").replace(/\/+$/, "")}/c`;
  return path === drive || path.startsWith(`${drive}/`);
}

/** A refusal as one line, for a reason code hided answered or the shell's own `already_registered`. */
export function refusalText(reason: string, t: TFunction<"translation">, home = false): string {
  switch (reason) {
    case "already_registered":
      return t("addProject.refusal.alreadyRegistered");
    case "needs_confirmation":
      return t(home ? "addProject.confirmHome" : "addProject.confirmOutside");
    case "outside_home":
      return t("addProject.refusal.outsideHome");
    case "home_root":
      return t("addProject.refusal.homeRoot");
    case "project_holds_home":
      return t("addProject.refusal.projectHoldsHome");
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
  const parents = registrations
    .filter((row) => row.device_id === node)
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
