// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { PhoneAgent } from "./protocol";
import { listedAgents } from "./protocol";
import { blockNetwork, type NetworkGuard } from "./demoNetworkGuard";
import { HOSTS_KEY } from "./hosts";

// The sample host as a person meets it: the hosted phone opened with `?demo`, read through the same store every screen reads. Only
// the browser's WebSocket, fetch and clock are substituted, and the first two are substituted to fail, because the demo may use neither.
let network: NetworkGuard;

beforeEach(() => {
  vi.useFakeTimers();
  network = blockNetwork((name, value) => vi.stubGlobal(name, value), () => vi.unstubAllGlobals());
  window.history.replaceState(null, "", "/p/5/?demo=1");
  localStorage.clear();
  sessionStorage.clear();
  vi.resetModules();
});

afterEach(async () => {
  const { stop } = await import("./connection");
  stop();
  vi.useRealTimers();
  network.release();
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
    expect(s.groups?.map((group) => group.group)).toEqual(["needs_you", "working", "done", "revisit", "seen"]);
    const parent = s.groups![0]!.rows[0]!;
    expect(parent.kind === "agent" && parent.children?.length).toBe(2);
    expect(s.groups!.find((group) => group.group === "seen")!.more?.length).toBeGreaterThan(0);
    expect(s.missions!.length).toBeGreaterThan(1);
    expect(s.missions!.some((mission) => mission.question !== null)).toBe(true);
    expect(s.prs!.items.length).toBeGreaterThan(2);
    expect(document.documentElement.dataset.phoneDemo).toBe("true");
    expect(network.calls).toEqual([]);
    expect(localStorage.length).toBe(0);
  });

  it("names no agent product on any sample row, conversation head or mission step", async () => {
    // A store screenshot of the demo carries no other company's name or logo: a kind Hide has an adapter for draws that agent's mark.
    const { agentAdapter } = await import("../agentAdapters");
    const { state } = await demoPhone();
    const kinds: Array<string | null> = [];
    for (const group of state().groups!) for (const row of [...group.rows, ...(group.more ?? [])]) if (row.kind === "agent") kinds.push(row.agent_kind, ...(row.children ?? []).map((child) => child.agent_kind));
    for (const mission of state().missions!) kinds.push(mission.conductor.agent_kind, ...mission.nodes.map((node) => node.worker_agent));
    expect(kinds.length).toBeGreaterThan(10);
    expect(kinds.filter((kind) => kind !== null && agentAdapter(kind) !== undefined)).toEqual([]);
    // VoiceOver reads the kind: it is a word of the phone's language, not a code.
    const { translate } = await import("../i18n/translator");
    expect(new Set(kinds.filter((kind) => kind !== null))).toEqual(new Set([translate("mobile.demo.kind")]));
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

  it("refuses a start past its cap with the sheet's own failure line, and keeps what it had", async () => {
    const { connection, settle, state } = await demoPhone();
    for (let at = 0; at < 10; at += 1) {
      connection.openStartSheet();
      await settle();
      connection.editStartSheet({ text: `Task ${at}` });
      expect(connection.submitStart()).toBe(true);
      await settle();
      expect(state().startSheet.error).toBeNull();
    }
    connection.openStartSheet();
    await settle();
    connection.editStartSheet({ text: "One too many" });
    connection.submitStart();
    await settle();
    expect(state().startSheet.error).toEqual({ key: "mobile.start.failed" });
    expect(state().startSheet.text).toBe("One too many");
    expect(titles(listedAgents(state().groups!))).not.toContain("One too many");
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
    expect(network.calls).toEqual([]);
  });

  it("leaves a real paired host's list as it was after a visit and leaving", async () => {
    const hosts = JSON.stringify([{ room: "a".repeat(43), name: "My Mac", noiseKey: "b".repeat(43), credential: "kept", protocol: 5, lastSeen: 1000 }]);
    localStorage.setItem(HOSTS_KEY, hosts);
    const assign = vi.fn();
    vi.stubGlobal("location", { ...window.location, pathname: "/p/5/", search: "?demo=1", hash: "", assign });
    const { connection, settle, state } = await demoPhone();
    connection.openStartSheet();
    await settle();
    connection.editStartSheet({ text: "A task during the visit" });
    connection.submitStart();
    await settle();
    expect(state().macName).toBe("Demo Mac");
    connection.leaveDemo();
    expect(assign).toHaveBeenCalledWith("/?list=1");
    expect(localStorage.getItem(HOSTS_KEY)).toBe(hosts);
    expect(localStorage.length).toBe(1);
    expect(network.calls).toEqual([]);
  });

  it("is not entered from a page that is not the hosted phone", async () => {
    window.history.replaceState(null, "", "/m/?demo=1");
    const { demoRequested } = await import("./demoSocket");
    expect(demoRequested()).toBe(false);
  });
});
