// Approved group-tabs B3/B5-B10/B12/B18/B21/B23: real private Herdr
// topology is the oracle for whether a blank tab starts anything.
import { expect, test, type Page } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { herdrHasFocus, startHerdr, type HerdrFixture } from "./herdr-fixture";
import { herdrGate } from "./herdr-gate";
import { herdrTabs } from "./failure-dump";
import { startHided, type Daemon } from "./hided-fixture";
import { countSent, enterWorkspace, screenshot } from "./wire";
import { quietFor } from "./wait";
import { chord } from "./chords";

test.describe.configure({ timeout: 150_000 });
test.use({ actionTimeout: 15_000 });

const stripTabs = (page: Page) => page.locator("[data-agent-tab-bar] [role=tab][data-tab]");
const selector = (page: Page) => page.locator("[data-agent-group-selector]");
const terminal = (page: Page) => page.locator("[data-group-start-terminal=true]");
const blank = (page: Page) => page.locator("[data-agent-tab-start]");

function topology(herdr: HerdrFixture) {
  const workspaces = herdrTabs(herdr) as { tabs: string[] }[];
  return workspaces.flatMap((workspace) => workspace.tabs);
}

async function open(page: Page, daemon: Daemon) {
  await page.goto("about:blank");
  await page.goto(`${daemon.origin}/?probe=1#token=${daemon.token}`);
  await enterWorkspace(page, "fixture");
  await expect(selector(page)).toBeVisible();
}

