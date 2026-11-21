// This app's own updates (PRD auto-update): when it looks, which release it
// takes, the download, and the install on quit or on Restart. It owns nothing
// else and shows nothing itself; the shell draws what `status()` says (the
// ↑ button and its card beside Help, the dot on Help and the update at its
// head, Settings > About) and the host relays it.
//
//   checks       at launch and every two hours, when asked (D-17), and when
//                the operator comes back to the app an hour or more after
//                the last look, so a ready update is still the newest
//   requires     an update when a release of this app's channel names a
//                minimum version above this app in its body
//                (`<!-- hide-min-version: X.Y.Z -->`): the card then cannot be
//                closed, and says the work keeps running across the restart
//   chooses      the highest release of with-hide/hide whose update file and
//                file for this system are both up, stable only unless preview
//                releases are on, never lower than this app (D-22, D-23)
//   downloads    in the background from that release, checked against the
//                sha512 its update file names; macOS also requires the same
//                Developer ID signature (D-24)
//   installs     on quit (macOS, Windows; on macOS closing the last window
//                quits to install a ready update), or on Restart, which closes and
//                reopens only this window: the core and its agents keep
//                running, and the reopened app replaces the core as it does
//                today (D-18, D-19)
//
// A development or CI build (any version that is not `X.Y.Z` or
// `X.Y.Z-rc.N`), an unpackaged app, and a Linux app not installed from the deb
// or rpm never look (D-27). Every failure is a state the About row shows and
// an event in the host's log; nothing else on screen changes (D-21).
//
// The feed is pinned to with-hide/hide here, not read from the package
// (electron-builder does not write one for a prepackaged app); the CI update
// test points it at a local folder with HIDE_UPDATE_FEED_URL (D-29).

import fs from "node:fs";
import path from "node:path";
import type { AppUpdater } from "electron-updater";
import type { UpdatePhase, UpdateStatus } from "../../../web/src/host";
import { readJsonFile, writeJsonFile } from "./jsonFile";
import type { HostLog } from "./log";

export const RELEASES_REPOSITORY = "with-hide/hide";
/** GitHub's most a page holds; evidence and preview tags share the list with releases. */
const RELEASES_API = `https://api.github.com/repos/${RELEASES_REPOSITORY}/releases?per_page=100`;
const RELEASE_DOWNLOADS = `https://github.com/${RELEASES_REPOSITORY}/releases/download`;
/** The first look waits for the window and the core to settle. */
export const FIRST_CHECK_DELAY_MS = 15_000;
export const CHECK_INTERVAL_MS = 2 * 60 * 60 * 1000;
/** Coming back to the app looks again once the last look is this old. */
export const LOOK_AGAIN_AFTER_MS = 60 * 60 * 1000;
const RELEASES_TIMEOUT_MS = 30_000;
/** A release list is a few hundred KB; more is not GitHub's answer. */
const RELEASES_BYTES_CAP = 8 * 1024 * 1024;

/** The versions a release is published under (`releaseKind` in scripts/check-release-assets.mjs). */
const RELEASE_VERSION = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-rc\.([1-9]\d*))?$/;

type Version = { major: number; minor: number; patch: number; rc: number | null };

export function parseVersion(text: string): Version | null {
  const match = RELEASE_VERSION.exec(text);
  if (!match) return null;
  return { major: Number(match[1]), minor: Number(match[2]), patch: Number(match[3]), rc: match[4] === undefined ? null : Number(match[4]) };
}

/** Semver order for the release shapes: a release is newer than every rc of the same version. */
export function compareVersions(a: Version, b: Version): number {
  const rank = (rc: number | null) => (rc === null ? Number.MAX_SAFE_INTEGER : rc);
  return a.major - b.major || a.minor - b.minor || a.patch - b.patch || rank(a.rc) - rank(b.rc);
}

/** The system's files, as `systemFiles` in scripts/check-release-assets.mjs names them. */
export type UpdateSystem = "macos" | "windows" | "deb" | "rpm";

export function updateFiles(version: string, system: UpdateSystem): { info: string; file: string } {
  const base = `hide-v${version}`;
  switch (system) {
    case "macos":
      return { info: "latest-mac.yml", file: `${base}-macos-arm64.zip` };
    case "windows":
      return { info: "latest.yml", file: `${base}-windows-x64-setup.exe` };
    case "deb":
      return { info: "latest-linux.yml", file: `${base}-linux-x64.deb` };
    case "rpm":
      return { info: "latest-linux.yml", file: `${base}-linux-x64.rpm` };
  }
}

