// Settings' list and its General, GitHub and About pages on an isolated
// pinned Herdr and hided (PRD settings-cleanup B1-B6, B23, B24;
// settings-left-nav B2-B5, B8, B9, B12, B13): the list in its groups, General's
// Appearance, separate Privacy, the GitHub row, About with its closed Details, the real
// Herdr version and protocol of a healthy connection, and the settings that
// moved into Agents and Hide AI.

import { expect, test } from "@playwright/test";
import { startHerdr } from "./herdr-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { enterWorkspace, screenshot } from "./wire";

test.describe.configure({ timeout: 120_000 });

test("Settings lists its pages in two groups with About apart, and About shows the real Herdr values behind a closed Details", async ({ page }) => {
  await page.setViewportSize({ width: 1200, height: 900 });
  const herdr = await startHerdr();
  let daemon: Daemon | null = null;
  try {
    daemon = await startHided(herdr, "settings-general");
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    await page.locator("[data-open-settings]").click();
    const sheet = page.locator('[data-settings="true"]');
    await expect(sheet).toBeVisible();

    // The user's Privacy-tab follow-up puts Privacy last, after About at the foot.
    await expect(sheet.locator('[data-settings-nav-group="app"]')).toHaveText(/^App/);
    await expect(sheet.locator('[data-settings-nav-group="app"] [data-settings-tab]')).toHaveText(["General", "Notifications", "Usage", "Devices", "Shortcuts"]);
    await expect(sheet.locator('[data-settings-nav-group="machine"]')).toHaveText(/^This (Mac|PC|computer)/);
    await expect(sheet.locator('[data-settings-nav-group="machine"] [data-settings-tab]')).toHaveText(["Agents", "Hide AI", "Conductor", "Mobile", "GitHub"]);
    await expect(sheet.locator("[data-settings-tab]")).toHaveText(["General", "Notifications", "Usage", "Devices", "Shortcuts", "Agents", "Hide AI", "Conductor", "Mobile", "GitHub", "About", "Privacy"]);
    const tabs = sheet.locator("[data-settings-tab]");
    const githubBox = (await sheet.locator('[data-settings-tab="github"]').boundingBox())!;
    const aboutBox = (await sheet.locator('[data-settings-tab="about"]').boundingBox())!;
    const privacyBox = (await sheet.locator('[data-settings-tab="privacy"]').boundingBox())!;
    const listBox = (await sheet.locator('[role="tablist"]').boundingBox())!;
    expect(aboutBox.y - (githubBox.y + githubBox.height)).toBeGreaterThan(githubBox.height);
    expect(listBox.y + listBox.height - (privacyBox.y + privacyBox.height)).toBeLessThan(privacyBox.height);
    await expect(tabs.first()).toHaveAttribute("data-state", "active");

    // B3, B4: Down moves through the list and shows each page under its title alone; a group name is never a stop.
    await tabs.first().focus();
    await page.keyboard.press("ArrowDown");
    await expect(sheet.locator('[data-settings-tab="notifications"]')).toBeFocused();
    await expect(sheet.locator('[data-settings-tab="notifications"]')).toHaveAttribute("data-state", "active");
    await expect(sheet.locator("[data-settings-page-title]")).toHaveText("Notifications");
    for (const next of ["usage", "devices", "shortcuts", "agents"]) {
      await page.keyboard.press("ArrowDown");
      await expect(sheet.locator(`[data-settings-tab="${next}"]`)).toHaveAttribute("data-state", "active");
    }
    await page.keyboard.press("ArrowUp");
    await expect(sheet.locator('[data-settings-tab="shortcuts"]')).toBeFocused();
    await expect(sheet.locator("[data-settings-page-title]")).toHaveText("Shortcuts");
    await sheet.locator('[data-settings-tab="general"]').click();

    // General keeps Appearance; the usage switch has its own Privacy page.
    const appearance = sheet.locator('[data-settings-group="appearance"]');
    await expect(sheet.locator("[data-settings-page] [data-settings-group]")).toHaveCount(1);
    await expect(sheet.locator("[data-product-analytics]")).toHaveCount(0);
    await expect(appearance.locator("[data-interface-language]")).toBeVisible();
    await expect(appearance.locator("[data-theme-choice]")).toBeVisible();
    await expect(appearance.locator("[data-accent]")).toHaveCount(4);
    await expect(appearance.locator("[data-font-size]")).toBeVisible();
    await expect(appearance).not.toContainText(/#[0-9a-f]{6}/i);
    // hide-color-theme B1, B2, B6: Color theme sits between Language and Mode,
    // offers Hide and Neutral, and a store that never chose opens on Hide.
    expect(await appearance.locator("[data-appearance-row]").evaluateAll((rows) => rows.map((row) => row.getAttribute("data-appearance-row")))).toEqual([
      "language",
      "color-theme",
      "mode",
      "accent",
      "interface-text",
    ]);
    await expect(appearance.locator("[data-color-theme-option]")).toHaveText(["Hide", "Neutral"]);
    await expect(appearance.locator("[data-color-theme-choice]")).toHaveAttribute("data-color-theme-choice", "hide");
    await expect(appearance.locator('[data-appearance-row="mode"]')).toContainText("Mode");

    // B6 (settings-cleanup): no ownership sentence and no Authentication paragraph, and no device chip on this Mac.
    await expect(sheet).not.toContainText("kept by hided");
    await expect(sheet).not.toContainText("Authentication");
    await expect(sheet.locator("[data-settings-device]")).toHaveCount(0);

    // B8: the GitHub row has a page of its own, with no Local row.
    await sheet.locator('[data-settings-tab="github"]').click();
    const github = sheet.locator('[data-settings-group="github"]');
    await expect(github).toContainText("GitHub");
    await expect(github).not.toContainText("Always available");

    // B9: About's Details is closed until opened, and then holds the runtime facts.
    await sheet.locator('[data-settings-tab="about"]').click();
    await expect(sheet.locator("[data-settings-page-title]")).toHaveText("About");
    const details = sheet.locator("[data-settings-details]");
    await expect(details).not.toHaveAttribute("open", "");
    await expect(details.getByText("Protocol")).toBeHidden();
    await details.locator("summary").click();
    await expect(details).toHaveAttribute("open", "");
    for (const label of ["Process", "State file", "Socket", "Binary", "Recent"]) await expect(details.getByText(label, { exact: true })).toBeVisible();

    // A healthy Herdr shows its real version and protocol, never "unavailable".
    await expect(details.locator("[data-herdr-version]")).toHaveText(/^\d+\.\d+\.\d+/);
    await expect(details.locator("[data-herdr-protocol]")).toHaveAttribute("data-herdr-protocol", "matches");
    await expect(details.locator("[data-herdr-protocol]")).toHaveText(/^\d+$/);
    await expect(sheet.locator('[data-settings-group="about"] [data-herdr-state="connected"]')).toContainText(/Connected · \d+\.\d+\.\d+/);
    await screenshot(page, "settings-about-dark");
  } finally {
    daemon?.stop();
    herdr.stop();
  }
});

test("Sleep after, the issue link and the worktree names live in Agents and Hide AI with their stored values", async ({ page }) => {
  await page.setViewportSize({ width: 1200, height: 900 });
  const herdr = await startHerdr();
  let daemon: Daemon | null = null;
  try {
    daemon = await startHided(herdr, "settings-moved");
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    await page.locator("[data-open-settings]").click();

    // B23: the idle choice and its sentence, the same value the Performance tab held.
    await page.locator('[data-settings-tab="agents"]').click();
    const idle = page.locator('[data-settings-group="idle-agents"]');
    await expect(idle.locator("[data-agent-sleep-after]")).toHaveAttribute("data-agent-sleep-after", "never");
    await expect(idle).toContainText("0 sleeping now. Working, unread and on-screen agents never sleep.");
    await idle.locator("[data-agent-sleep-after]").click();
    await page.locator('[data-agent-sleep-option="12"]').click();
    await expect(idle.locator("[data-agent-sleep-after]")).toHaveAttribute("data-agent-sleep-after", "12");

    // B24: the pull request link switch, on by default as the Issues tab had it.
    const starting = page.locator('[data-settings-group="issue-start"]');
    const link = starting.locator("[data-issue-closes-instruction]");
    await expect(link).toHaveAttribute("data-issue-closes-instruction", "true");
    await expect(starting).toContainText("Link the issue in pull requests");
    await link.click();
    await expect(link).toHaveAttribute("data-issue-closes-instruction", "false");

    // D-24: Let AI name worktrees is a Hide AI feature now.
    await page.locator('[data-settings-tab="hideAi"]').click();
    const features = page.locator('[data-settings-group="hide-ai-features"]');
    const names = features.locator("[data-issue-ai-worktree-name]");
    await expect(names).toHaveAttribute("data-issue-ai-worktree-name", "true");
    await names.click();
    await expect(names).toHaveAttribute("data-issue-ai-worktree-name", "false");
    await expect(features.locator("[data-ai-agent-summary]")).toBeVisible();
  } finally {
    daemon?.stop();
    herdr.stop();
  }
});

test("General and About read without sideways scrolling in Korean and English, light and dark, on the smallest window", async ({ page }) => {
  // B65, settings-left-nav B13: the sheet shrinks with its window to the smallest one, the list beside the page; every row has to wrap, not clip.
  await page.setViewportSize({ width: 720, height: 480 });
  const herdr = await startHerdr();
  let daemon: Daemon | null = null;
  try {
    daemon = await startHided(herdr, "settings-narrow");
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    await page.locator("[data-open-settings]").click();
    const sheet = page.locator('[data-settings="true"]');
    await expect(sheet).toBeVisible();
    const panel = sheet.locator("[data-settings-page]");
    const overflow = () => panel.evaluate((node) => node.scrollWidth - node.clientWidth);
    const details = sheet.locator("[data-settings-details]");

    // hide-color-theme B5, D-14: both rows and both choices are named in each
    // language and fit the narrow sheet, none cut off.
    const appearanceWords = {
      en: ["Color theme", "Mode", "Hide", "Neutral"],
      ko: ["색 테마", "모드", "Hide", "뉴트럴"],
      ja: ["カラーテーマ", "モード", "Hide", "ニュートラル"],
      "zh-CN": ["配色主题", "模式", "Hide", "中性"],
    } as const;
    await sheet.locator('[data-settings-tab="general"]').click();
    for (const [language, [colorTheme, mode, hide, neutral]] of Object.entries(appearanceWords)) {
      await sheet.locator("[data-interface-language]").click();
      await page.locator(`[data-language-option="${language}"]`).click();
      await expect(page.locator("html")).toHaveAttribute("lang", language);
      await expect(sheet.locator('[data-appearance-row="color-theme"]')).toContainText(colorTheme);
      await expect(sheet.locator('[data-appearance-row="mode"]')).toContainText(mode);
      await expect(sheet.locator("[data-color-theme-option]")).toHaveText([hide, neutral]);
      await expect.poll(overflow).toBeLessThanOrEqual(0);
      const cut = await sheet.locator('[data-appearance-row="color-theme"], [data-appearance-row="mode"]').evaluateAll((rows) =>
        rows.flatMap((row) => [...row.querySelectorAll<HTMLElement>("div, button")].filter((node) => node.scrollWidth > node.clientWidth + 1).map((node) => node.textContent)),
      );
      expect(cut, language).toEqual([]);
      await screenshot(page, `settings-appearance-${language}-narrow`);
    }

    for (const [language, theme] of [
      ["en", "light"],
      ["ko", "light"],
      ["ko", "dark"],
    ] as const) {
      await sheet.locator('[data-settings-tab="general"]').click();
      await sheet.locator("[data-interface-language]").click();
      await page.locator(`[data-language-option="${language}"]`).click();
      await expect(page.locator("html")).toHaveAttribute("lang", language);
      await sheet.locator(`[data-theme-option="${theme}"]`).click();
      await expect(sheet.locator("[data-theme-choice]")).toHaveAttribute("data-theme-choice", theme);
      await expect.poll(overflow).toBeLessThanOrEqual(0);
      await screenshot(page, `settings-general-${language}-${theme}-narrow`);
      await sheet.locator('[data-settings-tab="privacy"]').click();
      await expect.poll(overflow).toBeLessThanOrEqual(0);
      await screenshot(page, `settings-privacy-${language}-${theme}-narrow`);
      await sheet.locator('[data-settings-tab="about"]').click();
      if (!(await details.evaluate((node) => (node as HTMLDetailsElement).open))) await details.locator("summary").click();
      await expect.poll(overflow).toBeLessThanOrEqual(0);
      // B13: the page scrolls to its end while the list stays where it is.
      const list = (await sheet.locator('[role="tablist"]').boundingBox())!;
      await panel.evaluate((node) => node.scrollTo({ top: node.scrollHeight }));
      await expect.poll(() => panel.evaluate((node) => node.scrollTop + node.clientHeight - node.scrollHeight)).toBeGreaterThanOrEqual(-1);
      expect((await sheet.locator('[role="tablist"]').boundingBox())!.y).toBe(list.y);
      await expect(sheet.locator("[data-copy-diagnostics]")).toBeInViewport();
      // Privacy stays reachable at the foot; the grouped pages may scroll.
      await expect(sheet.locator('[data-settings-tab="privacy"]')).toBeInViewport({ ratio: 1 });
      expect(await sheet.locator("nav").evaluate((node) => node.scrollHeight - node.clientHeight)).toBeLessThanOrEqual(0);
      await screenshot(page, `settings-about-details-${language}-${theme}-narrow`);
    }

    // Larger text gives only the page list a scroll region; About stays at
    // the foot, and keyboard navigation can still reach every page.
    for (const [language, theme] of [["en", "light"], ["ko", "dark"]] as const) {
      await sheet.locator('[data-settings-tab="general"]').click();
      await sheet.locator("[data-interface-language]").click();
      await page.locator(`[data-language-option="${language}"]`).click();
      await sheet.locator(`[data-theme-option="${theme}"]`).click();
      const slider = sheet.locator('[data-font-size] [role="slider"]');
      await slider.press("End");
      await expect(slider).toHaveAttribute("aria-valuenow", (await slider.getAttribute("aria-valuemax"))!);
      const about = sheet.locator('[data-settings-tab="about"]');
      await sheet.locator('[data-settings-tab="general"]').focus();
      await expect(about).toBeInViewport({ ratio: 1 });
      const foot = (await about.boundingBox())!.y;
      const pages = sheet.locator("[data-settings-page-list]");
      expect(await pages.evaluate(node => node.scrollHeight - node.clientHeight)).toBeGreaterThan(0);
      await sheet.locator('[data-settings-tab="general"]').focus();
      for (const name of ["general", "notifications", "usage", "devices", "shortcuts", "agents", "hideAi", "conductor", "mobile", "github", "about", "privacy"]) {
        const tab = sheet.locator(`[data-settings-tab="${name}"]`);
        await expect(tab).toBeFocused();
        await expect(tab).toBeInViewport({ ratio: 1 });
        await expect(about).toBeInViewport({ ratio: 1 });
        expect((await about.boundingBox())!.y).toBe(foot);
        if (name !== "privacy") await page.keyboard.press("ArrowDown");
      }
      await expect(sheet.locator("[data-product-analytics]")).toBeInViewport({ ratio: 1 });
      await expect.poll(overflow).toBeLessThanOrEqual(0);
      await screenshot(page, `settings-privacy-${language}-${theme}-maximum-text`);
      await sheet.locator('[data-settings-tab="about"]').click();
      if (!(await details.evaluate(node => (node as HTMLDetailsElement).open))) await details.locator("summary").click();
      const listScroll = await pages.evaluate(node => node.scrollTop);
      await panel.evaluate(node => node.scrollTo({ top: node.scrollHeight }));
      expect(await pages.evaluate(node => node.scrollTop)).toBe(listScroll);
      const pageScroll = await panel.evaluate(node => node.scrollTop);
      expect(pageScroll).toBeGreaterThan(0);
      await pages.evaluate(node => node.scrollTo({ top: 0 }));
      expect(await panel.evaluate(node => node.scrollTop)).toBe(pageScroll);
      expect((await about.boundingBox())!.y).toBe(foot);
      await screenshot(page, `settings-about-${language}-${theme}-maximum-text`);
    }
  } finally {
    daemon?.stop();
    herdr.stop();
  }
});
