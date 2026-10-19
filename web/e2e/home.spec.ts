// This Mac's Home (PRD home-device-rail B16-B18, B21) against a private
// Herdr and hided: the sidebar has no Home row (sidebar-titlebar-refresh B11),
// so Home is reached from the start panel, whose start aimed at Home makes
// `~/hide` in the daemon's HOME with a link per registered project and hide's
// guide files, and runs the agent there; a `~/hide` that is not Hide's is left
// alone, and the reason shows inside the start panel.

import { expect, test, type Page } from "@playwright/test";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { startHerdr, type HerdrFixture } from "./herdr-fixture";
import { herdrGate } from "./herdr-gate";
import { startHided, type Daemon } from "./hided-fixture";
import { enterWorkspace, screenshot } from "./wire";
import { chord } from "./chords";
import { toPage } from "../../desktop/src/main/wirePath";

test.describe.configure({ timeout: 120_000 });

async function pinFixture(page: Page) {
  // The fixture is a plain folder, which the sidebar draws as one row.
  const row = page.locator("nav[data-sidebar]").getByRole("button", { name: /^fixture(,|$)/ });
  await row.click({ button: "right" });
  await page.getByRole("menuitem", { name: "Pin", exact: true }).click();
  await expect(page.locator('[data-section="pinned"]')).toBeVisible({ timeout: 20_000 });
}

/** ⌘K's Start an agent… with Home chosen as the target, the start panel left open. */
async function aimAtHome(page: Page, node: string) {
  await page.keyboard.press(chord("search"));
  await expect(page.locator('[data-palette="Search"] [data-palette-input]')).toBeFocused();
  await page.keyboard.type("Start an agent");
  await page.locator('[data-palette-row="command:start-agent"]').click();
  const panel = page.locator("[data-start-panel]");
  await panel.locator("[data-start-target]").click();
  await page.locator(`[data-start-target-option="home:${node}"]`).click();
  await expect(panel.locator("[data-start-target]")).toHaveAttribute("data-start-target", `home:${node}`);
}

type SnapshotPane = { pane_id: string; workspace_id: string; tab_id: string; cwd?: string | null; foreground_cwd?: string | null };
type HerdrSnapshot = { result: { snapshot: { workspaces: { workspace_id: string; label?: string }[]; panes: SnapshotPane[] } } };
type PaneProcess = { result: { process_info: { shell_pid: number; foreground_process_group_id: number } } };

/**
 * Written only when the test fails, so the next failure says where each pane's
 * cwd came from. Herdr's `cwd` is a pane's OSC 7 report when it made one and
 * the shell's /proc cwd otherwise, and `foreground_cwd` is always /proc; the
 * shell's /proc cwd read here (Linux) and the per-process cwds Herdr lists in
 * `process-info` tell which of the two was wrong. Hide's side is where the
 * path back landed. Each read stands alone, so one that
 * fails is recorded instead of hiding the rest.
 */
async function recordFailure(page: Page, herdr: HerdrFixture, daemon: Daemon | null) {
  const read = <T,>(what: () => T): T | { error: string } => {
    try {
      return what();
    } catch (error) {
      return { error: String(error) };
    }
  };
  const snapshot = read(() => herdr.run(["api", "snapshot"]) as HerdrSnapshot);
  const panes = "error" in snapshot ? [] : snapshot.result.snapshot.panes;
  const diagnostics = {
    daemonHome: daemon?.home ?? null,
    hide: {
      location: await page.locator('nav[aria-label="Location"]').textContent({ timeout: 1_000 }).catch((error: unknown) => `unreadable: ${String(error)}`),
      project: await page.locator("[data-go-overview]").getAttribute("data-go-overview", { timeout: 1_000 }).catch(() => null),
      kind: await page.locator("[data-workspace-location]").getAttribute("data-workspace-location", { timeout: 1_000 }).catch(() => null),
    },
    workspaces: "error" in snapshot ? snapshot : snapshot.result.snapshot.workspaces,
    panes: panes.map((pane) => {
      const info = read(() => (herdr.run(["pane", "process-info", "--pane", pane.pane_id]) as PaneProcess).result.process_info);
      const procCwd = (pid: number) => (process.platform === "linux" ? read(() => fs.readlinkSync(`/proc/${pid}/cwd`)) : "no /proc on this system");
      return {
        ...pane,
        process: info,
        shellProcCwd: "error" in info ? null : procCwd(info.shell_pid),
        foregroundProcCwd: "error" in info ? null : procCwd(info.foreground_process_group_id),
        visibleText: read(() => {
          const result = spawnSync(herdr.bin, ["pane", "read", pane.pane_id, "--source", "visible", "--format", "text"], { env: herdr.env, encoding: "utf8", timeout: 10_000 });
          return result.status === 0 ? result.stdout : `pane read exited ${result.status}: ${result.stderr.trim()}`;
        }),
      };
    }),
  };
  const file = test.info().outputPath("home-diagnostics.json");
  fs.writeFileSync(file, JSON.stringify(diagnostics, null, 2));
  await test.info().attach("home-diagnostics", { path: file, contentType: "application/json" });
}

