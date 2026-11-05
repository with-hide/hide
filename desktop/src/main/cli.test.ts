import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { describe, expect, it } from "vitest";
import {
  cliRecords,
  HAS_LOGIN_SHELL,
  healthFailure,
  loginPathCommand,
  parseConnect,
  parseLoginPath,
  parseRememberedCli,
  parseRuntimePrepare,
  parseStatus,
  rememberedCliPath,
  rememberedCliValue,
  REMEMBERED_SOURCES,
  resolveCli,
  systemDirs,
  wellKnownDirs,
  type CliSearch,
  type Locations,
  type FileProbe,
} from "./cli";
import { readJsonFile, writeJsonFile } from "./jsonFile";
import { ChildRunner } from "./spawn";
import type { ChildResult } from "./spawn";

const result = (stdout: string, extra: Partial<ChildResult> = {}): ChildResult => ({
  code: 0,
  signal: null,
  stdout,
  stderr: "",
  timedOut: false,
  spawnError: null,
  ...extra,
});

/** An absolute path written in POSIX for the reader, on a drive on Windows, where a path without one is not absolute. */
const abs = (posix: string): string => (process.platform === "win32" ? `C:${posix}` : posix);
/** The CLI's file name, spelled here rather than taken from the product so a wrong name fails. */
const CLI_FILE = process.platform === "win32" ? "hide.exe" : "hide";
/**
 * A path the resolver builds, written in POSIX for the reader and spelled
 * the way this system's `path.join` spells it, with the CLI's file name for
 * `hide` (`C:\w\target\debug\hide.exe` on Windows). Paths the resolver
 * only passes along (an override, the remembered CLI) are compared as given.
 */
const built = (posix: string): string => (path.posix.basename(posix) === "hide" ? path.join(path.posix.dirname(posix), CLI_FILE) : path.join(posix));
/** A PATH value as this system separates it. */
const dirs = (...entries: string[]): string => entries.join(path.delimiter);
/** Executable files, each found by the spelling the resolver builds and by the spelling it was given. */
const probe = (files: Record<string, number>): FileProbe => {
  const spelled = new Map(Object.entries(files).flatMap(([file, mtime]) => [[file, mtime] as const, [built(file), mtime] as const]));
  return {
    isExecutable: (file) => spelled.has(file),
    mtimeMs: (file) => spelled.get(file) ?? null,
  };
};

