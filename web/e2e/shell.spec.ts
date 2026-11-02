import { expect, test, type Page } from "@playwright/test";
import type { ChildProcess } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { startEngine, startFixture } from "./engine-fixture";
import { herdrBinary, linkFixtureTranscripts, type HerdrFixture } from "./herdr-fixture";
import { aiSettingsFile, spawnDaemon, startHided, stopDaemon } from "./hided-fixture";
import { enterWorkspace } from "./wire";
import { fixtureExecutable, fixtureHomeEnv, fixtureOpenCommand, fixtureToolPath, inheritedFixtureEnv } from "./platform-fixture";
import { ownUntilWorkerExit } from "./worker-owned";

type Daemon = {
  origin: string;
  token: string;
  /** Kills the daemon and removes its state directory. */
  stop: () => void;
};

/** A daemon with no engine at all and no Herdr socket, for the one test about a socket Hide cannot find; `herdr` lends it its transcripts and its `claude` label provider. */
async function startBareHided(extra: Record<string, string> = {}, herdr?: HerdrFixture): Promise<Daemon> {
  const dir = fs.realpathSync.native(fs.mkdtempSync(path.join(os.tmpdir(), "hide-e2e-")));
  const bin = path.resolve("..", "target", "debug", fixtureExecutable("hided"));
  let env: NodeJS.ProcessEnv = {};
  let child: ChildProcess | undefined;
  let spawnAttempted = false;
  let spawnFailed: Error | null = null;
  let stopping = false;
  let ended = false;
  let cleaned = false;
  const canRemove = () => !spawnAttempted || ended || (child !== undefined &&
    (child.exitCode != null || child.signalCode != null || (spawnFailed !== null && child.pid === undefined)));
  const incomplete = () => new Error(`hided fixture cleanup incomplete; preserve ${dir}: child exit unconfirmed (PID: ${child?.pid ?? "unavailable"}); wait for the recorded child's exit/close and owned cleanup, or confirm its exit before removing this retained root after worker loss`);
  const cleanup = () => {
    if (cleaned) return;
    if (!canRemove()) throw incomplete();
    // The child has exited or was never started; retry only filesystem removal.
    for (let attempt = 0; attempt < 40; attempt += 1) {
      try {
        fs.rmSync(dir, { recursive: true, force: true });
        cleaned = true;
        owned.disown();
        return;
      } catch {
        Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 50);
      }
    }
    fs.rmSync(dir, { recursive: true, force: true });
    cleaned = true;
    owned.disown();
  };
  const stop = () => {
    if (cleaned) return;
    stopping = true;
    if (spawnAttempted && !child) throw incomplete();
    if (child && !canRemove()) {
      try {
        if (process.platform === "win32") child.kill();
        else ended = stopDaemon(child, bin, env, dir);
      } catch (error) {
        throw new Error(incomplete().message, { cause: error });
      }
    }
    if (canRemove()) cleanup();
    else console.error("hided fixture cleanup pending", { root: dir, pid: child?.pid, action: "wait for the recorded child's exit/close; its root remains owned" });
  };
  const owned = ownUntilWorkerExit(() => {
    try { stop(); cleanup(); } catch (error) { process.exitCode = 1; throw error; }
  });
  try {
    if (herdr) {
      linkFixtureTranscripts(herdr, dir);
      // Agent titles come from Hide AI, which asks no model until an agent is chosen (B47):
      // the operator of this fixture has already chosen the `claude` fixture shim.
      const aiFile = aiSettingsFile(dir);
      fs.mkdirSync(path.dirname(aiFile), { recursive: true });
      fs.writeFileSync(aiFile, JSON.stringify({ provider: "claude" }));
    }
    env = {
      ...inheritedFixtureEnv(),
      ...fixtureHomeEnv(dir),
      HIDE_STATE_DIR: path.join(dir, "hide"),
      HIDE_KEEP_ALIVE: "1",
      HIDE_PORT: "0",
      HIDED_UI_DIR: path.resolve("dist"),
      // Resolve the fixture binary even when this test creates no server.
      // Keep HERDR_SOCKET_PATH absent in those missing-socket/auth cases.
      HERDR_BIN_PATH: herdrBinary(),
      PATH: herdr?.fixturePath ?? fixtureToolPath(path.join(dir, "bin")),
      HIDE_OPEN_COMMAND: fixtureOpenCommand(herdr?.root, dir),
      ...extra,
    };
    spawnAttempted = true;
    child = spawnDaemon(bin, env);
    child.once("error", (error) => { spawnFailed = error; });
    let closeLog = () => {};
    child.once("close", () => {
      closeLog();
      if (stopping) cleanup();
    });
    // Two pipes share the evidence file; neither may end it before child close.
    const logDir = process.env.HIDE_E2E_SCREENSHOT_DIR;
    if (logDir) {
      const log = fs.createWriteStream(path.join(logDir, `hided-${path.basename(dir)}.log`), { flags: "a" });
      closeLog = () => log.end();
      child.stdout?.pipe(log, { end: false });
      child.stderr?.pipe(log, { end: false });
    }
    for (let i = 0; i < 50; i += 1) {
      if (spawnFailed) throw new Error(`hided did not start from ${bin}: ${(spawnFailed as Error).message}`, { cause: spawnFailed });
      const statePath = path.join(dir, "hide", "hided.json");
      if (fs.existsSync(statePath)) {
        try {
          // The file may be mid-write on the first read; the next tick reads it whole.
          const state = JSON.parse(fs.readFileSync(statePath, "utf8")) as {
            port: number;
            token: string;
          };
          const origin = `http://127.0.0.1:${state.port}`;
          const health = await fetch(`${origin}/health`);
          if (health.ok) {
            return { origin, token: state.token, stop };
          }
        } catch {
          /* still starting */
        }
      }
      await new Promise((resolve) => setTimeout(resolve, 100));
    }
    throw new Error("hided did not write a state file");
  } catch (error) {
    try {
      stop();
      cleanup();
    } catch (cleanupError) {
      throw new AggregateError([error, cleanupError], `hided setup failed; cleanup incomplete; preserve ${dir}`);
    }
    throw error;
  }
}

