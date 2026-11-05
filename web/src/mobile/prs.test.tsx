// @vitest-environment jsdom
// The phone's PR tab and pull request page (docs/UI_BEHAVIOR.md, The phone
// app; PRD mobile-companion-ui B28, B37-B45, D-12, D-13): the open pull
// requests in four groups, a page with the card the core read, Merge with
// the desktop card's blocking reasons and one confirmation, the same press
// reusing its request id, the hand-over with the prompt the desktop card
// builds, and what stays when the Mac cannot be reached.
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { HEAD, card, keyOf, pr, project, standing } from "./prFixture";
import type { PhonePr, PhonePrFeedback, ServerFrame } from "./protocol";

vi.hoisted(() => {
  HTMLCanvasElement.prototype.getContext = () => null;
});

class FakeSocket {
  static CONNECTING = 0;
  static OPEN = 1;
  static instances: FakeSocket[] = [];
  readyState = 1;
  sent: Record<string, unknown>[] = [];
  onopen: (() => void) | null = null;
  onmessage: ((event: { data: string }) => void) | null = null;
  onclose: (() => void) | null = null;
  onerror: (() => void) | null = null;
  constructor() {
    FakeSocket.instances.push(this);
  }
  send(data: string) {
    this.sent.push(JSON.parse(data) as Record<string, unknown>);
  }
  close() {}
}

const CATALOG: ServerFrame = {
  type: "start_catalog",
  targets: [],
  core_platform: null,
  kinds: [{ id: "claude", models: ["opus", "sonnet"] }, { id: "codex", models: ["gpt"] }],
  remembered: { kind: "codex", models: { codex: "gpt" } },
};

/** A phone paired and connected to a fake hided that has listed the given pull requests. */
async function connect(items: PhonePr[], feedback: PhonePrFeedback | null = null) {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("WebSocket", FakeSocket);
  FakeSocket.instances = [];
  vi.resetModules();
  const { App } = await import("./App");
  const connection = await import("./connection");
  const store = await import("./store");
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  store.patch({ screen: "pair", pair: { v: 1, endpoint: "https://mac.example.ts.net", code: "code" } });
  connection.pairNow();
  const socket = FakeSocket.instances[0]!;
  const serve = (frame: ServerFrame) => act(async () => socket.onmessage?.({ data: JSON.stringify(frame) }));
  await act(async () => socket.onopen?.());
  await serve({ type: "hello", mac_name: "Mac", phone_id: "phone-a", name: null, vapid_public_key: "key", notifications: "off" });
  await serve({ type: "agents", groups: [], devices: [], interface_language: "en", color_theme: null });
  const prs = (next: PhonePr[], reading = project(), now: PhonePrFeedback | null = feedback) => serve({ type: "prs", projects: [reading], items: next, feedback: now });
  await prs(items);
  await act(async () => root.render(<App />));
  // A sheet draws into the document, not into the page's container.
  const q = (selector: string) => document.body.querySelector<HTMLElement>(selector);
  const click = (selector: string) => act(async () => q(selector)!.click());
  const sent = (type: string) => socket.sent.filter((message) => message.type === type);
  const lose = () => act(async () => socket.onclose?.());
  return { container, socket, serve, prs, q, click, sent, lose, connection, store, unmount: () => act(async () => root.unmount()).then(() => container.remove()) };
}

// The first import of the app transforms its whole module graph, seconds on a busy machine; that cost is paid in this hook, with the
// hook's own budget, so the first test's budget is for what it asserts.
beforeAll(async () => {
  vi.stubGlobal("WebSocket", FakeSocket);
  await import("./App");
  vi.unstubAllGlobals();
});

afterEach(() => {
  vi.unstubAllGlobals();
});

const typeInto = (input: HTMLTextAreaElement, value: string) =>
  act(async () => {
    Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!.call(input, value);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });

