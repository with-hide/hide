// `hide skill` and `hide mcp` as an agent runs them from its pane (PRD
// skill-mcp-manager B31-B33, D-19, D-29): a list, a switch and a removal
// answer at once; an add waits on the confirm Hide shows in the operator's
// 내 차례 until they press [설치] or [취소], and no word skips it or carries a
// key's value.

import { expect, test, type Page } from "@playwright/test";
import type { HideAnswer } from "./agent-pane";
import { everyAgentOn, startStore, stopStore, type StoreFixture } from "./skills-mcp-fixture";
import { enterWorkspace, screenshot, showTasks } from "./wire";

test.skip(process.platform === "win32", "the fixture's sources are POSIX git repositories over file://");
test.describe.configure({ timeout: 240_000 }); // wait-ok: a daemon, an agent pane and the kit's real git reads and installs finish their real startup and passes

type Line = { ok: boolean; value?: { items?: { name: string; cells: { column: string; state: string }[] }[] }; error?: { code: string } };

/** The last JSON line one command printed. */
function last(ran: HideAnswer): Line {
  return JSON.parse(ran.out.trim().split("\n").at(-1) ?? "{}") as Line;
}

/** The confirm the child's command stands up, opened from its root's 확인하기 in Needs You. */
async function openAsk(page: Page, fixture: StoreFixture) {
  const [root] = fixture.engine.panes;
  const tasks = await showTasks(page);
  const task = tasks.locator(`[data-task-row="${root}"]`);
  await expect(task.locator("[data-human-turn-place]")).toContainText("Install confirm · archify", { timeout: 60_000 }); // wait-ok: the kit's read of the source before the install confirm stands
  await screenshot(page, "store-ask-needs-you");
  await task.locator("[data-human-turn-install]").click();
  const card = page.locator('[data-store-dialog="install"]');
  await expect(card.locator("[data-store-confirm-skills]")).toContainText("archify");
  await expect(card).not.toContainText("▲");
  return card;
}

/** The confirm opens over Settings › 스킬·MCP; the task list is under it. */
async function closeSettings(page: Page) {
  await page.keyboard.press("Escape");
  await expect(page.locator('[data-settings="true"]')).toHaveCount(0);
}

test("an agent's add waits on the operator's confirm, and list, off, on and remove answer at once", async ({ page }) => {
  await page.setViewportSize({ width: 1400, height: 1000 });
  const fixture = await startStore("store-cli", { family: true });
  try {
    await everyAgentOn(fixture.daemon);
    const [, child] = fixture.engine.panes;
    const hide = fixture.engine.runAs(child);
    await page.goto(`${fixture.daemon.origin}/#token=${fixture.daemon.token}`);
    await enterWorkspace(page, "fixture");

    // No option skips the confirm, and a key's value is never an argument (B33).
    for (const skip of ["--yes", "--force", "--no-confirm"]) expect((await hide(["skill", "add", fixture.repos.archify, skip])).status).not.toBe(0);
    expect((await hide(["mcp", "add", "github", "--env", "GITHUB_TOKEN=e2e-value", "--", "npx", "server-github"])).status).not.toBe(0);

    // [취소]: the command ends declined and nothing is installed.
    const declined = hide(["skill", "add", fixture.repos.archify, "--name", "archify"]);
    await (await openAsk(page, fixture)).locator("[data-store-cancel]").click();
    await closeSettings(page);
    expect(last(await declined).error?.code).toBe("store_declined");
    expect(last(await hide(["skill", "list"])).value?.items ?? []).toHaveLength(0);

    // [설치]: the command waits for it and answers with the item.
    const adding = hide(["skill", "add", fixture.repos.archify, "--name", "archify"]);
    const card = await openAsk(page, fixture);
    await screenshot(page, "store-ask-confirm");
    await card.locator("[data-store-install]").click();
    await closeSettings(page);
    const added = last(await adding);
    expect(added.ok, JSON.stringify(added)).toBe(true);
    const tasks = await showTasks(page);
    await expect(tasks.locator("[data-human-turn-install]")).toHaveCount(0);

    // A switch and a removal answer at once, as the screen's own press would.
    const listed = last(await hide(["skill", "list"]));
    expect(listed.value?.items?.map((item) => item.name)).toEqual(["archify"]);
    expect(last(await hide(["skill", "off", "archify", "--agent", "antigravity"])).ok).toBe(true);
    const off = last(await hide(["skill", "list"])).value?.items?.[0]?.cells.find((cell) => cell.column === "antigravity");
    expect(off?.state).toBe("off");
    expect(last(await hide(["skill", "on", "archify", "--agent", "antigravity"])).ok).toBe(true);
    expect(last(await hide(["skill", "list"])).value?.items?.[0]?.cells.every((cell) => cell.state === "on" || cell.state === "unsupported")).toBe(true);
    expect(last(await hide(["skill", "remove", "archify"])).ok).toBe(true);
    expect(last(await hide(["skill", "list"])).value?.items ?? []).toHaveLength(0);
    expect(last(await hide(["mcp", "list"])).ok).toBe(true);
  } finally {
    stopStore(fixture);
  }
});
