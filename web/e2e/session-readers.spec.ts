// Synthetic native-format writers on actual pinned Herdr and hided.
// Installed CLI measurements remain a separate acceptance boundary.
import { expect, test, type BrowserContext, type Page } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";
import { DatabaseSync } from "node:sqlite";
import { startHerdr } from "./herdr-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import crypto from "node:crypto";
import {
  CURSOR_GOAL, CURSOR_ID, GROK_ID, GROK_PLAN, GROK_TITLE, OMP_ID, OMP_TITLE, OMP_UPDATED_TITLE, OPENCODE_CHILD_TITLE, OPENCODE_ID, OPENCODE_TITLE, PI_ID,
  PI_TITLE, appendOmpQuestion, prepareNativeWriter, prepareOpenCodeWriter, reportNativeWriter, reportOpenCodeState, setGrokPlanApproval, updateOmpTitle,
  writeOpenCodeQuestion,
} from "./session-reader-fixture";
import { openSleepingProject, screenshot, sendEvent } from "./wire";
import { afterCleanup } from "./worker-owned";
import { FakeTailscale } from "./fake-tailscale";
import { dumpOnFailure } from "./failure-dump";
import type { AgentRow } from "../src/snapshot";

type QuestionRow = AgentRow & { user_turn?: { kind: string; content: { text: string; choices: string[]; truncated: boolean } | null } };

test.describe.configure({ timeout: 150_000 });

async function disableAgentSummaries(page: Page): Promise<void> {
  await page.locator("[data-open-settings]").click();
  await page.locator('[data-settings-tab="hideAi"]').click();
  const summaries = page.locator("[data-ai-agent-summary]");
  await expect(summaries).toHaveAttribute("data-ai-agent-summary", "true");
  await summaries.click();
  await expect(summaries).toHaveAttribute("data-ai-agent-summary", "false");
  await page.locator("[data-settings-close]").click();
  await expect(page.locator("[data-settings]")).toHaveCount(0);
}

