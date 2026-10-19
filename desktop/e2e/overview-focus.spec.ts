// Overview focus against the actual private daemon and desktop renderer.
import { expect, type ElectronApplication } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { startHerdr, setFixtureLifecycle } from '../../web/e2e/herdr-fixture';
import { openOverview } from '../../web/e2e/overview-entry';
import { isolate, launch, test } from './fixture';
import { captureElectronWindow } from './electron-window';

test('Overview selects explicitly, shares filters and preserves layout in a private desktop', async () => {
  const herdr = await startHerdr();
  const run = isolate(herdr, 'overview-focus');
  let app: ElectronApplication | undefined;
  try {
    const launched = await launch(run.env);
    app = launched.app;
    const page = launched.page;
    await setFixtureLifecycle(herdr, herdr.panes[0]!, 'blocked');
    await openOverview(page, 'work');
    const graph = page.locator('[data-overview-graph]');
    await expect(graph).toHaveAttribute('data-graph-phase', 'ready', { timeout: 30_000 });
    await expect(graph.getByRole('checkbox', { name: 'Include resting' })).not.toBeChecked();
    const target = graph.locator('.og-session-target').first();
    await expect(target).toBeVisible();
    const positions = () => graph.locator('.react-flow__node').evaluateAll(nodes => nodes.map(node => [node.getAttribute('data-id'), (node as HTMLElement).style.transform]));
    const before = await positions();
    await target.click();
    await expect(graph.locator('[data-graph-preview]')).toBeVisible();
    await page.keyboard.press('Escape');
    await expect(graph.locator('[data-graph-preview]')).toHaveCount(0);
    await target.hover();
    await expect(graph.locator('[data-graph-preview]')).toHaveCount(0);
    await target.focus();
    await expect(graph.locator('[data-graph-preview]')).toHaveCount(0);
    await target.press('Enter');
    await expect(graph.locator('[data-graph-preview]')).toBeVisible();
    await expect(target).toHaveAttribute('aria-pressed', 'true');
    expect(await positions()).toEqual(before);
    await page.keyboard.press('Escape');
    await expect(graph.locator('[data-graph-preview]')).toHaveCount(0);
    await expect(graph).toBeVisible();
    await target.press('Space');
    await expect(graph.locator('[data-graph-preview]')).toBeVisible();
    await page.keyboard.press('Escape');
    await graph.getByRole('checkbox', { name: 'Letters', exact: true }).uncheck();
    expect(await positions()).toEqual(before);
    await graph.getByRole('checkbox', { name: 'Letters', exact: true }).check();
    expect(await positions()).toEqual(before);
    const groupSize = () => graph.locator('.react-flow__node-project').first().getAttribute('style');
    const originalSize = await groupSize();
    const originalWidth = await target.evaluate(node => node.getBoundingClientRect().width);
    await page.evaluate(() => document.documentElement.style.setProperty('--interface-scale', '1.3'));
    await expect.poll(groupSize).not.toBe(originalSize);
    expect(await target.evaluate(node => node.getBoundingClientRect().width)).toBeCloseTo(originalWidth * 1.3, 0);
    await page.evaluate(() => document.documentElement.style.removeProperty('--interface-scale'));
    await expect.poll(groupSize).toBe(originalSize);
    await graph.getByRole('button', { name: /^My turn/ }).click();
    await graph.getByRole('button', { name: 'Sessions', exact: true }).click();
    await expect(graph.locator('[data-graph-session-list] > button')).not.toHaveCount(0);
    await expect(graph.getByRole('button', { name: /^My turn/ })).toHaveAttribute('aria-pressed', 'true');
    await graph.getByRole('button', { name: 'Graph', exact: true }).click();
    await expect(graph).toHaveAttribute('data-graph-phase', 'ready');
    const evidence = process.env.HIDE_E2E_SCREENSHOT_DIR;
    await captureElectronWindow(app, 'overview-focus-renderer', { state: run.env.HIDE_STATE_DIR, socket: herdr.socket, daemonPid: run.daemonPid(), scenario: 'explicit selection, filters, stable coordinates' });
    if (evidence && process.platform === 'darwin') {
      const identity = await app.evaluate(({ BrowserWindow }) => {
        const windows = BrowserWindow.getAllWindows();
        if (windows.length !== 1) throw new Error('expected one owned candidate window');
        return { pid: process.pid, executable: process.execPath, window: windows[0]!.getMediaSourceId() };
      });
      fs.writeFileSync(path.join(evidence, 'overview-focus-native.json'), JSON.stringify({ ...identity, state: run.env.HIDE_STATE_DIR, socket: herdr.socket, head: execFileSync('git', ['rev-parse', 'HEAD'], { encoding: 'utf8' }).trim() }));
      execFileSync('/usr/sbin/screencapture', ['-x', '-o', '-l', identity.window.split(':')[1]!, path.join(evidence, 'overview-focus-native.png')]);
    }
  } finally {
    await app?.close();
    run.cleanup();
    herdr.stop();
  }
});
