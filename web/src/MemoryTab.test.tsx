// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterAll, afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createActions } from "./actions";
import { TooltipProvider } from "./components/ui/tooltip";
import { ToolRail } from "./Tools";
import { MemorySettings, MemoryTab, useMemoryProject } from "./MemoryTab";
import { clientI18n } from "./i18n/translator";
import type { MemoryRow, ProjectMemory, SnapshotRest } from "./snapshot";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";
import { emptyScope } from "../test/legacyAgentScope";
import type { DispatchFn } from "./ws";

const browserCanvas = vi.hoisted(() => {
  const original = HTMLCanvasElement.prototype.getContext;
  HTMLCanvasElement.prototype.getContext = () => null;
  return { restore: () => { HTMLCanvasElement.prototype.getContext = original; } };
});
afterAll(() => browserCanvas.restore());
// jsdom lays nothing out; the card shown in full asks to be scrolled to.
Element.prototype.scrollIntoView = () => {};

// The front checkout `c` belongs to Project `p` (design v7, PRD
// project-memory-v2 B19-B32).
const rest = {
  navigator: {
    focused_device_id: "local", focused_checkout_id: "c", focused_workspace_id: "p",
    devices: [{ id: "local", label: "This Mac", kind: "local", state: "local" }],
    workspaces: [{ id: "p", label: "herdr-ide", device_id: "local", agent_scope: emptyScope(), checkouts: [{ id: "c", workspace_id: "p", path: "/p", label: "main", tabs: [], active_tab_id: null, strip: [] }] }],
  },
} as unknown as SnapshotRest;
const row = (number: number, overrides: Partial<MemoryRow> = {}): MemoryRow => ({
  number, title: `card ${number}`, kind: "rule", human_source_count: 1, provided_session_count: 2,
  pull_count: 3, updated_at_unix_ms: Date.now(), verifying: false, promotion: null, ...overrides,
});
const memory = (overrides: Partial<ProjectMemory> = {}): ProjectMemory => ({
  workspace_id: "p", unavailable: null, loading: false, enabled: true, capacity_reached: false, evicted_cards: 0, evicted_through_unix_ms: null,
  rows: [row(3), row(7)], questions: [], analysis: "idle", consent_required: false, usable_cards: 2, checking_card: null, check_resume_at_unix_ms: null, forgotten: null, pending_action: null,
  expanded: null, search: null, ...overrides,
});