for (const [kind, id, initialTitle, resumeFlag] of [["pi", PI_ID, PI_TITLE, "--session"], ["omp", OMP_ID, OMP_TITLE, "--resume"], ["grok", GROK_ID, GROK_TITLE, "--resume"]] as const) {
test(`${kind} native title and durable sleep wake the exact conversation in a fresh pane`, async ({ page }) => {
  let agents: QuestionRow[] = [];
  page.on("websocket", (socket) => socket.on("framereceived", (frame) => {
    if (typeof frame.payload !== "string") return;
    const incoming = JSON.parse(frame.payload) as { type: string; payload?: { rest?: { navigator?: { agents?: QuestionRow[] } } } };
    if (["snapshot", "delta"].includes(incoming.type) && incoming.payload?.rest?.navigator?.agents) agents = incoming.payload.rest.navigator.agents;
  }));
  const herdr = await startHerdr({ agents: false });
  let daemon: Daemon | null = null;
  try {
    const sourcePane = herdr.panes[0];
    const session = prepareNativeWriter(herdr, kind);
    expect(fs.existsSync(session)).toBe(false);
    herdr.run(["agent", "start", `${kind}-reader`, "--kind", kind, "--pane", sourcePane]);
    await expect.poll(() => fs.existsSync(session)).toBe(true);
    reportNativeWriter(herdr, kind, sourcePane, session);
    daemon = await startHided(herdr, `${kind}-reader`, herdr.env.HOME);
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await page.locator("[data-checkout]").first().click();
    const row = page.locator(`nav[data-sidebar] [data-checkout-agents-open] [data-pane="${sourcePane}"]`);
    // status-model.md, Task identity: an automatic/source-less name follows the goal.
    const selected = kind === "pi" ? "Generated Pi goal" : kind === "grok" ? "Generated Grok goal" : initialTitle;
    await expect(row).toContainText(selected, { timeout: 30_000 });
    if (kind === "pi" || kind === "grok") await disableAgentSummaries(page);
    await expect(row).toContainText(initialTitle, { timeout: 30_000 });
    await screenshot(page, `${kind}-native-title`);
    let title: string = initialTitle;
    if (kind === "omp") {
      const size = fs.statSync(session).size;
      updateOmpTitle(session, OMP_UPDATED_TITLE);
      expect(fs.statSync(session).size).toBe(size);
      title = OMP_UPDATED_TITLE;
      await expect(row).toContainText(title, { timeout: 30_000 });
      await expect(row).not.toContainText("Old audit title");
      const observed = () => agents.find((agent) => agent.pane_id === sourcePane);
      const question = { kind: "question", content: { text: "배포 대상을 골라주세요", choices: ["미리보기", "운영"], truncated: false } };
      appendOmpQuestion(session);
      await expect.poll(() => observed()?.user_turn, { timeout: 30_000 }).toEqual(question);
      await expect(row.locator('[data-agent-status="question"]')).toBeVisible();
      await row.click();
      await expect(page.locator(`[data-pane-view="${sourcePane}"] [data-pane-head-turn]`)).toHaveAttribute("data-pane-head-turn", "answer");
      await screenshot(page, "omp-current-title-question");
      appendOmpQuestion(session, true);
      await expect.poll(() => observed()?.user_turn, { timeout: 30_000 }).toBeUndefined();
      await expect(row.locator('[data-agent-status="question"]')).toHaveCount(0);
    }
    if (kind === "grok") {
      // Herdr reads Grok's plan approval as working; Grok's own state is the wait.
      const observed = () => agents.find((agent) => agent.pane_id === sourcePane);
      setGrokPlanApproval(session, true);
      await expect.poll(() => observed()?.user_turn, { timeout: 30_000 }).toEqual({ kind: "plan_approval", content: { text: GROK_PLAN, choices: [], truncated: false } });
      await expect(row.locator('[data-agent-status="approval"]')).toBeVisible();
      await screenshot(page, "grok-plan-approval");
      setGrokPlanApproval(session, false);
      await expect.poll(() => observed()?.user_turn, { timeout: 30_000 }).toBeUndefined();
    }
    await row.click();
    await page.locator(`[data-pane-menu="${sourcePane}"]`).click();
    await expect(page.locator('[data-menu-item="sleep_agent"]')).toBeEnabled({ timeout: 20_000 });
    await page.locator('[data-menu-item="sleep_agent"]').click();
    // Every agent of the project sleeps now, so it folds (PRD sidebar-titlebar-refresh B4).
    await openSleepingProject(page);
    const sleeping = page.locator('[data-sleeping-session]');
    await expect(sleeping).toContainText(title, { timeout: 30_000 });
    await expect(row).toHaveCount(0);
    await expect.poll(() => JSON.stringify(herdr.run(["pane", "list"]))).not.toContain(`"pane_id":"${sourcePane}"`);
    await screenshot(page, `${kind}-durable-sleep`);
    const prior = fs.readFileSync(session);
    await sleeping.getByRole("button", { name: "Wake agent" }).click();
    type Listed = { result: { agents: { pane_id: string; agent: string }[] } };
    let fresh = "";
    await expect.poll(() => {
      const agents = (herdr.run(["agent", "list"]) as Listed).result.agents;
      fresh = agents.find(agent => agent.agent === kind && agent.pane_id !== sourcePane)?.pane_id ?? "";
      return fresh;
    }, { timeout: 30_000 }).not.toBe("");
    await expect.poll(() => fs.readFileSync(path.join(herdr.root, `${kind}-launches.jsonl`), "utf8").trim().split("\n").map(line => JSON.parse(line) as string[])).toContainEqual([resumeFlag, id]);
    reportNativeWriter(herdr, kind, fresh, session);
    await expect(sleeping).toHaveCount(0, { timeout: 30_000 });
    await expect(page.locator(`nav[data-sidebar] [data-pane="${fresh}"]`)).toContainText(title);
    expect(fs.readFileSync(session)).toEqual(prior);
    await screenshot(page, `${kind}-exact-wake`);
  } catch (error) {
    throw afterCleanup(afterCleanup(error, () => daemon?.stop()), () => herdr.stop());
  }
  try { daemon?.stop(); } catch (error) { throw afterCleanup(error, () => herdr.stop()); }
  herdr.stop();
});
}