it("lists the open pull requests in four groups with the desktop's line, no badge on the tab, and leaves a merged one out", async () => {
  const items = [
    pr(2090, { title: "fix: keep paths on the wire" }),
    standing(2091, "ci", { group: "waiting", state: "pending", reason: { kind: "ci", count: 2 } }),
    standing(2088, "failed", { group: "blocked", state: "failed", reason: { kind: "failed", name: "web-e2e", count: 1 } }),
    standing(2079, "waiting_review", { group: "waiting", state: "pending", reason: { kind: "waiting_review" } }),
    standing(2050, "merged", { group: "merged", state: "merged", badge: "merged", reason: { kind: "merged" } }),
  ];
  const { container, click, unmount } = await connect(items);
  try {
    const tab = container.querySelector('[data-phone-tab="prs"]')!;
    expect(tab.hasAttribute("data-phone-tab-count")).toBe(false);
    await click('[data-phone-tab="prs"]');

    expect([...container.querySelectorAll("[data-phone-pr-group]")].map((group) => [group.getAttribute("data-phone-pr-group"), group.getAttribute("data-phone-pr-group-count")])).toEqual([
      ["ready", "1"], ["running", "1"], ["failed", "1"], ["review", "1"],
    ]);
    expect(container.querySelector("[data-phone-prs-open]")!.textContent).toBe("with-hide/hide · 4 open");
    expect(container.querySelector(`[data-phone-pr="${keyOf(2050)}"]`)).toBeNull();
    const failed = container.querySelector(`[data-phone-pr="${keyOf(2088)}"]`)!;
    expect(failed.getAttribute("data-phone-pr-reason")).toBe("failed");
    expect(failed.textContent).toContain("web-e2e");
    expect(container.querySelector(`[data-phone-pr="${keyOf(2090)}"] [data-phone-pr-line]`)!.textContent).toContain("Merge");
  } finally {
    await unmount();
  }
});

it("says there are no open pull requests, keeps the last list dimmed under one line when GitHub cannot be read, and names a gh that is not signed in", async () => {
  const { container, click, prs, unmount } = await connect([]);
  try {
    await click('[data-phone-tab="prs"]');
    expect(container.querySelector('[data-phone-prs-state="empty"]')).not.toBeNull();

    const stale = project({ github: [{ failure_category: "network_or_rate_limit", available: false, loading: false, stale: true, last_success_at_unix_ms: 1_000 }] });
    await prs([pr(1)], stale);
    expect(container.querySelector('[data-phone-prs-state="stale"]')).not.toBeNull();
    expect(container.querySelector(`[data-phone-pr="${keyOf(1)}"]`)).not.toBeNull();

    const signedOut = project({ github: [{ failure_category: "not_logged_in", available: false, loading: false, stale: false, last_success_at_unix_ms: null }] });
    await prs([], signedOut);
    expect(container.querySelector('[data-phone-prs-state="blocked"]')!.getAttribute("data-phone-prs-gh")).toBe("not_logged_in");
  } finally {
    await unmount();
  }
});

