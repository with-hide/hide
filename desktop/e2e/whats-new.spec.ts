// The window after an update and the update history in the desktop app (PRD
// whats-new): the host fetches the releases file from a local server standing
// in for withhide.dev (`HIDE_RELEASES_FEED_URL`), the shell opens the window
// once for the version the app runs, and its close and the history's open are
// kept apart in the profile. A development app reports its package version,
// `0.1.0`, which the served file names as the version just updated to.
// End to end because what is proved crosses the three owners no cheaper layer
// holds together: the host's fetch and profile files, the shell's decision and
// drawing, and the practice window the host opens on Try it.

import { expect, type ElectronApplication } from "@playwright/test";
import fs from "node:fs";
import http from "node:http";
import type { AddressInfo } from "node:net";
import path from "node:path";
import { startEngine, type EngineServer } from "../../web/e2e/engine-fixture";
import { hostLog, isolate, launchShell, screenshot, test, type Isolated } from "./fixture";

const APP_VERSION = JSON.parse(fs.readFileSync(path.join(__dirname, "..", "package.json"), "utf8")).version as string;

const words = (en: string, ko: string) => ({ en, ko, "zh-CN": en, ja: en });
const RELEASES = {
  schema: 1,
  versions: [
    {
      version: APP_VERSION,
      date: "2026-11-20",
      highlights: [
        { drawing: "mission", chapter: "mission", text: { en: { title: "Faster missions", line: "The next part starts as soon as the one before ends." }, ko: { title: "미션이 더 빨라짐", line: "앞 단계가 끝나는 대로 다음 노드가 바로 출발해요." } } },
        { drawing: "lane", chapter: null, text: { en: { title: "Lane counts", line: "The right lane counts the last seven days." }, ko: { title: "레인 숫자", line: "오른쪽 레인이 지난 7일을 세어요." } } },
      ],
      changes: [
        { kind: "change", text: words("Review limits per mission", "리뷰 횟수 한도를 미션마다 정해요") },
        { kind: "fix", text: words("Long letters are no longer cut", "긴 편지가 잘려 보이던 문제") },
      ],
    },
    { version: "0.0.9", date: "2026-11-12", highlights: [], changes: [{ kind: "fix", text: words("An older fix", "예전 수정") }] },
  ],
};

let engine: EngineServer;
let run: Isolated;
let app: ElectronApplication | null = null;
let server: http.Server;
let served = 0;

test.describe.configure({ timeout: 180_000 }); // wait-ok: the app starts its own daemon before the shell draws

test.beforeEach(async () => {
  served = 0;
  server = http.createServer((request, response) => {
    if (request.url !== "/app/releases.json") {
      response.writeHead(404).end();
      return;
    }
    served += 1;
    response.writeHead(200, { "content-type": "application/json" }).end(JSON.stringify(RELEASES));
  });
  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  engine = await startEngine();
  run = isolate(engine, "whats-new");
  run.env.HIDE_RELEASES_FEED_URL = `http://127.0.0.1:${(server.address() as AddressInfo).port}/app/releases.json`;
  // The app ran 0.0.9 before this launch and closed that version's window.
  const profile = run.env.HIDE_DESKTOP_USER_DATA_DIR!;
  fs.mkdirSync(profile, { recursive: true });
  fs.writeFileSync(path.join(profile, "whats-new.json"), JSON.stringify({ shown: "0.0.9", seen: "0.0.9", opened: [] }));
});

test.afterEach(async () => {
  const info = test.info();
  if (info.status !== info.expectedStatus) console.log(hostLog(run.env).map((line) => JSON.stringify(line)).join("\n"));
  await app?.close().catch(() => undefined);
  app = null;
  run.cleanup();
  engine.stop();
  await new Promise<void>((resolve) => server.close(() => resolve()));
});

const record = () => JSON.parse(fs.readFileSync(path.join(run.env.HIDE_DESKTOP_USER_DATA_DIR!, "whats-new.json"), "utf8")) as { shown: string | null; seen: string | null; opened: string[] };

test("the window opens once for the version just updated to, its close and the history's open are kept apart, and Try it opens the practice card", async () => {
  let page;
  ({ app, page } = await launchShell(run.env));

  // B1, B2: the window over the shell, for the running version, with both features and the folded changes.
  const dialog = page.locator(`[data-whats-new="${APP_VERSION}"]`);
  await expect(dialog).toBeVisible({ timeout: 30_000 }); // wait-ok: the host fetched the file and the core snapshot arrived
  await expect(dialog.locator("[data-whats-new-feature]")).toHaveCount(2);
  await expect(dialog.locator('[data-whats-new-stage="mission"]')).toBeVisible();
  await expect(dialog.locator('[data-whats-new-try="mission"]')).toBeVisible();
  await expect(dialog.locator("[data-whats-new-feature='1'] ~ [data-whats-new-try]")).toHaveCount(0);
  // B24: the keyboard starts on OK.
  await expect(dialog.locator("[data-whats-new-ok]")).toBeFocused();
  // B4: the changes and fixes open in place.
  await expect(dialog.locator("[data-whats-new-changes]")).toHaveCount(0);
  await dialog.locator("[data-whats-new-more-toggle]").click();
  await expect(dialog.locator("[data-change-kind]")).toHaveCount(2);
  await screenshot(page, "whats-new-window");
  // B3: pressing a feature shows its drawing.
  await dialog.locator("[data-whats-new-feature='1']").click();
  await expect(dialog.locator('[data-whats-new-stage="lane"]')).toBeVisible();

  // B5: OK closes it and the record says this version's window closed; the history's record stays.
  await dialog.locator("[data-whats-new-ok]").click();
  await expect(dialog).toHaveCount(0);
  await expect.poll(record).toEqual({ shown: APP_VERSION, seen: "0.0.9", opened: [] });

  // B15, B16, B17: Help opens the history on the running version, marked NEW until it is opened.
  await page.locator("[data-help-menu-trigger]").click();
  await page.locator('[data-help-item="update_history"]').click();
  const history = page.locator("[data-update-history]");
  await expect(history.locator(`[data-update-history-entry="${APP_VERSION}"]`)).toBeVisible();
  await expect(history.locator("[data-update-history-version]")).toHaveCount(2);
  await expect(history.locator('[data-update-history-version="0.0.9"]')).toHaveAttribute("data-major", "false");
  await expect.poll(record).toEqual({ shown: APP_VERSION, seen: APP_VERSION, opened: [] });
  await expect(history.locator("[data-update-history-new]")).toHaveCount(0);
  await screenshot(page, "whats-new-history");

  // B12: Try it closes the history and opens the practice window with the chapter's card open.
  const practice = app.waitForEvent("window");
  await history.locator('[data-update-history-try="mission"]').click();
  await expect(history).toHaveCount(0);
  const window = await practice;
  await expect(window.locator('[data-practice-detail="mission"]')).toBeVisible({ timeout: 30_000 }); // wait-ok: the practice window loaded its page
  expect(served).toBeGreaterThan(0);
});
