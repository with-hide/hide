// The web design system reset on an isolated engine and hided: Settings
// chooses System, Light or Dark and an accent, the page switches its tokens at
// once, an open terminal is re-colored without being recreated (B13), the
// choice survives a daemon restart (B6, B7), and the production build hided
// serves carries no /gallery (B14). The color theme (hide-color-theme) opens
// on Hide, switches to Neutral the same way, keeps the mode and the accent,
// and survives a restart.

import { expect, test, type Page } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";
import { startEngine, startFixture } from "./engine-fixture";
import { openProject, withControl } from "./daemon-actions";
import { startHided, type Daemon } from "./hided-fixture";
import { enterWorkspace, screenshot, showExplorer } from "./wire";
import { chord } from "./chords";
import { toPage } from "../../desktop/src/main/wirePath";

test.describe.configure({ timeout: 120_000 });

async function open(page: Page, daemon: Daemon): Promise<void> {
  // The probe reads the terminal buffer, which xterm draws on a canvas.
  await page.goto(`${daemon.origin}/?probe=1#token=${daemon.token}`);
}

/** What the focused terminal shows; xterm draws on a canvas, so the page's probe reads its buffer. */
function screenText(page: Page): Promise<string> {
  return page.evaluate(() => window.__hideProbe?.screenText() ?? "");
}

/** The colors the focused terminal draws with, read through the page's probe. */
function screenColors(page: Page): Promise<Record<string, string> | null> {
  return page.evaluate(() => window.__hideProbe?.screenColors() ?? null);
}

/** A color token as the page resolves it now, as the browser's rgb() form. */
function token(page: Page, name: string): Promise<string> {
  return page.evaluate((property) => {
    const probe = document.createElement("span");
    probe.style.setProperty("color", `var(${property})`);
    document.body.append(probe);
    const color = getComputedStyle(probe).color;
    probe.remove();
    return color;
  }, name);
}

/** A tokens.json hex in the same rgb() form. */
function rgb(hex: string): string {
  const [r, g, b] = [1, 3, 5].map((at) => parseInt(hex.slice(at, at + 2), 16));
  return `rgb(${r}, ${g}, ${b})`;
}

type Token = { type: string; value: string; light?: string; hide?: { value: string; light: string } };

function tokenFile(): Record<string, Token> {
  return JSON.parse(fs.readFileSync(path.resolve("..", "design", "tokens.json"), "utf8")).tokens;
}

/** A color token's hex in one color theme and mode, as tokens.json gives it. */
function hexOf(token: Token, palette: "hide" | "neutral", mode: "dark" | "light"): string {
  const values = palette === "hide" && token.hide ? token.hide : token;
  const hex = mode === "dark" ? values.value : values.light;
  if (!hex) throw new Error(`no ${palette} ${mode} value`);
  return hex;
}

async function openAppearance(page: Page): Promise<void> {
  await page.keyboard.press(chord("settings"));
  await expect(page.locator('[data-settings="true"]')).toBeVisible();
  await page.locator('[data-settings-tab="general"]').click();
}

