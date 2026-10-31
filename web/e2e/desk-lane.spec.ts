import { expect, test, type Page } from "@playwright/test";
import { createHash } from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { startHerdr, type HerdrFixture } from "./herdr-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { chord } from "./chords";
import { enterWorkspace } from "./wire";

// PRD session-docs-and-assistant B28, B29, B32, B35, B41: Desk's lane, its
// rail, its pin, ⌘J between a thread and its Agent tab, and Settings › Desk's
// switch, off on a new install. Each test starts one stack and asserts one
// contract.
//
// A Desk job is a spawn of Desk's own identity in the delivery ledger. Only
// Desk's agent makes one, and the stack runs none, so the fixture writes the
// spawn the core would have recorded (its shape is `coordination::started`'s
// `register` and `spawn`) for a real pane of the fixture's Herdr, while the
// daemon is down.

type Desk = { page: Page; herdr: HerdrFixture; daemon: Daemon; pane: string };

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
      { id: "agent-3", name: "gallery-compare", machine: node, host_scope: "claude", native_machine: "claude", session: "desk-lane-session", instance: pane, pane, parent: null, origin: "agent-1", project: repo, actor: { pane_id: pane, name: "gallery-compare", kind: "claude", device_id: node, session: digest("desk-lane-session") }, ended: false, forked: false },
    ],
    spawns: [
      { names: { workspace: "pending", tab: "pending" }, id: "spawn-2", parent: "agent-1", mode: "handoff", machine: null, intent: "desk-lane-intent", name: "gallery-compare", kind: "claude", repo, branch: "main", requested_path: null, args: [], path: repo, pane, terminal: null, typed: 0, shell: null, child: "agent-3", completed: true, auto_watch_id: null, reserved_at_unix_ms: 1 },
    ],
    spawn_placement_revision: 1,
    node_clocks: {},
  };
}

/** Settings › Desk's switch, turned to `on` and closed again; every window follows the core without a restart. */
async function switchDesk(page: Page, on: boolean) {
  await page.locator("[data-open-settings]").click();
  await page.locator('[data-settings-tab="desk"]').click();
  const use = page.locator("[data-desk-use]");
  if ((await use.getAttribute("data-desk-use")) !== String(on)) await use.click();
  await expect(use).toHaveAttribute("data-desk-use", String(on));
  await page.locator("[data-settings-close]").click();
  await expect(page.locator("[data-settings]")).toHaveCount(0);
}

/** A Workspace whose second pane is a thread Desk started, Desk switched on, and the page on Desk's room unless `open` is false. */
async function startDesk(page: Page, name: string, open = true): Promise<Desk> {
  const herdr = await startHerdr({ agents: false });
  const repo = path.join(herdr.root, "fixture");
  const pane = herdr.panes[1];
  let daemon = await startHided(herdr, name);
  daemon = await daemon.restart((stateDir) => {
    fs.writeFileSync(path.join(stateDir, "delivery-ledger.json"), JSON.stringify(deskLedger(daemon.node, pane, repo)));
  });
  await page.goto("about:blank");
  await page.goto(`${daemon.origin}/?probe=1#token=${daemon.token}`);
  await enterWorkspace(page, "fixture");
  if (!open) return { page, herdr, daemon, pane };
  await switchDesk(page, true);
  await page.locator("[data-desk-sidebar-row]").click();
  await expect(page.locator("[data-desk-screen]")).toBeVisible();
  return { page, herdr, daemon, pane };
}

const room = (page: Page) => page.locator("[data-desk-room]");
const rail = (page: Page, key: "threads" | "outputs") => page.locator(`[data-desk-rail="${key}"]`);

test("☰ opens the Threads list over the chat and a row opens its thread in the tool column (PRD session-docs-and-assistant B28, B25) @platform", async ({ page }) => {
  test.setTimeout(120_000);
  const desk = await startDesk(page, "desk-lane-threads");
  try {
    await expect(room(page)).toHaveAttribute("data-desk-room", "closed");
    await rail(page, "threads").click();
    await expect(room(page)).toHaveAttribute("data-desk-room", "over");
    await expect(rail(page, "threads")).toHaveAttribute("aria-pressed", "true");
    const row = page.locator('[data-desk-thread="desk-lane-intent"]');
    await expect(row).toBeVisible();
    await row.click();
    await expect(page.locator(`[data-desk-thread-panel="${desk.pane}"]`)).toBeVisible();
    // ‹ goes back to the list; ☰ pressed again closes the column and the lane with it.
    await page.locator("[data-desk-thread-back]").click();
    await expect(row).toBeVisible();
    await rail(page, "threads").click();
    await expect(room(page)).toHaveAttribute("data-desk-room", "closed");
    // ▣ opens the 보기 column alone, with every thread's outputs, none yet.
    await rail(page, "outputs").click();
    await expect(page.locator("[data-desk-lane-column]")).toHaveAttribute("data-desk-lane-column", "view");
    await expect(page.locator('[data-desk-view-column] [data-desk-outputs="0"]')).toBeVisible();
  } finally {
    desk.daemon.stop();
    desk.herdr.stop();
  }
});

