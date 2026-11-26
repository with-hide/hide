import { expect, test, type Page } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";
import { startFixture, type AgentFixture } from "./engine-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { enterWorkspace, screenshot } from "./wire";

// 한눈에 at Desk's right (the approved picture's sections
// 1 and 2): shown whenever no session is open beside Desk, listing the
// sessions opened beside Desk and the files they opened; a session open
// beside Desk shows `‹ 한눈에 › <session>` above it, whose `‹` and Escape go
// back, and its head has no `✕`; the width dragged on 한눈에's edge is the
// session panel's width too. The family fixture's agents run `hide` from
// their own panes, so a session's file open is a real agent's.

type Stack = { page: Page; engine: AgentFixture; daemon: Daemon };

async function startGlance(page: Page, name: string): Promise<Stack> {
  const engine = await startFixture({ family: true });
  const daemon = await startHided(engine, name);
  await page.goto("about:blank");
  await page.goto(`${daemon.origin}/?probe=1#token=${daemon.token}`);
  await enterWorkspace(page, "fixture");
  await page.locator("[data-open-settings]").click();
  await page.locator('[data-settings-tab="labs"]').click();
  const use = page.locator("[data-desk-use]");
  if ((await use.getAttribute("data-desk-use")) !== "true") await use.click();
  await expect(use).toHaveAttribute("data-desk-use", "true");
  await page.locator("[data-settings-close]").click();
  await page.locator("[data-desk-sidebar-row]").click();
  await expect(page.locator("[data-desk-screen]")).toBeVisible();
  return { page, engine, daemon };
}

const room = (page: Page) => page.locator("[data-desk-room]");
const row = (page: Page, pane: string) => page.locator(`[data-agent-open="${pane}"]`);

/** Opens `pane`'s session beside Desk from its sidebar row, unfolding its parent's children first. */
async function openBeside(page: Page, engine: AgentFixture, pane: string) {
  const [parent] = engine.panes;
  if (!(await row(page, pane).count())) await page.locator(`[data-tree-chevron="${parent}"]`).click();
  await row(page, pane).click();
  await expect(page.locator(`[data-desk-thread-panel="${pane}"]`)).toBeVisible();
}

test("한눈에 lists the sessions opened beside Desk and the file one opened, whose row opens it on Desk's desk and turns it seen @platform", async ({ page }) => {
  test.setTimeout(150_000);
  const { engine, daemon } = await startGlance(page, "desk-glance-list");
  try {
    const [parent, child] = engine.panes;
    await expect(room(page)).toHaveAttribute("data-desk-room", "glance");
    await expect(page.locator("[data-desk-glance-empty]")).toBeVisible();

    await openBeside(page, engine, parent!);
    await openBeside(page, engine, child!);
    const notes = path.join(engine.root, "fixture", "notes.md");
    fs.writeFileSync(notes, "# notes\n");
    const opened = await engine.runAs(child!)(["file", "open", notes]);
    expect(opened.status, opened.out).toBe(0);

    await page.locator("[data-desk-glance-back]").click();
    await expect(room(page)).toHaveAttribute("data-desk-room", "glance");
    // Both work, so the newest opened is first; the file names its session as the sidebar does.
    await expect(page.locator("[data-desk-glance-session]")).toHaveCount(2);
    await expect(page.locator("[data-desk-glance-session]").first()).toHaveAttribute("data-desk-glance-session", child!);
    const file = page.locator("[data-desk-glance-file]");
    await expect(file).toHaveCount(1);
    await expect(file).toContainText("notes.md");
    await expect(file.locator("[data-desk-glance-by]")).toHaveText("Agent two");
    await expect(file).toHaveAttribute("data-desk-glance-unseen", "true");
    await screenshot(page, "desk-glance-list");

    await file.click();
    await expect(page.locator('[data-desk-tab-panel] [role="tab"][aria-selected="true"]')).toContainText("notes.md");
    await expect(page.locator("[data-desk-tab-panel]")).toHaveAttribute("data-desk-tab-panel", "beside_session");
    await page.keyboard.press("Escape");
    await expect(page.locator("[data-desk-tab-panel]")).toHaveCount(0);
    await expect(file).not.toHaveAttribute("data-desk-glance-unseen", "true");

    await page.locator(`[data-desk-glance-session="${parent}"]`).click();
    await expect(page.locator(`[data-desk-thread-panel="${parent}"]`)).toBeVisible();
  } finally {
    daemon.stop();
    engine.stop();
  }
});

