import { expect, test, type Page } from "@playwright/test";
import { createHash } from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { agentIn, sendKeys, tabOfPane, withControl } from "./daemon-actions";
import { startFixture, type AgentFixture } from "./engine-fixture";
import { seedLedger, startHided, type Daemon } from "./hided-fixture";
import { chord } from "./chords";
import { draftId, seedDraft, storedDrafts } from "./drafts";
import { nativeSessionId, prepareEngineWriter } from "./session-reader-fixture";
import { enterWorkspace, screenshot } from "./wire";

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

/** Opens `name` from the session's Explorer in Desk's tab panel, with the session beside Desk, and gives its editor. */
async function openDeskFile(page: Page, desk: Desk, name: string) {
  await row(page, desk.pane).click();
  await expect(page.locator(`[data-desk-thread-panel="${desk.pane}"]`)).toBeVisible();
  await page.locator("[data-desk-session-files]").click();
  await page.locator(`[data-desk-tab-panel] [data-explorer-row$="/${name}"]`).dblclick();
  return page.locator(`[data-desk-file$="/${name}"] [data-editor-body] .cm-content`);
}

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

test("the session's Explorer opens a file in the tab panel as its checkout's own document, ⌘S saves it, and no Workspace View takes it in (PRD desk-2-ui B19, B28, B35, B36) @platform", async ({ page }) => {
  test.setTimeout(120_000);
  const desk = await startDesk(page, "desk-room-file");
  try {
    const file = path.join(desk.engine.root, "fixture", "plan.md");
    fs.writeFileSync(file, "plan\n");
    const editor = await openDeskFile(page, desk, "plan.md");
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

    // B28: a file the core cannot read says why, and 다시 시도 reads it once it can.
    // A mode bit locks a file only for an account that is not root, on a system that has them.
    if (process.platform !== "win32" && process.getuid?.() !== 0) {
      const locked = path.join(path.dirname(file), "locked.md");
      fs.writeFileSync(locked, "unlocked\n");
      fs.chmodSync(locked, 0o000);
      await page.locator('[data-desk-tab-panel] [data-explorer-row$="/locked.md"]').dblclick();
      const retry = page.locator('[data-desk-file-retry$="/locked.md"]');
      await expect(retry).toBeVisible();
      fs.chmodSync(locked, 0o644);
      await retry.click();
      await expect(page.locator('[data-desk-file$="/locked.md"] [data-editor-body] .cm-content')).toContainText("unlocked");
    }
  } finally {
    desk.daemon.stop();
    desk.engine.stop();
  }
});

test("a file saved with ⌘S in Desk's tab panel leaves no unsaved draft in the browser, so a reload raises no draft notice (PRD desk-2-ui B36) @platform", async ({ page }) => {
  test.setTimeout(120_000);
  const desk = await startDesk(page, "desk-room-saved-draft");
  try {
    const file = path.join(desk.engine.root, "fixture", "plan.md");
    fs.writeFileSync(file, "plan\n");
    const editor = await openDeskFile(page, desk, "plan.md");
    await expect(editor).toContainText("plan");
    await editor.click();
    await page.keyboard.press("ControlOrMeta+Home");
    await page.keyboard.press("End");
    await page.keyboard.type(" saved");
    // The edit is kept in the browser until it is saved, as a Workspace View keeps it.
    await expect.poll(async () => (await storedDrafts(page)).some((draft) => draft.path === file)).toBe(true);
    await page.keyboard.press(chord("save_file"));
    await expect.poll(() => fs.readFileSync(file, "utf8")).toBe("plan saved\n");
    // The saved file's copy goes with the save that landed.
    await expect.poll(async () => (await storedDrafts(page)).some((draft) => draft.path === file)).toBe(false);

    await page.reload();
    await page.locator("[data-desk-sidebar-row]").click();
    await expect(page.locator("[data-desk-screen]")).toBeVisible();
    expect(await storedDrafts(page)).toEqual([]);
    await expect(page.locator("[data-draft-recovery]")).toHaveCount(0);
    await screenshot(page, "desk-room-saved-reload");
  } finally {
    desk.daemon.stop();
    desk.engine.stop();
  }
});

