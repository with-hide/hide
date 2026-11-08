import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { describe, expect, it } from "vitest";
import type { UpdateStatus } from "../../../web/src/host";
import type { HostLog } from "./log";
import { AppUpdates, chooseRelease, CHECK_INTERVAL_MS, LOOK_AGAIN_AFTER_MS, FIRST_CHECK_DELAY_MS, macosLocation, readUpdateChoices, saveUpdateChoices, updatesFile, updateSystem, type UpdateChoices, type UpdaterLike, type UpdateSystem } from "./updater";

const uploaded = (name: string) => ({ name, state: "uploaded" });

function release(tag: string, options: { prerelease?: boolean; draft?: boolean; files?: string[]; uploading?: string[] } = {}) {
  const version = tag.slice(1);
  const files = options.files ?? [`latest-mac.yml`, `hide-v${version}-macos-arm64.zip`, `latest.yml`, `hide-v${version}-windows-x64-setup.exe`, `latest-linux.yml`, `hide-v${version}-linux-x64.deb`, `hide-v${version}-linux-x64.rpm`];
  return {
    tag_name: tag,
    draft: options.draft ?? false,
    prerelease: options.prerelease ?? false,
    assets: [...files.map(uploaded), ...(options.uploading ?? []).map((name) => ({ name, state: "new" }))],
  };
}

const choose = (releases: unknown[], current: string, prerelease = false, system: UpdateSystem = "macos") => chooseRelease(releases, { current, prerelease, system });

describe("the release an update takes (D-22, D-23)", () => {
  it("takes the highest newer stable release", () => {
    expect(choose([release("v0.3.0"), release("v0.4.1"), release("v0.4.0")], "0.3.0")).toEqual({ tag: "v0.4.1", version: "0.4.1" });
  });

  it("takes nothing at or below this version", () => {
    expect(choose([release("v0.3.0"), release("v0.2.9")], "0.3.0")).toBeNull();
  });

  it("skips a tag that is not a release version, such as an evidence tag", () => {
    expect(choose([{ ...release("v0.9.0"), tag_name: "evidence-pr45" }, release("v0.4.0")], "0.3.0")).toEqual({ tag: "v0.4.0", version: "0.4.0" });
  });

  it("skips a draft and a release whose files are still uploading", () => {
    const uploading = release("v0.5.0", { files: ["latest-mac.yml"], uploading: ["hide-v0.5.0-macos-arm64.zip"] });
    expect(choose([release("v0.6.0", { draft: true }), uploading, release("v0.4.0")], "0.3.0")).toEqual({ tag: "v0.4.0", version: "0.4.0" });
  });

  it("needs this system's own file, not another system's", () => {
    const macOnly = release("v0.5.0", { files: ["latest-mac.yml", "hide-v0.5.0-macos-arm64.zip"] });
    expect(choose([macOnly], "0.3.0", false, "macos")).toEqual({ tag: "v0.5.0", version: "0.5.0" });
    expect(choose([macOnly], "0.3.0", false, "windows")).toBeNull();
    expect(choose([macOnly], "0.3.0", false, "rpm")).toBeNull();
  });

  it("takes a preview release only with preview releases on, and only one marked as a pre-release", () => {
    const rc = release("v0.5.0-rc.2", { prerelease: true });
    expect(choose([rc, release("v0.4.0")], "0.3.0")).toEqual({ tag: "v0.4.0", version: "0.4.0" });
    expect(choose([rc, release("v0.4.0")], "0.3.0", true)).toEqual({ tag: "v0.5.0-rc.2", version: "0.5.0-rc.2" });
    expect(choose([release("v0.5.0-rc.2"), release("v0.4.0")], "0.3.0", true)).toEqual({ tag: "v0.4.0", version: "0.4.0" });
  });

  it("orders a release above every rc of its version, and never goes back from an rc", () => {
    expect(choose([release("v0.5.0"), release("v0.5.0-rc.3", { prerelease: true })], "0.5.0-rc.1", true)).toEqual({ tag: "v0.5.0", version: "0.5.0" });
    // Preview releases turned off on an rc: the stable release of that version is still newer.
    expect(choose([release("v0.5.0")], "0.5.0-rc.1")).toEqual({ tag: "v0.5.0", version: "0.5.0" });
    // Turned off on an rc above every stable release: nothing until a higher stable one.
    expect(choose([release("v0.4.0")], "0.5.0-rc.1")).toBeNull();
  });

  it("looks for nothing from a development build", () => {
    expect(choose([release("v9.0.0")], "0.1.0-main.6369ab")).toBeNull();
  });
});

