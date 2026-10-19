import { expect, type ElectronApplication } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";
import { startHerdr } from "../../web/e2e/herdr-fixture";
import { checkoutGitFixture } from "../../web/e2e/checkout-git-fixture";
import { enterWorkspace } from "../../web/e2e/wire";
import { git } from "../../web/e2e/work-prs-fixture";
import { fitWindow, isolate, launchShell, test } from "./fixture";
import { captureNativeWindow } from "./native-window";

test("checkout Git header fits the isolated native candidate at 720 by 480", async () => {
  const herdr = await startHerdr();
  let run: ReturnType<typeof isolate> | undefined;
  let app: ElectronApplication | undefined;
  const errors: unknown[] = [];
  try {
    const fixture = checkoutGitFixture(herdr, { divergedUpstream: true });
    git(fixture.root, ["branch", "-m", "feature/매우-긴-한글과-English-checkout-name"]);
    run = isolate(herdr, "checkout-header");
    const opened = await launchShell(run.env);
    app = opened.app;
    const page = opened.page;
    await fitWindow(app, { width: 720, height: 480 });
    await enterWorkspace(page, "fixture");
    fs.writeFileSync(path.join(fixture.root, "new.txt"), "new\n");
    await expect(page.locator("[data-checkout-files]")).toHaveText("1", { timeout: 15_000 });
    await expect(page.locator('[data-checkout-ci="failed"]')).toBeVisible({ timeout: 15_000 });
    await expect(page.locator('[data-checkout-upstream="tracked"]')).toHaveText("11");
    for (const theme of ["light", "dark"]) {
      await page.getByRole("button", { name: /^Settings \(/ }).click();
      await page.locator('[data-settings-tab="general"]').click();
      await page.locator(`[data-theme-option="${theme}"]`).click();
      const size = page.locator('[data-font-size="true"] [role="slider"]');
      await size.focus();
      await page.keyboard.press("End");
      await expect(size).toHaveAttribute("aria-valuenow", "17");
      await page.keyboard.press("Escape");
      const toolbar = page.locator("[data-workspace-toolbar]");
      expect(await toolbar.evaluate(node => node.scrollWidth > node.clientWidth)).toBe(false);
      // The 32px owning row includes its 1px border; the toolbar fills its content box.
      expect(await page.locator('[data-title-row]').evaluate(node => node.getBoundingClientRect().height)).toBe(32);
      const geometry = await toolbar.evaluate(node => {
        const controls = ["data-checkout-commit", "data-checkout-ci", "data-open-overview", "data-open-server"].map(attribute => {
          const button = node.querySelector<HTMLButtonElement>(`[${attribute}]`)!;
          const box = button.getBoundingClientRect();
          const children = [...button.children].map(child => child.getBoundingClientRect());
          return { attribute, left: box.left, right: box.right, width: box.width, contentRight: Math.max(...children.map(child => child.right)), hit: button.contains(document.elementFromPoint(box.x + box.width / 2, box.y + box.height / 2)) };
        });
        return { toolbarWidth: node.getBoundingClientRect().width, controls };
      });
      console.log(`${theme} geometry: ${JSON.stringify(geometry)}`);
      const upstream = await page.locator('[data-checkout-upstream="tracked"]').evaluate(node => ({
        label: node.getAttribute("aria-label"),
        color: getComputedStyle(node).color,
        children: [...node.children].map(child => ({ tag: child.tagName, text: child.textContent, color: getComputedStyle(child).color })),
      }));
      console.log(`${theme} upstream: ${JSON.stringify(upstream)}`);
      await captureNativeWindow(app, `checkout-header-native-720-${theme}`, { geometry, upstream, sha: fixture.sha, provider: "private synthetic check-runs and commit statuses; actual local Git diverged from origin/main", home: run.env.HOME, socket: herdr.socket, state: run.env.HIDE_STATE_DIR });
      for (const [index, control] of geometry.controls.entries()) {
        expect(control.hit, `${control.attribute} center reaches its own button`).toBe(true);
        expect(control.contentRight, `${control.attribute} contents fit its button`).toBeLessThanOrEqual(control.right + 1);
        const next = geometry.controls[index + 1];
        if (next) expect(next.left - control.right, `${control.attribute} is separate from ${next.attribute}`).toBeGreaterThanOrEqual(2);
      }
      await page.locator('[data-checkout-ci]').click();
      await expect(page.locator('[data-ci-sha]')).toHaveText(fixture.sha);
      await page.keyboard.press("Escape");
      await page.locator('[data-open-overview]').click();
      await expect(page.locator('[data-overview-page]')).toBeVisible();
      await page.keyboard.press("Escape");
      await expect(page.locator('[data-overview-page]')).toHaveCount(0);
      await page.locator('[data-open-server]').click();
      await expect(page.getByRole('dialog', { name: 'Running servers' })).toBeVisible();
      await page.keyboard.press("Escape");
    }
  } catch (error) {
    errors.push(error);
  } finally {
    try { await app?.close(); } catch (error) { errors.push(error); }
    try { run?.cleanup(); } catch (error) { errors.push(error); }
    try { herdr.stop(); } catch (error) { errors.push(error); }
  }
  if (errors.length === 1) throw errors[0];
  if (errors.length) throw new AggregateError(errors, "private checkout header fixture failed; original and cleanup errors are retained");
});