test("the theme and accent switch at once, keep the terminal, and survive a restart", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  const tokens = tokenFile();
  const fixture = await startFixture({ agents: false });
  let daemon: Daemon | null = null;
  try {
    daemon = await startHided(fixture, "theme");
    await open(page, daemon);
    await enterWorkspace(page);

    // The first paint is Dark, the default (D-14), in the Hide color theme
    // (hide-color-theme D-04).
    const root = page.locator("html");
    await expect(root).toHaveClass(/(^|\s)dark(\s|$)/);
    expect(await token(page, "--background")).toBe(rgb(hexOf(tokens["--background"]!, "hide", "dark")));

    // A line typed into the terminal, and a mark on its element, tell a
    // re-colored terminal from a recreated one.
    const pane = page.locator('[data-pane-view][data-focused="true"]');
    await pane.locator(".xterm-helper-textarea").focus();
    await page.keyboard.type("echo theme-probe-7");
    await page.keyboard.press("Enter");
    await expect.poll(() => screenText(page), { timeout: 15_000 }).toContain("theme-probe-7");
    await pane.locator(".xterm").evaluate((element) => {
      (element as HTMLElement).dataset.themeProbe = "kept";
    });
    await pane.locator(".xterm-helper-textarea").focus();
    await page.keyboard.type("half-typed");

    await openAppearance(page);
    await page.locator('[data-theme-option="light"]').click();
    await expect(root).toHaveClass(/(^|\s)light(\s|$)/);
    await expect(root).not.toHaveClass(/(^|\s)dark(\s|$)/);
    expect(await token(page, "--background")).toBe(rgb(hexOf(tokens["--background"]!, "hide", "light")));
    await page.locator('[data-accent="sky"]').click();
    await expect.poll(() => token(page, "--primary")).toBe(rgb(tokens["--accent-choice-sky"]!.light!));
    await screenshot(page, "theme-light-settings");
    await page.keyboard.press("Escape");
    await expect(page.locator('[data-settings="true"]')).toHaveCount(0);

    // The terminal is the same element with the same content and the same
    // half-typed line (B13).
    await expect(pane.locator('.xterm[data-theme-probe="kept"]')).toHaveCount(1);
    const shown = await screenText(page);
    expect(shown).toContain("theme-probe-7");
    expect(shown).toContain("half-typed");
    await screenshot(page, "theme-light-workspace");

    // System follows the OS appearance live (D-14).
    await openAppearance(page);
    await page.locator('[data-theme-option="system"]').click();
    await page.emulateMedia({ colorScheme: "dark" });
    await expect(root).toHaveClass(/(^|\s)dark(\s|$)/);
    await page.emulateMedia({ colorScheme: "light" });
    await expect(root).toHaveClass(/(^|\s)light(\s|$)/);
    await page.locator('[data-theme-option="light"]').click();
    await page.keyboard.press("Escape");

    // The choice is the core's, so a restarted daemon and a reloaded page
    // bring it back.
    daemon = await daemon.restart();
    await page.goto("about:blank");
    await open(page, daemon);
    await expect(page.locator("[data-main-screen]").or(page.locator("[data-workspace-screen]"))).toBeVisible({ timeout: 20_000 });
    await expect(root).toHaveClass(/(^|\s)light(\s|$)/);
    await expect.poll(() => token(page, "--primary")).toBe(rgb(tokens["--accent-choice-sky"]!.light!));
  } finally {
    daemon?.stop();
    fixture.stop();
  }
});

