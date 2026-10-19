import { describe, expect, it } from "vitest";
import { landingDevices, landingMemory, landingSidebar } from "./landingSceneData";

// The site's shots are English: a row the frozen sidebar fixture gains that this
// scene does not reword would reach a public image, so it fails here first.
const HANGUL = /[ㄱ-힝]/;

describe("the landing scene's example work", () => {
  it("words every agent row, project and branch in English", () => {
    const { rest, agents } = landingSidebar(Date.now(), ["a1"], 380);
    const words = JSON.stringify([agents.map((row) => [row.identity_label, row.state.line?.text]), rest.navigator?.workspaces?.map((row) => [row.label, row.checkouts.map((checkout) => [checkout.label, checkout.purpose?.text])]), rest.navigator?.sleeping_sessions?.map((row) => row.identity_label)]);
    expect(words).not.toMatch(HANGUL);
  });

  it("words every Memory card and question in English", () => {
    const memory = landingMemory(Date.now(), "hide");
    expect(JSON.stringify([memory.rows.map((row) => row.title), memory.questions])).not.toMatch(HANGUL);
  });

  it("names the core's machine, a laptop and a server", () => {
    const { rest, mobile } = landingDevices(Date.now(), 340);
    expect(rest.navigator?.devices?.map((device) => device.label)).toEqual(expect.arrayContaining(["This Mac", "MacBook", "workbox"]));
    expect(mobile.phones).toHaveLength(1);
  });
});