it("opens a page that reads the card, shows its files and lines and GitHub's address, and blocks Merge with the desktop's reason", async () => {
  const blocked = pr(7, { checks: "pending", state: "pending", work: { ...pr(7).work!, merge_state: "blocked", pending_checks: 2, check_counts: null } });
  const { container, click, sent, q, unmount } = await connect([pr(1), blocked]);
  try {
    await click('[data-phone-tab="prs"]');
    await click(`[data-phone-pr="${keyOf(1)}"]`);
    expect(sent("read_pr")).toHaveLength(1);
    expect(sent("read_pr")[0]).toMatchObject({ type: "read_pr", pr: "p1#1" });
    expect(container.querySelector("[data-phone-tabs]")).toBeNull();
    expect(q("[data-phone-pr-github]")!.getAttribute("href")).toBe("https://github.com/with-hide/hide/pull/1");
    expect(q("[data-phone-pr-branch]")!.textContent).toContain("feat/change-1");
    expect(q('[data-phone-pr-file="hided/src/mobile/wire.rs"] [data-phone-pr-lines]')!.getAttribute("data-phone-pr-lines")).toBe("+71 -8");
    expect(q("[data-phone-pr-files]")!.querySelector("[data-phone-pr-lines]")!.getAttribute("data-phone-pr-lines")).toBe("+77 -10");
    expect(q('[data-phone-pr-reviewer="tansfil"]')!.getAttribute("data-phone-pr-reviewer-state")).toBe("approved");
    expect((q("[data-phone-pr-merge]") as HTMLButtonElement).disabled).toBe(false);

    await click('[data-phone-back="prs"]');
    await click(`[data-phone-pr="${keyOf(7)}"]`);
    expect((q("[data-phone-pr-merge]") as HTMLButtonElement).disabled).toBe(true);
    expect(q("[data-phone-pr-merge-why]")!.getAttribute("data-phone-pr-merge-why")).toBe("ci");
  } finally {
    await unmount();
  }
});

it("asks once with nothing preselected, sends one merge for the head and method it showed, and the same press again reuses its request id", async () => {
  const { click, sent, q, serve, unmount } = await connect([pr(1)]);
  try {
    await click('[data-phone-tab="prs"]');
    await click(`[data-phone-pr="${keyOf(1)}"]`);
    await click("[data-phone-pr-merge]");
    const sheet = q("[data-phone-pr-confirm]")!;
    expect(sheet.getAttribute("data-phone-pr-confirm")).toBe("merge");
    expect(sheet.textContent).toContain("Merge PR #1 into main?");
    // Neither button holds the keyboard, and nothing was sent by opening the question.
    expect(sheet.contains(document.activeElement) && document.activeElement !== sheet).toBe(false);
    expect(sent("merge_pr")).toHaveLength(0);

    await click("[data-phone-pr-confirm-no]");
    expect(q("[data-phone-pr-confirm]")).toBeNull();
    expect(sent("merge_pr")).toHaveLength(0);

    await click("[data-phone-pr-merge]");
    await click("[data-phone-pr-confirm-yes]");
    expect(sent("merge_pr")).toHaveLength(1);
    const first = sent("merge_pr")[0]!;
    expect(first).toMatchObject({ type: "merge_pr", pr: "p1#1", head: HEAD, method: "squash" });
    // The button shows the merge going out and cannot be pressed again.
    expect((q("[data-phone-pr-merge]") as HTMLButtonElement).disabled).toBe(true);

    // hided could not say whether it sent: the press goes again under the same id.
    await serve({ type: "pr_result", request_id: first.request_id as string, action: "merge_pr", pr: "p1#1", ok: false, reason: "in_flight" });
    expect(q("[data-phone-pr-notice]")).not.toBeNull();
    await click("[data-phone-pr-merge]");
    await click("[data-phone-pr-confirm-yes]");
    expect(sent("merge_pr")).toHaveLength(2);
    expect(sent("merge_pr")[1]!.request_id).toBe(first.request_id);
  } finally {
    await unmount();
  }
});

it("closes the confirmation without sending when the head moved while it was open, and says so when hided refuses a head the list moved past", async () => {
  const { click, sent, q, serve, prs, unmount } = await connect([pr(1)]);
  try {
    await click('[data-phone-tab="prs"]');
    await click(`[data-phone-pr="${keyOf(1)}"]`);
    await click("[data-phone-pr-merge]");
    expect(q("[data-phone-pr-confirm]")).not.toBeNull();
    await prs([pr(1, { card: card({ head: "b".repeat(40) }) })]);
    expect(q("[data-phone-pr-confirm]")).toBeNull();
    expect(sent("merge_pr")).toHaveLength(0);

    await click("[data-phone-pr-merge]");
    await click("[data-phone-pr-confirm-yes]");
    const request = sent("merge_pr")[0]!;
    expect(request.head).toBe("b".repeat(40));
    await serve({ type: "pr_result", request_id: request.request_id as string, action: "merge_pr", pr: "p1#1", ok: false, reason: "head_moved" });
    expect(q("[data-phone-pr-notice]")!.textContent).toContain("changed in the meantime");
    // A refusal that sent nothing is a new press: a new request id.
    await click("[data-phone-pr-merge]");
    await click("[data-phone-pr-confirm-yes]");
    expect(sent("merge_pr")[1]!.request_id).not.toBe(request.request_id);
  } finally {
    await unmount();
  }
});

