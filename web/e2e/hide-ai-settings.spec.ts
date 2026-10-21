// Settings > Hide AI on an isolated pinned Herdr and hided (PRD settings-cleanup B33 to B39, B64):
// the Use Hide AI switch and what it stores, Runs on from the agent the fixture shim answers for, and the
// fallback list built with Add agent from stand-in `grok`, `pi` and `opencode` programs (the machine has
// none of them): selectable agents are offered, an installed agent Hide AI cannot use is dimmed with its
// reason, the list keeps the order agents were added in, and every change is what ends up in `ai.json`.
// How a state reads (who is answering, signed out, paused) is the component test's, on invented snapshots.

import { expect, test, type Page } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";
import { startHerdr, type HerdrFixture } from "./herdr-fixture";
import { aiSettingsFile, startHided, type Daemon } from "./hided-fixture";
import { fixtureProgram } from "./platform-fixture";
import { enterWorkspace, screenshot } from "./wire";

test.describe.configure({ timeout: 120_000 });

type Stored = { enabled?: boolean; provider?: string; fallback?: { provider: string; model?: string | null }[]; models?: Record<string, string> };
const stored = (daemon: Daemon): Stored => JSON.parse(fs.readFileSync(aiSettingsFile(daemon.home), "utf8")) as Stored;

/** Programs the machine does not have, standing in for the agents Hide AI can name. */
function standIns(herdr: HerdrFixture): string {
  const dir = path.join(herdr.root, "stand-ins");
  // `grok models` lists the account's models, one per line, `*` marking the default.
  fixtureProgram(dir, "grok", `const args = process.argv.slice(2);\nif (args[0] === "models") { console.log("* grok-fixture (default)\\n- grok-fixture-mini"); process.exit(0); }\nprocess.exit(1);\n`);
  // `pi --list-models` lists `provider/model` references; a Pi that lists any has a provider set up.
  fixtureProgram(dir, "pi", `if (process.argv.includes("--list-models")) { console.log("xai/pi-fixture\\nxai/pi-fixture-mini"); process.exit(0); }\nprocess.exit(1);\n`);
  // OpenCode can be named but not yet used (no way to keep its call read-only): it only has to exist.
  fixtureProgram(dir, "opencode", "process.exit(0);\n");
  // A retired CLI can still be installed. Finding it must never probe or run it.
  fixtureProgram(dir, "gemini", `require("node:fs").writeFileSync(${JSON.stringify(path.join(herdr.root, "gemini-invoked"))}, "unexpected invocation"); process.exit(1);\n`);
  return dir;
}

async function start(label: string, agents: boolean) {
  const herdr = await startHerdr();
  const extra = agents ? { PATH: `${standIns(herdr)}${path.delimiter}${herdr.fixturePath}` } : {};
  const daemon = await startHided(herdr, label, undefined, extra);
  return { herdr, daemon };
}

async function openHideAi(page: Page, daemon: Daemon) {
  await page.setViewportSize({ width: 1200, height: 1000 });
  await page.goto(`${daemon.origin}/#token=${daemon.token}`);
  await enterWorkspace(page, "fixture");
  await page.locator("[data-open-settings]").click();
  await page.locator('[data-settings-tab="hideAi"]').click();
  return page.locator("[data-hide-ai-tab]");
}

test("Use Hide AI dims the rest and keeps its values, and Runs on shows the signed-in agent with its models", async ({ page }) => {
  const { herdr, daemon } = await start("hide-ai-use", false);
  try {
    const tab = await openHideAi(page, daemon);
    const runsOn = tab.locator('[data-settings-group="hide-ai-runs-on"]');
    // B34: the one agent Hide AI can use here, with its sign-in state in the same row.
    await expect(runsOn.locator("[data-ai-provider]")).toContainText("Claude Code");
    await expect(runsOn.locator("[data-ai-state='claude:ready']")).toHaveText(/Signed in/);
    // B36: the list the CLI answered with, the stored model among them.
    await expect(runsOn.locator("[data-ai-model]")).toBeEnabled();
    await runsOn.locator("[data-ai-model]").click();
    await expect(page.getByRole("option", { name: "sonnet" })).toBeVisible();
    await page.keyboard.press("Escape");
    // B35 and B38: no other agent can answer here, so Add agent offers none to choose. A daemon that
    // does not run from an installed app has no kit answer, so the agents Hide AI cannot use yet are
    // still listed dimmed and the button opens; what it must never hold is a choosable agent.
    await tab.locator("[data-ai-add-agent]").click();
    await expect(page.locator("[data-ai-add-menu]")).toBeVisible();
    await expect(page.locator("[data-ai-add-item]")).toHaveCount(0);
    await page.keyboard.press("Escape");

    // B33: off dims and disables the rest; the stored values are untouched.
    const body = tab.locator("[data-hide-ai-body]");
    await expect(body).not.toHaveAttribute("inert", "");
    await tab.locator("[data-ai-use]").click();
    await expect(tab).toHaveAttribute("data-ai-enabled", "false");
    await expect(body).toHaveAttribute("inert", "");
    await expect.poll(() => stored(daemon).enabled).toBe(false);
    expect(stored(daemon).provider).toBe("claude");
    await screenshot(page, "hide-ai-off");

    // Back on, everything is as it was.
    await tab.locator("[data-ai-use]").click();
    await expect(tab).toHaveAttribute("data-ai-enabled", "true");
    await expect(body).not.toHaveAttribute("inert", "");
    await expect(runsOn.locator("[data-ai-provider]")).toContainText("Claude Code");
    await expect.poll(() => stored(daemon).enabled).toBe(true);

    // D-24 and B37: the two features, no "chosen" tail and no Memory sentence.
    const features = tab.locator('[data-settings-group="hide-ai-features"]');
    await expect(features).toContainText("Agent summaries");
    await expect(features).toContainText("Worktree names");
    await expect(tab).not.toContainText("does not turn on Memory");
    await expect(tab).not.toContainText("chosen");
    await screenshot(page, "hide-ai-on");
  } finally {
    daemon.stop();
    herdr.stop();
  }
});