describe("whether this app updates itself (D-27)", () => {
  const base = { packaged: true, version: "0.4.0", platform: "darwin" as NodeJS.Platform, packageType: null, macosLocation: "ok" as const };

  it("updates a packaged release on each system", () => {
    expect(updateSystem(base)).toBe("macos");
    expect(updateSystem({ ...base, platform: "win32" })).toBe("windows");
    expect(updateSystem({ ...base, platform: "linux", packageType: "deb" })).toBe("deb");
    expect(updateSystem({ ...base, platform: "linux", packageType: "rpm" })).toBe("rpm");
  });

  it("leaves a development build, an unpackaged app and an unpackaged Linux app alone", () => {
    expect(updateSystem({ ...base, version: "0.1.0-main.6369ab" })).toEqual({ off: "development" });
    expect(updateSystem({ ...base, packaged: false })).toEqual({ off: "development" });
    expect(updateSystem({ ...base, platform: "linux", packageType: null })).toEqual({ off: "development" });
    expect(updateSystem({ ...base, platform: "linux", packageType: "AppImage" })).toEqual({ off: "development" });
  });

  it("asks a macOS app on the disk image or a read-only place to be moved (B27)", () => {
    expect(updateSystem({ ...base, macosLocation: "read_only" })).toEqual({ off: "location" });
    const writable = () => true;
    expect(macosLocation("/Applications/hide.app", writable)).toBe("ok");
    expect(macosLocation("/Volumes/hide 0.4.0/hide.app", writable)).toBe("read_only");
    expect(macosLocation("/private/var/folders/x/AppTranslocation/ABC/d/hide.app", writable)).toBe("read_only");
    expect(macosLocation("/Applications/hide.app", () => false)).toBe("read_only");
  });
});