test("📌 beside ✕ docks Desk's lane beside the chat, the core keeps it through a reload, and Escape closes only an unpinned lane (PRD session-docs-and-assistant B32) @platform", async ({ page }) => {
  test.setTimeout(120_000);
  const desk = await startDesk(page, "desk-lane-pin");
  try {
    await rail(page, "threads").click();
    const pin = page.locator('[data-desk-lane-column="tool"] [data-lane-pin]');
    await expect(pin).toHaveAttribute("data-lane-pin", "off");
    const over = await page.locator("[data-desk-chat-column]").boundingBox();
    await pin.click();
    await expect(room(page)).toHaveAttribute("data-desk-room", "pinned");
    await expect(pin).toHaveAttribute("data-lane-pin", "on");
    const docked = await page.locator("[data-desk-chat-column]").boundingBox();
    expect(docked!.width).toBeLessThan(over!.width);
    // Escape leaves a pinned lane where it is. A row's hint opens with its
    // focus and takes the first Escape, as any popover does.
    const row = page.locator('[data-desk-thread="desk-lane-intent"]');
    const escape = async () => {
      await row.focus();
      await page.keyboard.press("Escape");
      await page.keyboard.press("Escape");
    };
    await escape();
    await expect(room(page)).toHaveAttribute("data-desk-room", "pinned");

    // The pin is the core's: a new page opens the lane docked.
    await page.reload();
    await page.locator("[data-desk-sidebar-row]").click();
    await rail(page, "threads").click();
    await expect(room(page)).toHaveAttribute("data-desk-room", "pinned");
    await pin.click();
    await expect(room(page)).toHaveAttribute("data-desk-room", "over");
    await escape();
    await expect(room(page)).toHaveAttribute("data-desk-room", "closed");
    await expect(page.locator("[data-desk-chat-column] [data-desk-composer-input]")).toBeFocused();
  } finally {
    desk.daemon.stop();
    desk.herdr.stop();
  }
});

test("⌘J goes from a thread to its Agent tab and back to Desk with the thread still open (PRD session-docs-and-assistant B35) @platform", async ({ page }) => {
  test.setTimeout(120_000);
  const desk = await startDesk(page, "desk-lane-terminal");
  try {
    await rail(page, "threads").click();
    await page.locator('[data-desk-thread="desk-lane-intent"]').click();
    const panel = page.locator(`[data-desk-thread-panel="${desk.pane}"]`);
    await expect(panel).toBeVisible();
    await expect(panel.locator("[data-desk-thread-terminal-chord]")).toBeVisible();

    await page.keyboard.press(chord("desk_terminal"));
    await expect(page.locator("[data-workspace-screen]")).toBeVisible();
    await expect(page.locator(`[data-pane-view="${desk.pane}"]`)).toHaveAttribute("data-focused", "true");
    await expect(page.locator(`[data-pane-view="${desk.pane}"] [data-desk-pane-to-desk]`)).toBeVisible();

    await page.keyboard.press(chord("desk_terminal"));
    await expect(page.locator("[data-desk-screen]")).toBeVisible();
    await expect(panel).toBeVisible();

    // `터미널로` does what ⌘J does.
    await panel.locator("[data-desk-thread-terminal]").click();
    await expect(page.locator(`[data-pane-view="${desk.pane}"]`)).toHaveAttribute("data-focused", "true");
  } finally {
    desk.daemon.stop();
    desk.herdr.stop();
  }
});

test("Desk is off on a new install, Settings › Desk shows it without a restart, and switching it off in the room goes back to the tab before it with Desk's session kept (PRD session-docs-and-assistant B41) @platform", async ({ page }) => {
  test.setTimeout(120_000);
  const desk = await startDesk(page, "desk-lane-switch", false);
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
    desk.herdr.stop();
  }
});