describe("the Memory tab", () => {
  /** Whether the socket takes what the tab sends. */
  let accepted = true;
  let container: HTMLDivElement;
  let root: Root;
  let saved: ReturnType<typeof useShellStore.getState>;
  let events: Parameters<DispatchFn>[0][];
  let savedLanguage: string;
  let savedUi: ReturnType<typeof useUiStore.getState>;

  beforeEach(() => {
    saved = useShellStore.getState();
    savedLanguage = clientI18n.language;
    savedUi = useUiStore.getState();
    vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
    events = [];
    vi.spyOn(window, "open").mockReturnValue(null);
    container = document.createElement("div");
    document.body.append(container);
    root = createRoot(container);
  });
  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
    useShellStore.setState(saved, true);
    await clientI18n.changeLanguage(savedLanguage);
    useUiStore.setState(savedUi, true);
    accepted = true;
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  /** The rail, which opens the front checkout's Project and wears Memory's dot, and the Memory tool, or the rail alone. */
  async function render(project: ProjectMemory | null, tool: "memory" | "explorer" = "memory") {
    const actions = createActions((event) => { events.push(event); return accepted; });
    await act(async () => {
      useShellStore.setState({ connection: "live", rest, memory: { project } });
      root.render(<TooltipProvider><ToolRail actions={actions} />{tool === "memory" ? <MemoryTab actions={actions} /> : null}</TooltipProvider>);
    });
  }
  /** Opens a question row's `⋯` menu, which a pointer press on its trigger does. */
  const openRowMenu = async (number: number) => {
    const trigger = document.body.querySelector<HTMLElement>(`[data-memory-row-menu='${number}']`);
    if (!trigger) throw new Error(`missing menu of ${number}`);
    await act(async () => { trigger.dispatchEvent(new MouseEvent("pointerdown", { bubbles: true, button: 0 })); });
  };
  const sent = (kind: string) => events.filter((event) => event.kind === kind).map((event) => event.payload);
  const click = async (selector: string) => {
    // A dialog renders in a portal beside the tab.
    const element = document.body.querySelector<HTMLElement>(selector);
    if (!element) throw new Error(`missing ${selector}`);
    await act(async () => element.click());
  };

  it("opens the front checkout's Project and reads until the core answers for that Project", async () => {
    await render(memory({ workspace_id: "another" }));
    expect(sent("memory_open")).toEqual([{ workspace_id: "p" }]);
    expect(container.querySelector("[data-memory]")).toBeNull();
    expect(container.textContent).toContain("Reading");

    // Another window named another Project; this one names its own again
    // when it comes forward.
    await act(async () => window.dispatchEvent(new Event("focus")));
    expect(sent("memory_open")).toHaveLength(2);
  });

  it("opens the front Project's Memory from the rail whichever tool shows", async () => {
    await render(memory({ questions: [{ kind: "failed", number: 7, title: "card 7", start_failure: null }] }), "explorer");
    expect(sent("memory_open")).toEqual([{ workspace_id: "p" }]);
  });

  it("counts the open documents on the rail while they show as well as while they are folded (PRD lane-two-columns B2)", async () => {
    const withViews = { ...rest, workspace_view: { ...(rest.workspace_view ?? {}), device_id: "local", path: "/p", open: true, documents: true, tool: "explorer", pinned: false, document_width: null, detail_width: null, explorer_width: null, panel_width: null, lane_called: 0, lane_calls: 0, lane_called_side: "view", layout: { root: { area: { id: "a1", active: null, displays: [] } }, active_area: "a1", display_count: 2, limits: { max_view_areas: 6, max_split_depth: 3, max_displays: 32 } } } } as unknown as SnapshotRest;
    const actions = createActions((event) => { events.push(event); return accepted; });
    for (const view of ["documents", "none"] as const) {
      await act(async () => {
        useShellStore.setState({ connection: "live", rest: withViews, memory: { project: null } });
        useUiStore.setState({ drawnLane: { mode: "overlay", view, tool: "explorer", stacked: false } });
        root.render(<TooltipProvider><ToolRail actions={actions} /></TooltipProvider>);
      });
      expect(container.querySelector('[data-lane-tool="documents"]')?.getAttribute("aria-pressed")).toBe(view === "documents" ? "true" : "false");
      expect(container.querySelector('[data-lane-tool="documents"] [data-lane-badge]')?.textContent).toBe("2");
    }
  });

  it("gives Settings the only Memory read slot and restores Tools on close", async () => {
    const actions = createActions((event) => { events.push(event); return true; });
    function SettingsReader() { useMemoryProject(actions, "other-project"); return null; }
    await render(memory());
    events.length = 0;
    await act(async () => {
      useUiStore.setState({ overlay: "project_settings" });
      root.render(<TooltipProvider><ToolRail actions={actions} /><MemoryTab actions={actions} /><SettingsReader /></TooltipProvider>);
    });
    expect(sent("memory_open")).toEqual([{ workspace_id: "other-project" }]);
    await act(async () => window.dispatchEvent(new Event("focus")));
    expect(sent("memory_open")).toEqual([{ workspace_id: "other-project" }, { workspace_id: "other-project" }]);
    events.length = 0;
    await act(async () => {
      useUiStore.getState().closeOverlay("project_settings");
      root.render(<TooltipProvider><ToolRail actions={actions} /><MemoryTab actions={actions} /></TooltipProvider>);
    });
    expect(sent("memory_open")).toEqual([{ workspace_id: "p" }]);
  });

  it("asks only whether a memory or its document is right, and leaves promotions to the 제안 tab (PRD hide-ai-suggestions D-02, B2)", async () => {
    const document = { authority: "card_newer" as const, document: "AGENTS.md", document_at_unix_ms: null, operator_at_unix_ms: 1 };
    await render(memory({
      rows: [row(3), row(7), row(9)],
      questions: [
        { kind: "promote", number: 3, title: "card 3", human_session_count: 3, pulled_session_count: 9 },
        { kind: "docs_disagree", number: 7, title: "card 7", document },
        { kind: "failed", number: 9, title: "card 9", start_failure: null },
      ],
    }));
    const asked = container.querySelector("[data-memory-questions]");
    expect([...asked!.querySelectorAll("[data-memory-question]")].map((element) => element.getAttribute("data-memory-question"))).toEqual(["docs_disagree"]);
    // The promoted and failed memories stay in the list; their rows are in 제안.
    expect([...container.querySelectorAll("[data-memory-cards] [data-memory-card]")].map((element) => element.getAttribute("data-memory-card"))).toEqual(["3", "9"]);
    // Checked from the start: the one button applies it, and `기억이 틀림` is in the row's menu.
    expect(container.querySelector<HTMLInputElement>("[data-memory-select='7']")?.getAttribute("aria-checked")).toBe("true");
    await click("[data-memory-open-batch]");
    await openRowMenu(7);
    await click("[data-memory-dismiss='7']");
    expect(sent("memory_action")).toEqual([
      { workspace_id: "p", action: "promote", number: null, numbers: [7] },
      { workspace_id: "p", action: "doc_right", number: 7 },
    ]);
    await render(memory({ questions: [{ kind: "promote", number: 3, title: "card 3", human_session_count: 3, pulled_session_count: 9 }] }));
    expect(container.querySelector("[data-memory-questions]")).toBeNull();
  });

  it("keeps a memory on its way into the documents in the list with its line until merged", async () => {
    const promotion = { state: "pr_open" as const, agent: null, pr_number: 1050, pr_url: "https://github.com/acme/one/pull/1050", target_path: "AGENTS.md", github_failure: null };
    await render(memory({ rows: [row(3, { promotion: { ...promotion, state: "running", pr_number: null } }), row(7, { promotion })] }));
    expect(container.querySelector("[data-memory-cards] [data-memory-running='3']")).not.toBeNull();
    // PR numbers stay plain digits.
    expect(container.querySelector("[data-memory-card='7']")?.textContent).toContain("1050");
    expect(container.querySelector("[data-memory-card='7']")?.textContent).not.toContain("1,050");
    await render(memory({ rows: [row(3), row(7, { promotion: { ...promotion, state: "merged" } })] }));
    expect(container.querySelector("[data-memory-card='7']")?.textContent).toContain("AGENTS.md");
  });

  it("checks every confirmed correction up to five, leaves an unanswered comparison unchecked, and applies the checked ones", async () => {
    const document = (authority: "card_newer" | "ask", at: number) => ({ authority, document: "docs/TESTING.md", document_at_unix_ms: null, operator_at_unix_ms: at });
    const confirmed = [1, 2, 3, 4, 5, 6].map((number) => ({ kind: "docs_disagree" as const, number, title: `card ${number}`, document: document("card_newer", number) }));
    await render(memory({ rows: [], questions: [...confirmed, { kind: "docs_disagree", number: 9, title: "card 9", document: document("ask", 99) }] }));
    const box = (number: number) => container.querySelector<HTMLButtonElement>(`[data-memory-select='${number}']`)!;
    // The five most recent are checked and the sixth cannot be added; the unanswered one cannot be checked at all.
    expect([6, 5, 4, 3, 2, 1].map((number) => box(number).getAttribute("aria-checked"))).toEqual(["true", "true", "true", "true", "true", "false"]);
    expect(box(1).disabled).toBe(true);
    expect(box(9).disabled).toBe(true);
    expect(container.querySelector("[data-memory-open-batch]")?.textContent).toBe("Apply to 5 documents");
    await click("[data-memory-select='6']");
    expect(box(1).disabled).toBe(false);
    await click("[data-memory-open-batch]");
    expect(sent("memory_action").at(-1)).toEqual({ workspace_id: "p", action: "promote", number: null, numbers: [5, 4, 3, 2] });
    for (const number of [5, 4, 3, 2]) await click(`[data-memory-select='${number}']`);
    expect(container.querySelector<HTMLButtonElement>("[data-memory-open-batch]")?.disabled).toBe(true);
  });

  it("tells a full branch queue instead of starting", async () => {
    const document = { authority: "card_newer" as const, document: "AGENTS.md", document_at_unix_ms: null, operator_at_unix_ms: 1 };
    await render(memory({ branches_full: true, questions: [{ kind: "docs_disagree", number: 3, title: "card 3", document }] }));
    expect(container.querySelector<HTMLButtonElement>("[data-memory-open-batch]")?.disabled).toBe(true);
    expect(container.querySelector("[data-memory-selection]")?.textContent).toContain("5 branches are waiting");
  });

  it("shows a confirmed correction as the document sentence struck, its replacement, and the operator's words", async () => {
    const document = { authority: "card_newer" as const, document: "docs/TESTING.md", document_at_unix_ms: null, operator_at_unix_ms: Date.UTC(2026, 9, 20, 12), operator_line: "never run the full suite", document_sentence: "Run the full suite before a merge.", replacement: "Run only the lane the diff touches." };
    await render(memory({ questions: [{ kind: "docs_disagree", number: 3, title: "card 3", document }] }));
    const shown = container.querySelector("[data-memory-card='3'][data-memory-question]")!;
    expect(shown.querySelector("del[data-memory-sentence='document']")?.textContent).toBe("Run the full suite before a merge.");
    expect(shown.querySelector("ins[data-memory-sentence='replacement']")?.textContent).toBe("Run only the lane the diff touches.");
    expect(shown.querySelector("[data-memory-said]")?.textContent).toContain('"never run the full suite"');
    expect(shown.querySelector("[data-memory-document]")?.textContent).toBe("TESTING.md");
    // Without a stored pair only the operator's line and 비교 remain.
    await render(memory({ questions: [{ kind: "docs_disagree", number: 3, title: "card 3", document: { ...document, document_sentence: null, replacement: null } }] }));
    expect(container.querySelector("[data-memory-pair]")).toBeNull();
    expect(container.querySelector("[data-memory-said]")).not.toBeNull();
    expect(container.querySelector("[data-memory-compare='3']")).not.toBeNull();
  });

  it("shows an unanswered comparison as the document's and the memory's sentence, and answers it", async () => {
    const document = { authority: "ask" as const, document: "AGENTS.md", document_at_unix_ms: null, operator_at_unix_ms: 1, document_sentence: "Keep the hook slow.", replacement: "Keep the hook under 100 ms." };
    await render(memory({ questions: [{ kind: "docs_disagree", number: 3, title: "card 3", document }] }));
    expect(container.querySelector("[data-memory-sides]")?.textContent).toBe("Doc:Keep the hook slow.Memory:Keep the hook under 100 ms.");
    expect(container.querySelector("[data-memory-open-batch]")).toBeNull();
    await click("[data-memory-promote='3']");
    expect(sent("memory_action").at(-1)).toEqual({ workspace_id: "p", action: "card_right", number: 3 });
  });

  it("discloses a whole question row while its checkbox and buttons act on their own", async () => {
    const document = { authority: "card_newer" as const, document: "AGENTS.md", document_at_unix_ms: null, operator_at_unix_ms: 1 };
    await render(memory({ questions: [{ kind: "docs_disagree", number: 3, title: "card 3", document }] }));
    await click("[data-memory-select='3']");
    expect(sent("memory_expand")).toEqual([]);
    await click("[data-memory-compare='3']");
    expect(sent("memory_expand")).toEqual([{ workspace_id: "p", number: 3 }]);
    events.length = 0;
    await click("[data-memory-row-toggle='3']");
    expect(sent("memory_expand")).toEqual([{ workspace_id: "p", number: 3 }]);
  });

  it("shows a promotion's agent and a merged card dimmed with its file", async () => {
    await render(memory({
      rows: [
        row(7, { promotion: { state: "running", agent: { name: "memory-promote-m7", pane_id: "w1:p2" }, pr_number: null, target_path: null, github_failure: null } }),
        row(8, { promotion: { state: "merged", agent: null, pr_number: 924, pr_url: "https://github.com/acme/one/pull/924", target_path: "AGENTS.md", github_failure: null } }),
        row(4, { promotion: { state: "merged", agent: null, pr_number: 931, pr_url: "https://github.com/acme/one/pull/931", target_path: null, github_failure: null } }),
        row(5, { promotion: { state: "unconfirmed", agent: null, pr_number: null, target_path: null, github_failure: "not_logged_in" } }),
      ],
    }));
    // GitHub could not say whether the gone agent's pull request came: a quiet
    // line with its reason, no spinner and nothing to press.
    const unconfirmed = container.querySelector("[data-memory-card='5']");
    expect(unconfirmed?.querySelector("[data-memory-unconfirmed]")?.textContent).toBe("Couldn't check for a PR·gh not logged in");
    expect(unconfirmed?.querySelector(".animate-spin")).toBeNull();
    // A merged card no file was named for shows the pull request it went in by.
    expect(container.querySelector("[data-memory-card='4'] [data-pr-mark]")?.textContent).toContain("931");
    expect(container.querySelector("[data-memory-running='7']")?.textContent).toContain("memory-promote-m7");
    const merged = container.querySelector("[data-memory-card='8']");
    expect(merged?.className).toContain("opacity-");
    expect(merged?.textContent).toContain("In the file · not given to agents");
  });

  it("explains waiting once above the list and dims the cards waiting for a check", async () => {
    await render(memory({
      rows: [
        row(3, { verifying: true }),
        row(7),
        row(9, { verifying: true, promotion: { state: "running", agent: null, pr_number: null, target_path: null, github_failure: null } }),
      ],
    }));
    const mark = container.querySelector("[data-memory-card='3'] [data-memory-verifying]");
    expect(mark).toBeNull();
    expect(container.querySelectorAll("[data-memory-checks]")).toHaveLength(1);
    expect(container.querySelector("[data-memory-checks]")?.textContent).toContain("Unverified memories wait for a check");
    expect(container.querySelector("[data-memory-card='3']")?.className).toContain("opacity-");
    expect(container.querySelector("[data-memory-card='7'] [data-memory-verifying]")).toBeNull();
    // One status line at a time: a promotion that runs draws its own.
    expect(container.querySelector("[data-memory-card='9'] [data-memory-verifying]")).toBeNull();
    expect(container.querySelector("[data-memory-card='3']")?.textContent).not.toContain("Retry");
  });

  it("does not call cards already in repository documents a pending check", async () => {
    await render(memory({ usable_cards: 0, rows: [row(3, { promotion: { state: "merged", agent: null, pr_number: 44, pr_url: "https://github.com/acme/one/pull/44", target_path: "AGENTS.md", github_failure: null } })] }));
    expect(container.querySelector("[data-memory-empty]")).toBeNull();
    expect(container.textContent).toContain("In the file · not given to agents");
  });

  it("marks only the running card as checking and explains zero usable cards", async () => {
    await render(memory({ usable_cards: 0, analysis: "checking", checking_card: 3, rows: [row(3, { verifying: true }), row(7, { verifying: true })] }));
    expect(container.querySelector("[data-memory-card='3'] [data-memory-verifying]")?.textContent).toBe("Checking");
    expect(container.querySelector("[data-memory-card='7'] [data-memory-verifying]")).toBeNull();
    expect(container.querySelectorAll("[data-memory-checks]")).toHaveLength(1);
    expect(container.querySelector("[data-memory-empty]")?.textContent).toContain("Each memory becomes available as it passes");
  });

  it("names the day's budget and leaves re-consent to the switch", async () => {
    await render(memory({ usable_cards: 0, analysis: "check_budget_exhausted", check_resume_at_unix_ms: Date.UTC(2026, 9, 14), rows: [row(3, { verifying: true }), row(7, { verifying: true })] }));
    expect(container.querySelector("[data-memory-empty]")?.textContent).toContain("Checks resume from");
    expect(container.querySelectorAll("[data-memory-checks]")).toHaveLength(1);
    expect(container.querySelector("[data-memory-verifying]")).toBeNull();
    expect(container.textContent).not.toContain("UTC");
    await render(memory({ enabled: false, consent_required: true, usable_cards: 0 }));
    expect(container.querySelector("[data-memory-consent]")?.textContent).toContain("agree again");
    expect(container.querySelector("[data-memory-empty]")).toBeNull();
    expect(container.querySelector("[data-memory-card]")).toBeNull();
    expect(container.querySelector("[data-memory-checks]")).toBeNull();
    expect(sent("memory_action")).toEqual([]);
  });

  it("draws the budget reset in the viewer's timezone, including a different calendar day", async () => {
    const budget = memory({ usable_cards: 0, analysis: "check_budget_exhausted", check_resume_at_unix_ms: Date.UTC(2026, 9, 14), rows: [row(3, { verifying: true })] });
    try {
      vi.stubEnv("TZ", "Asia/Seoul");
      await render(budget);
      expect(container.querySelector("[data-memory-checks]")?.textContent).toContain("October 14 at 9:00 AM");
      vi.stubEnv("TZ", "America/Los_Angeles");
      await render(budget);
      expect(container.querySelector("[data-memory-checks]")?.textContent).toContain("October 13 at 5:00 PM");
    } finally {
      vi.unstubAllEnvs();
    }
  });

  it("deletes the Project's data only after the confirmation names the count", async () => {
    await render(memory({ enabled: false }));
    await click("[data-memory-delete='link']");
    expect(sent("memory_action")).toEqual([]);
    expect(document.body.textContent).toContain("Delete herdr-ide's Memory data?");
    await click("[data-memory-delete='confirm']");
    expect(sent("memory_action")).toEqual([{ workspace_id: "p", action: "delete", number: null }]);
  });

  it("marks cleared cards seen only while their line is on a visible page, and keeps a mark that was not sent", async () => {
    const page = vi.spyOn(document, "visibilityState", "get").mockReturnValue("hidden");
    await render(memory({ evicted_cards: 2, evicted_through_unix_ms: 50 }));
    expect(container.querySelector("[data-memory-evicted]")).not.toBeNull();
    expect(sent("memory_evictions_seen")).toEqual([]);

    accepted = false;
    page.mockReturnValue("visible");
    await act(async () => document.dispatchEvent(new Event("visibilitychange")));
    expect(sent("memory_evictions_seen")).toHaveLength(1);
    // The core never took it, so the line is not held once it counts none.
    await render(memory({ evicted_cards: 0, evicted_through_unix_ms: null }));
    expect(container.querySelector("[data-memory-evicted]")).toBeNull();

    accepted = true;
    await render(memory({ evicted_cards: 2, evicted_through_unix_ms: 50 }));
    expect(sent("memory_evictions_seen")).toHaveLength(2);
  });

  it("keeps no cleared-cards line for a mark the core never counted", async () => {
    await render(memory({ evicted_cards: 2, evicted_through_unix_ms: 50 }));
    expect(sent("memory_evictions_seen")).toHaveLength(1);
    // The Memory fills before the core counts the mark, so the line gives way while two are still uncounted.
    await render(memory({ evicted_cards: 2, evicted_through_unix_ms: 50, capacity_reached: true }));
    // The count reaches none another way: nothing says the core took the mark.
    await render(memory({ evicted_cards: 0, evicted_through_unix_ms: null }));
    expect(container.querySelector("[data-memory-evicted]")).toBeNull();
  });

  it("says once how many older cards made room, and says full only when nothing can", async () => {
    await render(memory({ evicted_cards: 2, evicted_through_unix_ms: 50 }));
    expect(container.querySelector("[data-memory-evicted]")?.textContent).toBe("2 older memories were cleared to make room for new ones.");
    expect(container.textContent).not.toContain("Memory is full");
    expect(sent("memory_evictions_seen")).toEqual([{ workspace_id: "p", through_unix_ms: 50 }]);

    // The core counts them seen; the open tab keeps its line and says nothing again.
    await render(memory({ evicted_cards: 0, evicted_through_unix_ms: null }));
    expect(container.querySelector("[data-memory-evicted]")?.textContent).toBe("2 older memories were cleared to make room for new ones.");
    expect(sent("memory_evictions_seen")).toHaveLength(1);

    // Opened again, the tab has nothing to say until another card is cleared.
    await act(async () => root.unmount());
    root = createRoot(container);
    await render(memory({ evicted_cards: 0, evicted_through_unix_ms: null }));
    expect(container.querySelector("[data-memory-evicted]")).toBeNull();
    await render(memory({ evicted_cards: 1, evicted_through_unix_ms: 80 }));
    expect(container.querySelector("[data-memory-evicted]")?.textContent).toBe("1 older memory was cleared to make room for new ones.");
    expect(sent("memory_evictions_seen")).toEqual([{ workspace_id: "p", through_unix_ms: 50 }, { workspace_id: "p", through_unix_ms: 80 }]);

    await render(memory({ evicted_cards: 2, evicted_through_unix_ms: 90, capacity_reached: true }));
    expect(container.querySelector("[data-memory-evicted]")).toBeNull();
    expect(container.textContent).toContain("This Project's Memory is full, so new memories are not kept.");
  });

  it("offers a forgotten card back and a replaced text's undo by card number", async () => {
    await render(memory({
      forgotten: 9,
      expanded: { number: 3, body: "new text", cited_paths: ["docs/TESTING.md"], sources: [{ provider_label: "Codex", at_unix_ms: 1, human: true }], previous: { body: "old text", changed_at_unix_ms: Date.now() } },
    }));
    expect(container.querySelector("[data-memory-forgotten]")?.textContent).toContain("m9 forgotten");
    await click("[data-memory-forgotten] button");
    await click("[data-memory-previous='3']");
    expect(container.querySelector("[data-memory-detail='3']")?.textContent).toContain("old text");
    await click("[data-memory-undo='3']");
    expect(sent("memory_action")).toEqual([
      { workspace_id: "p", action: "undo", number: 9 },
      { workspace_id: "p", action: "undo", number: 3 },
    ]);
  });

  it("marks an unused card, brings an archived one back, and takes back a change or says why it cannot (PRD memory-self-improve B21, B22, B34)", async () => {
    const now = Date.now();
    await render(memory({
      rows: [row(3, { stale: true }), row(7)],
      archived: [{ number: 12, title: "Old deploy rule", archived_at_unix_ms: now }],
      expanded: {
        number: 3, body: "text", cited_paths: [], sources: [], previous: null,
        changes: [
          { id: 41, kind: "recheck", by_agent: true, letters: 1, unreadable: 2, at_unix_ms: now, undone: false, undo_blocked: "recheck_finished" },
          { id: 40, kind: "merge", cards: [5, 6], at_unix_ms: now, undone: false, undo_blocked: null },
          { id: 39, kind: "alias", aliases: ["린트"], at_unix_ms: now, undone: true, undo_blocked: null },
        ],
      },
    }));
    expect(container.querySelector("[data-memory-stale='3']")?.textContent).toBe("Unused 30 days");
    expect(container.querySelector("[data-memory-stale='7']")).toBeNull();
    expect(container.querySelector("[data-memory-card='12']")).toBeNull();
    const history = container.querySelector("[data-memory-history='3']")?.textContent ?? "";
    expect(history).toContain("Check asked by an agent");
    expect(history).toContain("Sources that could not be read: 2");
    expect(history).toContain("Merged m5, m6");
    expect(history).toContain("Undone");
    expect(container.querySelector("[data-memory-undo-blocked='recheck_finished']")).not.toBeNull();
    // A finished recheck leaves the card free to be checked again, once Hide AI is on to check it (D-41).
    expect(container.querySelector("[data-memory-recheck='3']")?.hasAttribute("disabled")).toBe(true);
    expect(container.querySelector("[data-memory-recheck='3']")?.getAttribute("title")).toBe("Checking again needs Hide AI. Turn it on in Settings › Hide AI");
    await act(async () => useShellStore.setState({ rest: { ...rest, status: { background_ai: { enabled: true, provider: "claude", chosen: true, unavailable_reason: null, providers: [], refusal: null } } } as unknown as SnapshotRest }));
    expect(container.querySelector("[data-memory-recheck='3']")?.hasAttribute("disabled")).toBe(false);
    await click("[data-memory-undo-change='40']");
    expect(container.querySelector("[data-memory-undo-change='39']")).toBeNull();
    await click("[data-memory-recheck='3']");
    await click("[data-memory-archived] button");
    await click("[data-memory-revive='12']");
    expect(sent("memory_action")).toEqual([
      { workspace_id: "p", action: "undo_change", number: 3, change: 40 },
      { workspace_id: "p", action: "recheck", number: 3 },
      { workspace_id: "p", action: "revive", number: 12 },
    ]);
  });

  it("shows a card waiting on its recheck as checking instead of offering another", async () => {
    await render(memory({
      rows: [row(3, { verifying: true })],
      expanded: {
        number: 3, body: "text", cited_paths: [], sources: [], previous: null,
        changes: [{ id: 41, kind: "recheck", by_agent: false, letters: null, unreadable: null, at_unix_ms: Date.now(), undone: false, undo_blocked: null }],
      },
    }));
    expect(container.querySelector("[data-memory-recheck='3']")).toBeNull();
    expect(container.querySelector("[data-memory-rechecking='3']")?.textContent).toBe("Checking again");
    expect(container.querySelector("[data-memory-history='3']")?.textContent).toContain("Check asked by you");
  });

  it("sends one search for one edit while two windows show the Project's shared answer", async () => {
    // Two windows on one Project share the core's search; the second follows it.
    const other = document.createElement("div");
    document.body.append(other);
    const otherRoot = createRoot(other);
    const dispatch: DispatchFn = (event) => {
      events.push(event);
      if (event.kind === "memory_search") {
        const { query } = event.payload as { query: string };
        const project = useShellStore.getState().memory?.project ?? null;
        // The core trims the words it searches by.
        useShellStore.setState({ memory: { project: project && { ...project, search: { query: query.trim(), numbers: [7] } } } });
      }
      return true;
    };
    vi.useFakeTimers();
    try {
      useShellStore.setState({ connection: "live", rest, memory: { project: memory() } });
      const tools = <TooltipProvider><ToolRail actions={createActions(dispatch)} /><MemoryTab actions={createActions(dispatch)} /></TooltipProvider>;
      await act(async () => root.render(tools));
      await act(async () => otherRoot.render(tools));
      const field = container.querySelector<HTMLInputElement>("[data-memory-search]")!;
      await act(async () => {
        Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(field, "lint ");
        field.dispatchEvent(new Event("input", { bubbles: true }));
      });
      await act(async () => { vi.advanceTimersByTime(5_000); });
      expect(sent("memory_search")).toEqual([{ workspace_id: "p", query: "lint " }]);
      // The space typed before the next word stays in the field.
      expect(field.value).toBe("lint ");
      expect(other.querySelector<HTMLInputElement>("[data-memory-search]")!.value).toBe("lint");
    } finally {
      vi.useRealTimers();
      await act(async () => otherRoot.unmount());
      other.remove();
    }
  });

  it("says Hide AI is paused when every agent is past the usage limit, and keeps the settings button (B5)", async () => {
    useShellStore.setState({
      connection: "live",
      rest: {
        ...rest,
        status: {
          background_ai: {
            enabled: true, provider: "claude", chosen: true, unavailable_reason: null,
            providers: [{ id: "claude", label: "Claude Code", agent: "claude-code" }],
            refusal: { provider: "claude", reason: "past_limit", retry_at_ms: null, using: null, used_percent: 93, limit: 90 },
          },
        },
      } as unknown as SnapshotRest,
      memory: { project: memory({ analysis: "ai_not_ready" }) },
    });
    const actions = createActions((event) => { events.push(event); return true; });
    await act(async () => root.render(<TooltipProvider><ToolRail actions={actions} /><MemoryTab actions={actions} /></TooltipProvider>));
    expect(container.querySelector("[data-memory-ai='not_ready']")?.textContent).toContain("every agent is past your usage limit this week");
    await click("[data-memory-ai='not_ready'] button");
    expect(useUiStore.getState()).toMatchObject({ overlay: "settings", settingsTab: "hideAi" });
  });

  it("says when a Project's Memory could not be read", async () => {
    await render(memory({ unavailable: { reason: "unreadable" } }));
    expect(container.querySelector('[data-memory="unreadable"]')).not.toBeNull();
    expect(container.querySelector("[data-memory-cards]")).toBeNull();
  });
  it("shows the requested Memory failure with scoped Retry and keeps the confirmed switch", async () => {
    const actions = createActions((event) => { events.push(event); return true; });
    await act(async () => {
      await clientI18n.changeLanguage("en");
      root.render(<TooltipProvider><MemorySettings project={memory({ action_failure: "disable" })} actions={actions} disabled={false} /></TooltipProvider>);
    });
    expect(container.querySelector("[data-memory-switch='on']")).not.toBeNull();
    expect(container.querySelector("[role=alert]")?.textContent).toContain("Could not change Memory");
    await click("[role=alert] button");
    expect(sent("memory_action")).toEqual([{ workspace_id: "p", action: "disable", number: null }]);
    for (const action of ["delete", "forget", "promote"] as const) {
      await act(async () => root.render(<TooltipProvider><MemorySettings project={memory({ action_failure: action })} actions={actions} disabled={false} /></TooltipProvider>));
      expect(container.querySelector("[role=alert]")).toBeNull();
    }
    await act(async () => root.render(<TooltipProvider><MemorySettings project={memory({ pending_action: "enable", preparing_hooks: true })} actions={actions} disabled={false} /></TooltipProvider>));
    expect(container.textContent).toContain("Preparing agent hooks");
    expect(container.querySelector("[data-memory-switch]")?.hasAttribute("disabled")).toBe(true);
  });

});