test("a draft notice stands above Desk's input boxes, never over them (PRD desk-2-ui B36, UI_BEHAVIOR Notices) @platform", async ({ page }) => {
  test.setTimeout(120_000);
  const desk = await startDesk(page, "desk-room-notice");
  try {
    // A draft of another host's document: kept for recovery, and its notice stays until the operator acts.
    const repo = path.join(desk.engine.root, "fixture");
    const elsewhere = `${repo}/plan.md`;
    await seedDraft(page, { id: draftId("host-another", repo, elsewhere, "local"), host: "host-another", device: "local", root: repo, path: elsewhere, contents: "other host\n", updated_at: 1 });
    await page.reload();
    await page.locator("[data-desk-sidebar-row]").click();
    await row(page, desk.pane).click();
    await expect(page.locator(`[data-desk-thread-panel="${desk.pane}"]`)).toBeVisible();
    const notice = page.locator("[data-draft-recovery]");
    await expect(notice).toBeVisible({ timeout: 20_000 });
    await screenshot(page, "desk-room-notice");

    // Desk's own box and the session's beside it are both drawn, and the notice covers neither.
    const boxes = page.locator("[data-desk-composer]");
    await expect(boxes).toHaveCount(2);
    const over = (a: Box, b: Box) => a.x < b.x + b.width && b.x < a.x + a.width && a.y < b.y + b.height && b.y < a.y + a.height;
    await expect.poll(async () => {
      const shown = (await notice.boundingBox())!;
      const drawn = await Promise.all((await boxes.all()).map(async (box) => (await box.boundingBox())!));
      return drawn.filter((box) => over(shown, box)).length;
    }).toBe(0);
  } finally {
    desk.daemon.stop();
    desk.engine.stop();
  }
});

type Box = { x: number; y: number; width: number; height: number };

test("a session beside Desk that sleeps in its own pane offers 에이전트 깨우기 in its input box's place, which wakes it in that pane (PRD desk-2-ui B43) @platform", async ({ page }) => {
  test.setTimeout(150_000);
  const desk = await startDesk(page, "desk-room-sleep");
  try {
    await row(page, desk.pane).click();
    const panel = page.locator(`[data-desk-thread-panel="${desk.pane}"]`);
    await expect(panel).toBeVisible();
    await expect(panel.locator("[data-desk-composer]")).toHaveCount(1);
    // Sleep agent from the session head's own pane menu: the agent ends and its pane stays.
    await page.locator(`[data-desk-session-menu="${desk.pane}"]`).click();
    const item = page.locator('[data-menu-item="sleep_agent"]');
    await expect(item).toBeEnabled({ timeout: 20_000 });
    await item.click();

    await expect(panel.locator("[data-desk-session-sleeping]")).toHaveAttribute("data-desk-session-sleeping", /.+/, { timeout: 30_000 });
    await screenshot(page, "desk-room-sleep");
    const wake = panel.locator("[data-desk-session-wake]");
    await expect(wake).toBeVisible({ timeout: 30_000 });
    await expect(panel.locator("[data-desk-session-sleeping]")).toHaveAttribute("data-desk-session-sleeping", "sleeping");
    await expect(panel.locator("[data-desk-composer]")).toHaveCount(0);
    await expect(page.locator("[data-desk-session-terminal]")).toHaveCount(0);

    await wake.click();
    await expect(page.locator("[data-desk-session-terminal]")).toBeVisible({ timeout: 40_000 });
    await expect(panel.locator("[data-desk-session-sleeping]")).toHaveCount(0);
    await expect(panel.locator("[data-desk-composer]")).toHaveCount(1);
    await screenshot(page, "desk-room-woke");
  } finally {
    desk.daemon.stop();
    desk.engine.stop();
  }
});

