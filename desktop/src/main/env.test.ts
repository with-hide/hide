import fs from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";
import { ENV_REGISTRY, loadEnv } from "./env";

const SRC = path.resolve(__dirname, "..");

function sources(dir: string): string[] {
  return fs.readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) return sources(full);
    return entry.name.endsWith(".ts") && !entry.name.endsWith(".test.ts") ? [full] : [];
  });
}

describe("the environment registry", () => {
  it("is the only reader of process.env, and every key it reads is registered", () => {
    const readers = sources(SRC).filter((file) => fs.readFileSync(file, "utf8").includes("process.env"));
    expect(readers.map((file) => path.relative(SRC, file).split(path.sep).join("/"))).toEqual(["main/index.ts"]);
    const envSource = fs.readFileSync(path.join(SRC, "main/env.ts"), "utf8");
    const read = [...envSource.matchAll(/(?:absolute|windows|feed)\("([A-Za-z_]+)"|source\.([A-Za-z_]+)/g)].map((match) => match[1] ?? match[2]);
    const registered = ENV_REGISTRY.map((entry) => entry.key);
    for (const key of read) expect(registered, key).toContain(key);
  });

  it("names every bad key at once and never echoes a value", () => {
    expect(() => loadEnv({ HIDE_CLI_PATH: "relative/hide", HIDE_DESKTOP_USER_DATA_DIR: "also-relative" })).toThrow(
      "desktop environment is not usable: HIDE_CLI_PATH: not an absolute path; HIDE_DESKTOP_USER_DATA_DIR: not an absolute path",
    );
    // A rooted path without a drive names a different file per current drive on Windows, and is relative elsewhere.
    expect(() => loadEnv({ HERDR_BIN_PATH: "\\herdr" })).toThrow("HERDR_BIN_PATH: not an absolute path");
  });

  it("takes an update feed only as an http or https URL", () => {
    expect(loadEnv({ HIDE_UPDATE_FEED_URL: "http://127.0.0.1:8080/feed" }).updateFeedUrl).toBe("http://127.0.0.1:8080/feed");
    for (const value of ["file:///tmp/feed", "not a url"]) expect(() => loadEnv({ HIDE_UPDATE_FEED_URL: value })).toThrow("HIDE_UPDATE_FEED_URL: not an http or https URL");
    for (const value of ["https://example.test/feed", "http://10.0.0.2:8080/"]) expect(() => loadEnv({ HIDE_UPDATE_FEED_URL: value })).toThrow("HIDE_UPDATE_FEED_URL: not a loopback address");
  });

  it.runIf(process.platform !== "win32")("reads the macOS and Linux keys", () => {
    const source = { PATH: "/bin", HOME: "/h", SHELL: "/bin/sh", HIDE_STATE_DIR: "/s", LOCALAPPDATA: "/l" };
    expect(loadEnv(source)).toEqual({
      cliPath: null,
      userDataDir: null,
      herdrBinPath: null,
      herdrPaneId: null,
      ownEngine: false,
      systemLanguage: null,
      updateFeedUrl: null,
      shell: "/bin/sh",
      home: "/h",
      localAppData: null,
      appData: null,
      programFiles: null,
      systemRoot: null,
      path: "/bin",
      inherited: source,
    });
    expect(loadEnv({ ...source, HERDR_BIN_PATH: "/opt/herdr" }).herdrBinPath).toBe("/opt/herdr");
    expect(loadEnv({ ...source, HERDR_PANE_ID: "w1:p1" }).herdrPaneId).toBe("w1:p1");
    expect(loadEnv({ ...source, HIDE_DEV_PANE_HOLDER: "1" }).ownEngine).toBe(true);
    expect(loadEnv({ ...source, HIDE_DEV_PANE_HOLDER: "0" }).ownEngine).toBe(false);
    expect(() => loadEnv({ ...source, HERDR_BIN_PATH: "herdr" })).toThrow("HERDR_BIN_PATH: not an absolute path");
  });

  it.runIf(process.platform === "win32")("reads the Windows keys, and no login shell", () => {
    const source = {
      PATH: "C:\\Windows",
      USERPROFILE: "C:\\Users\\example",
      HOME: "/c/Users/example",
      SHELL: "/usr/bin/bash",
      LOCALAPPDATA: "C:\\Users\\example\\AppData\\Local",
      APPDATA: "C:\\Users\\example\\AppData\\Roaming",
      ProgramFiles: "C:\\Program Files",
      SystemRoot: "C:\\Windows",
    };
    expect(loadEnv(source)).toEqual({
      cliPath: null,
      userDataDir: null,
      herdrBinPath: null,
      herdrPaneId: null,
      ownEngine: false,
      systemLanguage: null,
      updateFeedUrl: null,
      shell: null,
      home: "C:\\Users\\example",
      localAppData: "C:\\Users\\example\\AppData\\Local",
      appData: "C:\\Users\\example\\AppData\\Roaming",
      programFiles: "C:\\Program Files",
      systemRoot: "C:\\Windows",
      path: "C:\\Windows",
      inherited: source,
    });
    expect(() => loadEnv({ ...source, USERPROFILE: "\\Users\\example", ProgramFiles: "Program Files" })).toThrow(
      "desktop environment is not usable: USERPROFILE: not an absolute path; ProgramFiles: not an absolute path",
    );
  });
});
