import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { describe, expect, it } from "vitest";
import { HostLog, loadFailureFields } from "./log";
import cases from "../../../contracts/fixtures/diagnostic-redaction.json";

describe("the host log", () => {
  it("masks ordinary records in both the file and stderr while retaining field types", () => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "hide-desktop-log-"));
    const log = new HostLog(dir);
    const stderr = process.stderr.write;
    let echoed = "";
    process.stderr.write = ((chunk: string) => ((echoed += chunk), true)) as typeof process.stderr.write;
    try {
      for (const { input } of cases) log.event("worker.failed", { detail: typeof input === "string" ? input : input.join(""), pid: 42, retry: false, empty: null, password: "q7Z", accessToken: "opaque-private-value", Authorization: "Basic cXVpZXQ6c2VjcmV0" });
      const written = fs.readFileSync(log.path, "utf8");
      expect(echoed).toBe(written);
      const records = written.trim().split("\n").map((line) => JSON.parse(line) as Record<string, unknown>);
      expect(records.map((record) => record.detail)).toEqual(cases.map(({ expected }) => expected));
      for (const record of records) expect(record).toMatchObject({ event: "worker.failed", pid: 42, retry: false, empty: null, password: "[redacted]", accessToken: "[redacted]", Authorization: "[redacted]" });
      // Exercise the real filesystem error path too: its exception quotes
      // the directory. It must obey the same stderr policy.
      fs.rmSync(dir, { recursive: true, force: true });
      const failing = new HostLog(path.join(dir, "someone@example.invalid"));
      fs.rmSync(dir, { recursive: true, force: true });
      failing.event("worker.failed");
      expect(echoed).toContain("log.write_failed");
      expect(echoed).not.toContain("someone@example.invalid");
    } finally {
      process.stderr.write = stderr;
      fs.rmSync(dir, { recursive: true, force: true });
    }
  });

  it("never writes the daemon token from a failed load", () => {
    // The shape Electron 44 rejects `loadURL` with.
    const error = Object.assign(new Error("ERR_CONNECTION_REFUSED (-102) loading 'http://127.0.0.1:7001/#token=SECRET123'"), {
      code: "ERR_CONNECTION_REFUSED",
    });
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "hide-desktop-log-"));
    const log = new HostLog(dir);
    const stderr = process.stderr.write;
    let echoed = "";
    process.stderr.write = ((chunk: string) => ((echoed += chunk), true)) as typeof process.stderr.write;
    try {
      log.event("window.load_failed", { state: "attached", ...loadFailureFields(error) });
    } finally {
      process.stderr.write = stderr;
    }
    const written = fs.readFileSync(log.path, "utf8");
    expect(JSON.parse(written)).toMatchObject({ event: "window.load_failed", state: "attached", code: "ERR_CONNECTION_REFUSED" });
    expect(written).not.toContain("SECRET123");
    expect(echoed).not.toContain("SECRET123");
    expect(loadFailureFields("not an error")).toEqual({ code: "unknown" });
    fs.rmSync(dir, { recursive: true, force: true });
  });
});
