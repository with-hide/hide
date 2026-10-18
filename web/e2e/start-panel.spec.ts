// The start panel (PRD home-device-rail B23-B33, B35) on an isolated pinned
// Herdr and hided with the fixture's `claude` shim: ⌘K's `Start an agent…`
// opens it on any screen with the keyboard in the text box; the target is the
// checkout in front; a start with a model runs the agent with `--model <id>`
// and hands it the first prompt; the next open preselects the kind and model;
// Esc keeps what was written until a start spends it.

import { expect, test, type Page } from "@playwright/test";
import { spawnSync } from "node:child_process";
import { herdrHasFocus, startHerdr } from "./herdr-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { countSent, enterWorkspace, screenshot } from "./wire";
import { chord } from "./chords";

test.describe.configure({ timeout: 150_000 });

async function openFromPalette(page: Page) {
  await page.keyboard.press(chord("search"));
  const input = page.locator('[data-palette="Search"] [data-palette-input]');
  await expect(input).toBeFocused();
  await page.keyboard.type("Start an agent");
  await page.locator('[data-palette-row="command:start-agent"]').click();
  await expect(page.locator("[data-start-panel]")).toBeVisible();
  await expect(page.locator("[data-start-text]")).toBeFocused();
}

test("⌘K Start an agent… starts an agent in the checkout in front with the chosen model and first prompt", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  const herdr = await startHerdr();
  let daemon: Daemon | null = null;
  try {
    daemon = await startHided(herdr, "start-panel");
    const last = new Map<string, Record<string, unknown>>();
    countSent(page, last);
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");

    // B33: the command is in ⌘K, and it opens the panel with the keyboard in its text box.
    await openFromPalette(page);
    const panel = page.locator("[data-start-panel]");
    // B23: no backdrop dims the screen behind it.
    await expect(page.locator('[data-slot="dialog-overlay"]')).toHaveCount(0);
    // B25: the checkout in front is the target.
    await expect(panel.locator("[data-start-target]")).toHaveAttribute("data-start-target", new RegExp(`^checkout:${daemon.node}:.*/fixture$`));
    // B24: text box, target, kind, model, ⏎ and 시작.
    await expect(panel.locator("[data-agent-kind]")).toHaveAttribute("data-agent-kind", "claude");
    await expect(panel.locator("[data-agent-model-kind]")).toBeVisible();
    await expect(panel.locator("[data-start-submit]")).toHaveText("Start");
    await screenshot(page, "start-panel-open");

    // The kind list draws each agent's mark beside its name, as a menu item does (issue 1279).
    await panel.locator("[data-agent-kind]").click();
    const codex = await page.locator('[data-agent-kind-option="codex"]').evaluate((option) => {
      const mark = option.querySelector("[data-agent-mark]")!.getBoundingClientRect();
      const walker = document.createTreeWalker(option, NodeFilter.SHOW_TEXT);
      let text = walker.nextNode();
      while (text && !text.textContent?.trim()) text = walker.nextNode();
      const range = document.createRange();
      range.selectNodeContents(text!);
      const name = range.getBoundingClientRect();
      return { beside: name.left >= mark.right, oneLine: name.top < mark.bottom && mark.top < name.bottom };
    });
    expect(codex).toEqual({ beside: true, oneLine: true });
    await page.keyboard.press("Escape");
    await expect(page.locator("[data-agent-kind-option]")).toHaveCount(0);
    await expect(panel).toBeVisible();

    // B27: the model menu is the chosen kind's catalog; B28 waits for it (Claude's is its aliases).
    const model = panel.locator("[data-agent-model-kind]");
    await expect(model).toBeEnabled({ timeout: 20_000 });
    await model.click();
    await page.locator('[data-agent-model-option="opus"]').click();
    await expect(model).toHaveAttribute("data-agent-model", "opus");
    // The closed menu hands the keyboard back to its trigger a moment later;
    // typing before that lands puts the next ⏎ on the trigger, which reopens it.
    await expect(model).toBeFocused();

    await page.locator("[data-start-text]").fill("첫 지시 확인용 문장");
    const known = new Set<string>(herdr.panes);
    await page.keyboard.press("Enter");

    // The panel is done: closed, and its text spent.
    await expect(panel).toHaveCount(0, { timeout: 30_000 });
    expect(last.get("agent_start_in_checkout")).toMatchObject({ provider: "claude", model: "opus", prompt: "첫 지시 확인용 문장" });
    expect(String(last.get("agent_start_in_checkout")?.request_id)).toMatch(/^start-[A-Za-z0-9_-]+$/);

    // B24: the center went to the new pane, which runs the agent with the model and got the prompt.
    let pane = "";
    await expect.poll(async () => {
      pane = await page.locator('[data-pane-view][data-focused="true"]').getAttribute("data-pane-view") ?? "";
      return pane !== "" && !known.has(pane);
    }, { timeout: 30_000 }).toBe(true);
    expect(known.has(pane)).toBe(false);
    await expect.poll(() => herdrHasFocus(herdr, pane)).toBe(true);
    await expect
      .poll(
        () => {
          const info = spawnSync(herdr.bin, ["pane", "process-info", "--pane", pane], { env: herdr.env, encoding: "utf8", timeout: 10_000 }).stdout;
          return info.includes("--model") && info.includes("opus");
        },
        { timeout: 30_000 },
      )
      .toBe(true);
    await expect
      .poll(() => spawnSync(herdr.bin, ["pane", "read", pane, "--source", "visible", "--format", "text"], { env: herdr.env, encoding: "utf8", timeout: 10_000 }).stdout, { timeout: 30_000 })
      .toContain("첫 지시 확인용 문장");

    // B29: the next open preselects the kind and its model, with no 최근 mark, and the text is empty.
    await openFromPalette(page);
    await expect(panel.locator("[data-agent-kind]")).toHaveAttribute("data-agent-kind", "claude");
    await expect(panel.locator("[data-agent-model-kind]")).toHaveAttribute("data-agent-model", "opus", { timeout: 20_000 });
    await expect(panel).not.toContainText("최근");
    await expect(page.locator("[data-start-text]")).toHaveValue("");

    // B30: Esc closes and the draft is still there at the next open.
    await page.locator("[data-start-text]").fill("아직 안 보낸 글");
    await page.keyboard.press("Escape");
    await expect(panel).toHaveCount(0);
    await openFromPalette(page);
    await expect(page.locator("[data-start-text]")).toHaveValue("아직 안 보낸 글");
    // An outside press closes it too, keeping the draft.
    const main = (await page.locator("[data-main], main").first().boundingBox())!;
    await page.mouse.click(main.x + 5, main.y + 5);
    await expect(panel).toHaveCount(0);
  } finally {
    daemon?.stop();
    herdr.stop();
  }
});