it("words the merge queue where the repository has one, and sends no method for it", async () => {
  const queued = pr(1, { card: card({ merge: { methods: ["squash"], queue: true, can_merge: true } }) });
  const { click, sent, q, unmount } = await connect([queued]);
  try {
    await click('[data-phone-tab="prs"]');
    await click(`[data-phone-pr="${keyOf(1)}"]`);
    expect(q("[data-phone-pr-merge]")!.getAttribute("data-phone-pr-merge")).toBe("queue");
    expect(q("[data-phone-pr-merge]")!.textContent).toBe("Add to merge queue");
    await click("[data-phone-pr-merge]");
    expect(q("[data-phone-pr-confirm]")!.textContent).toContain("Add PR #1 to the main merge queue?");
    await click("[data-phone-pr-confirm-yes]");
    expect(sent("merge_pr")[0]).not.toHaveProperty("method");
  } finally {
    await unmount();
  }
});

it("shows what a refused or finished merge did in the desktop's words, and reads the card again on Look again", async () => {
  const refused = pr(1, { card: card({ write: { action: "merge", head: HEAD, method: "squash", phase: "failed", reason: "not_mergeable" } }) });
  const { click, sent, q, prs, unmount } = await connect([refused]);
  try {
    await click('[data-phone-tab="prs"]');
    await click(`[data-phone-pr="${keyOf(1)}"]`);
    expect(q("[data-phone-pr-write]")!.getAttribute("data-phone-pr-write-reason")).toBe("not_mergeable");
    // GitHub's refusal would only repeat, so Merge stays off until the card is read again.
    expect((q("[data-phone-pr-merge]") as HTMLButtonElement).disabled).toBe(true);
    await click("[data-phone-pr-look-again]");
    expect(sent("read_pr").at(-1)).toMatchObject({ pr: "p1#1", look_again: true });

    // Into a merge queue the pull request stays open and says it is queued; merged, its state changes and the buttons go.
    const done = card({ merge: { methods: ["squash"], queue: true, can_merge: true }, write: { action: "merge", head: HEAD, method: null, phase: "done", reason: null } });
    await prs([pr(1, { card: done, work: { ...pr(1).work!, queue: { state: "at", position: 1 } } })]);
    expect(q("[data-phone-pr-write]")!.textContent).toBe("Added to the merge queue");
    expect((q("[data-phone-pr-merge]") as HTMLButtonElement).disabled).toBe(true);
    await prs([pr(1, { state: "merged", badge: "merged", card: card({ write: { action: "merge", head: HEAD, method: "squash", phase: "done", reason: null } }) })]);
    expect(q("[data-phone-pr-pill]")!.getAttribute("data-phone-pr-pill")).toBe("merged");
    expect(q("[data-phone-pr-merge]")).toBeNull();
    expect(q("[data-phone-pr-delegate]")).toBeNull();
  } finally {
    await unmount();
  }
});

