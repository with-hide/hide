import { expect, test, type Page } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";
import { agentPane } from "./agent-pane";
import { startHerdr } from "./herdr-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { enterWorkspace, screenshot, setLanePinned, showExplorer } from "./wire";

// View openers and the operator's comments (PRD view-opener-feedback): an
// agent's `hide file open` marks the View it opened, the operator comments on
// a line of it, and Send delivers one letter from the operator to that agent.

test.describe.configure({ timeout: 180_000 });
test.use({ actionTimeout: 15_000 });

const front = (page: Page) => page.locator('[data-view-tab-bar] [role="tab"][aria-selected="true"]');

/** Opens the note under line `line` of the View in front through the `+` the gutter shows on the pointer's row (B12). */
async function noteOnLine(page: Page, line: number, note: string): Promise<void> {
  await page.locator(".cm-lineNumbers .cm-gutterElement", { hasText: new RegExp(`^${line}$`) }).hover();
  await page.locator(`[data-view-comment-add="${line}"]`).click();
  const field = page.locator("[data-view-note-field]");
  await expect(field).toBeFocused();
  await field.fill(note);
  await page.keyboard.press("ControlOrMeta+Enter");
  await expect(page.locator("[data-view-note-form]")).toHaveCount(0);
}

test("an agent's View wears its mark, and the operator's line comment reaches that agent as one letter", async ({ page }) => {
  await page.setViewportSize({ width: 1920, height: 1080 });
  const herdr = await startHerdr({ agents: false });
  let daemon: Daemon | null = null;
  try {
    const fixture = path.join(herdr.root, "fixture");
    fs.writeFileSync(path.join(fixture, "notes.md"), ["# Notes", "", "The first line.", "The second line, long enough to wrap onto more rows of the View than one. ".repeat(8).trim(), "The end."].join("\n") + "\n");
    const pane = herdr.panes[0]!;
    daemon = await startHided(herdr, "view-feedback");
    const agent = await agentPane(herdr, daemon.stateDir, pane);
    const hide = async (args: string[]) => {
      const answer = await agent(args);
      expect(answer.status, answer.out).toBe(0);
      return answer.out;
    };
    await page.goto(`${daemon.origin}/?probe=1#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    await showExplorer(page);
    await setLanePinned(page, true);

    // B1, B5: the agent's open marks the View with its logo; the mark says who and leads to its pane.
    await hide(["file", "open", "notes.md"]);
    await expect(front(page)).toHaveAttribute("aria-label", /\/notes\.md/);
    await expect(front(page).locator("[data-view-opener]")).toHaveCount(0);
    const mark = page.locator("[data-document-header] [data-view-opener]");
    await expect(mark).toHaveAttribute("data-view-opener", pane);
    await expect(mark).toHaveAttribute("aria-label", /Claude Code/);
    // B8: the agent's pane counts the View it opened.
    await expect(page.locator(`[data-pane-views="${pane}"]`)).toHaveText(/1 view/);

    // B12: on a wrapped line the `+` sits beside the line number, on the line's first row, not halfway down.
    const numberOf = (line: number) => page.locator(".cm-lineNumbers .cm-gutterElement", { hasText: new RegExp(`^${line}$`) });
    const oneRow = (await numberOf(3).boundingBox())!;
    const wrapped = (await numberOf(4).boundingBox())!;
    expect(wrapped.height, "line 4 must wrap for this check").toBeGreaterThan(oneRow.height * 1.5);
    await numberOf(4).hover();
    await screenshot(page, "comment-plus-wrapped-line");
    const plus = (await page.locator('[data-view-comment-add="4"]').boundingBox())!;
    expect(plus.y).toBeGreaterThanOrEqual(wrapped.y);
    expect(plus.y + plus.height).toBeLessThanOrEqual(wrapped.y + oneRow.height);

    // B12, B14: a comment on line 3 shows under it, on the tab and on the line under the View.
    await expect(page.locator(".cm-content")).toContainText("The first line.");
    await noteOnLine(page, 3, "Say what the first line is for.");
    await expect(page.locator('[data-view-comment-number="1"]')).toContainText("Say what the first line is for.");
    await expect(front(page).locator("[data-view-comments]")).toHaveText("1");
    const row = page.locator("[data-view-feedback-row]");
    await expect(row).toHaveAttribute("data-view-feedback-state", "unsent");
    await expect(row.locator("[data-view-feedback-summary]")).toHaveText(/1 comment · 1 view/);

    // B43: a deleted comment comes back with Undo, at the same number.
    await noteOnLine(page, 5, "Throwaway.");
    const second = page.locator('[data-view-comment-number="2"]');
    await second.locator("[data-view-comment-delete]").click();
    await expect(second).toHaveCount(0);
    await expect(row).toHaveAttribute("data-view-feedback-state", "deleted");
    await row.locator("[data-view-feedback-undo]").click();
    await expect(page.locator('[data-view-comment-number="2"]')).toContainText("Throwaway.");
    await page.locator('[data-view-comment-number="2"] [data-view-comment-delete]').click();
    await expect(page.locator("[data-view-comment]")).toHaveCount(1);

    // B16, B21: once Undo's moment passes, Send carries the comment to the opener as one letter from the operator, and the View lets it go.
    await expect(row).toHaveAttribute("data-view-feedback-state", "unsent", { timeout: 15_000 });
    await screenshot(page, "view-feedback-unsent");
    await row.locator('[data-view-feedback-send-main="default"]').click();
    await expect(row).toHaveAttribute("data-view-feedback-state", "sent");
    await expect(row).toContainText("delivered when it rests");
    await expect(page.locator("[data-view-comment]")).toHaveCount(0);
    await expect(front(page).locator("[data-view-comments]")).toHaveCount(0);
    await screenshot(page, "view-feedback-sent");

    // B22, B24: the agent's inbox holds the letter, with the path, the line and the quoted text.
    const inbox = await hide(["inbox"]);
    expect(inbox).toContain("notes.md");
    expect(inbox).toContain("The first line.");
    expect(inbox).toContain("Say what the first line is for.");
    expect(inbox).not.toContain("Throwaway.");
  } finally {
    daemon?.stop();
    herdr.stop();
  }
});

test("an area dragged on an image reaches its agent with a screenshot of that area", async ({ page }) => {
  await page.setViewportSize({ width: 1920, height: 1080 });
  const herdr = await startHerdr({ agents: false });
  let daemon: Daemon | null = null;
  try {
    // Both suites run from their package folder, one below the repository.
    fs.copyFileSync(path.resolve("..", "assets", "pet-theme", "default", "assets", "sleeping.png"), path.join(herdr.root, "fixture", "mock.png"));
    const pane = herdr.panes[0]!;
    daemon = await startHided(herdr, "view-feedback-image");
    const agent = await agentPane(herdr, daemon.stateDir, pane);
    const hide = async (args: string[]) => {
      const answer = await agent(args);
      expect(answer.status, answer.out).toBe(0);
      return answer.out;
    };
    await page.goto(`${daemon.origin}/?probe=1#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    await showExplorer(page);
    await setLanePinned(page, true);
    await hide(["file", "open", "mock.png"]);
    await expect(page.locator("[data-document-header] [data-view-opener]")).toHaveAttribute("data-view-opener", pane);

    // B13: Comment draws the image's page; a drag over it opens the note for that area.
    await page.locator("[data-view-comment-mode]").click();
    const sheet = page.locator("[data-view-region-page]");
    const box = (await sheet.boundingBox())!;
    await page.mouse.move(box.x + box.width * 0.2, box.y + box.height * 0.2);
    await page.mouse.down();
    await page.mouse.move(box.x + box.width * 0.6, box.y + box.height * 0.6, { steps: 4 });
    await page.mouse.up();
    const field = page.locator("[data-view-note-field]");
    await expect(field).toBeFocused();
    // The picked area stays outlined while its note is open.
    await screenshot(page, "comment-image-area-note-open");
    await expect(sheet.locator("[data-view-area-outline]")).toHaveCount(1);
    await field.fill("Crop the owl tighter.");
    await page.keyboard.press("ControlOrMeta+Enter");
    await expect(page.locator('[data-view-region-pin="1"]')).toBeVisible();
    await expect(front(page).locator("[data-view-comments]")).toHaveText("1");

    // B25, B31: Send gives the agent the image, the area and a screenshot of it on its machine.
    const row = page.locator("[data-view-feedback-row]");
    await row.locator('[data-view-feedback-send-main="default"]').click();
    await expect(row).toHaveAttribute("data-view-feedback-state", "sent");
    const inbox = await hide(["inbox"]);
    expect(inbox).toContain("mock.png");
    expect(inbox).toContain("Crop the owl tighter.");
    const shot = /Screenshot: (\S+\.png)/.exec(inbox)?.[1];
    expect(shot, inbox).toBeTruthy();
    expect(fs.readFileSync(shot!).subarray(0, 8).toString("hex")).toBe("89504e470d0a1a0a");
  } finally {
    daemon?.stop();
    herdr.stop();
  }
});