export type ChosenRelease = { tag: string; version: string };

type ChannelRelease = { tag: string; version: string; parsed: Version; assets: unknown[]; body: string };

/**
 * The releases this app's channel takes, from GitHub's list (newest first,
 * any order accepted): published, tagged `vX.Y.Z` or, with preview releases
 * on, `vX.Y.Z-rc.N` marked as a pre-release. A tag of another shape (an
 * evidence tag) is skipped.
 */
function* channelReleases(releases: unknown, prerelease: boolean): Generator<ChannelRelease> {
  if (!Array.isArray(releases)) return;
  for (const release of releases) {
    if (!release || typeof release !== "object") continue;
    const { tag_name: tag, draft, prerelease: marked, assets, body } = release as Record<string, unknown>;
    if (typeof tag !== "string" || !tag.startsWith("v") || draft !== false || !Array.isArray(assets)) continue;
    const version = tag.slice(1);
    const parsed = parseVersion(version);
    if (!parsed || (parsed.rc !== null) !== (marked === true)) continue;
    if (parsed.rc !== null && !prerelease) continue;
    yield { tag, version, parsed, assets, body: typeof body === "string" ? body : "" };
  }
}

/**
 * The release to update to: one of the channel's releases with this system's
 * update file and update both uploaded, newer than `current`. A release still
 * uploading is skipped. Null when none is.
 */
export function chooseRelease(releases: unknown, input: { current: string; prerelease: boolean; system: UpdateSystem }): ChosenRelease | null {
  const current = parseVersion(input.current);
  if (!current) return null;
  let best: { tag: string; version: string; parsed: Version } | null = null;
  for (const { tag, version, parsed, assets } of channelReleases(releases, input.prerelease)) {
    if (compareVersions(parsed, current) <= 0 || (best && compareVersions(parsed, best.parsed) <= 0)) continue;
    const { info, file } = updateFiles(version, input.system);
    const uploaded = (name: string) => assets.some((asset) => asset && typeof asset === "object" && (asset as Record<string, unknown>).name === name && (asset as Record<string, unknown>).state === "uploaded");
    if (!uploaded(info) || !uploaded(file)) continue;
    best = { tag, version, parsed };
  }
  return best ? { tag: best.tag, version: best.version } : null;
}

/** The line a release's body carries to name the oldest version that may keep running without updating. */
const MINIMUM_VERSION_LINE = /<!--\s*hide-min-version:\s*(\S+)\s*-->/;

/**
 * The oldest version an app of this channel may keep running: the highest
 * `<!-- hide-min-version: X.Y.Z -->` one of the channel's releases names in
 * its body. A line naming a version above its own release's is skipped, so a
 * typo cannot require an update no release can give; null when none names one.
 * The line can be added to a release already published, and removing it
 * lifts the requirement at the next look.
 */
export function minimumVersion(releases: unknown, prerelease: boolean): string | null {
  let floor: { version: string; parsed: Version } | null = null;
  for (const { parsed: own, body } of channelReleases(releases, prerelease)) {
    const named = MINIMUM_VERSION_LINE.exec(body)?.[1];
    const parsed = named === undefined ? null : parseVersion(named);
    if (!named || !parsed || compareVersions(parsed, own) > 0 || (floor && compareVersions(parsed, floor.parsed) <= 0)) continue;
    floor = { version: named, parsed };
  }
  return floor?.version ?? null;
}

/**
 * Whether this app updates itself, and as which system. `packageType` is the
 * `resources/package-type` a Linux package carries; `writable` whether a
 * macOS app's bundle can be replaced where it is.
 */
export function updateSystem(input: { packaged: boolean; version: string; platform: NodeJS.Platform; packageType: string | null; macosLocation: "ok" | "read_only" }): UpdateSystem | { off: "development" | "location" } {
  if (!input.packaged || !parseVersion(input.version)) return { off: "development" };
  switch (input.platform) {
    case "darwin":
      return input.macosLocation === "ok" ? "macos" : { off: "location" };
    case "win32":
      return "windows";
    case "linux":
      return input.packageType === "deb" || input.packageType === "rpm" ? input.packageType : { off: "development" };
    default:
      return { off: "development" };
  }
}

