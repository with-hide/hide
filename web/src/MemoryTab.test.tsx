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
  workspace_id: "p", unavailable: null, loading: false, enabled: true, capacity_reached: false,
  rows: [row(3), row(7)], questions: [], analysis: "idle", consent_required: false, usable_cards: 2, checking_card: null, check_resume_at_unix_ms: null, forgotten: null, pending_action: null,
  expanded: null, search: null, ...overrides,
});

describe("the Memory tab", () => {
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
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  /** The rail, which opens the front checkout's Project and wears Memory's dot, and the Memory tool, or the rail alone. */
  async function render(project: ProjectMemory | null, tool: "memory" | "files" = "memory") {
    const actions = createActions((event) => { events.push(event); return true; });
    await act(async () => {
      useShellStore.setState({ connection: "live", rest, memory: { project } });
      root.render(<TooltipProvider><ToolRail actions={actions} />{tool === "memory" ? <MemoryTab actions={actions} /> : null}</TooltipProvider>);
    });
  }
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

  it("opens the Project and shows the rail's dot from any tool while its Project has a question", async () => {
    await render(memory({ questions: [{ kind: "failed", number: 7, title: "card 7", start_failure: null }] }), "files");
    expect(sent("memory_open")).toEqual([{ workspace_id: "p" }]);
    expect(container.querySelector('[data-lane-tool-dot="memory"]')).not.toBeNull();
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

  it("asks about a suggested card and a failed promotion, and sends the card's number", async () => {
    await render(memory({
      questions: [
        { kind: "promote", number: 3, title: "card 3", human_session_count: 3, pulled_session_count: 9 },
        { kind: "failed", number: 7, title: "card 7", start_failure: null },
      ],
    }));
    expect(container.querySelector("[data-memory-questions]")?.textContent).toContain("your words 3x · 9 sessions");
    // Both cards are asked about, so no list repeats them.
    expect(container.querySelector("[data-memory-cards]")).toBeNull();
    await click("[data-memory-promote='3']");
    await click("[data-memory-dismiss='3']");
    await click("[data-memory-retry='7']");
    expect(sent("memory_action")).toEqual([
      { workspace_id: "p", action: "promote", number: 3 },
      { workspace_id: "p", action: "dismiss", number: 3 },
      { workspace_id: "p", action: "promote", number: 7 },
    ]);
  });

  it("keeps running and bound open promotions in To ask until merged or closed", async () => {
    const promotion = { state: "pr_open" as const, agent: null, pr_number: 72, pr_url: "https://github.com/acme/one/pull/72", target_path: "AGENTS.md", github_failure: null };
    await render(memory({ rows: [row(3, { promotion: { ...promotion, state: "running", pr_number: null } }), row(7, { promotion })] }));
    expect(container.querySelector("[data-memory-questions] [data-memory-running='3']")).not.toBeNull();
    expect(container.querySelector("[data-memory-questions] [data-memory-pr-group='72']")?.textContent).toContain("m7");
    expect(container.querySelector("[data-memory-cards]")).toBeNull();
    await act(async () => useShellStore.setState({ rest: { ...rest, navigator: { ...rest.navigator, workspaces: [{ ...rest.navigator!.workspaces![0], pull_requests: [{ number: 72, url: "https://github.com/acme/one/pull/72", review: "changes_requested" }] }] } } as unknown as SnapshotRest }));
    expect(container.querySelector("[data-memory-pr-group='72']")?.textContent).toContain("Changes requested");
    await render(memory({ rows: [row(3), row(7, { promotion: { ...promotion, state: "merged" } })] }));
    expect(container.querySelector("[data-memory-questions]")).toBeNull();
    expect(container.querySelector("[data-memory-card='3']")).not.toBeNull();
    expect(container.querySelector("[data-memory-card='7']")?.textContent).toContain("AGENTS.md");
  });

  it.each([
    ["en", "PR 1050"],
    ["ko", "PR 1050"],
    ["ja", "PR 1050"],
    ["zh-CN", "PR 1050"],
  ])("renders PR identifiers as plain digits in %s", async (language, expected) => {
    await act(async () => { await clientI18n.changeLanguage(language); });
    await render(memory({ rows: [row(7, { promotion: { state: "pr_open", agent: null, pr_number: 1050, pr_url: "https://github.com/acme/one/pull/1050", target_path: null, github_failure: null } })] }));
    const group = container.querySelector("[data-memory-pr-group='1050']");
    expect(group?.textContent).toContain(expected);
    expect(group?.textContent).not.toContain("1,050");
    await click("[data-memory-open-pr='1050']");
    expect(window.open).toHaveBeenCalledWith("https://github.com/acme/one/pull/1050", "_blank", "noopener,noreferrer");
  });

  it("retains the original promotion URL after Connections changes to a same-number PR", async () => {
    const original = "https://github.com/Acme/One/pull/72";
    await render(memory({ rows: [row(7, { promotion: { state: "pr_open", agent: null, pr_number: 72, pr_url: original, target_path: null, github_failure: null } })] }));
    await act(async () => useShellStore.setState({ rest: { ...rest, navigator: { ...rest.navigator, workspaces: [{ ...rest.navigator!.workspaces![0], pull_requests: [{ number: 72, url: "https://github.com/acme/two/pull/72", state: "merged", checks: "failed" }] }] } } as unknown as SnapshotRest }));
    expect(container.querySelector("[data-memory-pr-group='72']")?.textContent).not.toContain("Merged");
    expect(container.querySelector("[data-memory-pr-group='72']")?.textContent).not.toContain("Checks failed");
    await click("[data-memory-open-pr='72']");
    expect(window.open).toHaveBeenCalledWith(original, "_blank", "noopener,noreferrer");
    await act(async () => useShellStore.setState({ rest: { ...rest, navigator: { ...rest.navigator, workspaces: [{ ...rest.navigator!.workspaces![0], pull_requests: [{ number: 72, url: "https://github.com/acme/one/pull/72", state: "merged" }] }] } } as unknown as SnapshotRest }));
    expect(container.querySelector("[data-memory-pr-group='72']")?.textContent).toContain("Merged");
    await click("[data-memory-open-pr='72']");
    expect(sent("pr_open")).toEqual([{ workspace_id: "p", pr_number: 72, open_workspace: true }]);
    await render(memory({ rows: [row(7, { promotion: { state: "pr_open", agent: null, pr_number: 72, target_path: null, github_failure: null } })] }));
    expect(container.querySelector("[data-memory-pr-group='72']")?.textContent).toContain("Couldn't check for a PR");
    expect(container.querySelector<HTMLButtonElement>("[data-memory-open-pr='72']")?.disabled).toBe(true);
    await click("[data-memory-open-pr='72']");
    expect(window.open).toHaveBeenCalledTimes(1);
  });

  it("selection hides row promotion actions while unknown authority still needs an answer", async () => {
    await render(memory({ questions: [
      { kind: "promote", number: 3, title: "card 3", human_session_count: 2, pulled_session_count: 0 },
      { kind: "docs_disagree", number: 7, title: "card 7", document: { authority: "ask", document: "AGENTS.md", document_at_unix_ms: null, operator_at_unix_ms: 1 } },
    ] }));
    expect(container.querySelector("[data-memory-card='3']")?.textContent).toContain("your words 2x");
    expect(container.querySelector("[data-memory-card='3']")?.textContent).not.toContain("0 sessions");
    await click("[data-memory-select='3']");
    expect(container.querySelector("[data-memory-promote='3']")).toBeNull();
    expect(container.querySelector("[data-memory-dismiss='3']")).toBeNull();
    expect(container.querySelector("[data-memory-promote='7']")).not.toBeNull();
    await click("[data-memory-open-batch]");
    expect(sent("memory_action").at(-1)).toEqual({ workspace_id: "p", action: "promote", number: null, numbers: [3] });
  });

  it("discloses a whole question or PR row while its checkbox, promotion and PR buttons act independently", async () => {
    const promotion = { state: "pr_open" as const, agent: null, pr_number: 72, pr_url: "https://github.com/acme/one/pull/72", target_path: null, github_failure: null };
    const data = memory({ rows: [row(3), row(7, { promotion, body_preview: "A rule's body" }), row(9, { promotion, body_preview: "Another rule's body" })], questions: [{ kind: "promote", number: 3, title: "card 3", human_session_count: 2, pulled_session_count: 0 }] });
    await render(data);
    await click("[data-memory-promote='3']");
    await click("[data-memory-select='3']");
    await click("[data-memory-open-pr='72']");
    expect(sent("memory_expand")).toEqual([]);
    await click("[data-memory-row-toggle='3']");
    await click("[data-memory-row-toggle='7']");
    expect(sent("memory_expand")).toEqual([{ workspace_id: "p", number: 3 }, { workspace_id: "p", number: 7 }]);
    await render({ ...data, expanded: { number: 7, body: "A rule's body", cited_paths: [], sources: [], previous: null } });
    expect(container.querySelectorAll("[data-memory-progress-detail] [data-memory-preview]")).toHaveLength(2);
    expect(container.querySelector("[data-memory-preview='9']")?.textContent).toContain("Another rule's body");
    expect(container.querySelector("[data-memory-cards]")).toBeNull();
    expect(container.querySelector("[data-memory-row-toggle='7']")?.getAttribute("aria-expanded")).toBe("true");
    await click("[data-memory-row-toggle='7']");
    expect(sent("memory_expand").at(-1)).toEqual({ workspace_id: "p", number: null });
  });

  it("says in a failed card's status why its agent never started, with the question its shell stopped at", async () => {
    await render(memory({
      questions: [
        { kind: "failed", number: 7, title: "card 7", start_failure: null },
        { kind: "failed", number: 8, title: "card 8", start_failure: { code: "shell_starting", detail: "Update now? [Y/n]" } },
        { kind: "failed", number: 9, title: "card 9", start_failure: { code: "agent_not_started", detail: null } },
      ],
    }));
    const status = (number: number) => container.querySelector(`[data-memory-card='${number}'] [data-memory-failure]`)?.textContent;
    expect(status(7)).toBe("Ended without a PR");
    expect(status(8)).toBe("The shell stopped at a question: Update now? [Y/n]");
    expect(status(9)).toBe("The agent didn't start");
    // Each failed card is retried or left out.
    expect(container.querySelectorAll("[data-memory-retry]")).toHaveLength(3);
    await click("[data-memory-dismiss='9']");
    expect(sent("memory_action")).toEqual([{ workspace_id: "p", action: "dismiss", number: 9 }]);
  });

  it("names a card the agent could not process first and keeps its reason behind the row", async () => {
    const question = { kind: "failed" as const, number: 7, title: "card 7", start_failure: { code: "card_not_processed" as const, detail: "The behavior is not on main" } };
    await render(memory({ questions: [question] }));
    expect(container.querySelector("[data-memory-card='7'] [data-memory-failure]")?.textContent).toBe("Card was not processed · Ended without a PR");
    expect(container.querySelector("[data-memory-told]")).toBeNull();
    await render(memory({ questions: [question], expanded: { number: 7, body: "A decision", cited_paths: [], sources: [], previous: null } }));
    expect(container.querySelector("[data-memory-told='7']")?.textContent).toBe("The agent's reason: The behavior is not on main");
  });

  it("asks once about a decision nothing builds yet and only offers to keep it", async () => {
    await render(memory({
      rows: [row(3), row(7, { kind: "decision" })],
      questions: [{ kind: "unbuilt", number: 7, title: "card 7", reason: "No branch builds the backlog source" }],
    }));
    const asked = container.querySelector("[data-memory-question='unbuilt']");
    expect(asked?.textContent).toContain("Not built yet · nothing builds it");
    // Nothing to promote: it is never selected for a batch, retried or promoted.
    expect(asked?.querySelector("[data-memory-select]")).toBeNull();
    expect(asked?.querySelector("[data-memory-unbuilt='7']")).not.toBeNull();
    expect(asked?.querySelector("[data-memory-promote], [data-memory-retry]")).toBeNull();
    await click("[data-memory-keep='7']");
    expect(sent("memory_action")).toEqual([{ workspace_id: "p", action: "dismiss", number: 7 }]);
  });

  it("keeps a card another branch builds in To ask with that work, then its pull request", async () => {
    const building = { state: "building" as const, agent: { name: "impl-overview", pane_id: "w2:p1" }, pr_number: null, pr_url: null, target_path: "docs/UI_BEHAVIOR.md", github_failure: null };
    await render(memory({ rows: [row(3), row(7, { promotion: building })] }));
    const progress = container.querySelector("[data-memory-questions] [data-memory-progress][data-memory-card='7']");
    expect(progress?.textContent).toContain("Goes in with the PR that builds it");
    expect(progress?.querySelector("[data-memory-agent='7']")).not.toBeNull();
    // A card on its way is not drawn again in the list, and nothing is asked.
    expect(container.querySelector("[data-memory-cards] [data-memory-card='7']")).toBeNull();
    expect(container.querySelector("[data-memory-question]")).toBeNull();
    await render(memory({ rows: [row(3), row(7, { promotion: { ...building, pr_number: 1160, pr_url: "https://github.com/acme/one/pull/1160" } })] }));
    expect(container.querySelector("[data-memory-pr-group='1160']")?.textContent).toContain("Goes in with the PR that builds it");
  });

  it("shows a promotion's agent, its pull request, and a merged card dimmed with its file", async () => {
    await render(memory({
      rows: [
        row(7, { promotion: { state: "running", agent: { name: "memory-promote-m7", pane_id: "w1:p2" }, pr_number: null, target_path: null, github_failure: null } }),
        row(9, { promotion: { state: "pr_open", agent: null, pr_number: 924, pr_url: "https://github.com/acme/one/pull/924", target_path: null, github_failure: null } }),
        row(8, { promotion: { state: "merged", agent: null, pr_number: 924, pr_url: "https://github.com/acme/one/pull/924", target_path: "AGENTS.md", github_failure: null } }),
        row(4, { promotion: { state: "merged", agent: null, pr_number: 931, pr_url: "https://github.com/acme/one/pull/931", target_path: null, github_failure: null } }),
        row(6, { promotion: { state: "running", agent: null, pr_number: null, target_path: null, github_failure: null } }),
        row(5, { promotion: { state: "unconfirmed", agent: null, pr_number: null, target_path: null, github_failure: "not_logged_in" } }),
      ],
    }));
    // GitHub could not say whether the gone agent's pull request came: a quiet
    // line with its reason, no spinner and nothing to press.
    const unconfirmed = container.querySelector("[data-memory-card='5']");
    expect(unconfirmed?.querySelector("[data-memory-unconfirmed]")?.textContent).toBe("Couldn't check for a PR · gh not logged in");
    expect(unconfirmed?.querySelector(".animate-spin")).toBeNull();
    expect(unconfirmed?.textContent).not.toContain("Retry");
    // A merged card no file was named for shows the pull request it went in by.
    expect(container.querySelector("[data-memory-card='4'] [data-pr-mark]")?.textContent).toContain("931");
    expect(container.querySelector("[data-memory-running='7']")?.textContent).toContain("Writing PR");
    expect(container.querySelector("[data-memory-agent='7']")?.textContent).toBe("Agent ↗");
    // An agent not seen yet: no name, and nothing it would follow.
    expect(container.querySelector("[data-memory-running='6']")?.textContent).toContain("Writing PR");
    expect(container.querySelector("[data-memory-agent='6']")).toBeNull();
    expect(container.querySelector("[data-memory-pr-group='924']")?.textContent).toContain("924");
    const merged = container.querySelector("[data-memory-card='8']");
    expect(merged?.className).toContain("opacity-");
    expect(merged?.textContent).toContain("AGENTS.md");
    expect(merged?.textContent).toContain("In the file · not injected");
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
    expect(container.querySelector("[data-memory-checks]")?.textContent).toContain("Unverified cards wait for a check");
    expect(container.querySelector("[data-memory-card='3']")?.className).toContain("opacity-");
    expect(container.querySelector("[data-memory-card='7'] [data-memory-verifying]")).toBeNull();
    // One status line at a time: a promotion that runs draws its own.
    expect(container.querySelector("[data-memory-card='9'] [data-memory-verifying]")).toBeNull();
    expect(container.querySelector("[data-memory-card='3']")?.textContent).not.toContain("Retry");
  });

  it("does not call cards already in repository documents a pending check", async () => {
    await render(memory({ usable_cards: 0, rows: [row(3, { promotion: { state: "merged", agent: null, pr_number: 44, pr_url: "https://github.com/acme/one/pull/44", target_path: "AGENTS.md", github_failure: null } })] }));
    expect(container.querySelector("[data-memory-empty]")).toBeNull();
    expect(container.textContent).toContain("In the file · not injected");
  });

  it("marks only the running card as checking and explains zero usable cards", async () => {
    await render(memory({ usable_cards: 0, analysis: "checking", checking_card: 3, rows: [row(3, { verifying: true }), row(7, { verifying: true })] }));
    expect(container.querySelector("[data-memory-card='3'] [data-memory-verifying]")?.textContent).toBe("Checking");
    expect(container.querySelector("[data-memory-card='7'] [data-memory-verifying]")).toBeNull();
    expect(container.querySelectorAll("[data-memory-checks]")).toHaveLength(1);
    expect(container.querySelector("[data-memory-empty]")?.textContent).toContain("Each card becomes available as it passes");
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

  it("says Hide AI is paused when every agent is past the usage limit, and keeps the dimmed chip and the settings button (B5)", async () => {
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
    expect(container.querySelector("[data-memory-ai-chip]")?.className).toContain("opacity-(--opacity-dimmed)");
    await click("[data-memory-ai='not_ready'] button");
    expect(useUiStore.getState()).toMatchObject({ overlay: "settings", settingsTab: "hideAi" });
  });

  it("names the agent standing in on the chip while Runs on is past the limit (B4)", async () => {
    useShellStore.setState({
      connection: "live",
      rest: {
        ...rest,
        status: {
          background_ai: {
            enabled: true, provider: "claude", chosen: true, unavailable_reason: null,
            providers: [{ id: "claude", label: "Claude Code", agent: "claude-code" }, { id: "codex", label: "Codex", agent: "codex" }],
            refusal: { provider: "claude", reason: "past_limit", retry_at_ms: null, using: "codex", used_percent: 93, limit: 90 },
          },
        },
      } as unknown as SnapshotRest,
      memory: { project: memory() },
    });
    const actions = createActions((event) => { events.push(event); return true; });
    await act(async () => root.render(<TooltipProvider><ToolRail actions={actions} /><MemoryTab actions={actions} /></TooltipProvider>));
    expect(container.querySelector("[data-memory-ai-chip]")?.textContent).toBe("Hide AI · Codex");
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