for (const refusal of ["override", "duplicate"] as const) {
test(`Cursor ${refusal} root retains its historical goal while retired Sleep refuses to close its pane`, async ({ page }) => {
  const herdr = await startHerdr({ agents: false });
  let daemon: Daemon | null = null;
  try {
    const pane = herdr.panes[0];
    const session = prepareNativeWriter(herdr, "cursor");
    herdr.run(["agent", "start", "cursor-root", "--kind", "cursor", "--pane", pane]);
    await expect.poll(() => fs.existsSync(session)).toBe(true);
    reportNativeWriter(herdr, "cursor", pane, session);
    const alternate = path.join(herdr.env.HOME, "alternate-config");
    const other = path.join(alternate, "cursor", "chats", path.basename(path.dirname(path.dirname(session))), CURSOR_ID);
    fs.cpSync(path.dirname(session), other, { recursive: true });
    const copied = new DatabaseSync(path.join(other, "store.db"));
    try {
      const metadata = copied.prepare("SELECT value FROM meta WHERE key='0'").get() as { value: string };
      const native = JSON.parse(Buffer.from(metadata.value, "hex").toString()) as { latestRootBlobId: string };
      const graph = JSON.parse(fs.readFileSync(path.resolve("../hide-session/tests/fixtures/cursor-2026.10.01/browser-graph.json"), "utf8")) as { roots: { append: string } };
      native.latestRootBlobId = graph.roots.append;
      copied.prepare("UPDATE meta SET value=? WHERE key='0'").run(Buffer.from(JSON.stringify(native)).toString("hex"));
    } finally { copied.close(); }
    const before = fs.readFileSync(session);
    const otherBefore = fs.readFileSync(path.join(other, "store.db"));
    expect(otherBefore).not.toEqual(before);
    const launches = path.join(herdr.root, "cursor-launches.jsonl");
    const starts = fs.readFileSync(launches, "utf8");
    const env = refusal === "override"
      ? { CURSOR_CONFIG_DIR: path.join(alternate, "cursor"), XDG_CONFIG_HOME: undefined }
      : { CURSOR_CONFIG_DIR: path.join(herdr.env.HOME, ".cursor"), XDG_CONFIG_HOME: alternate };
    daemon = await startHided(herdr, `cursor-root-${refusal}`, herdr.env.HOME, env);
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await page.locator("[data-checkout]").first().click();
    const row = page.locator(`nav[data-sidebar] [data-checkout-agents-open] [data-pane="${pane}"]`);
    await expect(row).toContainText(CURSOR_GOAL, { timeout: 30_000 });
    await row.click();
    await page.locator(`[data-pane-menu="${pane}"]`).click();
    await expect(page.locator('[data-menu-group="fork"]')).toHaveCount(0);
    await expect(page.locator('[data-menu-item="sleep_agent"]')).toBeDisabled();
    // A stale client must also be refused by the core, before any close or launch.
    await sendEvent(page, daemon, "agent_sleep", { pane_id: pane });
    const log = () => fs.readFileSync(path.join(daemon!.stateDir, "Logs", "core.jsonl"), "utf8");
    await expect.poll(log).toContain('"error_kind":"agent_sleep.refused"');
    await expect.poll(log).toContain("This agent's reader does not support sleep");
    await expect(row).toContainText(CURSOR_GOAL);
    await expect(page.locator('[data-sleeping-session]')).toHaveCount(0);
    expect(JSON.stringify(herdr.run(["pane", "list"]))).toContain(`"pane_id":"${pane}"`);
    expect(fs.readFileSync(launches, "utf8")).toEqual(starts);
    expect(fs.readFileSync(session)).toEqual(before);
    expect(fs.readFileSync(path.join(other, "store.db"))).toEqual(otherBefore);
    await screenshot(page, `cursor-${refusal}-sleep-refusal`);
  } catch (error) {
    throw afterCleanup(afterCleanup(error, () => daemon?.stop()), () => herdr.stop());
  }
  try { daemon?.stop(); } catch (error) { throw afterCleanup(error, () => herdr.stop()); }
  herdr.stop();
});
}