/**
 * Whether a macOS app at `bundle` can replace itself: not on a disk image or
 * a read-only volume, not run from Gatekeeper's translocated copy, and in a
 * folder this account may write.
 */
export function macosLocation(bundle: string, canWrite: (folder: string) => boolean): "ok" | "read_only" {
  if (bundle.startsWith("/Volumes/") || bundle.includes("/AppTranslocation/")) return "read_only";
  return canWrite(path.dirname(bundle)) ? "ok" : "read_only";
}

/** The operator's update choices, kept in the desktop profile. */
export function updatesFile(userData: string): string {
  return path.join(userData, "updates.json");
}

/**
 * What the operator chose about updates: preview releases (D-22), and the
 * ready version whose card they closed, so it stays closed across launches
 * until a newer version is ready. A file that cannot be read is no choice.
 */
export type UpdateChoices = { prerelease: boolean; dismissed: string | null };

export function readUpdateChoices(file: string): UpdateChoices {
  const stored = readJsonFile(file);
  const record = stored && typeof stored === "object" ? (stored as Record<string, unknown>) : {};
  const dismissed = typeof record.dismissed === "string" && parseVersion(record.dismissed) ? record.dismissed : null;
  return { prerelease: record.prerelease === true, dismissed };
}

/** The slice of electron-updater this module drives, so tests can stand in for it. */
export type UpdaterLike = Pick<AppUpdater, "setFeedURL" | "checkForUpdates" | "downloadUpdate" | "quitAndInstall" | "on"> & {
  autoDownload: boolean;
  autoInstallOnAppQuit: boolean;
  allowDowngrade: boolean;
};

export type UpdatesIo = {
  /** GitHub's release list, parsed; throws on a failed or oversized answer. */
  releases: () => Promise<unknown>;
  now: () => number;
  /** Schedules `run` after `ms`; the answer cancels it. */
  later: (run: () => void, ms: number) => () => void;
  saveChoices: (choices: UpdateChoices) => void;
};

/**
 * The updater's state machine. `system` decides whether it ever looks;
 * `updater` is created only for a system that does. One check or download
 * runs at a time. A ready update stays ready while a later check looks; a
 * newer release that check finds is downloaded in its place, and the line
 * names it once it is ready (B19).
 */
export class AppUpdates {
  private phase: UpdatePhase;
  private prerelease: boolean;
  private dismissed: string | null;
  /** Whether a release's minimum version is above this app (`minimumVersion`); read at each look of GitHub's list. */
  private required = false;
  private running: Promise<void> | null = null;
  /** When the last look began; null until the launch look. */
  private lookedAt: number | null = null;
  private cancelTimer: (() => void) | null = null;
  private installing = false;
  private stopped = false;
  private readonly feedOverride: string | null;
  private readonly listeners = new Set<(status: UpdateStatus) => void>();

  constructor(
    private readonly system: UpdateSystem | { off: "development" | "location" },
    private readonly current: string,
    private readonly updater: UpdaterLike | null,
    private readonly io: UpdatesIo,
    private readonly log: HostLog,
    options: UpdateChoices & { feedOverride: string | null },
  ) {
    this.prerelease = options.prerelease;
    this.dismissed = options.dismissed;
    this.feedOverride = options.feedOverride;
    this.phase = typeof system === "string" ? { kind: "idle", checkedAt: null } : { kind: "off", reason: system.off };
    if (updater) {
      updater.autoDownload = false;
      updater.allowDowngrade = false;
      updater.autoInstallOnAppQuit = system !== "deb" && system !== "rpm";
      updater.on("download-progress", (progress: { percent: number }) => {
        if (this.phase.kind === "downloading") this.set({ ...this.phase, percent: Math.max(0, Math.min(100, Math.round(progress.percent))) });
      });
      // An install that failed after Restart (a cancelled password prompt on
      // Linux) keeps the update ready; electron-updater reports it here.
      updater.on("error", (error: Error) => {
        if (!this.installing) return;
        this.installing = false;
        this.log.event("update.install_failed", { detail: errorDetail(error) });
      });
    }
  }

  status(): UpdateStatus {
    return { phase: this.phase, prerelease: this.prerelease, installOnQuit: this.system !== "deb" && this.system !== "rpm", dismissed: this.dismissed, required: this.required };
  }

