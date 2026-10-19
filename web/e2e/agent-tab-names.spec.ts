import { expect, test } from "@playwright/test";
import { labelAgent, paneShellTabName, startHerdr } from "./herdr-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { localEndpoint } from "./platform-fixture";
import { enterWorkspace, screenshot } from "./wire";

test("focused agent titles, inline rename, clear and reconnect", async ({ page }) => {
  test.setTimeout(120_000);
  const herdr = await startHerdr({ agents: false });
  let daemon: Daemon | null = null;
  try {
    daemon = await startHided(herdr, "tab-names");
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    const tab = page.locator(`[data-tab="${herdr.tab}"]`);
    await expect(tab).toBeVisible();
    await expect(tab).toContainText(paneShellTabName(herdr.shell));
    const [first, second] = herdr.panes;
    for (const [pane, name, title] of [[first, "one", "첫 번째 작업 진행"], [second, "two", "두 번째 작업 진행"]]) {
      herdr.run(["agent", "start", name!, "--kind", "claude", "--pane", pane!]);
      labelAgent(herdr, pane!, { task: title! });
    }
    await page.locator(`[data-pane-view="${first}"]`).click({ position: { x: 30, y: 60 } });
    await expect(tab).toContainText("첫 번째 작업 진행", { timeout: 15_000 });
    await expect(tab.locator("[data-tab-status]")).toHaveCount(1);
    await expect(tab.locator("img")).toHaveCount(0);
    // The logo stays on the pane's own header and names its product (agent-mark-quiet B4, B6).
    await expect(page.locator(`[data-pane-view="${first}"] header [aria-label="Claude Code"] [data-agent-mark="claude"]`)).toHaveCount(1);
    await page.locator(`[data-pane-view="${second}"]`).click({ position: { x: 30, y: 60 } });
    await expect(tab).toContainText("두 번째 작업 진행");
    await screenshot(page, "tab-names-automatic");
    const rename = async () => {
      await tab.click({ button: "right" });
      const menu = page.getByRole("menu");
      await expect(menu.getByRole("menuitem")).toHaveText([
        "New tab",
        "Move to new group…",
        "Move to group",
        "Rename…",
        /^Split/,
        "Copy name",
        "Close tab…",
      ]);
      await menu.getByRole("menuitem", { name: "Rename…", exact: true }).click();
      const input = page.getByRole("textbox", { name: "Tab name", exact: true });
      await expect(input).toBeFocused();
      return input;
    };
    let input = await rename();
    await expect(input).toHaveValue("두 번째 작업 진행");
    expect(await input.evaluate((node: HTMLInputElement) => [node.selectionStart, node.selectionEnd])).toEqual([0, "두 번째 작업 진행".length]);
    await input.fill("취소할 이름");
    await input.press("Escape");
    await expect(tab).toContainText("두 번째 작업 진행");
    input = await rename();
    await input.fill("고정 이름");
    await input.press("Enter");
    await expect(input).toHaveCount(0);
    await expect(tab).toContainText("고정 이름");
    await page.locator(`[data-pane-view="${first}"]`).click({ position: { x: 30, y: 60 } });
    await expect(tab).toContainText("고정 이름");
    await page.reload();
    await expect(tab).toContainText("고정 이름");
    await screenshot(page, "tab-names-custom");
    input = await rename();
    await input.fill("");
    await input.press("Enter");
    await expect(input).toHaveCount(0);
    await expect(tab).toContainText("첫 번째 작업 진행");
    input = await rename();
    await input.fill("blur 취소");
    await page.locator("[data-workspace-toolbar]").click();
    await expect(input).toHaveCount(0);
    await expect(tab).toContainText("첫 번째 작업 진행");
  } finally {
    daemon?.stop();
    herdr.stop();
  }
});

test("rename refusal and timeout preserve text for retry", async ({ page }) => {
  test.setTimeout(120_000);
  const net = await import("node:net");
  const path = await import("node:path");
  const herdr = await startHerdr();
  const socket = path.join(herdr.root, "rename-proxy.sock");
  const peers = new Set<import("node:net").Socket>();
  let failures = 0;
  const proxy = net.createServer((client) => {
    peers.add(client);
    client.on("close", () => peers.delete(client));
    client.on("error", () => client.destroy());
    let buffer = Buffer.alloc(0);
    const first = (chunk: Buffer) => {
      buffer = Buffer.concat([buffer, chunk]);
      const end = buffer.indexOf(10);
      if (end < 0) return;
      client.off("data", first);
      const request = JSON.parse(buffer.subarray(0, end).toString()) as { id: string; method: string };
      if (request.method === "tab.rename" && failures++ < 2) {
        if (failures === 1) client.end(`${JSON.stringify({ id: request.id, error: { code: "rename_refused", message: "fixture refusal" } })}\n`);
        // The second request receives no answer. The production client must
        // enforce its deadline and return the same retry state.
        return;
      }
      const upstream = net.connect(localEndpoint(herdr.socket), () => upstream.write(buffer));
      peers.add(upstream);
      upstream.on("close", () => peers.delete(upstream));
      upstream.on("error", () => client.destroy());
      client.on("close", () => upstream.destroy());
      client.pipe(upstream).pipe(client);
    };
    client.on("data", first);
  });
  proxy.maxConnections = 32;
  await new Promise<void>((resolve, reject) => { proxy.once("error", reject); proxy.listen(localEndpoint(socket), resolve); });
  let daemon: Daemon | null = null;
  try {
    daemon = await startHided({ ...herdr, socket }, "tab-rename-failure");
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    const tab = page.locator(`[data-tab="${herdr.tab}"]`);
    // The refusal preserves the established name, after automatic naming.
    await expect(tab).toContainText("Agent one");
    const before = await tab.getAttribute("aria-label");
    await tab.click({ button: "right" });
    await page.getByRole("menuitem", { name: "Rename…", exact: true }).click();
    const input = page.getByRole("textbox", { name: "Tab name", exact: true });
    await expect(input).toBeFocused();
    await input.fill("다시 저장할 이름");
    await input.press("Enter");
    const caption = page.getByText("The name could not be saved · Retry", { exact: true }).first();
    await expect(caption).toBeVisible({ timeout: 15_000 });
    await expect(input).toHaveValue("다시 저장할 이름");
    await expect(input).toBeFocused();
    await expect(tab).toHaveAttribute("aria-label", before!);
    await screenshot(page, "tab-names-retry");
    await input.press("Enter");
    await expect(input).toHaveAttribute("readonly", "");
    await expect(caption).toBeVisible({ timeout: 15_000 });
    await expect(input).not.toHaveAttribute("readonly", "");
    await expect(input).toHaveValue("다시 저장할 이름");
    await input.press("Enter");
    await expect(input).toHaveCount(0);
    await expect(tab).toContainText("다시 저장할 이름");
  } finally {
    daemon?.stop();
    for (const peer of peers) peer.destroy();
    await new Promise<void>((resolve) => proxy.close(() => resolve()));
    herdr.stop();
  }
});