describe("resolveCli (B2)", () => {
  const input: CliSearch = { override: null, worktreeRoot: abs("/w"), bundledDir: null, searchPath: dirs(abs("/usr/local/bin"), abs("/opt/bin")), remembered: null, loginPath: null, wellKnown: [abs("/h/.local/bin"), abs("/opt/homebrew/bin"), abs("/usr/local/bin")] };
  const packaged: CliSearch = { ...input, worktreeRoot: null, searchPath: dirs(abs("/usr/bin"), abs("/bin")) };
  const login = (answer: string | null) => {
    const asked = { count: 0 };
    return { asked, loginPath: async () => (asked.count++, answer) };
  };

  it("takes the override first and never searches past an unusable one", async () => {
    expect((await resolveCli({ ...input, override: abs("/x/hide") }, probe({ [abs("/x/hide")]: 1, [abs("/w/target/debug/hide")]: 1 }))).found).toEqual({ path: abs("/x/hide"), source: "env" });
    const missing = await resolveCli({ ...input, override: abs("/x/hide"), remembered: abs("/r/hide") }, probe({ [abs("/w/target/debug/hide")]: 1, [abs("/r/hide")]: 1 }));
    expect(missing).toEqual({ found: null, tried: [abs("/x/hide")] });
  });

  it("takes the newest worktree build before PATH", async () => {
    const files = { [abs("/w/target/debug/hide")]: 10, [abs("/w/target/release/hide")]: 20, [abs("/opt/bin/hide")]: 30 };
    expect((await resolveCli(input, probe(files))).found).toEqual({ path: built(abs("/w/target/release/hide")), source: "worktree" });
  });

  it("searches PATH in order when no build exists, and reports every path it tried", async () => {
    const resolved = await resolveCli(input, probe({ [abs("/opt/bin/hide")]: 1 }));
    expect(resolved.found).toEqual({ path: built(abs("/opt/bin/hide")), source: "path" });
    expect(resolved.tried).toEqual([abs("/w/target/debug/hide"), abs("/w/target/release/hide"), abs("/usr/local/bin/hide"), abs("/opt/bin/hide")].map(built));
  });

  it("takes the CLI the app ships after the worktree build and before PATH, and never remembers it", async () => {
    const shipped: CliSearch = { ...packaged, bundledDir: abs("/App.app/Contents/Resources"), ...login(abs("/l")) };
    const bundled = await resolveCli(shipped, probe({ [abs("/App.app/Contents/Resources/hide")]: 1, [abs("/usr/bin/hide")]: 1, [abs("/l/hide")]: 1 }));
    expect(bundled).toEqual({ found: { path: built(abs("/App.app/Contents/Resources/hide")), source: "bundled" }, tried: [built(abs("/App.app/Contents/Resources/hide"))] });
    expect(REMEMBERED_SOURCES.has("bundled")).toBe(false);
    const build = await resolveCli({ ...shipped, worktreeRoot: abs("/w") }, probe({ [abs("/w/target/debug/hide")]: 1, [abs("/App.app/Contents/Resources/hide")]: 1 }));
    expect(build.found).toEqual({ path: built(abs("/w/target/debug/hide")), source: "worktree" });
    // A bundle whose CLI cannot run is logged as tried and the search goes on (D-07), so a missing bundle still reaches an installed CLI.
    const broken = await resolveCli(shipped, probe({ [abs("/usr/bin/hide")]: 1 }));
    expect(broken).toEqual({ found: { path: built(abs("/usr/bin/hide")), source: "path" }, tried: [abs("/App.app/Contents/Resources/hide"), abs("/usr/bin/hide")].map(built) });
  });

  it("prefers PATH to the remembered CLI, and the remembered CLI to asking the login shell", async () => {
    const shell = login(abs("/l"));
    const onPath = await resolveCli({ ...packaged, searchPath: abs("/opt/bin"), remembered: abs("/r/hide"), ...shell }, probe({ [abs("/opt/bin/hide")]: 1, [abs("/r/hide")]: 1 }));
    expect(onPath.found).toEqual({ path: built(abs("/opt/bin/hide")), source: "path" });
    const remembered = await resolveCli({ ...packaged, remembered: abs("/r/hide"), ...shell }, probe({ [abs("/r/hide")]: 1, [abs("/l/hide")]: 1 }));
    expect(remembered).toEqual({ found: { path: abs("/r/hide"), source: "remembered" }, tried: [built(abs("/usr/bin/hide")), built(abs("/bin/hide")), abs("/r/hide")] });
    expect(shell.asked.count).toBe(0);
  });

  it("asks the login shell once the remembered CLI is gone, skipping what PATH already tried", async () => {
    const shell = login(dirs(abs("/usr/bin"), abs("/l")));
    const resolved = await resolveCli({ ...packaged, remembered: abs("/r/hide"), ...shell }, probe({ [abs("/l/hide")]: 1 }));
    expect(resolved).toEqual({ found: { path: built(abs("/l/hide")), source: "login" }, tried: [built(abs("/usr/bin/hide")), built(abs("/bin/hide")), abs("/r/hide"), built(abs("/l/hide"))] });
    expect(shell.asked.count).toBe(1);
  });

  it("looks in the usual install directories when the login shell does not answer", async () => {
    const resolved = await resolveCli({ ...packaged, ...login(null) }, probe({ [abs("/opt/homebrew/bin/hide")]: 1 }));
    expect(resolved).toEqual({
      found: { path: built(abs("/opt/homebrew/bin/hide")), source: "well-known" },
      tried: [abs("/usr/bin/hide"), abs("/bin/hide"), abs("/h/.local/bin/hide"), abs("/opt/homebrew/bin/hide")].map(built),
    });
    const missing = await resolveCli({ ...packaged, ...login(null) }, probe({}));
    expect(missing.found).toBeNull();
    expect(missing.tried.at(-1)).toBe(built(abs("/usr/local/bin/hide")));
  });

  it("skips a PATH entry that is not absolute, a rooted one without a drive included", async () => {
    const resolved = await resolveCli({ ...input, worktreeRoot: null, searchPath: dirs("\\tools", "tools", abs("/opt/bin")) }, probe({ "\\tools\\hide.exe": 1, "\\tools/hide": 1 }));
    expect(resolved.tried.slice(0, 1)).toEqual([built(abs("/opt/bin/hide"))]);
  });

  it("never asks a login shell when there is none to ask (unpackaged)", async () => {
    expect(await resolveCli({ ...input, worktreeRoot: null, searchPath: "" }, probe({}))).toEqual({
      found: null,
      tried: [abs("/h/.local/bin/hide"), abs("/opt/homebrew/bin/hide"), abs("/usr/local/bin/hide")].map(built),
    });
  });
});

