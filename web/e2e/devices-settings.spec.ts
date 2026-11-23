// Settings > Devices on an isolated engine and hided (PRD settings-cleanup
// B50, B54, B55, B56, B66): a device row is one line with Test and a ⋯ menu,
// Add device is one button above the list, Connection details holds what a
// healthy row no longer shows, and Revoke and Remove ask before they act. The
// device is a Host of the fixture's ssh config that never resolves, so it is registered and unreachable;
// a failed kit part and a healthy kit are the component test's
// (`web/src/settings/DevicesTab.test.tsx`), because a reachable device needs
// the desktop suite's isolated sshd (`desktop/e2e/device-kit.spec.ts`).

import { expect, test, type Page } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";
import { startFixture } from "./engine-fixture";
import { startHided, writeSshHost, type Daemon } from "./hided-fixture";
import { fixtureExecutable } from "./platform-fixture";
import { copyFixtureShim } from "./shims/build";
import { countSent, enterWorkspace, screenshot } from "./wire";

test.describe.configure({ timeout: 120_000 });

const ALIAS = "unreachable-e2e";

/**
 * A `bin` holding this spec's stand-in `ssh`, for the daemon's PATH.
 *
 * Hide reads an alias with `ssh -G` under a three-second deadline, and reading
 * the runner's own `ssh` made this spec depend on the runner finishing inside
 * it: a loaded one did not, the row read `unresolved` (`timed_out`), and it
 * stays that way until the dialog is opened again, which is what made the
 * press in this spec wait out its whole limit (with-hide/hide-private#842).
 * The parser and that deadline keep their own tests in
 * `hide-node/src/ssh/hosts.rs`; what this spec checks is that a Host reaches
 * the list and can be chosen. The stand-in answers `ssh -G` and nothing else,
 * and only this spec's daemon gets it (a spec that dials a device keeps the
 * system's `ssh`).
 */
function sshStandIn(dir: string): string {
  const bin = path.join(dir, "ssh-bin");
  fs.mkdirSync(bin, { recursive: true });
  copyFixtureShim("ssh-shim", path.join(bin, fixtureExecutable("ssh")));
  return bin;
}

async function openDevices(page: Page, daemon: Daemon) {
  await page.goto(`${daemon.origin}/#token=${daemon.token}`);
  await enterWorkspace(page, "fixture");
  await page.locator("[data-open-settings]").click();
  await page.locator('[data-settings-tab="devices"]').click();
  return page.locator('[data-settings="true"]');
}

test("a device row is one state with a menu, Add device is one button, a rename reaches the rail, and Remove asks first", async ({ page }) => {
  await page.setViewportSize({ width: 900, height: 800 });
  const engine = await startFixture();
  let daemon: Daemon | null = null;
  try {
    daemon = await startHided(engine, "devices-settings", undefined, { PATH: `${sshStandIn(engine.root)}${path.delimiter}${engine.fixturePath}` });
    writeSshHost(daemon, ALIAS);
    const sent = countSent(page);
    const sheet = await openDevices(page, daemon);

    // B50: one Add device button above the list; the form is not on the page until it is pressed.
    await expect(sheet.locator("[data-add-device]")).toHaveCount(0);
    await expect(sheet.locator("[data-device-add-open]")).toBeVisible();

    // B55: This Mac's menu has Connection details and nothing that removes it or revokes a helper.
    const local = sheet.locator(`[data-device-row="${daemon.node}"]`);
    await expect(local.locator(`[data-device-name="${daemon.node}"]`)).toBeVisible();
    await local.locator(`[data-device-menu="${daemon.node}"]`).click();
    const localMenu = page.locator(`[data-device-menu-content="${daemon.node}"]`);
    await expect(localMenu.getByRole("menuitem")).toHaveText(["Connection details…"]);
    await localMenu.locator(`[data-device-details="${daemon.node}"]`).click();
    const details = page.locator(`[data-device-details-dialog="${daemon.node}"]`);
    await expect(details).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(details).toHaveCount(0);

    // B50: Add device opens the dialog, and the device is listed once it is registered; the dialog closes itself.
    await sheet.locator("[data-device-add-open]").click();
    await expect(page.locator("[data-add-device-dialog]")).toBeVisible();
    // It asks computer or phone first; a computer opens on one that runs Hide, and a machine over SSH is the other choice.
    await page.locator('[data-add-choice="computer"]').click();
    const sshChoice = page.locator('[data-add-kind-choice="ssh"]');
    await sshChoice.click();
    await page.locator(`[data-ssh-host="${ALIAS}"]`).click();
    await expect(page.locator("[data-device-label]")).toHaveValue(ALIAS);
    await page.locator("[data-device-label]").fill("연구실 빌드 서버");
    await page.locator("[data-add-device]").click();
    await expect(page.locator("[data-add-device-dialog]")).toHaveCount(0);

    // B54; operator review 2026-11-23: the device is one row: its name marked as the old way, and one state.
    const row = sheet.locator(`[data-device-row="${ALIAS}"]`);
    await expect(row).toBeVisible({ timeout: 20_000 });
    await expect(row.locator(`[data-device-old-link="${ALIAS}"]`)).toHaveText("old way");
    await expect(row.locator(`[data-device-state="${ALIAS}:unavailable"]`)).toBeVisible({ timeout: 30_000 });
    await expect(row.locator("[data-device-state]")).toHaveCount(1);
    // B56: no per-device Agents line, no Coordination retirement row, no kit part list.
    await expect(row).not.toContainText(/Agents:|Coordination|hide command/);
    await expect(row.locator("[data-kit-part]")).toHaveCount(0);
    await screenshot(page, "devices-row");

    // B55, B66: the menu opens from the keyboard, lists the actions in order, and Escape closes it back to its button.
    const trigger = row.locator(`[data-device-menu="${ALIAS}"]`);
    await trigger.focus();
    await page.keyboard.press("Enter");
    const menu = page.locator(`[data-device-menu-content="${ALIAS}"]`);
    await expect(menu).toBeVisible();
    const items = await menu.getByRole("menuitem").allTextContents();
    expect(items.slice(0, 3)).toEqual(["Rename", "Test", "Connection details…"]);
    expect(items.at(-1)).toBe("Remove…");
    await screenshot(page, "devices-menu");
    await page.keyboard.press("Escape");
    await expect(menu).toHaveCount(0);
    await expect(trigger).toBeFocused();

    // c2: Rename is one event, the core keeps the name, and the row and the rail show it.
    await trigger.click();
    await page.locator(`[data-device-rename="${ALIAS}"]`).click();
    await page.locator(`[data-device-rename-input="${ALIAS}"]`).fill("빌드 서버");
    await page.locator(`[data-device-rename-go="${ALIAS}"]`).click();
    await expect.poll(() => sent.get("rename_device") ?? 0).toBe(1);
    await expect(row.locator(`[data-device-name="${ALIAS}"]`)).toHaveText("빌드 서버");
    await expect(page.locator(`[data-rail-tile="${ALIAS}"]`)).toHaveAttribute("aria-label", /^빌드 서버/);

    // B55: Remove asks before it acts, Keep leaves the device, and Remove takes it away.
    await trigger.click();
    await page.locator(`[data-device-remove="${ALIAS}"]`).click();
    const confirm = page.locator(`[data-device-remove-confirm="${ALIAS}"]`);
    await expect(confirm).toBeVisible();
    await confirm.getByRole("button", { name: "Keep device" }).click();
    await expect(confirm).toHaveCount(0);
    await expect(row).toBeVisible();
    expect(sent.get("remove_device") ?? 0).toBe(0);
    await trigger.click();
    await page.locator(`[data-device-remove="${ALIAS}"]`).click();
    await page.locator("[data-device-remove-go]").click();
    await expect(row).toHaveCount(0, { timeout: 20_000 });
    expect(sent.get("remove_device")).toBe(1);
  } finally {
    daemon?.stop();
    engine.stop();
  }
});

