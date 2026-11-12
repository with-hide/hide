// Markdown Live draws HTML blocks and images (PRD markdown-html): the
// repository's own README opens as the page GitHub shows, a caret inside a
// block shows its source, and what a document names but may not load (a
// script, a path outside the checkout, an http address) never reaches the
// page. Runs on an isolated engine and hided, either engine; the operator's is
// never touched.

import { expect, test, type Page } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";
import { toPage } from "../../desktop/src/main/wirePath";
import { startEngine, type EngineServer } from "./engine-fixture";
import { openProject, withControl } from "./daemon-actions";
import { startHided, type Daemon } from "./hided-fixture";
import { chord } from "./chords";
import { countSent, screenshot, showExplorer, showProjectRow } from "./wire";

/** A 1x1 PNG, so an image has real bytes to decode. */
const PNG = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==",
  "base64",
);

const UNSAFE = [
  "# Unsafe",
  "",
  '<p id="unsafe">kept text<script>window.pwned = "script"</script><iframe src="https://example.com"></iframe>',
  '<a href="javascript:window.pwned = 1">script link</a> <img src="shot.png" alt="onerror" onerror="window.pwned = 2"></p>',
  "",
  "![inside](shot.png)",
  "",
  "![outside](../outside.png)",
  "",
  "![gone](missing.png)",
  "",
  "![plain http](http://127.0.0.1:9/never.png)",
  "",
].join("\n");

type Stack = { engine: EngineServer; daemon: Daemon; repo: string; sent: Map<string, number> };

/** Starts the isolated stack with `files` written into the checkout, and leaves the Explorer showing it. */
async function openStack(page: Page, files: Record<string, Buffer | string>): Promise<Stack> {
  const engine = await startEngine();
  let daemon: Daemon | null = null;
  try {
    const repoDir = path.join(engine.root, "repo");
    fs.mkdirSync(repoDir, { recursive: true });
    for (const [name, contents] of Object.entries(files)) {
      fs.mkdirSync(path.dirname(path.join(repoDir, name)), { recursive: true });
      fs.writeFileSync(path.join(repoDir, name), contents);
    }
    fs.writeFileSync(path.join(engine.root, "outside.png"), PNG);
    daemon = await startHided(engine, "markdown-html");
    await withControl(daemon, (control) => openProject(control, repoDir, "repo"));
    const sent = countSent(page, new Map());
    await page.goto(`${daemon.origin}/?probe=1#token=${daemon.token}`);
    const row = await showProjectRow(page, page.locator("[data-project]", { hasText: "repo" }).locator("[data-checkout]").first());
    await row.click();
    await showExplorer(page);
    return { engine, daemon, repo: toPage(fs.realpathSync(repoDir)), sent };
  } catch (error) {
    daemon?.stop();
    engine.stop();
    throw error;
  }
}

function close({ daemon, engine }: Stack): void {
  daemon.stop();
  engine.stop();
}

test.describe.configure({ timeout: 90_000 });