describe("the install folders", () => {
  it.runIf(process.platform !== "win32")("are the user's, Homebrew's and the local ones on macOS and Linux, with launchd's PATH for a child without one", () => {
    const at: Locations = { home: "/h", localAppData: null, appData: null, programFiles: null, systemRoot: null };
    expect(wellKnownDirs(at)).toEqual({ dirs: ["/h/.local/bin", "/opt/homebrew/bin", "/usr/local/bin"], missing: [] });
    expect(systemDirs(at)).toEqual({ dirs: ["/usr/bin", "/bin", "/usr/sbin", "/sbin"], missing: [] });
  });

  // Each folder is the one its installer documents (`wellKnownDirs` names the source).
  it.runIf(process.platform === "win32")("are each Windows installer's own folder, and a folder whose variable is unset is named, not guessed", () => {
    const at: Locations = {
      home: "C:\\Users\\example",
      localAppData: "C:\\Users\\example\\AppData\\Local",
      appData: "C:\\Users\\example\\AppData\\Roaming",
      programFiles: "C:\\Program Files",
      systemRoot: "C:\\Windows",
    };
    expect(wellKnownDirs(at)).toEqual({
      dirs: [
        "C:\\Users\\example\\.local\\bin",
        "C:\\Users\\example\\AppData\\Local\\Programs\\OpenAI\\Codex\\bin",
        "C:\\Users\\example\\AppData\\Local\\Programs\\Git\\cmd",
        "C:\\Users\\example\\AppData\\Roaming\\npm",
        "C:\\Program Files\\Git\\cmd",
        "C:\\Program Files\\GitHub CLI",
        "C:\\Program Files\\nodejs",
      ],
      missing: [],
    });
    expect(systemDirs(at)).toEqual({
      dirs: ["C:\\Windows\\System32", "C:\\Windows", "C:\\Windows\\System32\\Wbem", "C:\\Windows\\System32\\WindowsPowerShell\\v1.0", "C:\\Windows\\System32\\OpenSSH"],
      missing: [],
    });
    const bare: Locations = { home: "C:\\Users\\example", localAppData: null, appData: null, programFiles: null, systemRoot: null };
    expect(wellKnownDirs(bare)).toEqual({ dirs: ["C:\\Users\\example\\.local\\bin"], missing: ["LOCALAPPDATA", "APPDATA", "ProgramFiles"] });
    expect(systemDirs(bare)).toEqual({ dirs: [], missing: ["SystemRoot"] });
  });
});

