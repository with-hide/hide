import path from "node:path";
import { describe, expect, it } from "vitest";
import { chooseHerdr, ensureServer, parseServerStatus, restartServer, serverEnvironment, type ServerStatus } from "./herdr";
import type { ChildResult } from "./spawn";

describe("chooseHerdr", () => {
  const packaged = { bundledDir: "/A/Contents/Resources", herdrBinPath: null, herdrPaneId: null };
  const bundled = path.join("/A/Contents/Resources", process.platform === "win32" ? "herdr.exe" : "herdr");

  it("hands a packaged app's children its bundled herdr when nothing names one", () => {
    expect(chooseHerdr(packaged)).toEqual({ path: bundled, source: "bundled", replacedPaneValue: null });
  });

  it("passes an explicit override through unchanged", () => {
    expect(chooseHerdr({ ...packaged, herdrBinPath: "/opt/herdr" })).toEqual({ path: null, source: "inherited", replacedPaneValue: null });
  });

  it("replaces the value a Herdr pane exported with the bundled herdr", () => {
    const gone = "/A/Contents/Resources/herdr-runtime/herdr";
    expect(chooseHerdr({ ...packaged, herdrBinPath: gone, herdrPaneId: "w1:p1" })).toEqual({
      path: bundled,
      source: "bundled",
      replacedPaneValue: gone,
    });
    const current = chooseHerdr({ ...packaged, herdrBinPath: bundled, herdrPaneId: "w1:p1" });
    expect(current).toEqual({ path: bundled, source: "bundled", replacedPaneValue: null });
  });

  it("adds nothing when unpackaged, pane or not", () => {
    const unpackaged = { bundledDir: null, herdrBinPath: "/opt/herdr", herdrPaneId: "w1:p1" };
    expect(chooseHerdr(unpackaged)).toEqual({ path: null, source: "inherited", replacedPaneValue: null });
    expect(chooseHerdr({ ...unpackaged, herdrBinPath: null, herdrPaneId: null })).toEqual({ path: null, source: "inherited", replacedPaneValue: null });
  });
});

describe("parseServerStatus", () => {
  const answered = (stdout: string, code: number | null = 0): ChildResult => ({ code, signal: null, stdout, stderr: "", timedOut: false, spawnError: null });

  it("reads Herdr's running answer, and nothing else, as the server's state", () => {
    // Herdr 0.9.1 answers not_running for a socket file a crashed or rebooted server left behind.
    expect(parseServerStatus(answered('{"status":"not_running","running":false,"socket":"/x/herdr.sock"}'))).toEqual({ running: false });
    expect(parseServerStatus(answered('{"status":"running","running":true,"version":"0.9.1"}'))).toEqual({ running: true });
  });

  it("reports an answer it cannot read instead of guessing a state", () => {
    expect(parseServerStatus(answered("not json"))).toEqual({ unreadable: "herdr status answered without a running field" });
    expect(parseServerStatus(answered('{"status":"running"}'))).toEqual({ unreadable: "herdr status answered without a running field" });
    expect(parseServerStatus(answered("", 2))).toEqual({ unreadable: "herdr status exited 2" });
    expect(parseServerStatus({ ...answered(""), code: null, timedOut: true })).toEqual({ unreadable: "herdr status timed out" });
    expect(parseServerStatus({ ...answered(""), code: null, spawnError: "spawn ENOENT" })).toEqual({ unreadable: "herdr could not start: spawn ENOENT" });
  });
});

describe("serverEnvironment", () => {
  it("passes on none of the values Herdr sets in each pane itself", () => {
    const fromPane = { HOME: "/Users/example", LANG: "en_US.UTF-8", HERDR_SOCKET_PATH: "/s.sock", HERDR_BIN_PATH: "/A/Contents/Resources/herdr", HERDR_PANE_ID: "w1:p1", HERDR_TAB_ID: "w1:t1", HERDR_WORKSPACE_ID: "w1", HERDR_ENV: "1" };
    expect(serverEnvironment(fromPane)).toEqual({ HOME: "/Users/example", LANG: "en_US.UTF-8", HERDR_SOCKET_PATH: "/s.sock" });
  });

  it("gives a server started from Finder a UTF-8 character type", () => {
    expect(serverEnvironment({ HOME: "/Users/example", PATH: "/usr/bin" })).toEqual({ HOME: "/Users/example", PATH: "/usr/bin", LC_CTYPE: "UTF-8" });
  });

  it("keeps any locale the operator's environment already names", () => {
    for (const key of ["LANG", "LC_ALL", "LC_CTYPE"]) {
      const child = { HOME: "/Users/example", [key]: "ko_KR.UTF-8" };
      expect(serverEnvironment(child)).toEqual(child);
    }
  });
});

