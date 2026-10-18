import { expect, test, type Page } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";
import { startHerdr } from "./herdr-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { enterWorkspace, screenshot } from "./wire";
import { chord } from "./chords";

/** The window's own machine as its system names itself, in each language (PRD core-host-node-move B2). */
const THIS_MACHINE: Record<string, string> =
  process.platform === "darwin"
    ? { en: "This Mac", ko: "이 Mac", "zh-CN": "此 Mac", ja: "この Mac" }
    : process.platform === "win32"
      ? { en: "This PC", ko: "내 PC", "zh-CN": "此电脑", ja: "PC" }
      : { en: "This computer", ko: "이 컴퓨터", "zh-CN": "此计算机", ja: "このコンピューター" };

async function openSettings(page: Page, daemon: Daemon) {
  // A hash-only navigation after restart does not remount the connection.
  if (page.url().startsWith(daemon.origin)) await page.goto("about:blank");
  await page.goto(`${daemon.origin}/#token=${daemon.token}`);
  await enterWorkspace(page, "fixture");
  await page.locator("[data-open-settings]").click();
  await expect(page.locator("[data-interface-language]")).toBeEnabled();
}

async function choose(page: Page, language: string) {
  await page.locator("[data-interface-language]").click();
  await page.locator(`[data-language-option="${language}"]`).click();
  await expect(page.locator("[data-interface-language]")).toHaveAttribute("data-interface-language", language);
  await expect(page.locator("[data-interface-language]")).toBeEnabled();
}

test("one confirmed choice follows every window, persists, and resets to each system language", async ({ browser }) => {
  const herdr = await startHerdr();
  let daemon: Daemon | null = null;
  const korean = await browser.newContext({ locale: "ko-KR" });
  const japanese = await browser.newContext({ locale: "ja-JP" });
  try {
    daemon = await startHided(herdr, "interface-language");
    const left = await korean.newPage();
    const right = await japanese.newPage();
    await openSettings(left, daemon);
    await openSettings(right, daemon);
    await expect(left.locator("html")).toHaveAttribute("lang", "ko");
    await expect(right.locator("html")).toHaveAttribute("lang", "ja");
    await expect(left.locator("[data-interface-language]")).toHaveAttribute("data-interface-language", "system");
    // The answers are product copy, independent of the translation catalog.
    const keycaps = await left.locator(".sidebar-command-keycap").allTextContents();
    for (const [language, heading, general, overview, backlog, close, change, clear, projectCommand, backlogCommand] of [
      ["en", "Settings", "General", "Overview", "Backlog", "Close Overview", "Change the shortcut for Overview, now {chord}", "Clear the shortcut for Overview", "Focus sidebar", "Add to backlog"],
      ["ko", "설정", "일반", "개요", "백로그", "개요 닫기", "개요 단축키 변경, 현재 {chord}", "개요 단축키 해제", "사이드바로 이동", "백로그에 추가"],
      ["zh-CN", "设置", "常规", "概览", "待办", "关闭概览", "更改概览的快捷键，当前为 {chord}", "清除概览的快捷键", "聚焦侧边栏", "添加到待办"],
      ["ja", "設定", "一般", "概要", "バックログ", "概要を閉じる", "概要のショートカットを変更、現在 {chord}", "概要のショートカットを解除", "サイドバーへ移動", "バックログに追加"],
    ]) {
      await choose(left, language!);
      for (const page of [left, right]) {
        await expect(page.locator("html")).toHaveAttribute("lang", language!);
        await expect(page.locator("[data-settings] nav h2")).toHaveText(heading!);
        await expect(page.locator('[data-settings-tab="general"]')).toHaveText(general!);
      }
      // The window's own machine is product copy and follows the language
      // (PRD core-host-node-move B2); paths remain byte-for-byte unchanged.
      await expect(left.locator("[data-sidebar-title-name]")).toHaveText(THIS_MACHINE[language!]!);
      await left.locator('[data-settings-tab="about"]').click();
      await expect(left.locator("[data-settings]")).toContainText(path.join(daemon.stateDir, "core-state.json"));
      await expect(left.locator("[data-sidebar-mode]")).toHaveCount(0);
      await expect(left.locator(".sidebar-command-keycap")).toHaveText(keycaps);
      await left.locator('[data-settings-tab="shortcuts"]').click();
      // The chip names the command and the chord it holds; the clear control its command.
      const overviewChord = (await left.locator('[data-shortcut-effective="overview"]').textContent())!;
      await expect(left.locator('[data-shortcut-record="overview"]')).toHaveAttribute("aria-label", change!.replace("{chord}", overviewChord));
      await expect(left.locator('[data-shortcut-clear="overview"]')).toHaveAttribute("aria-label", clear!);
      await left.keyboard.press("Escape");
      await left.locator("[data-open-overview]").click();
      const modal = left.getByRole("dialog", { name: overview!, exact: true });
      await expect(modal).toBeVisible();
      // The single-device switch still identifies the scope and connection state.
      await expect(modal.getByRole("heading", { level: 1, name: overview!, exact: true })).toBeVisible();
      await expect(modal.locator("[data-overview-devices]")).toBeVisible();
      // A fresh Overview opens on Work; the existing backlog label also translates.
      await expect(modal.locator('[data-main-tab="work"]')).toHaveAttribute("aria-selected", "true");
      await expect(modal.locator('[data-main-tab="backlog"]')).toContainText(backlog!);
      await expect(modal.getByRole("button", { name: close!, exact: true })).toHaveText("Esc");
      await screenshot(left, `interface-${language}`);
      await left.keyboard.press("Escape");
      // Command labels translate while command ids and effective chords stay stable.
      await left.keyboard.press(chord("shortcuts"));
      for (const [id, title] of [["overview", overview], ["sidebar_projects", projectCommand], ["add_backlog", backlogCommand]]) {
        await expect(left.locator(`[data-shortcut="${id}"]`).locator("span").first()).toHaveText(title!);
      }
      await left.keyboard.press("Escape");
      await left.locator("[data-open-settings]").click();
      await left.locator('[data-settings-tab="general"]').click();
    }
    await right.reload();
    await expect(right.locator("html")).toHaveAttribute("lang", "ja");
    daemon = await daemon.restart();
    await openSettings(left, daemon);
    await openSettings(right, daemon);
    await expect(left.locator("html")).toHaveAttribute("lang", "ja");
    await choose(right, "system");
    await expect(left.locator("html")).toHaveAttribute("lang", "ko");
    await expect(right.locator("html")).toHaveAttribute("lang", "ja");
    // Reset is durable as an absent explicit choice, not a cached resolved language.
    daemon = await daemon.restart();
    await openSettings(left, daemon);
    await expect(left.locator("html")).toHaveAttribute("lang", "ko");
  } finally {
    try {
      await korean.close();
      await japanese.close();
    } finally {
      daemon?.stop();
      herdr.stop();
    }
  }
});

