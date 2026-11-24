import { expect, test, type Page } from "@playwright/test";
import { createHash } from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { startFixture, type AgentFixture } from "./engine-fixture";
import { seedLedger, startHided, type Daemon } from "./hided-fixture";
import { chord } from "./chords";
import { enterWorkspace } from "./wire";

// PRD desk-2-ui B2-B7, B21, B44 and PRD session-docs-and-assistant B41:
// Desk's room with a session opened beside it from the sidebar, ⌘J between
// that session and its Agent tab, and Settings › Labs › Desk's switch, off on
// a new install. Each test starts one stack and asserts one contract.
//
// A Desk job is a spawn of Desk's own identity in the delivery ledger. Only
// Desk's agent makes one, and the stack runs none, so the fixture writes the
// spawn the core would have recorded (its shape is `coordination::started`'s
// `register` and `spawn`) for a real pane of the fixture's engine, while the
// daemon is down.

type Desk = { page: Page; engine: AgentFixture; daemon: Daemon; pane: string };

const digest = (value: string) => createHash("sha256").update(value).digest("hex");

/** The ledger with Desk registered and one completed spawn of Desk's whose child runs in `pane`. */
function deskLedger(node: string, pane: string, repo: string): unknown {
  const desk = { pane_id: "hide:desk", name: "hide:desk", kind: "hide", device_id: node, session: digest("hide:desk") };
  return {
    version: 1,
    next_id: 4,
    letters: [],
    watches: [],
    agents: [
      { id: "agent-1", name: "hide:desk", machine: node, host_scope: "hide", native_machine: "hide", session: "hide:desk", instance: "hide:desk", pane: "hide:desk", parent: null, origin: null, project: null, actor: desk, ended: false, forked: false },
      { id: "agent-3", name: "gallery-compare", machine: node, host_scope: "claude", native_machine: "claude", session: "fixture-two", instance: pane, pane, parent: null, origin: "agent-1", project: repo, actor: { pane_id: pane, name: "gallery-compare", kind: "claude", device_id: node, session: digest("fixture-two") }, ended: false, forked: false },
    ],
    spawns: [
      { names: { workspace: "pending", tab: "pending" }, id: "spawn-2", parent: "agent-1", mode: "handoff", machine: null, intent: "desk-room-intent", name: "gallery-compare", kind: "claude", repo, branch: "main", requested_path: null, args: [], path: repo, pane, terminal: null, typed: 0, shell: null, child: "agent-3", completed: true, auto_watch_id: null, reserved_at_unix_ms: 1 },
    ],
    spawn_placement_revision: 1,
  };
}

/** Settings › Labs › Desk's switch, turned to `on` and closed again; every window follows the core without a restart. */
async function switchDesk(page: Page, on: boolean) {
  await page.locator("[data-open-settings]").click();
  await page.locator('[data-settings-tab="labs"]').click();
  const use = page.locator("[data-desk-use]");
  if ((await use.getAttribute("data-desk-use")) !== String(on)) await use.click();
  await expect(use).toHaveAttribute("data-desk-use", String(on));
  await page.locator("[data-settings-close]").click();
  await expect(page.locator("[data-settings]")).toHaveCount(0);
}

/**
 * A Workspace whose second pane is a session Desk started, Desk switched on,
 * and the page on Desk's room unless `open` is false. The sidebar lists an
 * agent's pane as its row, so a spec that presses a row runs the fixture's
 * stand-in agents (`agents`); its second agent resumes `fixture-two`, the
 * session the ledger names for Desk's spawn.
 */
async function startDesk(page: Page, name: string, open = true, agents = true): Promise<Desk> {
  const engine = await startFixture({ agents });
  const repo = path.join(engine.root, "fixture");
  let daemon = await startHided(engine, name);
  const pane = engine.panes[1];
  daemon = await daemon.restart((stateDir) => {
    seedLedger(stateDir, JSON.stringify(deskLedger(daemon.node, pane, repo)));
  });
  await page.goto("about:blank");
  await page.goto(`${daemon.origin}/?probe=1#token=${daemon.token}`);
  await enterWorkspace(page, "fixture");
  if (!open) return { page, engine, daemon, pane };
  await switchDesk(page, true);
  await page.locator("[data-desk-sidebar-row]").click();
  await expect(page.locator("[data-desk-screen]")).toBeVisible();
  return { page, engine, daemon, pane };
}

const room = (page: Page) => page.locator("[data-desk-room]");
const row = (page: Page, pane: string) => page.locator(`[data-agent-open="${pane}"]`);

test("a sidebar row opens its session beside Desk, marked on its row, Escape closes it, and a ⌘-click goes to its tab (PRD desk-2-ui B2-B6, B21) @platform", async ({ page }) => {
  test.setTimeout(120_000);
  const desk = await startDesk(page, "desk-room-beside");
  try {
    await expect(room(page)).toHaveAttribute("data-desk-room", "closed");
    await row(page, desk.pane).click();
    await expect(room(page)).toHaveAttribute("data-desk-room", "session");
    await expect(page.locator("[data-desk-screen]")).toBeVisible();
    await expect(page.locator(`[data-desk-thread-panel="${desk.pane}"]`)).toBeVisible();
    await expect(page.locator("[data-desk-session-panel]")).toHaveAttribute("data-desk-session-panel", "docked");
    await expect(row(page, desk.pane)).toHaveAttribute("data-desk-beside", "true");

    await page.locator("[data-desk-chat-column] [data-desk-composer-input]").focus();
    await page.keyboard.press("Escape");
    await expect(room(page)).toHaveAttribute("data-desk-room", "closed");
    await expect(row(page, desk.pane)).not.toHaveAttribute("data-desk-beside", "true");

    await row(page, desk.pane).click({ modifiers: [process.platform === "darwin" ? "Meta" : "Control"] });
    await expect(page.locator("[data-workspace-screen]")).toBeVisible();
    await expect(page.locator(`[data-pane-view="${desk.pane}"]`)).toHaveAttribute("data-focused", "true");
  } finally {
    desk.daemon.stop();
    desk.engine.stop();
  }
});