test("Add agent offers the agents Hide AI can use, dims the one it cannot, and the list keeps the order they were added in", async ({ page }) => {
  const { herdr, daemon } = await start("hide-ai-fallback", true);
  try {
    const tab = await openHideAi(page, daemon);
    const group = tab.locator('[data-settings-group="hide-ai-fallback"]');
    await expect(group).toContainText("If Claude Code can't answer");
    // D-16: an empty list is no fallback, and there is no switch.
    await expect(group.locator("[data-ai-fallback]")).toHaveCount(0);
    await expect(group).toContainText("None added:");
    await expect(group).toContainText("your usage limit");
    await expect(group.getByRole("switch")).toHaveCount(0);

    // B38: Grok and Pi are choosable, OpenCode is installed but dimmed under its heading with the reason.
    await group.locator("[data-ai-add-agent]").click();
    const menu = page.locator("[data-ai-add-menu]");
    await expect(menu.locator("[data-ai-add-item]")).toHaveCount(2);
    await expect(menu.locator("[data-ai-add-item='grok']")).toBeVisible();
    await expect(menu.locator("[data-ai-add-item='pi']")).toBeVisible();
    await expect(menu).not.toContainText("Gemini");
    await expect(menu).toContainText("Hide AI can't use these yet");
    const dimmed = menu.locator("[data-ai-unusable-item='opencode']");
    await expect(dimmed).toHaveAttribute("data-disabled", "");
    await expect(dimmed).toContainText("Can't guarantee it only reads");
    await screenshot(page, "hide-ai-add-menu");

    // B39: added in the order chosen, numbered, each with its own model and a remove.
    await menu.locator("[data-ai-add-item='pi']").click();
    await expect(group.locator("[data-ai-fallback='pi']")).toBeVisible();
    await expect(group).not.toContainText("None added:");
    await group.locator("[data-ai-add-agent]").click();
    await page.locator("[data-ai-add-item='grok']").click();
    await expect(group.locator("[data-ai-fallback]")).toHaveCount(2);
    await expect(group).not.toContainText("None added:");
    await expect(group.locator("[data-ai-fallback-order]")).toHaveText(["1", "2"]);
    expect(await group.locator("[data-ai-fallback]").evaluateAll((rows) => rows.map((row) => row.getAttribute("data-ai-fallback")))).toEqual(["pi", "grok"]);
    await expect.poll(() => stored(daemon).fallback?.map((entry) => entry.provider)).toEqual(["pi", "grok"]);
    await screenshot(page, "hide-ai-fallback");

    // A fallback entry has its own model: a Grok model other than CLI default.
    await group.locator("[data-ai-fallback-model='grok']").click();
    await page.getByRole("option", { name: "grok-fixture-mini" }).click();
    await expect.poll(() => stored(daemon).fallback?.find((entry) => entry.provider === "grok")?.model).toBe("grok-fixture-mini");

    // × takes an agent out for good: it is no longer tried, and Add offers it again at the end.
    await group.locator("[data-ai-fallback-remove='pi']").click();
    await expect(group.locator("[data-ai-fallback='pi']")).toHaveCount(0);
    await expect(group.locator("[data-ai-fallback-order]")).toHaveText(["1"]);
    await expect.poll(() => stored(daemon).fallback?.map((entry) => entry.provider)).toEqual(["grok"]);
    await group.locator("[data-ai-add-agent]").click();
    await expect(page.locator("[data-ai-add-item]")).toHaveText([/Pi/]);
    await page.keyboard.press("Escape");

    // B44: choosing a listed agent as Runs on takes it off the list, and Claude Code becomes selectable to add.
    await tab.locator("[data-ai-provider]").click();
    await page.getByRole("option", { name: /Grok/ }).click();
    await expect(tab.locator("[data-ai-provider]")).toContainText("Grok");
    await expect(group.locator("[data-ai-fallback]")).toHaveCount(0);
    await expect.poll(() => stored(daemon).provider).toBe("grok");
    expect(stored(daemon).fallback ?? []).toEqual([]);
  } finally {
    daemon.stop();
    herdr.stop();
  }
});