describe("the remembered CLI", () => {
  it("reads back what the host wrote and refuses anything else", () => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "hide-cli-"));
    const file = rememberedCliPath(dir);
    expect(parseRememberedCli(readJsonFile(file))).toBeNull();
    writeJsonFile(file, rememberedCliValue(abs("/h/.local/bin/hide")));
    expect(parseRememberedCli(readJsonFile(file))).toBe(abs("/h/.local/bin/hide"));
    fs.writeFileSync(file, "{");
    expect(parseRememberedCli(readJsonFile(file))).toEqual({ unreadable: "not JSON" });
    for (const other of [{ schema: 2, path: "/a/hide" }, { schema: 1, path: "relative/hide" }, { schema: 1, path: "\\h\\hide" }, { schema: 1 }, "text"]) {
      expect(parseRememberedCli(other), JSON.stringify(other)).toEqual({ unreadable: "unexpected shape" });
    }
    fs.rmSync(dir, { recursive: true, force: true });
  });
});

describe("what hide connect did on its way", () => {
  it("keeps each record's kind and flat fields for the host log, and nothing else it printed", () => {
    const stderr = [
      "a line a library printed",
      '{"component":"hide","kind":"daemon.replace_failed","pid":721,"message":"refused","elapsed_ms":640}',
      '{"component":"hide","kind":"daemon.replacing","pid":5,"running_build":null,"nested":{"a":1}}',
      '{"no":"kind"}',
      "",
    ].join("\n");
    expect(cliRecords(stderr)).toEqual([
      { event: "daemon.replace_failed", fields: { pid: 721, message: "refused", elapsed_ms: 640 } },
      { event: "daemon.replacing", fields: { pid: 5, running_build: null } },
    ]);
  });
});

