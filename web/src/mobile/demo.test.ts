// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { PhoneAgent } from "./protocol";
import { listedAgents } from "./protocol";

// The sample host as a person meets it: the hosted phone opened with `?demo`, read through the same store every screen reads. Only
// the browser's WebSocket, fetch and clock are substituted, and the first two are substituted to fail, because the demo may use neither.
const sockets: string[] = [];
const requests: string[] = [];

beforeEach(() => {
  sockets.length = 0;
  requests.length = 0;
  vi.useFakeTimers();
  vi.stubGlobal("WebSocket", Object.assign(function Refused(url: string) { sockets.push(url); throw new Error("no socket in a demo"); }, { CONNECTING: 0, OPEN: 1, CLOSING: 2, CLOSED: 3 }));
  vi.stubGlobal("fetch", (url: string) => { requests.push(String(url)); return Promise.reject(new Error("no request in a demo")); });
  window.history.replaceState(null, "", "/p/5/?demo=1");
  localStorage.clear();
  sessionStorage.clear();
  vi.resetModules();
});

afterEach(async () => {
  const { stop } = await import("./connection");
  stop();
  vi.useRealTimers();
  vi.unstubAllGlobals();
  localStorage.clear();
  window.history.replaceState(null, "", "/");
  delete document.documentElement.dataset.phoneDemo;
});

/** A fresh copy of the modules (the socket and the store are module state), started on the demo address and let to answer. */
async function demoPhone() {
  const connection = await import("./connection");
  const { usePhone } = await import("./store");
  connection.start();
  const settle = (ms = 400) => vi.advanceTimersByTimeAsync(ms);
  await settle();
  return { connection, usePhone, settle, state: usePhone.getState };
}

const titles = (agents: PhoneAgent[]) => agents.map((agent) => agent.title);

describe("the sample host", () => {
  it("fills the list, missions and pull requests with no socket, request or storage", async () => {
    const { state } = await demoPhone();
    const s = state();
    expect(s).toMatchObject({ demo: true, screen: "app", connected: true, macName: "Demo Mac" });
    expect(s.groups?.map((group) => group.group)).toEqual(["needs_you", "working", "done", "seen"]);
    const parent = s.groups![0]!.rows[0]!;
    expect(parent.kind === "agent" && parent.children?.length).toBe(2);
    expect(s.groups!.find((group) => group.group === "seen")!.more?.length).toBeGreaterThan(0);
    expect(s.missions!.length).toBeGreaterThan(1);
    expect(s.missions!.some((mission) => mission.question !== null)).toBe(true);
    expect(s.prs!.items.length).toBeGreaterThan(2);
    expect(document.documentElement.dataset.phoneDemo).toBe("true");
    expect(sockets).toEqual([]);
    expect(requests).toEqual([]);
    expect(localStorage.length).toBe(0);
  });

  it("answers a reply with a sample answer, in the conversation it was sent from", async () => {
    const { connection, settle, state } = await demoPhone();
    connection.openDetail({ device_id: "local", pane_id: "a1" });
    await settle();
    const opening = state().detail!.conversation;
    expect(opening && opening !== "none" && opening.messages.map((message) => message.who)).toEqual(["you", "agent", "agent"]);
    expect(connection.sendReply("Apply all three")).toBe(true);
    await settle(2000);
    const after = state().detail!.conversation;
    if (!after || after === "none") throw new Error("conversation lost");
    expect(state().pendingInput).toBeNull();
    expect(state().inputError).toBeNull();
    expect(after.messages.at(-2)).toMatchObject({ who: "you", text: "Apply all three" });
    expect(after.messages.at(-1)).toMatchObject({ who: "agent" });
    expect(after.messages.at(-1)!.text).toMatch(/sample answer/i);
  });

  it("starts a sample agent from the start sheet and opens its detail", async () => {
    const { connection, settle, state } = await demoPhone();
    connection.openStartSheet();
    await settle();
    expect(state().startCatalog?.targets).toHaveLength(1);
    connection.editStartSheet({ text: "Tidy the changelog" });
    expect(connection.submitStart()).toBe(true);
    await settle();
    const listed = titles(listedAgents(state().groups!));
    expect(listed).toContain("Tidy the changelog");
    expect(state().startSheet.open).toBe(false);
    expect(state().detail?.key.pane_id).toMatch(/^s\d+$/);
  });

  it("draws one HTML result and one image through the phone's own look transfer", async () => {
    const { connection, settle, state } = await demoPhone();
    connection.openDetail({ device_id: "local", pane_id: "a2" }, "looks");
    await settle();
    const rows = state().detail!.looks!.rows;
    expect(rows.map((row) => row.kind).sort()).toEqual(["html", "image"]);
    connection.openLook(rows.find((row) => row.kind === "html")!);
    await settle();
    expect(state().detail!.look!.content).toMatchObject({ state: "html" });
    connection.closeLook();
    connection.openLook(rows.find((row) => row.kind === "image")!);
    await settle();
    expect(state().detail!.look!.content).toMatchObject({ state: "image" });
  });

  it("answers a mission's decision, merges a pull request and hands one to an agent", async () => {
    const { connection, settle, state } = await demoPhone();
    const mission = state().missions!.find((candidate) => candidate.question)!;
    expect(connection.answerMission(mission, { option: 1 }, "")).toBe(true);
    await settle();
    expect(state().missions!.find((candidate) => candidate.id === mission.id)!.question).toBeNull();

    connection.openPrPage("p1#2090");
    await settle();
    connection.askMerge();
    expect(connection.confirmMerge()).toBe(true);
    await settle(1000);
    expect(state().prs!.items.map((pr) => pr.key)).not.toContain("p1#2090");

    connection.openDelegate("p1#2088");
    await settle();
    expect(state().prs!.feedback?.pr_number).toBe(2088);
    connection.editDelegate({ prompt: "Fix the failing check" });
    await settle();
    expect(connection.submitDelegate()).toBe(true);
    await settle();
    expect(state().prDelegate).toBeNull();
    expect(state().prDelegated).toBe("p1#2088");
    expect(listedAgents(state().groups!).some((agent) => agent.pane_id.startsWith("s"))).toBe(true);
  });

  it("words its content again when the phone's language changes", async () => {
    const { settle, state } = await demoPhone();
    const { clientI18n } = await import("../i18n/translator");
    expect(state().groups![0]!.rows[0]).toMatchObject({ title: "Address review comments" });
    await clientI18n.changeLanguage("ko");
    await settle();
    expect(state().macName).toBe("체험용 Mac");
    expect(state().groups![0]!.rows[0]).toMatchObject({ title: "리뷰 코멘트 반영하기" });
    await clientI18n.changeLanguage("en");
  });

  it("leaves nothing in storage after a whole visit", async () => {
    const { connection, settle } = await demoPhone();
    connection.openDetail({ device_id: "local", pane_id: "a1" });
    await settle();
    connection.sendReply("hello");
    await settle(2000);
    connection.openStartSheet();
    await settle();
    connection.editStartSheet({ text: "Another task" });
    connection.submitStart();
    await settle();
    expect(localStorage.length).toBe(0);
    expect(sessionStorage.length).toBe(0);
    expect(sockets).toEqual([]);
    expect(requests).toEqual([]);
  });

  it("is not entered from a page that is not the hosted phone", async () => {
    window.history.replaceState(null, "", "/m/?demo=1");
    const { demoRequested } = await import("./demoSocket");
    expect(demoRequested()).toBe(false);
  });
});