test("the color theme switches the whole page at once from the keyboard, keeps the terminal, the mode and the accent, and survives a restart", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  const tokens = tokenFile();
  const fixture = await startFixture({ agents: false });
  let daemon: Daemon | null = null;
  // What the page and its focused terminal draw now, against tokens.json.
  // The built CSS may shorten a hex (#ffffff to #fff), so the terminal's colors are compared in six digits.
  const sixDigits = (hex: string) => (/^#[0-9a-f]{3}$/i.test(hex) ? `#${[...hex.slice(1)].map((digit) => digit + digit).join("")}` : hex).toLowerCase();
  const drawn = async () => {
    const terminal = await screenColors(page);
    return {
      background: await token(page, "--background"),
      primary: await token(page, "--primary"),
      seen: await token(page, "--status-seen"),
      swatches: [await token(page, "--swatch-hide"), await token(page, "--swatch-neutral")],
      terminal: terminal && Object.fromEntries(Object.entries(terminal).map(([name, hex]) => [name, sixDigits(hex)])),
    };
  };
  const expected = (palette: "hide" | "neutral", mode: "dark" | "light") => ({
    background: rgb(hexOf(tokens["--background"]!, palette, mode)),
    primary: rgb(hexOf(tokens["--accent-choice-lime"]!, palette, mode)),
    // B11: the Seen mark is Neutral's in both color themes. B2: each swatch is
    // its brand color in both modes, Hide's the owl mark's #B7C9CC.
    seen: rgb(hexOf(tokens["--status-seen"]!, "neutral", mode)),
    swatches: [rgb("#B7C9CC"), rgb(tokens["--swatch-neutral"]!.value)],
    terminal: expect.objectContaining({
      background: hexOf(tokens["--background"]!, palette, mode).toLowerCase(),
      blue: hexOf(tokens["--terminal-blue"]!, palette, mode).toLowerCase(),
      brightBlack: hexOf(tokens["--terminal-bright-black"]!, palette, mode).toLowerCase(),
      selectionBackground: hexOf(tokens["--terminal-selection"]!, palette, mode).toLowerCase(),
    }),
  });
  try {
    daemon = await startHided(fixture, "color-theme");
    await open(page, daemon);
    await enterWorkspace(page);
    const root = page.locator("html");

    // B6: a store that never chose opens Hide, in Dark.
    await expect(root).toHaveClass(/(^|\s)palette-hide(\s|$)/);
    const pane = page.locator('[data-pane-view][data-focused="true"]');
    await pane.locator(".xterm-helper-textarea").focus();
    await page.keyboard.type("echo palette-probe-3");
    await page.keyboard.press("Enter");
    await expect.poll(() => screenText(page), { timeout: 15_000 }).toContain("palette-probe-3");
    await expect.poll(drawn).toEqual(expected("hide", "dark"));
    await pane.locator(".xterm").evaluate((element) => {
      (element as HTMLElement).dataset.paletteProbe = "kept";
    });
    await pane.locator(".xterm-helper-textarea").focus();
    await page.keyboard.type("half-typed");

    // B2, B3: the arrow keys and Space choose Neutral, as on the Mode row, and
    // the page and the terminal change together; the mode and accent stay (B17).
    await openAppearance(page);
    const choice = page.locator("[data-color-theme-choice]");
    await expect(choice).toHaveAttribute("data-color-theme-choice", "hide");
    await page.locator('[data-color-theme-option="hide"]').focus();
    await page.keyboard.press("ArrowRight");
    await expect(page.locator('[data-color-theme-option="neutral"]')).toBeFocused();
    await page.keyboard.press("Space");
    await expect(choice).toHaveAttribute("data-color-theme-choice", "neutral");
    await expect(root).not.toHaveClass(/(^|\s)palette-hide(\s|$)/);
    await expect(root).toHaveClass(/(^|\s)dark(\s|$)/);
    await expect(page.locator("[data-theme-choice]")).toHaveAttribute("data-theme-choice", "dark");
    await expect.poll(drawn).toEqual(expected("neutral", "dark"));
    await screenshot(page, "color-theme-neutral-settings");
    await page.keyboard.press("Escape");
    await expect(page.locator('[data-settings="true"]')).toHaveCount(0);
    await expect(pane.locator('.xterm[data-palette-probe="kept"]')).toHaveCount(1);
    const shown = await screenText(page);
    expect(shown).toContain("palette-probe-3");
    expect(shown).toContain("half-typed");

    // B4: each color theme has its Light.
    await openAppearance(page);
    await page.locator('[data-theme-option="light"]').click();
    await expect(root).toHaveClass(/(^|\s)light(\s|$)/);
    await expect.poll(drawn).toEqual(expected("neutral", "light"));
    await page.keyboard.press("Escape");

    // B7: the choice is the core's, so a restarted daemon and a reloaded page bring it back.
    daemon = await daemon.restart();
    await page.goto("about:blank");
    await open(page, daemon);
    await expect(page.locator("[data-main-screen]").or(page.locator("[data-workspace-screen]"))).toBeVisible({ timeout: 20_000 });
    await expect(root).toHaveClass(/(^|\s)light(\s|$)/);
    await expect(root).not.toHaveClass(/(^|\s)palette-hide(\s|$)/);
    expect(await token(page, "--background")).toBe(rgb(hexOf(tokens["--background"]!, "neutral", "light")));
    await openAppearance(page);
    await page.locator('[data-color-theme-option="hide"]').click();
    await expect(root).toHaveClass(/(^|\s)palette-hide(\s|$)/);
    expect(await token(page, "--background")).toBe(rgb(hexOf(tokens["--background"]!, "hide", "light")));
  } finally {
    daemon?.stop();
    fixture.stop();
  }
});