test("a session beside Desk whose sleep closes its pane keeps its conversation above 에이전트 깨우기, which wakes it in a new pane beside Desk (PRD desk-2-ui B43) @platform", async ({ page }) => {
  test.setTimeout(180_000);
  // Pi's sleep closes its pane and keeps a sleeping session to wake in a new one (`session-readers.spec.ts`).
  const engine = await startFixture({ agents: false });
  const daemon = await startHided(engine, "desk-room-pi-sleep", path.join(engine.root, "home"));
  try {
    const pane = engine.panes[0]!;
    const session = prepareEngineWriter(engine, "pi");
    await withControl(daemon, (control) => sendKeys(control, pane, "pi\r"));
    await expect.poll(() => fs.existsSync(session)).toBe(true);
    await withControl(daemon, (control) => control.until((rest) => (agentIn(rest, pane)?.session_id === nativeSessionId("pi") ? true : undefined), `${pane} on its pi session`, 30_000));
    await page.goto("about:blank");
    await page.goto(`${daemon.origin}/?probe=1#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    await switchDesk(page, true);
    await page.locator("[data-desk-sidebar-row]").click();
    // A quiet agent's project can be folded before the page first reads its session.
    const folded = page.locator('nav[data-sidebar] [data-checkout-toggle][aria-expanded="false"]');
    await expect(row(page, pane).or(folded)).toBeVisible();
    if (!(await row(page, pane).isVisible())) await folded.click();
    await row(page, pane).click();
    const panel = page.locator(`[data-desk-thread-panel="${pane}"]`);
    await expect(panel.locator("[data-desk-thread-lines]")).toBeVisible({ timeout: 30_000 });

    await page.locator(`[data-desk-session-menu="${pane}"]`).click();
    await expect(page.locator('[data-menu-item="sleep_agent"]')).toBeEnabled({ timeout: 20_000 });
    await page.locator('[data-menu-item="sleep_agent"]').click();
    await withControl(daemon, (control) => control.until((rest) => (tabOfPane(rest, pane) === undefined ? true : undefined), `${pane} closed`, 30_000));
    const wake = panel.locator("[data-desk-session-wake]");
    await expect(wake).toBeVisible({ timeout: 30_000 });
    await screenshot(page, "desk-room-pi-asleep");
    await expect(panel.locator("[data-desk-thread-lines]")).toBeVisible();
    await expect(page.locator("[data-desk-session-terminal]")).toHaveCount(0);

    await wake.click();
    const fresh = await withControl(daemon, (control) => control.until((rest) => ((rest.navigator as { agents?: { pane_id: string; agent_kind: string | null }[] } | undefined)?.agents ?? []).find((agent) => agent.agent_kind === "pi" && agent.pane_id !== pane)?.pane_id, "a fresh pi agent", 30_000));
    await expect(page.locator(`[data-desk-thread-panel="${fresh}"]`)).toBeVisible({ timeout: 30_000 });
    await expect(page.locator("[data-desk-session-terminal]")).toBeVisible();
  } finally {
    daemon.stop();
    engine.stop();
  }
});

test("a child session beside Desk names its ancestors before its name, as its pane head does, and an ancestor's name opens it beside Desk (PRD desk-2-ui B47) @platform", async ({ page }) => {
  test.setTimeout(120_000);
  const engine = await startFixture({ family: true });
  const daemon = await startHided(engine, "desk-room-ancestors");
  try {
    const [parent, child] = engine.panes;
    await page.goto("about:blank");
    await page.goto(`${daemon.origin}/?probe=1#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    await switchDesk(page, true);
    await page.locator("[data-desk-sidebar-row]").click();
    // The child's row stands under its parent's.
    if (!(await row(page, child!).count())) await page.locator(`[data-tree-chevron="${parent}"]`).click();
    await row(page, child!).click();
    await expect(page.locator(`[data-desk-thread-panel="${child}"]`)).toBeVisible();

    const head = page.locator("[data-desk-session-head]");
    const ancestor = head.locator(`[data-pane-return="${parent}"]`);
    await expect(head.locator("[data-desk-session-title]")).toHaveText("Agent two");
    await screenshot(page, "desk-room-ancestors");
    await expect(ancestor.getByText("Agent one", { exact: true })).toBeVisible();
    await ancestor.click();
    await expect(page.locator(`[data-desk-thread-panel="${parent}"]`)).toBeVisible();
    await expect(head.locator("[data-pane-return]")).toHaveCount(0);
  } finally {
    daemon.stop();
    engine.stop();
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