it("hands a pull request to an agent with the desktop card's prompt, the remembered kind and model, and one request", async () => {
  const feedback: PhonePrFeedback = {
    request_id: "",
    pr_number: 1,
    phase: "ready",
    failed_checks: [{ name: "web-e2e", url: "https://ci.example/1" }],
    change_requests: [{ author: "tansfil", body: "Rename it" }],
    failure: null,
  };
  const { click, sent, q, serve, prs, unmount } = await connect([pr(1, { checkout: false })]);
  try {
    await click('[data-phone-tab="prs"]');
    await click(`[data-phone-pr="${keyOf(1)}"]`);
    await click("[data-phone-pr-delegate]");
    expect(sent("start_sheet").at(-1)).toMatchObject({ open: true });
    const read = sent("read_pr_feedback")[0]!;
    expect(read).toMatchObject({ pr: "p1#1" });
    expect(q("[data-phone-pr-delegate-new-worktree]")).not.toBeNull();
    await serve(CATALOG);
    await prs([pr(1, { checkout: false })], project(), { ...feedback, request_id: read.request_id as string });

    // The prompt is the desktop card's (`delegatePrompt`), and the kind and model are what the desktop remembers.
    const prompt = q("[data-phone-pr-delegate-prompt]") as HTMLTextAreaElement;
    expect(prompt.value).toContain("Fix the failed CI and change requests of PR #1 (feat/change-1): Change 1");
    expect(prompt.value).toContain("- web-e2e https://ci.example/1");
    expect(prompt.value).toContain("- tansfil: Rename it");
    expect((q("[data-phone-pr-delegate-kind]") as HTMLSelectElement).value).toBe("codex");
    expect((q("[data-phone-pr-delegate-model]") as HTMLSelectElement).value).toBe("gpt");

    await typeInto(prompt, `${prompt.value}\nAlso update the docs.`);
    await click("[data-phone-pr-delegate-submit]");
    expect(sent("delegate_pr")).toHaveLength(1);
    const request = sent("delegate_pr")[0]!;
    expect(request).toMatchObject({ type: "delegate_pr", pr: "p1#1", kind: "codex", model: "gpt" });
    expect(request.prompt).toContain("Also update the docs.");

    // A refusal keeps what was edited; a start closes the sheet and says so on the page.
    await serve({ type: "start_result", request_id: request.request_id as string, ok: false, reason: "agent_start.unknown_model" });
    expect(q("[data-phone-pr-delegate-error]")).not.toBeNull();
    expect((q("[data-phone-pr-delegate-prompt]") as HTMLTextAreaElement).value).toContain("Also update the docs.");
    await click("[data-phone-pr-delegate-submit]");
    const again = sent("delegate_pr")[1]!;
    await serve({ type: "start_result", request_id: again.request_id as string, ok: true, reason: null, device_id: "local", pane_id: "p9" });
    expect(q("[data-phone-pr-delegate-sheet]")).toBeNull();
    expect(q("[data-phone-pr-delegated]")).not.toBeNull();
    expect(sent("start_sheet").at(-1)).toMatchObject({ open: false });
  } finally {
    await unmount();
  }
});

it("keeps the last list and page dimmed under the unreachable line, and turns Merge, hand-over and Look again off", async () => {
  const refused = pr(1, { card: card({ write: { action: "merge", head: HEAD, method: "squash", phase: "failed", reason: "unreachable" } }) });
  const { container, click, q, lose, unmount } = await connect([refused, pr(2)]);
  try {
    await click('[data-phone-tab="prs"]');
    await lose();
    expect(container.querySelector("[data-phone-unreachable]")).not.toBeNull();
    expect(container.querySelectorAll("[data-phone-pr]")).toHaveLength(2);
    await click(`[data-phone-pr="${keyOf(1)}"]`);
    expect((q("[data-phone-pr-merge]") as HTMLButtonElement).disabled).toBe(true);
    expect((q("[data-phone-pr-delegate]") as HTMLButtonElement).disabled).toBe(true);
    expect((q("[data-phone-pr-look-again]") as HTMLButtonElement).disabled).toBe(true);
  } finally {
    await unmount();
  }
});
