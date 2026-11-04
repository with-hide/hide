import path from "node:path";
import { pathToFileURL } from "node:url";
import { describe, expect, it } from "vitest";
import { spawnSync } from "node:child_process";
import { ChildRunner, startDetached } from "./spawn";

// Node itself is the test program, so each child does the same on every system.
const node = process.execPath;
const script = (code: string): string[] => ["-e", code];
/** The environment a test hands a child: `extra` and, on Windows, the system folder Node needs to start. */
const handed = (extra: Record<string, string>): Record<string, string | undefined> => ({ ...extra, ...(process.env.SystemRoot ? { SystemRoot: process.env.SystemRoot } : {}) });

describe("the child runner", () => {
  it("returns a real child's output and exit code", async () => {
    const result = await new ChildRunner().run(node, script('process.stdout.write("out\\n"); process.stderr.write("err\\n"); process.exit(3)'), 5_000);
    expect(result).toMatchObject({ code: 3, stdout: "out\n", stderr: "err\n", timedOut: false, spawnError: null });
  });

  it("gives a child the environment it is handed, whole", async () => {
    const result = await new ChildRunner().run(node, script('process.stdout.write(`${process.env.HIDE_CLI_PATH}|${process.env.HOME ?? ""}`)'), 5_000, handed({ HIDE_CLI_PATH: "/r/hide" }));
    expect(result.stdout).toBe("/r/hide|");
  });

  it("kills a child past its timeout", async () => {
    const started = Date.now();
    const result = await new ChildRunner().run(node, script("setTimeout(() => {}, 10_000)"), 200);
    expect(result.timedOut).toBe(true);
    expect(Date.now() - started).toBeLessThan(5_000);
  });

  it("reports a spawn that never started", async () => {
    const result = await new ChildRunner().run("/nonexistent/hide", ["connect"], 5_000);
    expect(result.spawnError).toMatch(/ENOENT/);
  });

  it("refuses a second child while one runs", async () => {
    const runner = new ChildRunner();
    const first = runner.run(node, script("setTimeout(() => {}, 10_000)"), 5_000);
    await expect(runner.run(node, script(""), 1_000)).rejects.toThrow(/over budget/);
    runner.stop();
    expect((await first).signal).toBe("SIGKILL");
  });
});

describe("a detached start", () => {
  it("leaves a child that outlives the process that started it, in a group of its own", async () => {
    // A starter process runs `startDetached` and exits, as the app does when
    // it quits; on Windows Node ends every child it did not detach when it
    // exits, so the server surviving the starter is the detached start.
    const module = pathToFileURL(path.join(__dirname, "spawn.ts")).href;
    const starter = `import(${JSON.stringify(module)}).then(async (m) => {
      const started = await m.startDetached(process.execPath, ["-e", "setTimeout(() => {}, 30_000)"], process.env);
      process.stdout.write(String(started.pid ?? started.spawnError));
    })`;
    const result = await new ChildRunner().run(node, ["--experimental-strip-types", "--no-warnings", "-e", starter], 10_000, handed({}));
    const pid = Number(result.stdout);
    expect(Number.isInteger(pid), result.stdout + result.stderr).toBe(true);
    try {
      expect(result.code).toBe(0);
      expect(() => process.kill(pid, 0)).not.toThrow();
      // macOS `ps` reports no session id, so the new session shows as a
      // group the child leads. Windows starts it in a new process group too
      // (`CREATE_NEW_PROCESS_GROUP`), but nothing there reads a group back.
      if (process.platform !== "win32") {
        const group = (each: number) => Number(spawnSync("/bin/ps", ["-o", "pgid=", "-p", String(each)], { encoding: "utf8" }).stdout.trim());
        expect(group(pid)).toBe(pid);
        expect(group(process.pid)).not.toBe(pid);
      }
    } finally {
      process.kill(pid, "SIGKILL");
    }
  });

  it("reports a binary that cannot start", async () => {
    expect(await startDetached("/nonexistent/program", ["arg"], {})).toEqual({ spawnError: expect.stringMatching(/ENOENT/) });
  });
});