test("the repository README opens as drawn HTML and shows a block source under the caret", async ({ page }) => {
  const root = path.resolve("..");
  const stack = await openStack(page, {
    "README.md": fs.readFileSync(path.join(root, "README.md")),
    "design/brand/hide-icon-blinking.svg": fs.readFileSync(path.join(root, "design/brand/hide-icon-blinking.svg")),
    "site/public/assets/video/kick-conductor.gif": fs.readFileSync(path.join(root, "site/public/assets/video/kick-conductor.gif")),
  });
  const appRequests: string[] = [];
  page.on("request", (request) => {
    if (request.url().startsWith(stack.daemon.origin)) appRequests.push(new URL(request.url()).pathname);
  });
  try {
    await page.locator(`[data-explorer-row="${stack.repo}/README.md"]`).click();
    const blocks = page.locator(".cm-md-html-block");
    await expect(blocks.first()).toBeVisible({ timeout: 20_000 });
    expect(await blocks.count()).toBeGreaterThanOrEqual(6);
    const hero = page.locator('.cm-md-html img[alt^="A Conductor plans"]');
    await expect(hero).toHaveAttribute("src", /^blob:/);
    // The logo and the hero film come through the file-bytes path as blobs and decode.
    const logo = page.locator('.cm-md-html img[alt="hide owl app icon"]');
    await expect(logo).toBeVisible();
    await expect.poll(() => logo.evaluate((image: HTMLImageElement) => image.complete && image.naturalWidth > 0)).toBe(true);
    await expect(logo).toHaveAttribute("src", /^blob:/);
    // The file opens at its first line, with the logo in view, once it and the badges have loaded.
    await expect(page.locator('.cm-md-html img[alt="macOS, Windows and Linux"]')).toHaveJSProperty("complete", true);
    expect(await page.locator(".cm-scroller").first().evaluate((element) => element.scrollTop)).toBe(0);
    await expect(logo).toBeInViewport();
    await expect(page.locator(".cm-content")).toContainText("The ADE for humans.");
    await expect(page.locator(".cm-content")).not.toContainText('<p align="center">');
    // Nothing was fetched from the app's own address for a relative image.
    expect(appRequests.filter((pathname) => pathname.includes("/design/") || pathname.includes("/site/"))).toEqual([]);
    await screenshot(page, "markdown-html-readme-live");

    // The same page in the light theme: tokens only, so nothing is left dark.
    await page.keyboard.press(chord("settings"));
    await page.locator('[data-settings-tab="general"]').click();
    await page.locator('[data-theme-option="light"]').click();
    await expect(page.locator("html")).toHaveClass(/(^|\s)light(\s|$)/);
    await page.keyboard.press("Escape");
    expect(await blocks.count()).toBeGreaterThanOrEqual(6);
    await screenshot(page, "markdown-html-readme-live-light");

    // A caret in the hero block shows that block's source and no other.
    await blocks.filter({ has: page.locator('img[alt^="A Conductor plans"]') }).click();
    await expect(hero).toHaveCount(0);
    await expect(logo).toBeVisible();
    await expect(page.locator(".cm-content")).toContainText('<img src="site/public/assets/video/kick-conductor.gif"');
    await screenshot(page, "markdown-html-hero-source");
  } finally {
    close(stack);
  }
});

test("a document's script, outside path and http image never load, and a broken image shows its alt text", async ({ page }) => {
  const stack = await openStack(page, { "unsafe.md": UNSAFE, "shot.png": PNG });
  const external: string[] = [];
  page.on("request", (request) => {
    if (request.url().includes("127.0.0.1:9") || request.url().includes("example.com")) external.push(request.url());
  });
  try {
    await page.locator(`[data-explorer-row="${stack.repo}/unsafe.md"]`).click();
    // The id is not on the allowlist, so the block is found by its text.
    const block = page.locator(".cm-md-html-block", { hasText: "kept text" });
    await expect(block).toBeVisible({ timeout: 20_000 });
    await expect(block).not.toContainText("window.pwned");
    await expect(page.locator(".cm-md-html iframe, .cm-md-html script, .cm-md-html a[href^='javascript']")).toHaveCount(0);
    // The inside image loaded; the three others show their alt text and nothing else.
    await expect(page.locator('.cm-md-html img[alt="inside"]')).toHaveAttribute("src", /^blob:/);
    for (const alt of ["outside", "gone", "plain http"]) {
      await expect(page.locator(".cm-md-alt[data-state='failed']", { hasText: alt })).toBeVisible();
    }
    expect(await page.evaluate(() => (window as unknown as { pwned?: unknown }).pwned)).toBeUndefined();
    expect(external).toEqual([]);
    // shot.png twice (the tag and the Markdown image share one read) and missing.png; ../outside.png is never asked for.
    expect(stack.sent.get("file_bytes")).toBe(2);
    await screenshot(page, "markdown-html-broken-images");
  } finally {
    close(stack);
  }
});