test("Cursor phone reads native message units and omits an unrecorded time", async ({ browser, page }) => {
  const herdr = await startHerdr({ agents: false });
  const tailscale = new FakeTailscale();
  let daemon: Daemon | null = null;
  let phoneContext: BrowserContext | undefined;
  const errors: unknown[] = [];
  try {
    tailscale.ready();
    tailscale.install();
    const pane = herdr.panes[0];
    const session = prepareNativeWriter(herdr, "cursor");
    herdr.run(["agent", "start", "cursor-phone", "--kind", "cursor", "--pane", pane]);
    await expect.poll(() => fs.existsSync(session)).toBe(true);
    reportNativeWriter(herdr, "cursor", pane, session);
    daemon = await startHided(herdr, "cursor-phone", herdr.env.HOME, { HIDE_TAILSCALE_BIN: tailscale.bin });
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await page.locator("[data-open-settings]").click();
    await page.locator('[data-settings-tab="mobile"]').click();
    await page.locator('[data-mobile-switch="true"]').click();
    await expect(page.locator('[data-mobile-ready="true"]')).toBeVisible({ timeout: 30_000 });
    await page.locator('[data-mobile-show-code="true"]').click();
    const qr = page.locator("[data-mobile-qr]");
    await expect(qr).toBeVisible();
    const url = await qr.getAttribute("data-mobile-qr");
    expect(url).toContain("/m/#pair=");
    phoneContext = await browser.newContext({ viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true, locale: "ko-KR" });
    const phone = await phoneContext.newPage();
    await phone.goto(`${daemon.origin}/m/${url!.slice(url!.indexOf("#"))}`);
    await phone.locator('[data-phone-pair="true"]').tap();
    await expect(phone.locator('[data-phone-connected="true"]')).toBeVisible();
    await phone.locator(`[data-phone-agent$="|${pane}"]`).tap();
    const messages = phone.locator("[data-phone-message]");
    await expect(messages).toHaveCount(2, { timeout: 30_000 });
    const human = phone.locator('[data-phone-message="you"]');
    await expect(human).toContainText("요청 보기를 만들어줘");
    await expect(human.locator("time")).toHaveCount(0);
    await expect(phone.locator('[data-phone-message="agent"] time')).toHaveAttribute("datetime", "2026-10-03T01:01:00.000Z");
    await expect(phone.locator("[data-phone-older]")).toHaveCount(0);
    await screenshot(phone, "cursor-phone-native-units-time");
  } catch (error) { errors.push(error); }
  try { await phoneContext?.close(); } catch (error) { errors.push(error); }
  try { daemon?.stop(); } catch (error) { errors.push(error); }
  try { herdr.stop(); } catch (error) { errors.push(error); }
  try { tailscale.remove(); } catch (error) { errors.push(error); }
  if (errors.length) throw new AggregateError(errors, "Cursor phone fixture or cleanup failed");
});