test("an editor open during a theme switch is drawn like one opened after it", async ({ page }) => {
  // A wide window, as the other theme tests use; the second file opens from
  // the tree beside the first (PRD right-lane B10).
  await page.setViewportSize({ width: 1920, height: 1080 });
  const engine = await startEngine();
  let daemon: Daemon | null = null;
  try {
    const repoDir = path.join(engine.root, "demo");
    fs.mkdirSync(repoDir, { recursive: true });
    fs.writeFileSync(path.join(repoDir, "open.ts"), "export const open = 1;\n");
    fs.writeFileSync(path.join(repoDir, "later.ts"), "export const later = 2;\n");
    const repo = fs.realpathSync(repoDir);
    daemon = await startHided(engine, "editor-theme");
    await withControl(daemon, (control) => openProject(control, repoDir, "demo"));
    await open(page, daemon);
    await enterWorkspace(page, "demo");
    await showExplorer(page);

    // CodeMirror scopes its dark and light base styles by a class on the
    // editor. The switch swaps that class on the open editor, and what it
    // swaps in, and out, has to match an editor opened after the switch.
    const editor = (text: string) => page.locator("[data-editor-codemirror] .cm-editor", { hasText: text });
    // Focus is not the theme's: whichever editor holds it when a class list
    // is read carries `cm-focused`, and that moves with the keyboard.
    const classes = async (text: string) => new Set((await editor(text).getAttribute("class"))?.split(/\s+/).filter((name) => name !== "cm-focused") ?? []);
    await page.locator(`[data-explorer-row="${toPage(repo)}/open.ts"]`).dblclick();
    await expect(editor("open = 1")).toBeVisible();
    const dark = await classes("open = 1");

    await openAppearance(page);
    await page.locator('[data-theme-option="light"]').click();
    await expect(page.locator("html")).toHaveClass(/(^|\s)light(\s|$)/);
    await page.keyboard.press("Escape");
    await expect(page.locator('[data-settings="true"]')).toHaveCount(0);
    await expect.poll(async () => [...(await classes("open = 1"))].filter((name) => !dark.has(name)).length).toBeGreaterThan(0);
    const switched = await classes("open = 1");

    await page.locator(`[data-explorer-row="${toPage(repo)}/later.ts"]`).dblclick();
    await expect(editor("later = 2")).toBeVisible();
    const fresh = await classes("later = 2");

    const added = [...switched].filter((name) => !dark.has(name));
    const removed = [...dark].filter((name) => !switched.has(name));
    expect(removed.length).toBeGreaterThan(0);
    expect(added.filter((name) => !fresh.has(name))).toEqual([]);
    expect(removed.filter((name) => fresh.has(name))).toEqual([]);
  } finally {
    daemon?.stop();
    engine.stop();
  }
});

test("the production build hided serves has no gallery", async ({ page }) => {
  // The gallery is a dev-only module (B14): no built asset carries it...
  const assets = path.resolve("dist", "assets");
  for (const file of fs.readdirSync(assets).filter((name) => name.endsWith(".js"))) {
    expect(fs.readFileSync(path.join(assets, file), "utf8"), file).not.toContain("data-gallery-section");
  }
  // ...and /gallery on the served page opens nothing of it.
  const engine = await startEngine();
  let daemon: Daemon | null = null;
  try {
    daemon = await startHided(engine, "gallery");
    // hided answers an empty 404 for /gallery: the 404 is the claim, and no page loads to wait for.
    const response = await page.goto(`${daemon.origin}/gallery#token=${daemon.token}`);
    expect(response?.status()).toBe(404);
    await expect(page.locator("[data-gallery-section]")).toHaveCount(0);
  } finally {
    daemon?.stop();
    engine.stop();
  }
});