describe("ensureServer", () => {
  // Herdr's answers in order, the starts it was asked for, and a clock that moves only when the loop sleeps.
  const herdr = (answers: ServerStatus[], start: { pid: number } | { spawnError: string } = { pid: 42 }) => {
    let clock = 0;
    const starts: number[] = [];
    const deadlines: number[] = [];
    const io = {
      status: async (timeoutMs: number) => {
        deadlines.push(timeoutMs);
        return answers.shift() ?? { running: false };
      },
      start: async () => {
        starts.push(clock);
        return start;
      },
      sleep: async (ms: number) => {
        clock += ms;
      },
      now: () => clock,
      stopped: () => false,
      statusTimeoutMs: 5_000,
      waitMs: 5_000,
      pollMs: 200,
    };
    return { io, starts, deadlines };
  };

  it("leaves a server that answers alone", async () => {
    const { io, starts } = herdr([{ running: true }]);
    expect(await ensureServer(io)).toEqual({ outcome: "running" });
    expect(starts).toEqual([]);
  });

  it("starts one server when none runs and returns once it answers", async () => {
    const { io, starts } = herdr([{ running: false }, { running: false }, { running: true }]);
    expect(await ensureServer(io)).toEqual({ outcome: "started", pid: 42, elapsedMs: 400 });
    expect(starts).toEqual([0]);
  });

  it("starts nothing when Herdr cannot say whether a server runs", async () => {
    const { io, starts } = herdr([{ unreadable: "herdr status timed out" }]);
    expect(await ensureServer(io)).toEqual({ outcome: "status_failed", detail: "herdr status timed out" });
    expect(starts).toEqual([]);
  });

  it("reports a start that never answers after the bounded wait, without starting again", async () => {
    const { io, starts, deadlines } = herdr([]);
    expect(await ensureServer(io)).toEqual({ outcome: "start_failed", detail: "no answer", pid: 42, elapsedMs: 5_000 });
    expect(starts).toEqual([0]);
    // Each poll may only use what is left of the wait.
    expect(deadlines.at(-1)).toBe(200);
    expect(deadlines.slice(1).every((deadline, index) => deadline === 5_000 - 200 * (index + 1))).toBe(true);
  });

  it("reports a binary that could not start", async () => {
    const { io } = herdr([{ running: false }], { spawnError: "spawn EACCES" });
    expect(await ensureServer(io)).toEqual({ outcome: "start_failed", detail: "spawn EACCES" });
  });
});

describe("restartServer", () => {
  // Herdr's answers in order, what was asked of it, and a clock that moves only when the loop sleeps.
  const herdr = (answers: ServerStatus[], stop: { ok: true } | { failed: string } = { ok: true }) => {
    let clock = 0;
    const asked: string[] = [];
    const io = {
      stop: async () => {
        asked.push("stop");
        return stop;
      },
      status: async () => {
        asked.push("status");
        return answers.shift() ?? { running: false };
      },
      start: async () => {
        asked.push("start");
        return { pid: 43 };
      },
      sleep: async (ms: number) => {
        clock += ms;
      },
      now: () => clock,
      stopped: () => false,
      statusTimeoutMs: 5_000,
      waitMs: 5_000,
      pollMs: 200,
    };
    return { io, asked };
  };

  it("stops the server, waits until none runs, then starts this app's Herdr", async () => {
    const { io, asked } = herdr([{ running: true }, { running: false }, { running: false }, { running: true }]);
    expect(await restartServer(io)).toEqual({ outcome: "restarted", pid: 43, elapsedMs: 200 });
    expect(asked).toEqual(["stop", "status", "status", "status", "start", "status"]);
  });

  it("starts nothing when Herdr refuses the stop or the server keeps running", async () => {
    const refused = herdr([], { failed: "herdr server stop exited 1" });
    expect(await restartServer(refused.io)).toEqual({ outcome: "stop_failed", detail: "herdr server stop exited 1" });
    expect(refused.asked).toEqual(["stop"]);
    const stays = herdr(Array.from({ length: 40 }, () => ({ running: true })));
    expect(await restartServer(stays.io)).toEqual({ outcome: "stop_failed", detail: "the server still runs after the stop" });
    expect(stays.asked).not.toContain("start");
  });

  it("answers a server that does not come up as the start failure the status page shows", async () => {
    const { io } = herdr([{ running: false }, { running: false }, ...Array.from({ length: 40 }, () => ({ running: false }))]);
    expect(await restartServer(io)).toMatchObject({ outcome: "start_failed", detail: "no answer", pid: 43 });
  });
});