test("a badge that cannot be reached ends in its alt text and the logo still draws, scrolled to the top", async ({ page }) => {
  const root = path.resolve("..");
  const stack = await openStack(page, {
    "README.md": fs.readFileSync(path.join(root, "README.md")),
    "design/brand/hide-icon-blinking.svg": fs.readFileSync(path.join(root, "design/brand/hide-icon-blinking.svg")),
    "site/public/assets/video/kick-conductor.gif": fs.readFileSync(path.join(root, "site/public/assets/video/kick-conductor.gif")),
  });
  // One badge host refuses at once (offline), the other never answers.
  await page.route("https://img.shields.io/badge/**", (route) => route.abort("internetdisconnected"));
  await page.route("https://img.shields.io/github/**", () => {});
  try {
    await page.locator(`[data-explorer-row="${stack.repo}/README.md"]`).click();
    for (const alt of ["Latest release", "Functional Source License 1.1", "macOS, Windows and Linux"]) {
      await expect(page.locator(".cm-md-alt[data-state='failed']", { hasText: alt })).toBeVisible({ timeout: 30_000 });
    }
    const logo = page.locator('.cm-md-html img[alt="hide owl app icon"]');
    await expect(logo).toBeInViewport();
    expect(await page.locator(".cm-scroller").first().evaluate((element) => element.scrollTop)).toBe(0);
    await screenshot(page, "markdown-html-readme-top-offline");
  } finally {
    close(stack);
  }
});

test("a Markdown link opens on modifier-click only, and a plain click places the caret", async ({ page, context }) => {
  const stack = await openStack(page, { "links.md": "intro\n\nsee [the site](https://example.com/x) now\n\nend\n" });
  try {
    await page.locator(`[data-explorer-row="${stack.repo}/links.md"]`).click();
    const link = page.locator(".cm-md-link");
    await expect(link).toHaveText("the site", { timeout: 20_000 });
    await link.click();
    await expect(page.locator(".cm-content")).toContainText("[the site](https://example.com/x)");
    expect(context.pages()).toHaveLength(1);
    await page.locator(".cm-content").getByText("end").click();
    await expect(link).toHaveText("the site");
    const opened = context.waitForEvent("page");
    await link.click({ modifiers: ["ControlOrMeta"] });
    expect((await opened).url()).toContain("example.com");
  } finally {
    close(stack);
  }
});

const DIAGRAMS = [
  "intro",
  "",
  "```mermaid",
  "flowchart LR",
  "  E[설정 앱] --> L[로컬 수신 엔진]",
  '  L --> X["<img src=x onerror=window.pwned=3>"]',
  "```",
  "",
  "middle",
  "",
  "```mermaid",
  "flowchart LR",
  "  A -->",
  "```",
  "",
  "end",
  "",
].join("\n");

test("a mermaid fence draws as its diagram, a broken one stays code under its reason, and a caret shows the source", async ({ page }) => {
  const stack = await openStack(page, { "diagram.md": DIAGRAMS });
  const asked: string[] = [];
  page.on("request", (request) => asked.push(new URL(request.url()).pathname));
  try {
    await page.locator(`[data-explorer-row="${stack.repo}/diagram.md"]`).click();
    const drawn = page.locator(".cm-md-mermaid[data-state='drawn']");
    await expect(drawn.locator("svg")).toBeVisible({ timeout: 30_000 });
    await expect(drawn).toContainText("로컬 수신 엔진");
    await expect(drawn.locator("script, img, image, a, [onerror]")).toHaveCount(0);
    const failed = page.locator(".cm-md-mermaid[data-state='failed']");
    await expect(failed.locator(".cm-md-alt")).toBeVisible();
    await expect(failed.locator(".cm-md-mermaid-source")).toHaveText("flowchart LR\n  A -->");
    expect(await page.evaluate(() => (window as unknown as { pwned?: unknown }).pwned)).toBeUndefined();
    expect(asked.filter((pathname) => pathname.endsWith("/x"))).toEqual([]);
    // The diagram never runs wider than the column it sits in.
    const widths = await drawn.evaluate((element) => [element.querySelector("svg")!.getBoundingClientRect().width, element.getBoundingClientRect().width]);
    expect(widths[0]).toBeLessThanOrEqual(widths[1] + 1);
    await screenshot(page, "markdown-mermaid-live");

    await drawn.click();
    await expect(drawn).toHaveCount(0);
    await expect(page.locator(".cm-content")).toContainText("```mermaid");
    await expect(page.locator(".cm-content")).toContainText("E[설정 앱] --> L[로컬 수신 엔진]");
  } finally {
    close(stack);
  }
});