async function createGroup(page: Page, name: string) {
  await selector(page).click();
  await page.getByRole("menuitem", { name: "New group…", exact: true }).click();
  await page.getByRole("textbox", { name: "Group name", exact: true }).fill(name);
  await page.getByRole("button", { name: "Save", exact: true }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(selector(page)).toContainText(name);
  const group = (await selector(page).getAttribute("data-agent-group-selector"))!;
  const row = page.locator(`[data-sidebar-group-select="${group}"]`);
  if (!(await row.isVisible())) await page.locator("[data-checkout-toggle][aria-expanded=false]").click();
  await expect(row).toBeVisible();
  return group;
}

async function establishOwner(page: Page) {
  await page.locator("[data-new-agent-tab=true]").click();
  await expect(blank(page)).toBeVisible();
  const draft = await blank(page).getAttribute("data-agent-tab-start");
  await terminal(page).click();
  await expect(page.locator(`[data-agent-tab-start="${draft}"]`)).toHaveCount(0, { timeout: 30_000 });
  await expect(page.locator("[data-pane-view]")).toHaveCount(1);
}

async function chooseGroup(page: Page, name: string) {
  await selector(page).click();
  await page.getByRole("menuitem", { name, exact: true }).click();
  await expect(selector(page)).toContainText(name);
}

test("an empty group and draft survive navigation and restart without creating a shell", async ({ page }) => {
  const herdr = await startHerdr({ agents: false });
  let daemon: Daemon | null = null;
  try {
    daemon = await startHided(herdr, "group-draft");
    await page.setViewportSize({ width: 1440, height: 900 });
    await open(page, daemon);
    const original = topology(herdr);
    await screenshot(page, "group-tabs-ungrouped-work");
    const group = await createGroup(page, "문서 검토");
    await expect(stripTabs(page)).toHaveCount(0);
    await expect(blank(page)).toHaveAttribute("data-agent-tab-start", "empty");
    await expect(terminal(page)).toBeEnabled();
    await page.locator("[data-new-agent-tab=true]").click();
    await expect(stripTabs(page)).toHaveCount(1);
    const draft = await blank(page).getAttribute("data-agent-tab-start");
    expect(draft).toMatch(/^draft:/);
    await blank(page).getByRole("textbox").fill("아직 보내지 않은 작업");
    await chooseGroup(page, "Ungrouped");
    await expect(stripTabs(page)).toHaveCount(original.length);
    await chooseGroup(page, "문서 검토");
    await expect(blank(page).getByRole("textbox")).toHaveValue("아직 보내지 않은 작업");
    expect(topology(herdr)).toEqual(original);
    await screenshot(page, "group-tabs-blank-terminal-action");
    const evidence = process.env.HIDE_E2E_SCREENSHOT_DIR;
    if (evidence) fs.copyFileSync(path.join(daemon.stateDir, "workspace-views.json"), path.join(evidence, "group-restart-before.json"));
    await page.goto("about:blank");
    daemon = await daemon.restart();
    if (evidence) fs.copyFileSync(path.join(daemon.stateDir, "workspace-views.json"), path.join(evidence, "group-restart-after-launch.json"));
    await open(page, daemon);
    await expect(selector(page)).toHaveAttribute("data-agent-group-selector", group);
    await expect(blank(page)).toHaveAttribute("data-agent-tab-start", draft!);
    expect(topology(herdr)).toEqual(original);
  } finally { try { daemon?.stop(); } finally { herdr.stop(); } }
});

test("terminal start fills the draft slot and duplicate intent delivery creates only one tab", async ({ page }) => {
  const herdr = await startHerdr({ agents: false });
  const gate = await herdrGate(herdr);
  let daemon: Daemon | null = null;
  try {
    daemon = await startHided(herdr, "group-terminal", undefined, { HERDR_SOCKET_PATH: gate.socket });
    const last = new Map<string, Record<string, unknown>>();
    countSent(page, last);
    let repeat: (() => void) | undefined;
    await page.routeWebSocket(/\/ws$/, (socket) => {
      const server = socket.connectToServer();
      socket.onMessage((message) => {
        server.send(message);
        if (typeof message === "string" && (JSON.parse(message) as { kind?: string }).kind === "agent_start_in_checkout") repeat = () => server.send(message);
      });
    });
    await open(page, daemon);
    await establishOwner(page);
    const original = topology(herdr);
    await createGroup(page, "Terminal work");
    await page.locator("[data-new-agent-tab=true]").click();
    // Hide starts its integrated shell through layout.apply, including a
    // terminal-only request; tab.create is the raw Herdr shell path.
    const held = gate.arm("layout.apply");
    await terminal(page).click();
    await held;
    await expect(terminal(page)).toBeDisabled();
    const intent = last.get("agent_start_in_checkout")!;
    expect(intent.provider).toBe("terminal");
    expect(intent.target).toMatchObject({ area_id: "a1", add_pane: false });
    // A browser retransmission uses the same public event boundary.
    expect(repeat).toBeDefined();
    repeat!();
    await gate.release();
    await expect(blank(page)).toHaveCount(0, { timeout: 30_000 });
    await expect(stripTabs(page)).toHaveCount(1);
    const created = (await stripTabs(page).getAttribute("data-tab"))!;
    expect(original).not.toContain(created);
    expect(topology(herdr)).toHaveLength(original.length + 1);
    expect(gate.params("layout.apply")).toHaveLength(1);
    await screenshot(page, "group-tabs-terminal-promoted");
  } finally { try { daemon?.stop(); } finally { try { await gate.stop(); } finally { herdr.stop(); } } }
});

test("a late global start stays behind after returning to an empty group or draft", async ({ page }) => {
  for (const draft of [false, true]) {
    const herdr = await startHerdr({ agents: false });
    const gate = await herdrGate(herdr);
    let daemon: Daemon | null = null;
    try {
      const beta = path.join(herdr.root, "beta");
      fs.mkdirSync(beta);
      herdr.run(["workspace", "create", "--cwd", beta, "--label", "beta", "--env", `PATH=${herdr.fixturePath}`, "--no-focus"]);
      daemon = await startHided(herdr, "group-global-selection", undefined, { HERDR_SOCKET_PATH: gate.socket });
      await open(page, daemon);
      await createGroup(page, "Other");
      const origin = await createGroup(page, "Origin");
      if (draft) {
        await page.locator("[data-new-agent-tab=true]").click();
        await expect(blank(page)).toHaveAttribute("data-agent-tab-start", /^draft:/);
      }
      const originalDraft = await blank(page).getAttribute("data-agent-tab-start");

      await page.keyboard.press(chord("search"));
      await page.locator('[data-palette="Search"] [data-palette-input]').fill("Start an agent");
      await page.locator('[data-palette-row="command:start-agent"]').click();
      const panel = page.locator("[data-start-panel]");
      await panel.locator("[data-start-target]").click();
      await page.locator('[data-start-target-option$="/beta"]').click();
      await panel.locator("[data-start-text]").fill("selection-round-trip");
      const held = gate.arm("layout.apply");
      await panel.locator("[data-start-submit]").click();
      await held;
      await chooseGroup(page, "Other");
      await chooseGroup(page, "Origin");
      await gate.release();

      // The real CLI must start, but this later visit retains the screen and
      // Herdr focus even though its final group/draft equals the origin.
      let started: string | undefined;
      await expect.poll(() => {
        const panes = herdr.run(["pane", "list"]) as { result: { panes: { pane_id: string }[] } };
        started = panes.result.panes.find(({ pane_id }) => {
          const info = herdr.run(["pane", "process-info", "--pane", pane_id]) as { result: { process_info: { foreground_processes: { argv?: string[] | null }[] } } };
          return info.result.process_info.foreground_processes.some((process) => process.argv?.includes("selection-round-trip"));
        })?.pane_id;
        return started;
      }, { timeout: 30_000 }).toBeTruthy();
      await quietFor(page, 300, "a completed global start cannot undo a newer empty selection");
      await expect(selector(page)).toHaveAttribute("data-agent-group-selector", origin);
      await expect(blank(page)).toHaveAttribute("data-agent-tab-start", originalDraft!);
      await expect(page.locator("[data-pane-view]")).toHaveCount(0);
      expect(herdrHasFocus(herdr, started!)).toBe(false);
      await screenshot(page, draft ? "group-tabs-global-keeps-draft" : "group-tabs-global-keeps-empty");
    } finally { try { daemon?.stop(); } finally { try { await gate.stop(); } finally { herdr.stop(); } } }
  }
});

test("adding an agent keeps the live tab and its existing pane, with the chosen model and prompt", async ({ page }) => {
  const herdr = await startHerdr({ agents: false });
  let daemon: Daemon | null = null;
  try {
    daemon = await startHided(herdr, "group-add-agent");
    await open(page, daemon);
    const group = await createGroup(page, "Agent work");
    await terminal(page).click();
    await expect(blank(page)).toHaveCount(0, { timeout: 30_000 });
    await expect(selector(page)).toHaveAttribute("data-agent-group-selector", group);
    await expect(page.locator("[data-pane-view]")).toHaveCount(1);
    const tab = (await stripTabs(page).getAttribute("data-tab"))!;
    const originalPane = (await page.locator("[data-pane-view]").getAttribute("data-pane-view"))!;
    const originalTabs = topology(herdr);
    await page.locator(`[data-add-agent="${tab}"]`).click();
    await expect(blank(page)).toBeVisible();
    await blank(page).locator("[data-agent-kind]").click();
    await expect(page.locator('[data-agent-kind-option="claude"]')).toBeEnabled({ timeout: 20_000 });
    await page.locator('[data-agent-kind-option="claude"]').click();
    const model = blank(page).locator("[data-agent-model-kind]");
    await expect(model).toBeEnabled();
    await model.click();
    await page.locator('[data-agent-model-option="opus"]').click();
    await blank(page).getByRole("textbox").fill("같은 탭의 새 에이전트");
    await page.locator("[data-group-start-agent=true]").click();
    await expect(blank(page)).toHaveCount(0, { timeout: 30_000 });
    await expect(page.locator("[data-pane-view]")).toHaveCount(2);
    await expect(page.locator(`[data-pane-view="${originalPane}"]`)).toBeVisible();
    expect(topology(herdr)).toEqual(originalTabs);
    const added = (await page.locator("[data-pane-view]").evaluateAll((panes) => panes.map((pane) => pane.getAttribute("data-pane-view")!))).find((id) => id !== originalPane)!;
    await expect.poll(() => {
      const info = herdr.run(["pane", "process-info", "--pane", added]) as { result: { process_info: { foreground_processes: { argv?: string[] | null }[] } } };
      return info.result.process_info.foreground_processes.map((process) => process.argv);
    }, { timeout: 30_000 }).toContainEqual([expect.stringMatching(/(?:^|[/\\])claude(?:\.exe)?$/), "--model", "opus", "--", "같은 탭의 새 에이전트"]);
    await screenshot(page, "group-tabs-agent-added");
  } finally { try { daemon?.stop(); } finally { herdr.stop(); } }
});

test("a committed start with its answer lost across restart adopts the original shell", async ({ page }) => {
  const herdr = await startHerdr({ agents: false });
  const gate = await herdrGate(herdr);
  let daemon: Daemon | null = null;
  try {
    daemon = await startHided(herdr, "group-recover", undefined, { HERDR_SOCKET_PATH: gate.socket });
    await open(page, daemon);
    await establishOwner(page);
    const original = topology(herdr);
    await createGroup(page, "Recovery");
    const held = gate.arm("layout.apply");
    await terminal(page).click();
    await held;
    const saved = JSON.parse(fs.readFileSync(path.join(daemon.stateDir, "workspace-views.json"), "utf8")) as { workspaces: { agent_starts: Record<string, { state: string }> }[] };
    expect(saved.workspaces.flatMap((view) => Object.values(view.agent_starts ?? {})).map((start) => start.state)).toContain("pending");
    await page.goto("about:blank");
    // The gate forwards to real Herdr after the old daemon has stopped. Its
    // exact effect exists, but the caller cannot receive the acknowledgement.
    process.kill(daemon.pid, "SIGKILL");
    await expect.poll(() => { try { process.kill(daemon!.pid, 0); return true; } catch { return false; } }).toBe(false);
    daemon = await daemon.restart(async () => { await gate.release(); });
    expect(topology(herdr)).toHaveLength(original.length + 1);
    await open(page, daemon);
    await expect(blank(page)).toBeVisible();
    await page.getByRole("button", { name: "Check start status", exact: true }).click();
    await expect(blank(page)).toHaveCount(0, { timeout: 30_000 });
    await expect(stripTabs(page)).toHaveCount(1);
    await quietFor(page, 300, "a restored start must not create a replacement after its original shell was adopted");
    expect(topology(herdr)).toHaveLength(original.length + 1);
    expect(gate.params("layout.apply")).toHaveLength(1);
  } finally { try { daemon?.stop(); } finally { try { await gate.stop(); } finally { herdr.stop(); } } }
});

test("a lost join answer preserves the existing zoomed tab and all three shells after restart", async ({ page }) => {
  const herdr = await startHerdr({ agents: false });
  const gate = await herdrGate(herdr);
  let daemon: Daemon | null = null;
  const identity = (pane: string) => {
    const row = herdr.run(["pane", "get", pane]) as { result: { pane: { terminal_id: string } } };
    const process = herdr.run(["pane", "process-info", "--pane", pane]) as { result: { process_info: { shell_pid: number } } };
    return { terminal: row.result.pane.terminal_id, pid: process.result.process_info.shell_pid };
  };
  const output = (pane: string) => {
    const read = spawnSync(herdr.bin, ["pane", "read", pane, "--source", "visible", "--format", "text"], { env: herdr.env, encoding: "utf8", timeout: 10_000 });
    expect(read.status, read.stderr).toBe(0);
    return read.stdout;
  };
  const submit = (pane: string, line: string) => {
    const sent = spawnSync(herdr.bin, ["pane", "run", pane, line], { env: herdr.env, encoding: "utf8", timeout: 10_000 });
    expect(sent.status, sent.stderr).toBe(0);
  };
  try {
    daemon = await startHided(herdr, "group-join-recover", undefined, { HERDR_SOCKET_PATH: gate.socket });
    await open(page, daemon);
    await createGroup(page, "Join recovery");
    await terminal(page).click();
    await expect(blank(page)).toHaveCount(0, { timeout: 30_000 });
    const tab = (await stripTabs(page).getAttribute("data-tab"))!;
    const first = (await page.locator("[data-pane-view]").getAttribute("data-pane-view"))!;
    const split = herdr.run(["pane", "split", "--pane", first, "--direction", "right", "--no-focus"]) as { result: { pane: { pane_id: string } } };
    const second = split.result.pane.pane_id;
    await expect(page.locator("[data-pane-view]")).toHaveCount(2);
    for (const [index, pane] of [first, second].entries()) {
      submit(pane, herdr.shell === "cmd" ? `set GROUP_JOIN_VALUE=original-${index} & echo BEFORE_JOIN_${index}` : `GROUP_JOIN_VALUE=original-${index}; printf 'BEFORE_JOIN_${index}\\n'`);
      await expect.poll(() => output(pane)).toContain(`BEFORE_JOIN_${index}`);
    }
    const originals = [identity(first), identity(second)];
    const originalTabs = topology(herdr);
    herdr.run(["pane", "zoom", "--pane", first, "--on"]);
    await page.locator(`[data-add-agent="${tab}"]`).click();
    const held = gate.arm("pane.move");
    await terminal(page).click();
    await held;
    await expect(stripTabs(page)).toHaveCount(1);
    const move = gate.params("pane.move")[0];
    const staged = move.pane_id as string;
    expect(staged).toBeTruthy();
    const stagedIdentity = identity(staged);
    const saved = JSON.parse(fs.readFileSync(path.join(daemon.stateDir, "workspace-views.json"), "utf8")) as { workspaces: { agent_starts: Record<string, { state: string; effect?: { binding?: { pane_id: string } } }> }[] };
    expect(saved.workspaces.flatMap(view => Object.values(view.agent_starts ?? {}))).toContainEqual(expect.objectContaining({ state: "pending", effect: expect.objectContaining({ binding: expect.objectContaining({ pane_id: staged }) }) }));
    await page.goto("about:blank");
    process.kill(daemon.pid, "SIGKILL");
    await expect.poll(() => { try { process.kill(daemon!.pid, 0); return true; } catch { return false; } }).toBe(false);
    daemon = await daemon.restart(async () => { await gate.release(); });
    await open(page, daemon);
    await page.getByRole("button", { name: "Check start status", exact: true }).click();
    await expect(blank(page)).toHaveCount(0, { timeout: 30_000 });
    await expect(page.locator("[data-pane-view]")).toHaveCount(3);
    expect(topology(herdr)).toEqual(originalTabs);
    expect([identity(first), identity(second)]).toEqual(originals);
    expect(identity(staged)).toEqual(stagedIdentity);
    expect(gate.params("pane.move")).toHaveLength(1);
    expect(gate.params("layout.apply")).toHaveLength(1);
    for (const [index, pane] of [first, second].entries()) {
      submit(pane, herdr.shell === "cmd" ? "echo AFTER_JOIN_%GROUP_JOIN_VALUE%" : "printf 'AFTER_JOIN_%s\\n' \"$GROUP_JOIN_VALUE\"");
      await expect.poll(() => output(pane)).toContain(`AFTER_JOIN_original-${index}`);
    }
    submit(staged, "echo ADOPTED_SHELL_ACCEPTS_INPUT");
    await expect.poll(() => output(staged)).toContain("ADOPTED_SHELL_ACCEPTS_INPUT");
    await screenshot(page, "group-tabs-join-recovered");
  } finally { try { daemon?.stop(); } finally { try { await gate.stop(); } finally { herdr.stop(); } } }
});