test("a saved retired Gemini choice stays unchosen, preserves model data and never invokes its installed CLI", async ({ page }) => {
  const herdr = await startHerdr();
  const home = path.join(herdr.root, "retired-ai-home");
  const settings = aiSettingsFile(home);
  fs.mkdirSync(path.dirname(settings), { recursive: true });
  fs.writeFileSync(settings, JSON.stringify({
    provider: "gemini-cli",
    models: { "gemini-cli": "saved-gemini", antigravity: "gemini-3.8-flash-high" },
    fallback: [{ provider: "gemini-cli" }, { provider: "grok", model: "grok-fixture-mini" }],
  }));
  const daemon = await startHided(herdr, "hide-ai-retired", home, { PATH: `${standIns(herdr)}${path.delimiter}${herdr.fixturePath}` });
  try {
    const tab = await openHideAi(page, daemon);
    // The kit has no switched-on agent here, so the normal first-run rule makes no replacement choice.
    await expect(tab.locator("[data-ai-provider]")).toContainText("Choose");
    await expect(tab.locator('[data-settings-group="hide-ai-fallback"]')).toHaveCount(0);
    await tab.locator("[data-ai-provider]").click();
    await expect(page.getByRole("option", { name: /Grok/ })).toBeVisible();
    await expect(page.getByRole("option", { name: /Gemini/ })).toHaveCount(0);
    await page.keyboard.press("Escape");
    expect(fs.existsSync(path.join(herdr.root, "gemini-invoked"))).toBe(false);

    // A settings write must not persist the internal default as a choice or erase the retired model.
    await tab.locator("[data-ai-use]").click();
    await expect.poll(() => stored(daemon).enabled).toBe(false);
    expect(stored(daemon).provider).toBeUndefined();
    expect(stored(daemon).fallback).toEqual([{ provider: "grok", model: "grok-fixture-mini" }]);
    expect(stored(daemon).models).toMatchObject({ "gemini-cli": "saved-gemini", antigravity: "gemini-3.8-flash-high" });
    await screenshot(page, "hide-ai-retired-gemini");
    await tab.locator("[data-ai-use]").click();
    await tab.locator("[data-ai-provider]").click();
    await page.getByRole("option", { name: /Grok/ }).click();
    await expect.poll(() => stored(daemon).provider).toBe("grok");
    await expect(tab.locator("[data-ai-provider]")).toContainText("Grok");
    expect(stored(daemon).fallback).toEqual([]);
    expect(fs.existsSync(path.join(herdr.root, "gemini-invoked"))).toBe(false);
  } finally {
    daemon.stop();
    herdr.stop();
  }
});

test("Hide AI reads without sideways scrolling in all four languages, light and dark, on a narrow window", async ({ page }) => {
  // B65: the sheet shrinks with its window; the longest lines (the stand-in line, the fallback note, the dimmed reasons) wrap, not clip.
  const { herdr, daemon } = await start("hide-ai-narrow", true);
  try {
    const tab = await openHideAi(page, daemon);
    // The smallest window (PRD settings-left-nav B13): the list keeps its width beside the page.
    await page.setViewportSize({ width: 720, height: 1000 });
    const group = tab.locator('[data-settings-group="hide-ai-fallback"]');
    await group.locator("[data-ai-add-agent]").click();
    await page.locator("[data-ai-add-item='grok']").click();
    await expect(group.locator("[data-ai-fallback='grok']")).toBeVisible();
    const panel = page.locator('[data-settings="true"] [data-settings-page]');
    const overflow = () => panel.evaluate((node) => node.scrollWidth - node.clientWidth);

    for (const [language, theme] of [
      ["en", "light"],
      ["ko", "light"],
      ["ko", "dark"],
      ["ja", "light"],
      ["zh-CN", "dark"],
    ] as const) {
      await page.locator('[data-settings-tab="general"]').click();
      await page.locator("[data-interface-language]").click();
      await page.locator(`[data-language-option="${language}"]`).click();
      await expect(page.locator("html")).toHaveAttribute("lang", language);
      await page.locator(`[data-theme-option="${theme}"]`).click();
      await expect(page.locator("[data-theme-choice]")).toHaveAttribute("data-theme-choice", theme);
      await page.locator('[data-settings-tab="hideAi"]').click();
      await expect(tab).toBeVisible();
      await expect.poll(overflow).toBeLessThanOrEqual(0);
      await screenshot(page, `hide-ai-${language}-${theme}-narrow`);
      await group.locator("[data-ai-add-agent]").click();
      await expect(page.locator("[data-ai-add-menu]")).toBeVisible();
      await screenshot(page, `hide-ai-add-menu-${language}-${theme}-narrow`);
      await page.keyboard.press("Escape");
    }
  } finally {
    daemon.stop();
    herdr.stop();
  }
});
