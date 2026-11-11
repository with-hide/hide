// A person's turn an agent declares, on the core's own engine and an
// isolated hided (PRD ask-human B1, B4, B8, B9, B14, B15, B18, B19): the
// delegated child's `hide agent ask-human` puts its lineage root in Needs
// You with the key and `<kind> · <place>`, the place as written, a URL's
// query and fragment included, the root's task row and the child's own pane
// head carry Done, and Done sends the child one letter without the place and
// takes the turn down.

import { expect, test } from "@playwright/test";
import type { HideAnswer } from "./agent-pane";
import { focusPane, withControl, viewPane } from "./daemon-actions";
import { startFixture } from "./engine-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { enterWorkspace, screenshot, showTasks } from "./wire";

test.describe.configure({ timeout: 180_000 });

type Hide = (args: string[]) => Promise<HideAnswer>;
type Answer = { ok: boolean; reason?: string; value?: Record<string, unknown> };

/** The last JSON line one `hide` command printed, which must say it succeeded. */
async function ok(hide: Hide, args: string[]): Promise<Answer> {
  const ran = await hide(args);
  const line = ran.out.trim().split("\n").at(-1) ?? "";
  const answer = JSON.parse(line) as Answer;
  expect(answer.ok, `${args.join(" ")}: ${line}`).toBe(true);
  return answer;
}

test("a child's person's turn raises its root with the key, and Done sends the child one letter and takes the turn down", async ({ page }) => {
  await page.setViewportSize({ width: 1600, height: 1000 });
  const fixture = await startFixture({ family: true });
  let daemon: Daemon | null = null;
  try {
    daemon = await startHided(fixture, "ask-human");
    const [root, child] = fixture.panes;
    const asChild = fixture.runAs(child);
    // The place is kept as written, its query and fragment included (B4).
    const place = "https://github.com/login/device?user_code=WDJB-MJHT#step=2";
    const declared = await ok(asChild, ["agent", "ask-human", "auth", place]);
    expect(declared.value).toMatchObject({ outcome: "declared", declaration: { kind: "auth", place } });
    // The same declaration again is one declaration (B5).
    expect((await ok(asChild, ["agent", "ask-human", "auth", place])).value).toMatchObject({ outcome: "unchanged" });

    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    const tasks = await showTasks(page);
    // The root stands in Needs You with the key and what and where, its parent never asked (B8, B9).
    const task = tasks.locator(`[data-task-row="${root}"]`);
    await expect(task).toHaveAttribute("data-task-state", "needs_you");
    await expect(task.locator('[data-mark-human="true"]')).toHaveCount(1);
    await expect(task.locator("[data-human-turn-place]")).toHaveText(`Sign-in · ${place}`);
    // Copy is drawn only while the pointer is on the row (operator decision 2026-11-11).
    const copy = task.locator("[data-human-turn-copy]");
    await expect(copy).toHaveCSS("opacity", "0");
    await screenshot(page, "ask-human-task-row");
    await task.hover();
    await expect(copy).toHaveCSS("opacity", "1");
    await screenshot(page, "ask-human-task-row-hover");

    // The child's own pane head, in its own tab, says it with Done (B15).
    await withControl(daemon, async (control) => {
      viewPane(control, child);
      await focusPane(control, child);
    });
    const band = page.locator(`[data-pane-header-band="answer"]`).filter({ has: page.locator("[data-human-turn-done]") });
    await expect(band).toContainText(`Sign-in · ${place}`);
    await screenshot(page, "ask-human-pane-head");

    // Done from the task list sends one letter and takes the turn down everywhere (B18, B19); a repeat press is the core test's.
    await task.locator("[data-human-turn-done]").click();
    await expect(task.locator('[data-mark-human="true"]')).toHaveCount(0, { timeout: 20_000 });
    await expect(page.locator("[data-human-turn-done]")).toHaveCount(0);
    await expect.poll(async () => {
      const inbox = await asChild(["inbox"]);
      return (inbox.out.match(/운영자가 처리함: 인증/g) ?? []).length;
    }, { timeout: 20_000 }).toBe(1);
    // The letter stays after the turn, so it never carries the place (B4, B18).
    expect((await asChild(["inbox"])).out).not.toContain("WDJB-MJHT");
    // The child is no longer waiting on a person, and the letter waits for no answer.
    const shown = await ok(asChild, ["agent", "show", "here"]);
    expect(JSON.stringify(shown.value)).not.toContain("human_turn");

    // Its own `--clear` with nothing standing says so without failing (B6).
    expect((await ok(asChild, ["agent", "ask-human", "--clear"])).value).toMatchObject({ outcome: "nothing_to_clear" });
  } finally {
    daemon?.stop();
    fixture.stop();
  }
});