describe("the hide CLI's answers", () => {
  const url = "http://127.0.0.1:7001/#token=abc";

  it("reads an attach from the last JSON line", () => {
    expect(parseConnect(result(`noise\n{"ok":true,"url":"${url}","port":7001,"pid":42}\n`))).toEqual({
      kind: "attached",
      url,
      origin: "http://127.0.0.1:7001",
      port: 7001,
      pid: 42,
    });
  });

  it("carries the file a refused state folder names, and reads a refusal without one as unreadable", () => {
    expect(parseConnect(result('{"ok":false,"reason":"state_refused","detail":"belongs to node x","file":"/s/node.json"}', { code: 2 }))).toEqual({
      kind: "failed",
      reason: "state_refused",
      detail: "belongs to node x",
      file: "/s/node.json",
    });
    expect(parseConnect(result('{"ok":false,"reason":"state_refused","detail":"x"}', { code: 2 }))).toMatchObject({ reason: "start_failed" });
  });

  it("carries the core's machine and both builds for a newer core or a failed update, and reads one without them as unreadable", () => {
    const builds = { machine: "Mac mini", core: "0.4.2 (3f1c2a9)", app: "0.4.1 (b520d6e)" };
    for (const reason of ["core_newer", "update_failed"] as const) {
      expect(parseConnect(result(JSON.stringify({ ok: false, reason, detail: "d", ...builds }), { code: 2 }))).toEqual({ kind: "failed", reason, detail: "d", builds });
      expect(parseConnect(result(JSON.stringify({ ok: false, reason, detail: "d", core: "0.4.2", app: "0.4.1" }), { code: 2 }))).toMatchObject({ reason: "start_failed" });
    }
  });

  it("reads from an attached node's health only a newer core or a failed update that names its builds", () => {
    const builds = { machine: "Mac mini", core: "0.4.1 (b520d6e)", app: "0.4.2 (3f1c2a9)" };
    expect(healthFailure({ role: "node", core_link: "waiting", core_link_reason: "update_failed", builds })).toEqual({ reason: "update_failed", builds });
    expect(healthFailure({ role: "node", core_link: "waiting", core_link_reason: "core_newer", builds })).toEqual({ reason: "core_newer", builds });
    expect(healthFailure({ role: "node", core_link: "waiting", core_link_reason: "update_failed", builds: null })).toBeNull();
    expect(healthFailure({ role: "node", core_link: "updating", core_link_reason: null, builds: null })).toBeNull();
    expect(healthFailure({ role: "core" })).toBeNull();
    expect(healthFailure(null)).toBeNull();
  });

  it("keeps the CLI's failure category and turns everything unreadable into start_failed", () => {
    expect(parseConnect(result('{"ok":false,"reason":"no_response","detail":"not healthy"}', { code: 2 }))).toEqual({
      kind: "failed",
      reason: "no_response",
      detail: "not healthy",
    });
    expect(parseConnect(result("", { code: 2, stderr: "unknown command: connect" }))).toMatchObject({ kind: "failed", reason: "start_failed" });
    expect(parseConnect(result("", { spawnError: "spawn EACCES" }))).toMatchObject({ reason: "start_failed", detail: "spawn EACCES" });
    expect(parseConnect(result("", { timedOut: true, code: null }))).toMatchObject({ reason: "no_response" });
    // A dev hide meeting the app's daemon of another build names it rather than replacing it.
    expect(parseConnect(result('{"ok":false,"reason":"other_build","detail":"another build"}', { code: 2 }))).toEqual({
      kind: "failed",
      reason: "other_build",
      detail: "another build",
    });
  });

  it("refuses a URL that is not the daemon's loopback origin with a token", () => {
    for (const bad of ["http://example.com:7001/#token=abc", "http://127.0.0.1:7002/#token=abc", "http://127.0.0.1:7001/"]) {
      expect(parseConnect(result(JSON.stringify({ ok: true, url: bad, port: 7001, pid: 1 }))), bad).toMatchObject({ kind: "failed" });
    }
  });

  it("reads the attach-only probe, and null when it did not answer", () => {
    expect(parseStatus(result('{"running":false}'))).toEqual({ running: false });
    expect(parseStatus(result(`{"running":true,"url":"${url}","port":7001,"pid":42}`))).toMatchObject({ running: true, pid: 42 });
    expect(parseStatus(result("", { timedOut: true }))).toBeNull();
    expect(parseStatus(result("garbage"))).toBeNull();
  });

  it("finds the login shell's PATH past rc-file output", () => {
    expect(parseLoginPath("welcome!\n__HIDE_LOGIN_PATH__/a:/b__HIDE_LOGIN_PATH__\n")).toBe("/a:/b");
    expect(parseLoginPath("no marks")).toBeNull();
  });

  it("asks a login shell only where there is one: every system but Windows", () => {
    expect(HAS_LOGIN_SHELL).toBe(process.platform !== "win32");
  });

  it.runIf(process.platform !== "win32")("asks a real login shell for its PATH", async () => {
    const command = loginPathCommand("/bin/sh");
    const answer = await new ChildRunner().run(command.file, command.args, 10_000);
    expect(parseLoginPath(answer.stdout)).toBeTruthy();
  });
});

describe("parseRuntimePrepare", () => {
  const dir = process.platform === "win32" ? "C:\\Users\\alice\\.hide\\runtime\\0.4.0-aaaaaaaaaaaa" : "/home/alice/.hide/runtime/0.4.0-aaaaaaaaaaaa";
  it("reads the folder the runtime runs from", () => {
    expect(parseRuntimePrepare(result(`${JSON.stringify({ ok: true, dir, copied: true, removed: [] })}\n`))).toEqual({ dir });
  });
  it("names why there is none, never falling back to a folder", () => {
    expect(parseRuntimePrepare(result('{"ok":false,"reason":"not_packaged"}\n', { code: 1, stderr: "not from a desktop package" }))).toEqual({
      failed: "unreadable answer (exit 1): not from a desktop package",
    });
    expect(parseRuntimePrepare(result('{"ok":true,"dir":"relative"}\n'))).toHaveProperty("failed");
    expect(parseRuntimePrepare(result("", { timedOut: true }))).toEqual({ failed: "hide runtime prepare timed out" });
    expect(parseRuntimePrepare(result("", { spawnError: "ENOENT" }))).toEqual({ failed: "ENOENT" });
  });
});