test("unsupported systems use English, and an invalid stored preference is diagnosed and retained", async ({ browser }) => {
  const herdr = await startHerdr();
  let daemon: Daemon | null = null;
  const context = await browser.newContext({ locale: "fr-FR" });
  try {
    daemon = await startHided(herdr, "interface-language-invalid");
    const page = await context.newPage();
    await openSettings(page, daemon);
    await expect(page.locator("html")).toHaveAttribute("lang", "en");
    await choose(page, "ko");
    daemon = await daemon.restart((directory) => {
      const file = path.join(directory, "core-state.json");
      const stored = JSON.parse(fs.readFileSync(file, "utf8"));
      stored.interface_language = "unsupported";
      fs.writeFileSync(file, JSON.stringify(stored));
    });
    await openSettings(page, daemon);
    await expect(page.locator("html")).toHaveAttribute("lang", "en");
    await page.locator('[data-settings-tab="about"]').click();
    await expect(page.locator("[data-diagnostics]")).toContainText("ui_state.interface_language_invalid");
    await page.locator('[data-settings-tab="general"]').click();
    await expect(page.locator("[data-interface-language]")).toHaveAttribute("data-interface-language", "en");
    expect(JSON.parse(fs.readFileSync(path.join(daemon.stateDir, "core-state.json"), "utf8")).interface_language).toBe("unsupported");
    // Choosing English repairs an invalid value even though English was already drawn.
    await choose(page, "system");
    await choose(page, "en");
    daemon = await daemon.restart();
    await openSettings(page, daemon);
    await expect(page.locator("[data-interface-language]")).toHaveAttribute("data-interface-language", "en");
    expect(JSON.parse(fs.readFileSync(path.join(daemon.stateDir, "core-state.json"), "utf8")).interface_language).toBe("en");
  } finally {
    try {
      await context.close();
    } finally {
      daemon?.stop();
      herdr.stop();
    }
  }
});