const LONG = "흩어져 있던 이유와 모이는 이유가 한 논리여야 하고, 그 논리가 70년이라는 길이와 일곱이라는 수를 함께 설명해야 한다.";

const TABLES = [
  "intro",
  "",
  "| 장치 | 관객이 품는 질문 | 답이 회수해야 할 지점 |",
  "|---|:---:|---|",
  "| 70년 | 왜 하필 [지금](https://example.com/now)? | 70이라는 길이에 이유가 있어야 한다. |",
  `| 7체 | 왜 일곱이고, 왜 **흩어져** 있었나? | ${LONG} \`${"a1b2c3d4".repeat(8)}\` https://example.com/${"long/".repeat(16)}end |`,
  "",
  "below the table",
  "",
].join("\n");

test("a table draws as a header row and wrapping cells inside the column, its link opens on modifier-click, a press shows its source, and the gutter under it holds", async ({ page, context }) => {
  const stack = await openStack(page, { "table.md": TABLES });
  try {
    await page.locator(`[data-explorer-row="${stack.repo}/table.md"]`).click();
    const block = page.locator(".cm-md-table");
    const table = block.locator("table");
    await expect(table).toBeVisible({ timeout: 20_000 });
    await expect(table.locator("th")).toHaveText(["장치", "관객이 품는 질문", "답이 회수해야 할 지점"]);
    await expect(table.locator("th").nth(1)).toHaveCSS("text-align", "center");
    await expect(table.locator("tbody td").nth(4).locator("strong")).toHaveText("흩어져");
    await expect(page.locator(".cm-content")).not.toContainText("|---|");
    // The long cell wraps inside its own column, its hash and address too; the table stays inside the text column and the page never scrolls sideways.
    const oneLine = (await table.locator("th").first().boundingBox())!.height;
    expect((await table.locator("tbody td").nth(5).boundingBox())!.height).toBeGreaterThan(2 * oneLine);
    expect((await table.boundingBox())!.width).toBeLessThanOrEqual((await block.boundingBox())!.width + 1);
    expect(await page.locator(".cm-scroller").first().evaluate((element) => element.scrollWidth - element.clientWidth)).toBeLessThanOrEqual(0);
    await screenshot(page, "markdown-table-live");

    // The `+` of the line under the table sits on that line, so the table's drawn height reached the editor.
    const below = page.locator(".cm-content > .cm-line", { hasText: "below the table" });
    const row = (await below.boundingBox())!;
    const gutter = (await page.locator(".cm-view-comment-gutter").boundingBox())!;
    await page.mouse.move(gutter.x + gutter.width / 2, row.y + 4);
    const plus = (await page.locator('[data-view-comment-add="8"]').boundingBox())!;
    expect(plus.y).toBeGreaterThanOrEqual(row.y - 1);
    expect(plus.y + plus.height).toBeLessThanOrEqual(row.y + row.height + 1);

    // A link in a cell opens on modifier-click only and the table stays drawn; a plain press, on the link too, shows its source.
    const link = table.locator("a", { hasText: "지금" });
    const opened = context.waitForEvent("page");
    await link.click({ modifiers: ["ControlOrMeta"] });
    expect((await opened).url()).toContain("example.com/now");
    await expect(table).toBeVisible();
    await link.click();
    await expect(block).toHaveCount(0);
    expect(context.pages()).toHaveLength(2);
    await expect(page.locator(".cm-content")).toContainText("|---|:---:|---|");
    await below.click();
    await expect(table).toBeVisible();
  } finally {
    close(stack);
  }
});
