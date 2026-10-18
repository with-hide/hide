// Approved group-tabs B3/B5-B10/B12/B18/B21/B23: real private Herdr
// topology is the oracle for whether a blank tab starts anything.
import { expect, test, type Page } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";
import { startHerdr, type HerdrFixture } from "./herdr-fixture";
import { herdrGate } from "./herdr-gate";
import { startHided, type Daemon } from "./hided-fixture";
import { countSent, enterWorkspace, screenshot } from "./wire";
import { quietFor } from "./wait";

test.describe.configure({ timeout: 150_000 });
test.use({ actionTimeout: 15_000 });

const stripTabs = (page: Page) => page.locator("[data-agent-tab-bar] [role=tab][data-tab]");
const selector = (page: Page) => page.locator("[data-agent-group-selector]");
const terminal = (page: Page) => page.locator("[data-group-start-terminal=true]");
const blank = (page: Page) => page.locator("[data-agent-tab-start]");

function topology(herdr: HerdrFixture) {
  const answer = herdr.run(["tab", "list", "--workspace", herdr.workspace]) as { result: { tabs: { tab_id: string }[] } };
  return answer.result.tabs.map((tab) => tab.tab_id);
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
  return (await selector(page).getAttribute("data-agent-group-selector"))!;
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
    await page.goto("about:blank");
    daemon = await daemon.restart();
    await open(page, daemon);
    await expect(selector(page)).toHaveAttribute("data-agent-group-selector", group);
    await expect(blank(page)).toHaveAttribute("data-agent-tab-start", draft!);
    expect(topology(herdr)).toEqual(original);
  } finally { daemon?.stop(); herdr.stop(); }
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
    const original = topology(herdr);
    await createGroup(page, "Terminal work");
    await page.locator("[data-new-agent-tab=true]").click();
    const held = gate.arm("tab.create");
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
    expect(gate.params("tab.create")).toHaveLength(1);
    await screenshot(page, "group-tabs-terminal-promoted");
  } finally { daemon?.stop(); await gate.stop(); herdr.stop(); }
});

test("adding an agent keeps the live tab and its existing pane, with the chosen model and prompt", async ({ page }) => {
  const herdr = await startHerdr({ agents: false });
  let daemon: Daemon | null = null;
  try {
    daemon = await startHided(herdr, "group-add-agent");
    await open(page, daemon);
    await createGroup(page, "Agent work");
    await terminal(page).click();
    await expect(blank(page)).toHaveCount(0, { timeout: 30_000 });
    const tab = (await stripTabs(page).getAttribute("data-tab"))!;
    const originalPane = (await page.locator("[data-pane-view]").getAttribute("data-pane-view"))!;
    const originalTabs = topology(herdr);
    await page.locator(`[data-add-agent="${tab}"]`).click();
    await expect(blank(page)).toBeVisible();
    await blank(page).locator("[data-agent-kind]").click();
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
    await expect.poll(() => JSON.stringify(herdr.run(["pane", "process-info", "--pane", added])), { timeout: 30_000 }).toContain("opus");
    await expect.poll(() => JSON.stringify(herdr.run(["pane", "read", added, "--source", "visible"])), { timeout: 30_000 }).toContain("같은 탭의 새 에이전트");
    await screenshot(page, "group-tabs-agent-added");
  } finally { daemon?.stop(); herdr.stop(); }
});

test("a committed start with its answer lost across restart adopts the original shell", async ({ page }) => {
  const herdr = await startHerdr({ agents: false });
  const gate = await herdrGate(herdr);
  let daemon: Daemon | null = null;
  try {
    daemon = await startHided(herdr, "group-recover", undefined, { HERDR_SOCKET_PATH: gate.socket });
    await open(page, daemon);
    const original = topology(herdr);
    await createGroup(page, "Recovery");
    const held = gate.arm("tab.create");
    await terminal(page).click();
    await held;
    const saved = JSON.parse(fs.readFileSync(path.join(daemon.stateDir, "workspace-views.json"), "utf8")) as { workspaces: { agent_starts: Record<string, { state: string }> }[] };
    expect(saved.workspaces.flatMap((view) => Object.values(view.agent_starts ?? {})).map((start) => start.state)).toContain("pending");
    await page.goto("about:blank");
    // The gate forwards to real Herdr after the old daemon has stopped. Its
    // exact effect exists, but the caller cannot receive the acknowledgement.
    daemon = await daemon.restart(async () => { await gate.release(); });
    expect(topology(herdr)).toHaveLength(original.length + 1);
    await open(page, daemon);
    await expect(blank(page)).toBeVisible();
    await page.getByRole("button", { name: "Check start status", exact: true }).click();
    await expect(blank(page)).toHaveCount(0, { timeout: 30_000 });
    await expect(stripTabs(page)).toHaveCount(1);
    await quietFor(page, 300, "a restored start must not create a replacement after its original shell was adopted");
    expect(topology(herdr)).toHaveLength(original.length + 1);
    expect(gate.params("tab.create")).toHaveLength(1);
  } finally { daemon?.stop(); await gate.stop(); herdr.stop(); }
});