test("⌘J goes from the session beside Desk to its Agent tab and back to Desk with the session beside it again (PRD desk-2-ui B7) @platform", async ({ page }) => {
  test.setTimeout(120_000);
  const desk = await startDesk(page, "desk-room-terminal");
  try {
    await row(page, desk.pane).click();
    const panel = page.locator(`[data-desk-thread-panel="${desk.pane}"]`);
    await expect(panel).toBeVisible();

    await page.keyboard.press(chord("desk_terminal"));
    await expect(page.locator("[data-workspace-screen]")).toBeVisible();
    await expect(page.locator(`[data-pane-view="${desk.pane}"]`)).toHaveAttribute("data-focused", "true");
    await expect(page.locator(`[data-pane-view="${desk.pane}"] [data-desk-pane-to-desk]`)).toBeVisible();

    await page.keyboard.press(chord("desk_terminal"));
    await expect(page.locator("[data-desk-screen]")).toBeVisible();
    await expect(panel).toBeVisible();

    // `터미널` in the session head does what ⌘J does.
    await page.locator("[data-desk-session-terminal]").click();
    await expect(page.locator(`[data-pane-view="${desk.pane}"]`)).toHaveAttribute("data-focused", "true");
  } finally {
    desk.daemon.stop();
    desk.engine.stop();
  }
});

test("the session's Explorer opens a file in the tab panel as its checkout's own document, ⌘S saves it, and no Workspace View takes it in (PRD desk-2-ui B19, B35, B36) @platform", async ({ page }) => {
  test.setTimeout(120_000);
  const desk = await startDesk(page, "desk-room-file");
  try {
    const file = path.join(desk.engine.root, "fixture", "plan.md");
    fs.writeFileSync(file, "plan\n");
    await row(page, desk.pane).click();
    await expect(page.locator(`[data-desk-thread-panel="${desk.pane}"]`)).toBeVisible();
    await page.locator("[data-desk-session-files]").click();
    const entry = page.locator('[data-desk-tab-panel] [data-explorer-row$="/plan.md"]');
    await entry.dblclick();
    const editor = page.locator('[data-desk-file$="/plan.md"] [data-editor-body] .cm-content');
    await expect(editor).toContainText("plan");
    await editor.click();
    await page.keyboard.press("ControlOrMeta+Home");
    await page.keyboard.press("End");
    await page.keyboard.type(" saved");
    await page.keyboard.press(chord("save_file"));
    await expect.poll(() => fs.readFileSync(file, "utf8")).toBe("plan saved\n");
    // Desk is still in front, and the Workspace's View never showed the file.
    await expect(page.locator("[data-desk-screen]")).toBeVisible();
    await expect(page.locator('[data-view-area] [data-editor-path]')).toHaveCount(0);
  } finally {
    desk.daemon.stop();
    desk.engine.stop();
  }
});

test("Desk is off on a new install, its switch in Settings shows it without a restart, and switching it off in the room goes back to the tab before it with Desk's session kept (PRD session-docs-and-assistant B41) @platform", async ({ page }) => {
  test.setTimeout(120_000);
  const desk = await startDesk(page, "desk-room-switch", false, false);
  try {
    await expect(page.locator("[data-workspace-screen]")).toBeVisible();
    await expect(page.locator("[data-desk-sidebar-row]")).toHaveCount(0);
    await expect(page.locator(`[data-pane-view="${desk.pane}"] [data-desk-pane-to-desk]`)).toHaveCount(0);
    const front = await page.locator('[data-pane-view][data-focused="true"]').getAttribute("data-pane-view");

    await switchDesk(page, true);
    await expect(page.locator(`[data-pane-view="${desk.pane}"] [data-desk-pane-to-desk]`)).toBeVisible();
    await page.locator("[data-desk-sidebar-row]").click();
    await expect(page.locator("[data-desk-screen]")).toBeVisible();

    await switchDesk(page, false);
    await expect(page.locator("[data-desk-screen]")).toHaveCount(0);
    await expect(page.locator("[data-workspace-screen]")).toBeVisible();
    await expect(page.locator(`[data-pane-view="${front}"]`)).toHaveAttribute("data-focused", "true");
    await expect(page.locator("[data-desk-sidebar-row]")).toHaveCount(0);
    // The session Desk started stays, an ordinary session with no way back to a Desk that is off.
    await expect(page.locator(`[data-pane-view="${desk.pane}"]`)).toBeVisible();
    await expect(page.locator(`[data-pane-view="${desk.pane}"] [data-desk-pane-to-desk]`)).toHaveCount(0);
  } finally {
    desk.daemon.stop();
    desk.engine.stop();
  }
});