  onChange(listener: (status: UpdateStatus) => void): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  /** Looks soon after launch and then every two hours, until `stop`. */
  start(): void {
    // The override decides what the app installs, so a stray one is visible in the log.
    if (this.feedOverride) this.log.event("update.feed_override", { url: this.feedOverride });
    if (typeof this.system !== "string") {
      this.log.event("update.off", { reason: this.system.off, version: this.current });
      return;
    }
    this.schedule(FIRST_CHECK_DELAY_MS);
  }

  /** Ends the schedule; a check already running stops before its next step. */
  stop(): void {
    this.stopped = true;
    this.cancelTimer?.();
    this.cancelTimer = null;
  }

  /** A check now; one already running is the answer. */
  check(trigger: "launch" | "interval" | "operator" | "return"): Promise<void> {
    if (!this.updater || typeof this.system !== "string") return Promise.resolve();
    this.running ??= this.run(trigger, this.system, this.updater).finally(() => {
      this.running = null;
    });
    return this.running;
  }

  /**
   * The operator came back to the app (a window took the keyboard). A look an
   * hour old or older is repeated, so the update that is ready, or about to
   * be, is the newest release rather than one a later release passed while
   * the app stayed open. Before the launch look nothing happens: that look
   * waits for the window and the core to settle.
   */
  returned(): void {
    if (this.lookedAt === null || this.io.now() - this.lookedAt < LOOK_AGAIN_AFTER_MS) return;
    void this.check("return");
  }

  /**
   * The last window closed on macOS, where the app would otherwise keep
   * running with a ready update nobody installs. True when the caller should
   * quit, which installs it (`autoInstallOnAppQuit`); the next launch is the
   * new version. Any other system already quits with its last window.
   */
  installOnWindowClose(): boolean {
    if (this.system !== "macos" || this.phase.kind !== "ready" || this.installing || this.stopped) return false;
    this.log.event("update.install", { version: this.phase.version, trigger: "window_closed" });
    return true;
  }

  /** Installs the ready update and reopens the app; nothing happens unless one is ready. */
  restart(): void {
    if (!this.updater || this.phase.kind !== "ready" || this.installing || this.stopped) return;
    this.installing = true;
    this.log.event("update.install", { version: this.phase.version, trigger: "restart" });
    // Silent, and opening the new version once installed.
    this.updater.quitAndInstall(true, true);
  }

  setPrerelease(on: boolean): void {
    if (on === this.prerelease) return;
    this.prerelease = on;
    this.saveChoices();
    this.log.event("update.prerelease", { on });
    this.emit();
  }

  /**
   * Closes the ready card for `version`; Help's dot and line and About stay.
   * Only the version that is ready now can be closed, so a newer one shows
   * the card again, and a close that lost a race with a newer download changes nothing.
   * A required update's card cannot be closed.
   */
  dismiss(version: string): void {
    if (this.required || this.phase.kind !== "ready" || this.phase.version !== version || this.dismissed === version) return;
    this.dismissed = version;
    this.saveChoices();
    this.log.event("update.card_dismissed", { version });
    this.emit();
  }

  /** A choice that cannot be saved still holds for this launch; the log says so. */
  private saveChoices(): void {
    try {
      this.io.saveChoices({ prerelease: this.prerelease, dismissed: this.dismissed });
    } catch (error) {
      this.log.event("update.choices_save_failed", { detail: errorDetail(error) });
    }
  }

  private schedule(ms: number): void {
    if (this.stopped) return;
    this.cancelTimer?.();
    this.cancelTimer = this.io.later(() => {
      void this.check(ms === FIRST_CHECK_DELAY_MS ? "launch" : "interval").finally(() => this.schedule(CHECK_INTERVAL_MS));
    }, ms);
  }