/** A start aimed at Home with a pinned project: `~/hide` is made and the agent runs in it, Herdr's pane included. */
async function inOpenedHome(page: Page, check: (home: string, herdr: HerdrFixture) => Promise<void>, registeredFirst = false): Promise<void> {
  await page.setViewportSize({ width: 1280, height: 800 });
  const herdr = await startHerdr();
  let gate: Awaited<ReturnType<typeof herdrGate>> | null = null;
  let registeredWithoutPane = false;
  if (registeredFirst) page.on("websocket", (socket) => socket.on("framereceived", ({ payload }) => {
    try {
      const frame = JSON.parse(String(payload)) as { payload?: { rest?: { navigator?: { workspaces?: { is_home?: boolean; checkouts: { owner_workspace_id?: string | null; tabs: unknown[] }[] }[] } } } };
      registeredWithoutPane ||= frame.payload?.rest?.navigator?.workspaces?.some((workspace) => workspace.is_home && workspace.checkouts.some((checkout) => !checkout.owner_workspace_id && checkout.tabs.length === 0)) ?? false;
    } catch { /* Terminal frames are binary. */ }
  }));
  let daemon: Daemon | null = null;
  try {
    if (registeredFirst) gate = await herdrGate(herdr);
    daemon = await startHided(herdr, "home", undefined, gate ? { HERDR_SOCKET_PATH: gate.socket } : undefined);
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    await pinFixture(page);
    const home = path.join(daemon.home, "hide");
    // B18: no Home until it is first used.
    expect(fs.existsSync(home)).toBe(false);

    await aimAtHome(page, daemon.node);
    const held = gate?.arm("workspace.create");
    await page.locator("[data-start-submit]").click();
    if (gate) {
      await held;
      // Publish the existing workspace once while Home's creation is held:
      // the registered Home now has a row, but no Herdr owner or pane.
      herdr.run(["workspace", "rename", herdr.workspace, "catalog-ready"]);
      await expect.poll(() => registeredWithoutPane, { message: "Home is listed before Herdr creates its first pane" }).toBe(true);
      console.log(`[${Date.now()}] Home registration listed without an owner or pane; releasing workspace.create`);
      await gate.release();
    }
    // B17: the agent opens in Home once Home's sync answered, so everything the
    // sync writes is on disk by the time Herdr has a pane there. Herdr spells a
    // pane's folder as the system does, so both sides are read as the wire spells it.
    await expect
      .poll(() => fs.existsSync(home) && panesAt(herdr, toPage(fs.realpathSync(home))), { timeout: 30_000 })
      .toBe(true);
    await check(home, herdr);
  } catch (error) {
    await recordFailure(page, herdr, daemon);
    throw error;
  } finally {
    try { daemon?.stop(); } finally { try { await gate?.stop(); } finally { herdr.stop(); } }
  }
}

/** Whether a pane of Herdr's snapshot is in `folder` (a wire spelling) or below it. */
function panesAt(herdr: HerdrFixture, folder: string): boolean {
  const snapshot = herdr.run(["api", "snapshot"]) as HerdrSnapshot;
  const onWire = (native: string | null | undefined): string => {
    if (!native) return "";
    try {
      return toPage(native);
    } catch {
      return "";
    }
  };
  return snapshot.result.snapshot.panes.some((pane) => [pane.cwd, pane.foreground_cwd].some((cwd) => onWire(cwd).startsWith(folder)));
}

test("a start aimed at Home makes ~/hide with its guide files and runs the agent there", async ({ page }) => {
  await inOpenedHome(page, async (home) => {
    await expect(page.locator("[data-workspace-screen]")).toBeVisible();
    for (const file of [".hide-home.json", "AGENTS.md", "CLAUDE.md"]) expect(fs.existsSync(path.join(home, file))).toBe(true);

    // The Home is still no project (B16), and the sidebar draws no row for it (B11).
    await expect(page.locator("nav[data-sidebar]").getByRole("button", { name: /^(hide|Home)$/ })).toHaveCount(0);
    // The top row's path names Home's folder, never Home as a project.
    await expect(page.locator('nav[aria-label="Location"]')).toHaveText("~/hide");
    await screenshot(page, "home-tab-opened");
  });
});

test("Home stays selected when its empty registration is listed before its first pane", async ({ page }) => {
  await inOpenedHome(page, async () => {
    await expect(page.locator('nav[aria-label="Location"]')).toHaveText("~/hide");
  }, true);
});

// @platform: The link Home makes to a project: a symbolic link on macOS and Linux, a junction on Windows.
test("Home's ~/hide holds a link to each registered project", { tag: "@platform" }, async ({ page }) => {
  await inOpenedHome(page, async (home, herdr) => {
    const link = path.join(home, "fixture");
    expect(fs.lstatSync(link).isSymbolicLink()).toBe(true);
    expect(fs.realpathSync(link)).toBe(fs.realpathSync(path.join(herdr.root, "fixture")));
  });
});

test("a ~/hide that is not Hide's is left alone, and the reason shows in the start panel", async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 800 });
  const herdr = await startHerdr({ agents: false });
  let daemon: Daemon | null = null;
  try {
    daemon = await startHided(herdr, "home-conflict");
    const home = path.join(daemon.home, "hide");
    fs.mkdirSync(home);
    fs.writeFileSync(path.join(home, "mine.txt"), "the operator's own folder\n");
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");

    // B21: a start aimed at Home is refused inside the panel, keeps the text, and leaves the folder untouched.
    await aimAtHome(page, daemon.node);
    const panel = page.locator("[data-start-panel]");
    await panel.locator("[data-start-text]").fill("홈에서 할 일");
    await panel.locator("[data-start-submit]").click();
    await expect(panel.locator("[data-start-failure]")).toContainText("~/hide", { timeout: 20_000 });
    await expect(panel.locator("[data-start-text]")).toHaveValue("홈에서 할 일");
    expect(fs.readdirSync(home)).toEqual(["mine.txt"]);
    await screenshot(page, "home-conflict-panel");
  } finally {
    daemon?.stop();
    herdr.stop();
  }
});