test("the Add dialog lists the account's Hosts, fills the name from the chosen one, and has no field for a username, port or key", async ({ page }) => {
  await page.setViewportSize({ width: 900, height: 800 });
  const engine = await startFixture();
  let daemon: Daemon | null = null;
  try {
    daemon = await startHided(engine, "devices-add-dialog", undefined, { PATH: `${sshStandIn(engine.root)}${path.delimiter}${engine.fixturePath}` });
    const sheet = await openDevices(page, daemon);

    // B52: with no Host in the config the dialog is only what to add, and there is no form.
    await sheet.locator("[data-device-add-open]").click();
    // It asks computer or phone first; a computer opens on one that runs Hide, and a machine over SSH is the other choice.
    await page.locator('[data-add-choice="computer"]').click();
    const sshChoice = page.locator('[data-add-kind-choice="ssh"]');
    await sshChoice.click();
    const dialog = page.locator("[data-add-device-dialog]");
    await expect(dialog.locator('[data-ssh-hosts-state="empty"]')).toBeVisible();
    await expect(dialog.locator("[data-add-device-form], [data-add-device]")).toHaveCount(0);
    await screenshot(page, "add-device-empty");
    await page.keyboard.press("Escape");
    await expect(dialog).toHaveCount(0);

    // B50, B51: a Host written while the dialog was closed is listed when it opens again, with the address ssh resolves it to.
    writeSshHost(daemon, ALIAS);
    await sheet.locator("[data-device-add-open]").click();
    await sshChoice.click();
    const host = dialog.locator(`[data-ssh-host="${ALIAS}"]`);
    await expect(host).toBeVisible();
    await expect(host).toHaveAttribute("data-ssh-host-state", "available");
    await expect(host).toContainText("e2e@unreachable-e2e.invalid");
    await expect(dialog.locator("[data-add-device]")).toBeDisabled();
    await expect(dialog.locator("[data-device-alias]")).toHaveCount(0);

    // B50: choosing fills the Name with the alias; once the person writes their own, a later choice leaves it.
    await host.click();
    await expect(dialog.locator("[data-device-label]")).toHaveValue(ALIAS);
    await expect(dialog.locator("[data-add-device]")).toBeEnabled();

    // B53: the three install lines sit under Advanced, closed at first, and no socket is asked for.
    const advanced = dialog.locator("[data-add-device-advanced]");
    await expect(dialog.locator("[data-add-device-installs]")).toBeHidden();
    await advanced.click();
    await expect(dialog.locator("[data-add-device-installs] li")).toHaveCount(3);
    await expect(dialog.locator("[data-device-socket]")).toHaveCount(0);
    await screenshot(page, "add-device-host");
  } finally {
    daemon?.stop();
    engine.stop();
  }
});