  private async run(trigger: string, system: UpdateSystem, updater: UpdaterLike): Promise<void> {
    const ready = this.phase.kind === "ready" ? this.phase : null;
    this.lookedAt = this.io.now();
    if (!ready) this.set({ kind: "checking" });
    this.log.event("update.check", { trigger, version: this.current, prerelease: this.prerelease });
    let version: string;
    try {
      const releases = this.feedOverride ? null : await this.io.releases();
      if (releases !== null) this.readMinimum(releases);
      const chosen = releases === null ? null : chooseRelease(releases, { current: this.current, prerelease: this.prerelease, system });
      if (!this.feedOverride && (!chosen || chosen.version === ready?.version)) {
        this.log.event("update.none", { latest: chosen?.version ?? null });
        if (!ready) this.set({ kind: "idle", checkedAt: this.io.now() });
        return;
      }
      // Restart or quit may have begun while the list was read; a download now would empty the installer in use.
      if (this.installing || this.stopped) return;
      updater.setFeedURL({ provider: "generic", url: this.feedOverride ?? `${RELEASE_DOWNLOADS}/${chosen!.tag}` });
      const result = await updater.checkForUpdates();
      if (!result?.isUpdateAvailable || result.updateInfo.version === ready?.version) {
        this.log.event("update.none", { latest: result?.updateInfo.version ?? null });
        if (!ready) this.set({ kind: "idle", checkedAt: this.io.now() });
        return;
      }
      version = result.updateInfo.version;
      // The update file must describe the release chosen from GitHub's list, not another one.
      if (chosen && version !== chosen.version) throw new Error(`the update file of ${chosen.tag} names version ${version}`);
    } catch (error) {
      this.log.event("update.check_failed", { detail: errorDetail(error) });
      if (!ready) this.set({ kind: "failed", stage: "check" });
      return;
    }
    if (this.installing || this.stopped) return;
    this.log.event("update.found", { version });
    // electron-updater keeps one pending update and empties it when another
    // download starts, so a newer release replaces the ready one from here:
    // the row shows that download, and the line names the newer version once
    // it is ready (B19). Showing the older one as ready meanwhile would offer
    // a Restart with nothing to install.
    if (ready) this.log.event("update.replacing", { ready: ready.version, version });
    this.set({ kind: "downloading", version, percent: 0 });
    try {
      await updater.downloadUpdate();
    } catch (error) {
      // A file whose sha512 (or macOS signature) does not match is discarded
      // by electron-updater and lands here; nothing was installed.
      this.log.event("update.download_failed", { version, detail: errorDetail(error) });
      this.set({ kind: "failed", stage: "download" });
      return;
    }
    this.log.event("update.ready", { version });
    this.set({ kind: "ready", version });
  }

  /** Whether a release now requires an update of this app; the log and the card follow a change. */
  private readMinimum(releases: unknown): void {
    const minimum = minimumVersion(releases, this.prerelease);
    const current = parseVersion(this.current);
    const parsed = minimum === null ? null : parseVersion(minimum);
    const required = current !== null && parsed !== null && compareVersions(current, parsed) < 0;
    if (required === this.required) return;
    this.required = required;
    this.log.event(required ? "update.required" : "update.no_longer_required", { version: this.current, minimum });
    this.emit();
  }

  private set(phase: UpdatePhase): void {
    this.phase = phase;
    this.emit();
  }

  private emit(): void {
    const status = this.status();
    for (const listener of this.listeners) listener(status);
  }
}

/** A failure for the log: electron-updater's code and message, never a response body. */
function errorDetail(error: unknown): string {
  if (error instanceof Error) {
    const code = (error as Error & { code?: unknown }).code;
    return `${typeof code === "string" ? `${code}: ` : ""}${error.message}`.slice(0, 500);
  }
  return String(error).slice(0, 500);
}

/** GitHub's release list over Electron's network stack (the system's proxy), bounded in time and size. */
export async function fetchReleases(fetchImpl: (url: string, init: RequestInit) => Promise<Response>): Promise<unknown> {
  const response = await fetchImpl(RELEASES_API, {
    headers: { Accept: "application/vnd.github+json", "X-GitHub-Api-Version": "2022-11-28" },
    signal: AbortSignal.timeout(RELEASES_TIMEOUT_MS),
  });
  if (!response.ok) {
    await response.body?.cancel();
    throw new Error(`GitHub answered the release list with HTTP ${response.status}`);
  }
  const declared = Number(response.headers.get("content-length") ?? "0");
  if (declared > RELEASES_BYTES_CAP) {
    await response.body?.cancel();
    throw new Error("GitHub's release list exceeds 8 MiB");
  }
  const body = await response.arrayBuffer();
  if (body.byteLength > RELEASES_BYTES_CAP) throw new Error("GitHub's release list exceeds 8 MiB");
  return JSON.parse(new TextDecoder().decode(body));
}

/** The Linux package type electron-builder writes beside the app (`resources/package-type`), or null. */
export function readPackageType(resources: string): string | null {
  try {
    return fs.readFileSync(path.join(resources, "package-type"), "utf8").trim();
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "ENOENT") return null;
    throw error;
  }
}

export function saveUpdateChoices(file: string, choices: UpdateChoices): void {
  writeJsonFile(file, choices);
}