test("OpenCode's root session titles its row, waits on its question and wakes by id in a fresh pane", async ({ page }) => {
  let agents: QuestionRow[] = [];
  page.on("websocket", (socket) => socket.on("framereceived", (frame) => {
    if (typeof frame.payload !== "string") return;
    const incoming = JSON.parse(frame.payload) as { type: string; payload?: { rest?: { navigator?: { agents?: QuestionRow[] } } } };
    if (["snapshot", "delta"].includes(incoming.type) && incoming.payload?.rest?.navigator?.agents) agents = incoming.payload.rest.navigator.agents;
  }));
  const herdr = await startHerdr({ agents: false });
  let daemon: Daemon | null = null;
  try {
    const sourcePane = herdr.panes[0];
    const database = prepareOpenCodeWriter(herdr);
    const launches = () => fs.readFileSync(path.join(herdr.root, "opencode-launches.jsonl"), "utf8").trim().split("\n").map(line => JSON.parse(line) as string[]);
    herdr.run(["agent", "start", "opencode-reader", "--kind", "opencode", "--pane", sourcePane]);
    await expect.poll(() => fs.existsSync(database)).toBe(true);
    daemon = await startHided(herdr, "opencode-reader", herdr.env.HOME);
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await page.locator("[data-checkout]").first().click();
    const row = page.locator(`nav[data-sidebar] [data-pane="${sourcePane}"]`);
    // OpenCode records no manual source: goal first, native title with summaries off.
    await expect(row).toContainText("Generated OpenCode goal", { timeout: 30_000 });
    await disableAgentSummaries(page);
    await expect(row).toContainText(OPENCODE_TITLE, { timeout: 30_000 });
    await expect(page.locator("nav[data-sidebar]")).not.toContainText(OPENCODE_CHILD_TITLE);
    await screenshot(page, "opencode-native-title");

    // A question keeps OpenCode's answer unfinished; its answer completes it.
    const question = () => agents.find((agent) => agent.pane_id === sourcePane)?.user_turn;
    const nativeReadState = () => ({
      herdr: herdr.run(["agent", "list"]),
      row: agents.find((agent) => agent.pane_id === sourcePane),
      files: [database, `${database}-wal`, `${database}-shm`].map(file => ({ file, size: fs.existsSync(file) ? fs.statSync(file).size : null })),
      diagnostic: fs.readFileSync(path.join(daemon!.stateDir, "Logs", "core.jsonl"), "utf8").trim().split("\n").slice(-40),
    });
    await reportOpenCodeState(herdr, sourcePane, "working");
    writeOpenCodeQuestion(database, "running");
    await reportOpenCodeState(herdr, sourcePane, "idle");
    await dumpOnFailure("OpenCode question native read", nativeReadState, () =>
      expect.poll(question, { message: "the native question reached the row", timeout: 30_000 })
        .toEqual({ kind: "question", content: { text: "어느 브랜치에 올릴까요?", choices: ["main", "release"], truncated: false } }));
    await expect(page.locator(`[data-pane="${sourcePane}"] [data-agent-status="question"]`).first()).toBeVisible();
    await screenshot(page, "opencode-native-question");
    await reportOpenCodeState(herdr, sourcePane, "working");
    writeOpenCodeQuestion(database, "completed");
    await reportOpenCodeState(herdr, sourcePane, "idle");
    await expect.poll(question, { message: "the answer cleared the question", timeout: 30_000 }).toBeUndefined();

    // The answered turn is an unread completion, so the row is raised under Done as well.
    await row.first().click();
    await page.locator(`[data-pane-menu="${sourcePane}"]`).click();
    await dumpOnFailure("OpenCode sleep native proof", nativeReadState, () =>
      expect(page.locator('[data-menu-item="sleep_agent"]')).toBeEnabled({ timeout: 20_000 }));
    await page.locator('[data-menu-item="sleep_agent"]').click();
    await openSleepingProject(page);
    const sleeping = page.locator("[data-sleeping-session]");
    await expect(sleeping).toContainText(OPENCODE_TITLE, { timeout: 30_000 });
    await expect.poll(() => JSON.stringify(herdr.run(["pane", "list"]))).not.toContain(`"pane_id":"${sourcePane}"`);
    // OpenCode's database and its write-ahead log: a write lands in either.
    const digest = () => {
      const hash = crypto.createHash("sha256").update(fs.readFileSync(database));
      if (fs.existsSync(`${database}-wal`)) hash.update(fs.readFileSync(`${database}-wal`));
      return hash.digest("hex");
    };
    const prior = digest();
    await sleeping.getByRole("button", { name: "Wake agent" }).click();
    await expect.poll(launches, { timeout: 30_000 }).toContainEqual(["-s", OPENCODE_ID]);
    type Listed = { result: { agents: { pane_id: string; agent: string }[] } };
    let fresh = "";
    await expect.poll(() => {
      fresh = (herdr.run(["agent", "list"]) as Listed).result.agents.find(agent => agent.agent === "opencode" && agent.pane_id !== sourcePane)?.pane_id ?? "";
      return fresh;
    }, { timeout: 30_000 }).not.toBe("");
    await expect(sleeping).toHaveCount(0, { timeout: 30_000 });
    await expect(page.locator(`nav[data-sidebar] [data-pane="${fresh}"]`)).toContainText(OPENCODE_TITLE);
    expect(digest()).toBe(prior);
    await screenshot(page, "opencode-exact-wake");
  } finally {
    daemon?.stop();
    herdr.stop();
  }
});