type Deferred<T> = { promise: Promise<T>; resolve: (value: T) => void; reject: (error: unknown) => void };
function deferred<T>(): Deferred<T> {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

/** electron-updater as far as AppUpdates drives it: each call answers what the test queued. */
class FakeUpdater {
  autoDownload = true;
  autoInstallOnAppQuit = true;
  allowDowngrade = true;
  feeds: string[] = [];
  installs: [boolean | undefined, boolean | undefined][] = [];
  available: string | null = null;
  download: Deferred<string[]> | null = null;
  checkError: Error | null = null;
  readonly handlers = new Map<string, (...args: never[]) => void>();
  setFeedURL(options: { url: string }) { this.feeds.push(options.url); }
  async checkForUpdates() {
    if (this.checkError) throw this.checkError;
    return { isUpdateAvailable: this.available !== null, updateInfo: { version: this.available ?? "0.0.0" } };
  }
  downloadUpdate() {
    this.download = deferred<string[]>();
    return this.download.promise;
  }
  quitAndInstall(silent?: boolean, run?: boolean) { this.installs.push([silent, run]); }
  on(event: string, handler: (...args: never[]) => void) { this.handlers.set(event, handler); return this; }
  emit(event: string, value: unknown) { (this.handlers.get(event) as (value: unknown) => void)(value); }
}

function harness(options: { system?: UpdateSystem | { off: "development" | "location" }; releases?: unknown[]; prerelease?: boolean; dismissed?: string | null; feedOverride?: string | null; saveError?: Error } = {}) {
  const updater = new FakeUpdater();
  const events: [string, Record<string, unknown>][] = [];
  const timers: { run: () => void; ms: number; cancelled: boolean }[] = [];
  const saved: UpdateChoices[] = [];
  let releases: unknown = options.releases ?? [];
  let releasesError: Error | null = null;
  let clock = 1_000;
  const log = { event: (event: string, fields: Record<string, unknown> = {}) => events.push([event, fields]) } as unknown as HostLog;
  const updates = new AppUpdates(options.system ?? "macos", "0.4.0", updater as unknown as UpdaterLike, {
    releases: async () => { if (releasesError) throw releasesError; return releases; },
    now: () => clock,
    later: (run, ms) => { const timer = { run, ms, cancelled: false }; timers.push(timer); return () => { timer.cancelled = true; }; },
    saveChoices: (choices) => { if (options.saveError) throw options.saveError; saved.push(choices); },
  }, log, { prerelease: options.prerelease ?? false, dismissed: options.dismissed ?? null, feedOverride: options.feedOverride ?? null });
  const seen: UpdateStatus[] = [];
  updates.onChange((status) => seen.push(status));
  return {
    updates, updater, events, timers, saved, seen,
    setReleases: (value: unknown[]) => { releases = value; },
    failReleases: (error: Error | null) => { releasesError = error; },
    setNow: (ms: number) => { clock = ms; },
    phases: () => seen.map((status) => status.phase.kind),
  };
}

describe("the updater's states (D-17 to D-21)", () => {
  it("never downloads on its own, and installs on quit except from a Linux package", () => {
    expect(harness().updater).toMatchObject({ autoDownload: false, allowDowngrade: false, autoInstallOnAppQuit: true });
    expect(harness({ system: "windows" }).updates.status().installOnQuit).toBe(true);
    for (const system of ["deb", "rpm"] as const) {
      const { updater, updates } = harness({ system });
      expect(updater.autoInstallOnAppQuit).toBe(false);
      expect(updates.status().installOnQuit).toBe(false);
    }
  });

  it("checks, downloads from the chosen release, and is ready to restart into it", async () => {
    const { updates, updater, phases, events } = harness({ releases: [release("v0.5.0")] });
    updater.available = "0.5.0";
    const check = updates.check("operator");
    await settle(() => updater.download !== null);
    expect(updater.feeds).toEqual(["https://github.com/with-hide/hide/releases/download/v0.5.0"]);
    expect(updates.status().phase).toEqual({ kind: "downloading", version: "0.5.0", percent: 0 });
    updater.emit("download-progress", { percent: 41.6 });
    expect(updates.status().phase).toEqual({ kind: "downloading", version: "0.5.0", percent: 42 });
    updater.download!.resolve([]);
    await check;
    expect(updates.status().phase).toEqual({ kind: "ready", version: "0.5.0" });
    expect(phases()).toEqual(["checking", "downloading", "downloading", "ready"]);
    expect(events.map(([event]) => event)).toEqual(["update.check", "update.found", "update.ready"]);
    updates.restart();
    updates.restart();
    expect(updater.installs).toEqual([[true, true]]);
  });

  it("answers the latest version when no release is newer, with the time it looked", async () => {
    const { updates, updater } = harness({ releases: [release("v0.4.0")] });
    await updates.check("operator");
    expect(updates.status().phase).toEqual({ kind: "idle", checkedAt: 1_000 });
    expect(updater.feeds).toEqual([]);
  });

  it("shows a failed check and a failed download in the row, and logs the reason", async () => {
    const failing = harness();
    failing.failReleases(new Error("HTTP 503"));
    await failing.updates.check("operator");
    expect(failing.updates.status().phase).toEqual({ kind: "failed", stage: "check" });
    expect(failing.events.at(-1)).toEqual(["update.check_failed", { detail: "HTTP 503" }]);

    const download = harness({ releases: [release("v0.5.0")] });
    download.updater.available = "0.5.0";
    const check = download.updates.check("operator");
    await settle(() => download.updater.download !== null);
    download.updater.download!.reject(Object.assign(new Error("sha512 checksum mismatch"), { code: "ERR_CHECKSUM_MISMATCH" }));
    await check;
    expect(download.updates.status().phase).toEqual({ kind: "failed", stage: "download" });
    expect(download.events.at(-1)).toEqual(["update.download_failed", { version: "0.5.0", detail: "ERR_CHECKSUM_MISMATCH: sha512 checksum mismatch" }]);
  });

  it("keeps a ready update ready while a later check looks, and downloads a newer one in its place (B19)", async () => {
    const { updates, updater, setReleases, failReleases, events } = harness({ releases: [release("v0.5.0")] });
    updater.available = "0.5.0";
    const first = updates.check("operator");
    await settle(() => updater.download !== null);
    updater.download!.resolve([]);
    await first;

    // The same release again, and a failed look: the line still names 0.5.0.
    await updates.check("interval");
    expect(updates.status().phase).toEqual({ kind: "ready", version: "0.5.0" });
    failReleases(new Error("offline"));
    await updates.check("interval");
    expect(updates.status().phase).toEqual({ kind: "ready", version: "0.5.0" });

    failReleases(null);
    setReleases([release("v0.5.1"), release("v0.5.0")]);
    updater.available = "0.5.1";
    updater.download = null;
    const second = updates.check("interval");
    await settle(() => updater.download !== null);
    // electron-updater keeps one pending update, so the newer download replaces the ready one.
    expect(updates.status().phase).toEqual({ kind: "downloading", version: "0.5.1", percent: 0 });
    updater.download!.resolve([]);
    await second;
    expect(updates.status().phase).toEqual({ kind: "ready", version: "0.5.1" });
    expect(events.map(([event]) => event)).toContain("update.replacing");
  });

  it("stops a running check before it downloads once Restart or quit has begun", async () => {
    for (const end of ["restart", "stop"] as const) {
      const { updates, updater, setReleases } = harness({ releases: [release("v0.5.0")] });
      updater.available = "0.5.0";
      const first = updates.check("operator");
      await settle(() => updater.download !== null);
      updater.download!.resolve([]);
      await first;
      setReleases([release("v0.5.1")]);
      updater.available = "0.5.1";
      updater.download = null;
      const second = updates.check("interval");
      if (end === "restart") updates.restart();
      else updates.stop();
      await second;
      expect(updater.download).toBeNull();
      expect(updater.feeds).toHaveLength(1);
      expect(updates.status().phase).toEqual({ kind: "ready", version: "0.5.0" });
    }
  });

  it("schedules nothing more once stopped, even when a check ends after", async () => {
    const { updates, timers } = harness();
    updates.start();
    timers[0]!.run();
    updates.stop();
    await settle(() => true);
    await new Promise((resolve) => setImmediate(resolve));
    expect(timers).toHaveLength(1);
  });

  it("refuses an update file that names another version than the chosen release", async () => {
    const { updates, updater, events } = harness({ releases: [release("v0.5.0")] });
    updater.available = "0.6.0";
    await updates.check("operator");
    expect(updater.download).toBeNull();
    expect(updates.status().phase).toEqual({ kind: "failed", stage: "check" });
    expect(events.at(-1)).toEqual(["update.check_failed", { detail: "the update file of v0.5.0 names version 0.6.0" }]);
  });

  it("runs one check at a time", async () => {
    const { updates, updater } = harness({ releases: [release("v0.5.0")] });
    updater.available = "0.5.0";
    const first = updates.check("operator");
    const second = updates.check("operator");
    expect(second).toBe(first);
    await settle(() => updater.download !== null);
    updater.download!.resolve([]);
    await first;
    expect(updater.feeds).toHaveLength(1);
  });

  it("restarts into nothing unless an update is ready, and keeps it ready when the install fails", async () => {
    const { updates, updater, events } = harness({ releases: [release("v0.5.0")] });
    updates.restart();
    expect(updater.installs).toEqual([]);
    updater.available = "0.5.0";
    const check = updates.check("operator");
    await settle(() => updater.download !== null);
    updater.download!.resolve([]);
    await check;
    updates.restart();
    updater.emit("error", new Error("pkexec dismissed"));
    expect(updates.status().phase).toEqual({ kind: "ready", version: "0.5.0" });
    expect(events.at(-1)).toEqual(["update.install_failed", { detail: "pkexec dismissed" }]);
    updates.restart();
    expect(updater.installs).toHaveLength(2);
  });

  it("reads a test feed in place of GitHub (D-29), and says so in the log at start", async () => {
    const started = harness({ feedOverride: "http://127.0.0.1:8123/" });
    started.updates.start();
    expect(started.events[0]).toEqual(["update.feed_override", { url: "http://127.0.0.1:8123/" }]);
    started.updates.stop();
    const { updates, updater } = harness({ feedOverride: "http://127.0.0.1:8123/" });
    updater.available = "0.5.0";
    const check = updates.check("operator");
    await settle(() => updater.download !== null);
    updater.download!.resolve([]);
    await check;
    expect(updater.feeds).toEqual(["http://127.0.0.1:8123/"]);
    expect(updates.status().phase).toEqual({ kind: "ready", version: "0.5.0" });
  });

  it("looks after launch and then every six hours, until stopped", async () => {
    const { updates, timers, events } = harness();
    updates.start();
    expect(timers.map((timer) => timer.ms)).toEqual([FIRST_CHECK_DELAY_MS]);
    timers[0]!.run();
    await settle(() => timers.length === 2);
    expect(timers[1]!.ms).toBe(CHECK_INTERVAL_MS);
    expect(events[0]).toEqual(["update.check", { trigger: "launch", version: "0.4.0", prerelease: false }]);
    updates.stop();
    expect(timers[1]!.cancelled).toBe(true);
  });

  it("looks again when the operator comes back an hour after the last look, and replaces a ready update with a newer one", async () => {
    const { updates, updater, events, setReleases, setNow } = harness({ releases: [release("v0.5.0")] });
    // Before the launch look, coming back waits for it.
    updates.returned();
    expect(events).toEqual([]);
    updater.available = "0.5.0";
    const first = updates.check("launch");
    await settle(() => updater.download !== null);
    updater.download!.resolve([]);
    await first;
    expect(updates.status().phase).toEqual({ kind: "ready", version: "0.5.0" });

    // A newer release came out while the app stayed open with 0.5.0 ready.
    setReleases([release("v0.5.1"), release("v0.5.0")]);
    updater.available = "0.5.1";
    updater.download = null;
    setNow(1_000 + LOOK_AGAIN_AFTER_MS - 1);
    updates.returned();
    await Promise.resolve();
    expect(updater.download).toBeNull();
    setNow(1_000 + LOOK_AGAIN_AFTER_MS);
    updates.returned();
    await settle(() => updater.download !== null);
    expect(events.filter(([event]) => event === "update.check").at(-1)).toEqual(["update.check", { trigger: "return", version: "0.4.0", prerelease: false }]);
    updater.download!.resolve([]);
    await settle(() => updates.status().phase.kind === "ready");
    expect(updates.status().phase).toEqual({ kind: "ready", version: "0.5.1" });
  });

  it("installs a ready update when the last window closes on macOS, and only then", async () => {
    const idle = harness();
    expect(idle.updates.installOnWindowClose()).toBe(false);
    for (const system of ["macos", "windows"] as const) {
      const { updates, updater, events } = harness({ system, releases: [release("v0.5.0")] });
      updater.available = "0.5.0";
      const check = updates.check("operator");
      await settle(() => updater.download !== null);
      updater.download!.resolve([]);
      await check;
      // Windows already quits with its last window; only macOS needs the answer.
      expect(updates.installOnWindowClose()).toBe(system === "macos");
      if (system === "macos") expect(events.at(-1)).toEqual(["update.install", { version: "0.5.0", trigger: "window_closed" }]);
    }
  });

  it("never looks when off, and says why in the log", async () => {
    const { updates, timers, events, updater } = harness({ system: { off: "location" } });
    updates.start();
    await updates.check("operator");
    expect(timers).toEqual([]);
    expect(updater.feeds).toEqual([]);
    expect(updates.status().phase).toEqual({ kind: "off", reason: "location" });
    expect(events).toEqual([["update.off", { reason: "location", version: "0.4.0" }]]);
  });

  it("keeps the preview choice and takes rc releases from the next check", async () => {
    const { updates, updater, saved, seen } = harness({ releases: [release("v0.5.0-rc.1", { prerelease: true })] });
    await updates.check("operator");
    expect(updater.feeds).toEqual([]);
    updates.setPrerelease(true);
    updates.setPrerelease(true);
    expect(saved).toEqual([{ prerelease: true, dismissed: null }]);
    expect(seen.at(-1)?.prerelease).toBe(true);
    updater.available = "0.5.0-rc.1";
    const check = updates.check("operator");
    await settle(() => updater.download !== null);
    expect(updater.feeds).toEqual(["https://github.com/with-hide/hide/releases/download/v0.5.0-rc.1"]);
    updater.download!.resolve([]);
    await check;
  });
});

describe("the ready card's close", () => {
  /** A harness whose update `version` is downloaded and ready. */
  async function ready(version: string, options: Parameters<typeof harness>[0] = {}) {
    const subject = harness({ releases: [release(`v${version}`)], ...options });
    await readyAgain(subject, version);
    return subject;
  }
  async function readyAgain(subject: ReturnType<typeof harness>, version: string) {
    subject.setReleases([release(`v${version}`)]);
    subject.updater.available = version;
    subject.updater.download = null;
    const check = subject.updates.check("operator");
    await settle(() => subject.updater.download !== null);
    subject.updater.download!.resolve([]);
    await check;
    expect(subject.updates.status().phase).toEqual({ kind: "ready", version });
  }

  it("closes the card for the ready version only, keeps the update ready, and saves the close with the preview choice", async () => {
    const subject = await ready("0.5.0", { prerelease: true });
    const { updates, updater, saved, seen } = subject;
    expect(updates.status().dismissed).toBeNull();
    // A close for a version that is not the ready one (a page behind a newer download) changes nothing.
    updates.dismiss("0.4.9");
    expect(saved).toEqual([]);
    updates.dismiss("0.5.0");
    updates.dismiss("0.5.0");
    expect(saved).toEqual([{ prerelease: true, dismissed: "0.5.0" }]);
    expect(seen.at(-1)).toMatchObject({ phase: { kind: "ready", version: "0.5.0" }, dismissed: "0.5.0" });
    // The ↑ button, Help and About still restart into it.
    updates.restart();
    expect(updater.installs).toEqual([[true, true]]);
  });

  it("names the closed version while a newer one downloads and is ready, so the card can come back for it", async () => {
    const subject = await ready("0.5.0");
    subject.updates.dismiss("0.5.0");
    await readyAgain(subject, "0.5.1");
    expect(subject.updates.status()).toMatchObject({ phase: { kind: "ready", version: "0.5.1" }, dismissed: "0.5.0" });
  });

  it("refuses a close while nothing is ready", () => {
    const { updates, saved } = harness();
    updates.dismiss("0.5.0");
    expect(updates.status().dismissed).toBeNull();
    expect(saved).toEqual([]);
  });

  it("starts with the close a launch before saved", () => {
    expect(harness({ dismissed: "0.5.0" }).updates.status().dismissed).toBe("0.5.0");
  });

  it("logs a close that could not be saved and still holds it for this launch", async () => {
    const { updates, events } = await ready("0.5.0", { saveError: new Error("EACCES") });
    updates.dismiss("0.5.0");
    expect(updates.status().dismissed).toBe("0.5.0");
    expect(events.slice(-2)).toEqual([["update.choices_save_failed", { detail: "EACCES" }], ["update.card_dismissed", { version: "0.5.0" }]]);
  });
});

describe("the update choices file", () => {
  it("reads what it saved, an earlier build's preview-only file, and nothing from a missing or broken one", () => {
    const folder = fs.mkdtempSync(path.join(os.tmpdir(), "hide-update-choices-"));
    try {
      const file = updatesFile(folder);
      expect(readUpdateChoices(file)).toEqual({ prerelease: false, dismissed: null });
      fs.writeFileSync(file, JSON.stringify({ prerelease: true }));
      expect(readUpdateChoices(file)).toEqual({ prerelease: true, dismissed: null });
      saveUpdateChoices(file, { prerelease: true, dismissed: "0.5.0-rc.2" });
      expect(readUpdateChoices(file)).toEqual({ prerelease: true, dismissed: "0.5.0-rc.2" });
      fs.writeFileSync(file, JSON.stringify({ prerelease: "yes", dismissed: "../latest" }));
      expect(readUpdateChoices(file)).toEqual({ prerelease: false, dismissed: null });
      fs.writeFileSync(file, "{");
      expect(readUpdateChoices(file)).toEqual({ prerelease: false, dismissed: null });
    } finally {
      fs.rmSync(folder, { recursive: true, force: true });
    }
  });
});

/** Lets queued promise callbacks run until `condition` holds. */
async function settle(condition: () => boolean): Promise<void> {
  for (let turn = 0; turn < 50; turn += 1) {
    if (condition()) return;
    await new Promise((resolve) => setImmediate(resolve));
  }
  throw new Error("condition never held");
}
