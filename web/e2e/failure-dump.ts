// What a spec reads at the moment one of its waits gives up, kept with the failure.
// A wait that times out says only what it saw last; for a flake that does not reproduce
// the other side of the race (what Herdr holds, what the page drew) is gone by the time
// anyone looks, so the spec records both when it throws. The step's own error is rethrown
// unchanged: this explains a failure and never absorbs one.
import { test, type Page } from "@playwright/test";
import type { HerdrFixture } from "./herdr-fixture";

/** Runs `step`; if it throws, logs and attaches `read()`'s answer under `label`, then rethrows the same error. */
export async function dumpOnFailure<T>(label: string, read: () => unknown | Promise<unknown>, step: () => Promise<T>): Promise<T> {
  try {
    return await step();
  } catch (error) {
    let dump: string;
    try { dump = JSON.stringify(await read()); } catch (reading) { dump = JSON.stringify({ unavailable: String(reading) }); }
    console.log(`[failure dump] ${label}: ${dump}`);
    try { await test.info().attach(label, { body: dump, contentType: "application/json" }); } catch { /* no test is running: the log line above stands */ }
    throw error;
  }
}

/** Every Herdr workspace with the ids of its tabs, as the fixture's Herdr answers now. */
export function herdrTabs(herdr: HerdrFixture): { workspace_id: string; label: string; tabs: string[] }[] {
  const listed = herdr.run(["workspace", "list"]) as { result: { workspaces: { workspace_id: string; label: string }[] } };
  return listed.result.workspaces.map(({ workspace_id, label }) => {
    const tabs = herdr.run(["tab", "list", "--workspace", workspace_id]) as { result: { tabs: { tab_id: string }[] } };
    return { workspace_id, label, tabs: tabs.result.tabs.map((tab) => tab.tab_id) };
  });
}

/** The agent tab strip as the page drew it: each tab's id, label and whether it is selected. */
export function pageTabs(page: Page): Promise<unknown> {
  return page.locator("[data-agent-tab-bar] [role=tab]").evaluateAll((tabs) =>
    tabs.map((tab) => ({ tab: tab.getAttribute("data-tab"), selected: tab.getAttribute("aria-selected"), text: (tab.textContent ?? "").trim() })),
  );
}

/** Both sides of "the strip shows what Herdr has": what Herdr lists and what the page drew. */
export async function tabsBothSides(herdr: HerdrFixture, page: Page): Promise<unknown> {
  return { herdr: herdrTabs(herdr), page: await pageTabs(page) };
}
