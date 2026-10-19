import {expect, type ElectronApplication} from '@playwright/test';
import {execFileSync} from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import {startHerdr, setFixtureLifecycle} from '../../web/e2e/herdr-fixture';
import {enterWorkspace, showExplorer} from '../../web/e2e/wire';
import {isolate, launch, test} from './fixture';

test('shared Overview covers native pages, retains work and restores the keyboard', async () => {
  const herdr = await startHerdr();
  const run = isolate(herdr, 'overview-modal');
  let app: ElectronApplication | null = null;
  try {
    fs.writeFileSync(path.join(herdr.root, 'fixture', 'page.html'), '<!doctype html><title>Overview page fixture</title><input aria-label="Keyboard owner">');
    const launched = await launch(run.env);
    app = launched.app;
    const page = launched.page;
    const identity = await app.evaluate(({BrowserWindow}) => {
      const windows = BrowserWindow.getAllWindows();
      if (windows.length !== 1) throw new Error('expected one isolated candidate window');
      return {pid: process.pid, executable: process.execPath, window: windows[0]!.getMediaSourceId()};
    });
    const evidence = process.env.HIDE_E2E_SCREENSHOT_DIR;
    if (evidence) fs.writeFileSync(path.join(evidence, 'native-overview-identity.json'), JSON.stringify({...identity, daemonPid: run.daemonPid(), state: run.env.HIDE_STATE_DIR, socket: herdr.socket}));
    const capture = (name: string) => {
      if (evidence && process.platform === 'darwin') execFileSync('/usr/sbin/screencapture', ['-x','-o','-l',identity.window.split(':')[1]!,path.join(evidence, `${name}.png`)]);
    };
    await enterWorkspace(page);
    await showExplorer(page);
    await page.locator('[data-explorer-row$="/page.html"]').click({button:'right'});
    await page.locator('[data-explorer-menu] [data-menu-item="open-browser"]').click();
    await expect(page.locator('[data-browser-slot]')).toBeVisible();
    await expect.poll(() => app!.evaluate(({BrowserWindow}) => BrowserWindow.getAllWindows()[0]!.contentView.children.some(view => (view as {webContents?: Electron.WebContents}).webContents?.getTitle() === 'Overview page fixture'))).toBe(true);
    await page.locator('[data-open-overview]').click();
    const modal = page.getByRole('dialog',{name:'Overview',exact:true});
    await expect(modal).toBeVisible();
    await expect(page.locator('[data-browser-still]')).toBeVisible();
    capture('native-overview-dark');
    await page.keyboard.press('Escape');
    await expect(modal).toHaveCount(0);
    await expect(page.locator('[data-browser-still]')).toHaveCount(0);
    await setFixtureLifecycle(herdr,herdr.panes[0],'blocked');
    await expect(page.locator('[data-overview-dot]')).toBeVisible();
    await page.locator('[data-open-settings]').click();
    await page.locator('[data-settings-tab="general"]').click();
    await page.locator('[data-theme-option="light"]').click();
    await expect(page.locator('html')).toHaveClass(/\blight\b/);
    await page.keyboard.press('Escape');
    await page.locator('[data-open-overview]').click();
    await expect(modal).toBeVisible();
    capture('native-overview-light-needs-you');
    await page.keyboard.press('Escape');
    await app.evaluate(({Menu}) => Menu.getApplicationMenu()!.getMenuItemById('overview')!.click());
    await expect(modal).toBeVisible();
    await expect(modal.locator('[data-main-view="work"]')).toBeVisible();
    await app.evaluate(({Menu}) => Menu.getApplicationMenu()!.getMenuItemById('sidebar_projects')!.click());
    await expect(modal).toHaveCount(0);
    await app.evaluate(({Menu}) => Menu.getApplicationMenu()!.getMenuItemById('overview')!.click());
    await expect(modal).toBeVisible();
  } finally {
    await app?.close();
    run.cleanup();
    herdr.stop();
  }
});