// Herdr only: the sidebar's notice for a Herdr socket that is missing, which the own engine never has.
test("sidebar shows a missing Herdr socket and the badge goes live @herdr", async ({ page }) => {
  const daemon = await startBareHided();
  try {
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await expect(page.getByText("Herdr socket missing")).toBeVisible();
    await expect(page.locator("[data-connection]")).toHaveCount(0);
  } finally {
    daemon.stop();
  }
});

test("a bad token shows the refused connection state", async ({ page }) => {
  const engine = await startEngine();
  let daemon: Awaited<ReturnType<typeof startHided>> | null = null;
  try {
    daemon = await startHided(engine, "shell-bad-token");
    await page.goto(`${daemon.origin}/#token=${"aa".repeat(32)}`);
    await expect(page.getByText("connection refused - run hide again")).toBeVisible();
  } finally {
    daemon?.stop();
    engine.stop();
  }
});

async function typedTextEchoes(page: Page, marker: string): Promise<void> {
  await page.locator('[data-pane-view][data-focused="true"] .xterm-helper-textarea').focus();
  await page.keyboard.type(marker);
  await expect
    .poll(() => page.evaluate(() => window.__hideProbe?.screenText() ?? ""), { timeout: 10_000 })
    .toContain(marker);
}

// @platform: Real PTY input and echo through the platform's Herdr and its shell.
test("a sidebar row click switches the pane and typed text echoes there", { tag: "@platform" }, async ({ page }) => {
  const herdr = await startFixture();
  let daemon: Awaited<ReturnType<typeof startHided>> | null = null;
  try {
    daemon = await startHided(herdr, "shell");
    const [first, second] = herdr.panes;
    await page.goto(`${daemon.origin}/?probe=1#token=${daemon.token}`);
    await enterWorkspace(page);
    // The sidebar opens on Projects; the rows this test clicks are the Agents list's.
    await expect(page.locator(`[data-pane="${first}"]`)).toContainText("Agent one");
    await expect(page.locator(`[data-pane="${second}"]`)).toContainText("Agent two");
    await expect(page.locator('[data-pane-view][data-focused="true"]')).toHaveAttribute("data-pane-view", first);

    await page.locator(`[data-pane="${second}"]`).click();
    await expect(page.locator('[data-pane-view][data-focused="true"]')).toHaveAttribute("data-pane-view", second);
    await expect
      .poll(() => page.evaluate(() => window.__hideProbe?.screenText() ?? ""), { timeout: 10_000 })
      .toContain("claude");
    await typedTextEchoes(page, "echo-two-9f3a");

    await page.locator(`[data-pane="${first}"]`).click();
    await expect(page.locator('[data-pane-view][data-focused="true"]')).toHaveAttribute("data-pane-view", first);
    await expect
      .poll(() => page.evaluate(() => window.__hideProbe?.screenText() ?? ""), { timeout: 10_000 })
      .toContain("claude");
    await typedTextEchoes(page, "echo-one-7c1d 한글");
    // Run evidence for a reviewer; CI sets no directory and takes none.
    const screenshots = process.env.HIDE_E2E_SCREENSHOT_DIR;
    if (screenshots) {
      await page.screenshot({ path: path.join(screenshots, "s1-click-flow.png") });
    }
    await expect(page.evaluate(() => window.__hideProbe?.screenText() ?? "")).resolves.not.toContain(
      "echo-two-9f3a",
    );
  } finally {
    daemon?.stop();
    herdr.stop();
  }
});