test("⟩ folds 한눈에 so Desk's chat takes the room, the core keeps it folded across a reload, and Desk's head unfolds it @platform", async ({ page }) => {
  test.setTimeout(120_000);
  const { engine, daemon } = await startGlance(page, "desk-glance-fold");
  try {
    const chat = page.locator("[data-desk-chat-column]");
    const narrow = (await chat.boundingBox())!.width;
    await page.locator("[data-desk-glance-fold]").click();
    await expect(room(page)).toHaveAttribute("data-desk-room", "closed");
    await expect.poll(async () => (await chat.boundingBox())!.width).toBeGreaterThan(narrow);
    await page.reload();
    await page.locator("[data-desk-sidebar-row]").click();
    await expect(page.locator("[data-desk-screen]")).toBeVisible();
    await expect(room(page)).toHaveAttribute("data-desk-room", "closed");
    await page.locator("[data-desk-glance-unfold]").click();
    await expect(room(page)).toHaveAttribute("data-desk-room", "glance");
  } finally {
    daemon.stop();
    engine.stop();
  }
});

test("a session beside Desk shows ‹ 한눈에 › above it, whose ‹ and Escape go back to 한눈에, and its head has no ✕ but keeps ↗ @platform", async ({ page }) => {
  test.setTimeout(120_000);
  const { engine, daemon } = await startGlance(page, "desk-glance-crumb");
  try {
    const [parent] = engine.panes;
    await openBeside(page, engine, parent!);
    const crumb = page.locator("[data-desk-glance-crumb]");
    await expect(crumb).toContainText("Agent one");
    await expect(page.locator("[data-desk-session-head] [data-desk-session-close]")).toHaveCount(0);
    await expect(page.locator("[data-desk-session-head] [data-desk-session-own-tab]")).toBeVisible();
    await screenshot(page, "desk-glance-crumb");

    await crumb.locator("[data-desk-glance-back]").click();
    await expect(room(page)).toHaveAttribute("data-desk-room", "glance");
    await page.locator(`[data-desk-glance-session="${parent}"]`).click();
    await expect(room(page)).toHaveAttribute("data-desk-room", "session");
    await page.locator("[data-desk-chat-column] [data-desk-composer-input]").focus();
    await page.keyboard.press("Escape");
    await expect(room(page)).toHaveAttribute("data-desk-room", "glance");
  } finally {
    daemon.stop();
    engine.stop();
  }
});

test("the width dragged on 한눈에's edge is the width the session beside Desk opens at @platform", async ({ page }) => {
  test.setTimeout(120_000);
  const { engine, daemon } = await startGlance(page, "desk-glance-width");
  try {
    const [parent] = engine.panes;
    const right = page.locator("[data-desk-session-panel]");
    const width = async () => Math.round((await right.boundingBox())!.width);
    await expect(room(page)).toHaveAttribute("data-desk-room", "glance");
    const before = await width();
    const box = (await page.locator('[data-desk-edge="session"]').boundingBox())!;
    await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
    await page.mouse.down();
    await page.mouse.move(box.x + box.width / 2 - 120, box.y + box.height / 2, { steps: 8 });
    await page.mouse.up();
    await expect.poll(width).toBe(before + 120);
    await openBeside(page, engine, parent!);
    await expect(room(page)).toHaveAttribute("data-desk-room", "session");
    await expect.poll(width).toBe(before + 120);
  } finally {
    daemon.stop();
    engine.stop();
  }
});
