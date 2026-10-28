// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterAll, afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createActions } from "./actions";
import { TooltipProvider } from "./components/ui/tooltip";
import { HideAiTool } from "./HideAiTool";
import { clientI18n } from "./i18n/translator";
import type { ProjectSuggestions, SnapshotRest } from "./snapshot";
import { useShellStore } from "./store";
import { ToolRail } from "./Tools";
import { useUiStore } from "./ui";
import { emptyScope } from "../test/legacyAgentScope";
import { suggestionRow, suggestionsProject } from "../test/suggestionsProject";
import type { DispatchFn } from "./ws";

const browserCanvas = vi.hoisted(() => {
  const original = HTMLCanvasElement.prototype.getContext;
  HTMLCanvasElement.prototype.getContext = () => null;
  return { restore: () => { HTMLCanvasElement.prototype.getContext = original; } };
});
afterAll(() => browserCanvas.restore());
Element.prototype.scrollIntoView = () => {};

// The front checkout `c` belongs to Project `p` (PRD hide-ai-suggestions B1-B19).
const rest = {
  navigator: {
    focused_device_id: "local", focused_checkout_id: "c", focused_workspace_id: "p",
    devices: [{ id: "local", label: "This Mac", kind: "local", state: "local" }],
    workspaces: [{ id: "p", label: "herdr-ide", device_id: "local", agent_scope: emptyScope(), checkouts: [{ id: "c", workspace_id: "p", path: "/p", label: "main", tabs: [], active_tab_id: null, strip: [] }] }],
  },
  status: {
    background_ai: { enabled: true, provider: "claude", chosen: true, unavailable_reason: null, providers: [{ id: "claude", label: "Claude Code", agent: "claude-code" }, { id: "codex", label: "Codex", agent: "codex" }], refusal: null },
  },
} as unknown as SnapshotRest;

describe("the Hide AI tool", () => {
  let container: HTMLDivElement;
  let root: Root;
  let saved: ReturnType<typeof useShellStore.getState>;
  let savedUi: ReturnType<typeof useUiStore.getState>;
  let events: Parameters<DispatchFn>[0][];

  beforeEach(async () => {
    saved = useShellStore.getState();
    savedUi = useUiStore.getState();
    vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
    events = [];
    container = document.createElement("div");
    document.body.append(container);
    root = createRoot(container);
    await act(async () => { await clientI18n.changeLanguage("en"); });
  });
  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
    useShellStore.setState(saved, true);
    useUiStore.setState(savedUi, true);
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  async function render(project: ProjectSuggestions | null, backgroundAi = rest.status!.background_ai) {
    const actions = createActions((event) => { events.push(event); return true; });
    await act(async () => {
      useShellStore.setState({ connection: "live", rest: { ...rest, status: { ...rest.status, background_ai: backgroundAi } } as SnapshotRest, memory: { project: null }, suggestions: { project } });
      root.render(<TooltipProvider><ToolRail actions={actions} /><HideAiTool actions={actions} /></TooltipProvider>);
    });
  }
  const sent = (kind: string) => events.filter((event) => event.kind === kind).map((event) => event.payload);
  const q = (selector: string) => document.body.querySelector<HTMLElement>(selector);
  const click = async (selector: string) => {
    const element = q(selector);
    if (!element) throw new Error(`missing ${selector}`);
    await act(async () => element.click());
  };
  /** Radix opens its menus and popovers on a pointer press, not a synthetic click. */
  const press = async (selector: string) => {
    const element = q(selector);
    if (!element) throw new Error(`missing ${selector}`);
    await act(async () => {
      // jsdom has no PointerEvent; Radix reads only the button and the modifier keys.
      element.dispatchEvent(new MouseEvent("pointerdown", { bubbles: true, button: 0 }));
      element.click();
    });
  };

  it("reads until the core answers for the front Project, then counts each tab and the rail as the core does (B1)", async () => {
    await render(suggestionsProject({ workspace_id: "another" }));
    expect(q("[data-hide-ai-tool]")?.getAttribute("data-hide-ai")).toBe("loading");
    expect(container.textContent).toContain("Reading");
    await render(suggestionsProject({ rows: [suggestionRow("s1"), suggestionRow("s2")], counts: { suggestions: 2, rail: 3 } }));
    expect(q("[data-hide-ai-tool]")?.textContent).toContain("Hide AI· herdr-ide");
    expect(q('[data-hide-ai-tool-tab="suggestions"] [data-hide-ai-tool-tab-count]')?.textContent).toBe("2");
    expect(q('[data-hide-ai-tool-tab="memory"] [data-hide-ai-tool-tab-count]')?.textContent).toBe("1");
    expect(q('[data-lane-tool="memory"] [data-lane-todo-count]')?.textContent).toBe("3");
    expect(q('[data-lane-tool="memory"] svg.lucide-brain')).not.toBeNull();
    expect(q('[data-lane-tool="memory"]')?.getAttribute("aria-label")).toBe("Hide AI");
    await render(suggestionsProject({ counts: { suggestions: 0, rail: 0 } }));
    expect(q("[data-hide-ai-tool-tab-count]")).toBeNull();
    expect(q('[data-lane-tool="memory"] [data-lane-todo-count]')).toBeNull();
  });

  it("turns all of Hide AI off from the head, and with it off shows one line and nothing to count (B3)", async () => {
    await render(suggestionsProject());
    await click("[data-hide-ai-switch]");
    expect(sent("hide_ai_switch")).toEqual([{ workspace_id: "p", on: false }]);
    await render(suggestionsProject({ enabled: false, status: "off", rows: [suggestionRow("s1")] }));
    expect(q("[data-hide-ai-off]")?.textContent).toContain("Hide AI is off");
    // Off, the owl says what each tab gives once it is on.
    expect([...document.querySelectorAll("[data-hide-ai-explainer-group]")].map((group) => group.getAttribute("data-hide-ai-explainer-group"))).toEqual(["Suggestions", "Memory"]);
    expect(q('[data-hide-ai-explainer-group="Memory"]')?.textContent).toContain("Every agent uses the same memories");
    expect(q('[role="tablist"]')).toBeNull();
    expect(q("[data-suggestion]")).toBeNull();
    await click("[data-hide-ai-switch]");
    expect(sent("hide_ai_switch").at(-1)).toEqual({ workspace_id: "p", on: true });
  });

  it("says once, on the first open, that investigating uses the subscription and what is sent (B3)", async () => {
    await render(suggestionsProject({ intro_seen: false }));
    expect(q("[data-hide-ai-intro]")?.textContent).toContain("uses your own subscription");
    expect(sent("suggestions_intro_seen")).toEqual([{}]);
    // The core marks it seen; the open tab keeps the line until it closes.
    await render(suggestionsProject({ intro_seen: true }));
    expect(q("[data-hide-ai-intro]")).not.toBeNull();
    expect(sent("suggestions_intro_seen")).toHaveLength(1);
    await act(async () => root.unmount());
    root = createRoot(container);
    await render(suggestionsProject({ intro_seen: true }));
    expect(q("[data-hide-ai-intro]")).toBeNull();
  });

  it("draws the tab's state in one line, and 조사 중 or 조사 못 함 in the head with the rows as they were (B17, D-35)", async () => {
    await render(suggestionsProject({ status: "first_run_pending" }));
    expect(q("[data-suggestions-status] [data-hide-ai-explainer-title]")?.textContent).toBe("First investigation the next time you open Hide");
    // With nothing open, the owl says what the tab does in three steps.
    expect([...document.querySelectorAll("[data-suggestions-status] li")].map((step) => step.textContent)).toEqual([
      "Finds where agents keep getting stuck in sessions and PR reviews",
      "Suggests how to stop the same mistakes",
      "Pick one and it lands as a PR, so agents get smarter on their own",
    ]);
    await render(suggestionsProject({ status: "investigating", rows: [suggestionRow("s1")] }));
    expect(q("[data-hide-ai-investigating]")?.textContent).toBe("Investigating");
    expect(q("[data-suggestions-status]")).toBeNull();
    expect(q('[data-suggestion="s1"]')).not.toBeNull();
    // A run that could not investigate says so quietly in the head and keeps the rows (D-35).
    await render(suggestionsProject({ status: "not_investigated", rows: [suggestionRow("s1")] }));
    expect(q("[data-hide-ai-not-investigated]")?.textContent).toBe("Couldn't investigate");
    expect(q("[data-hide-ai-investigating]")).toBeNull();
    expect(q("[data-suggestions-status]")).toBeNull();
    expect(q('[data-suggestion="s1"]')).not.toBeNull();
    await render(suggestionsProject({ status: "none_today" }));
    expect(q("[data-hide-ai-not-investigated]")).toBeNull();
    expect(q("[data-suggestions-status] [data-hide-ai-explainer-title]")?.textContent).toBe("No suggestions today");
    await render(suggestionsProject({ status: "off", rows: [suggestionRow("s1")] }));
    expect(q("[data-suggestion]")).toBeNull();
    // Suggestions off: the owl says what they give, with 켜기 under it.
    expect(q("[data-suggestions-status] [data-hide-ai-explainer-title]")?.textContent).toBe("Suggestions are off");
    expect(q("[data-suggestions-status]")?.textContent).toContain("Suggests how to stop the same mistakes");
    await click("[data-suggestions-turn-on]");
    expect(sent("suggestion_settings")).toEqual([{ workspace_id: "p", suggestions_enabled: true }]);
  });

  it("shows 고칠 것 above a new suggestion's PR or issue, with 넘어가기 beside it and beside GitHub connection needed (B10, B14, B16, B18)", async () => {
    await render(suggestionsProject({ rows: [
      suggestionRow("s1"),
      suggestionRow("s2", { outcome: "issue" }),
      suggestionRow("s3", { hold: "github_needed" }),
      suggestionRow("s4", { discomfort: null, basis: { signal: "failed_tool", tag: "cargo test" } }),
      suggestionRow("s5", { outcome: "issue", fix: null }),
      suggestionRow("s6", { returned_after_skip: true }),
    ] }));
    expect(q('[data-suggestion="s1"] [data-suggestion-mark]')?.getAttribute("data-suggestion-mark")).toBe("new");
    expect(q('[data-suggestion="s1"]')?.textContent).toContain("Missed · cause s1");
    expect(q('[data-suggestion-fix="s1"]')?.textContent).toBe("Fixfix s1");
    expect(q('[data-suggestion="s1"] [data-suggestion-apply]')?.textContent).toBe("Open a PR that fixes the docs");
    expect(q('[data-suggestion="s2"] [data-suggestion-apply]')?.textContent).toBe("Leave it as an issue");
    // An issue says why it becomes one: in the core's words, else in Hide's.
    expect(q('[data-suggestion-fix="s2"]')?.textContent).toBe("Fixfix s2");
    expect(q('[data-suggestion-fix="s5"]')?.textContent).toBe("FixThe docs can't fix this. Leaves an issue with the CI record");
    expect(q('[data-suggestion="s3"] [data-suggestion-apply]')).toBeNull();
    expect(q('[data-suggestion="s3"] [data-suggestion-github-needed]')?.textContent).toBe("GitHub connection needed");
    // 넘어가기 is a button on every choice, ⋯ is gone.
    expect([...document.body.querySelectorAll("[data-suggestion-dismiss]")].map((button) => button.textContent)).toEqual(Array(6).fill("Skip"));
    expect(q("[data-suggestion-more]")).toBeNull();
    // A line the grouping wrote alone is named by its signal and tag.
    expect(q('[data-suggestion="s4"]')?.textContent).toContain("The same command keeps failing: cargo test");
    expect(q('[data-suggestion="s6"] [data-suggestion-returned]')?.textContent).toBe("Skipped, but it came back");
    expect(q('[data-suggestion="s6"] [data-suggestion-returned]')?.className).toContain("text-warning");
    expect(q('[data-suggestion="s1"] [data-suggestion-returned]')).toBeNull();
    await click('[data-suggestion="s1"] [data-suggestion-apply]');
    await click('[data-suggestion-dismiss="s3"]');
    expect(sent("suggestion_action")).toEqual([
      { workspace_id: "p", id: "s1", action: "apply" },
      { workspace_id: "p", id: "s3", action: "dismiss" },
    ]);
  });

  it("opens a row in place to its evidence, dimming evidence that is gone, with 고칠 것 once above the buttons (B10)", async () => {
    const at = Date.now();
    await render(suggestionsProject({ rows: [suggestionRow("s1", { evidence: [
      { kind: "session", signal: "correction", improvement: null, agent: "claude", session_id: "abc", pr_number: null, url: null, at_unix_ms: at, gone: false },
      { kind: "pr", signal: "check_failed", improvement: null, agent: null, session_id: null, pr_number: 924, url: "https://github.com/acme/one/pull/924", at_unix_ms: at, gone: false },
      { kind: "session", signal: "failed_tool", improvement: null, agent: "codex", session_id: "old", pr_number: null, url: null, at_unix_ms: at, gone: true },
    ] })] }));
    expect(q("[data-suggestion-detail]")).toBeNull();
    await click('[data-suggestion-toggle="s1"]');
    expect(useUiStore.getState().openSuggestion).toBe("s1");
    expect(q('[data-suggestion-detail="s1"]')?.textContent).not.toContain("fix s1");
    expect(q('[data-suggestion-fix="s1"]')?.textContent).toContain("fix s1");
    const evidence = [...document.body.querySelectorAll<HTMLButtonElement>("[data-suggestion-evidence]")];
    expect(evidence.map((button) => button.disabled)).toEqual([false, false, true]);
    expect(evidence[2]?.className).toContain("opacity-(--opacity-dimmed)");
    expect(evidence[1]?.textContent).toContain(`PR ${"#"}924 · Same CI failure`);
    // A pull request the catalog does not list opens at its address.
    const open = vi.spyOn(window, "open").mockReturnValue(null);
    await act(async () => evidence[1]!.click());
    expect(open).toHaveBeenCalledWith("https://github.com/acme/one/pull/924", "_blank", "noopener,noreferrer");
    await act(async () => evidence[2]!.click());
    expect(open).toHaveBeenCalledTimes(1);
  });

  it("names each improvement's kind on its evidence row in place of the signal (B25)", async () => {
    const at = Date.now();
    await render(suggestionsProject({ rows: [suggestionRow("s1", { basis: { signal: "improvement", tag: "the port rule is written nowhere" }, discomfort: null, evidence: [
      { kind: "session", signal: "improvement", improvement: "no_doc", agent: "claude", session_id: "a", pr_number: null, url: null, at_unix_ms: at, gone: false },
      { kind: "session", signal: "improvement", improvement: "searched_again", agent: "codex", session_id: "b", pr_number: null, url: null, at_unix_ms: at, gone: false },
      { kind: "session", signal: "failed_tool", improvement: null, agent: "codex", session_id: "c", pr_number: null, url: null, at_unix_ms: at, gone: false },
    ] })] }));
    expect(q('[data-suggestion="s1"]')?.textContent).toContain("Agents keep losing time the same way: the port rule is written nowhere");
    await click('[data-suggestion-toggle="s1"]');
    const evidence = [...document.body.querySelectorAll<HTMLButtonElement>("[data-suggestion-evidence]")];
    expect(evidence.map((button) => button.querySelector("[data-suggestion-evidence-improvement]")?.textContent ?? null)).toEqual(["No doc", "Searched again", null]);
    expect(evidence[0]?.textContent).not.toContain("Things to improve");
    expect(evidence[2]?.textContent).toContain("Session · Failed commands and tools");
  });

  it("marks a row being applied by whose turn it is, and offers 다시 시도 and 넘어가기 when it failed (B11, B12, B13)", async () => {
    const started = Date.now() - 120_000;
    await render(suggestionsProject({ rows: [
      suggestionRow("s1", { group: "applying", state: "hide_ai_working", auto: true, attempt: { phase: "working", started_at_unix_ms: started, pr_number: null, url: null, agent_id: null, failure: null } }),
      suggestionRow("s2", { group: "applying", state: "my_turn", attempt: { phase: "pr_open", started_at_unix_ms: started, pr_number: 1711, url: "https://github.com/acme/one/pull/1711", agent_id: null, failure: null } }),
      suggestionRow("s3", { group: "applying", state: "my_turn", attempt: { phase: "ci_failed", started_at_unix_ms: started, pr_number: 1712, url: "https://github.com/acme/one/pull/1712", agent_id: null, failure: null } }),
      suggestionRow("s4", { group: "applying", state: "my_turn", attempt: { phase: "failed", started_at_unix_ms: started, pr_number: null, url: null, agent_id: null, failure: "agent_not_started" } }),
    ] }));
    const working = q('[data-suggestion="s1"]');
    expect(working?.querySelector("[data-suggestion-mark]")?.getAttribute("data-suggestion-mark")).toBe("working");
    expect(working?.querySelector("[data-suggestion-attempt]")?.textContent).toContain("Making PR");
    expect(working?.querySelector("[data-suggestion-auto]")?.textContent).toBe("Auto");
    expect(working?.querySelector(".opacity-\\(--opacity-dimmed\\)")).not.toBeNull();
    expect(q('[data-suggestion="s2"] [data-suggestion-mark]')?.getAttribute("data-suggestion-mark")).toBe("my_turn");
    expect(q('[data-suggestion="s2"] [data-suggestion-attempt]')?.textContent).toContain(`PR ${"#"}1711 review`);
    expect(q('[data-suggestion="s2"] [data-suggestion-retry]')).toBeNull();
    expect(q('[data-suggestion="s3"] [data-suggestion-attempt]')?.textContent).toContain("CI failed");
    expect(q('[data-suggestion="s4"] [data-suggestion-attempt]')?.textContent).toContain("The agent didn't start");
    await click('[data-suggestion-retry="s3"]');
    await click('[data-suggestion="s4"] [data-suggestion-dismiss="s4"]');
    expect(sent("suggestion_action")).toEqual([
      { workspace_id: "p", id: "s3", action: "retry" },
      { workspace_id: "p", id: "s4", action: "dismiss" },
    ]);
  });

  it("words a Memory card's failed way into the documents, with its card id and the core's detail (D-02, B12)", async () => {
    const failed = (failure: string, cause: string | null) => ({ group: "applying" as const, state: "my_turn" as const, source: "memory" as const, cause, attempt: { phase: "failed" as const, started_at_unix_ms: 1, pr_number: null, url: null, agent_id: null, failure } });
    await render(suggestionsProject({ rows: [
      suggestionRow("m8", failed("shell_starting", "[oh-my-zsh] Would you like to update? [Y/n]")),
      suggestionRow("m14", failed("unbuilt", "The backlog screen is not built yet")),
      suggestionRow("m15", failed("card_not_processed", null)),
      suggestionRow("m16", failed("start_failed", null)),
      suggestionRow("m17", failed("a_code_of_a_newer_build", null)),
      suggestionRow("s18", { ...failed("issue_failed", null), source: "notebook", outcome: "issue" }),
    ] }));
    const line = (id: string) => q(`[data-suggestion="${id}"] [data-suggestion-attempt]`)?.textContent;
    expect(line("m8")).toBe("The shell didn't finish starting");
    expect(q('[data-suggestion="m8"] [data-suggestion-failure-detail]')?.textContent).toBe("[oh-my-zsh] Would you like to update? [Y/n]");
    expect(line("m14")).toBe("Not built yet, so the docs were left as they are");
    expect(q('[data-suggestion="m14"] [data-suggestion-failure-detail]')?.textContent).toBe("The backlog screen is not built yet");
    expect(line("m15")).toBe("The agent didn't write this memory in");
    expect(q('[data-suggestion="m15"] [data-suggestion-failure-detail]')).toBeNull();
    expect(line("m16")).toBe("Couldn't start the agent");
    expect(line("m17")).toBe("Ended without a PR");
    expect(line("s18")).toBe("Couldn't open the issue");
    expect(q('[data-suggestion="m8"] [data-suggestion-memory]')?.textContent).toBe("m8");
    await click('[data-suggestion-retry="m14"]');
    expect(sent("suggestion_action")).toEqual([{ workspace_id: "p", id: "m14", action: "retry" }]);
  });

  it("folds 반영됨 with how many worked, and keeps the watch a quiet line inside it (B15)", async () => {
    await render(suggestionsProject({ rows: [
      suggestionRow("s1", { group: "applied", state: "watching", watch: { sessions_seen: 2, sessions_needed: 5 } }),
      suggestionRow("s2", { group: "applied", state: "effective" }),
      suggestionRow("s3", { group: "applied", state: "applied" }),
      suggestionRow("s4", { escalated: { level: 1, previous_pr_url: "https://github.com/acme/one/pull/1720" } }),
    ] }));
    expect(q("[data-suggestions-fold]")?.textContent).toBe("Applied 3· worked 1");
    expect(q('[data-suggestion="s1"]')).toBeNull();
    // It came back after its fix merged: back in 고를 것 with the last PR.
    expect(q('[data-suggestion="s4"] [data-suggestion-escalated]')?.textContent).toContain("The doc didn't do it");
    expect(q('[data-suggestion="s4"] [data-suggestion-escalated]')?.textContent).toContain(`Last PR ${"#"}1720`);
    await click("[data-suggestions-fold]");
    expect(q('[data-suggestion="s1"] [data-suggestion-watch]')?.textContent).toBe("Watching · sessions 2/5");
    expect(q('[data-suggestion="s1"] [data-suggestion-mark]')).toBeNull();
    expect(q('[data-suggestion="s2"] [data-suggestion-effective]')?.textContent).toBe("Worked");
  });

  it("opens on the row a Needs You row named, in 제안 (B11)", async () => {
    await act(async () => useUiStore.getState().showHideAiTab("memory"));
    await act(async () => useUiStore.getState().showHideAiTab("suggestions", "m72"));
    await render(suggestionsProject({ rows: [suggestionRow("s1"), suggestionRow("m72", { source: "memory", group: "applying", state: "my_turn", attempt: { phase: "pr_open", started_at_unix_ms: 1, pr_number: 924, url: "https://github.com/acme/one/pull/924", agent_id: null, failure: null } })] }));
    expect(q('[data-hide-ai-tool-tab="suggestions"]')?.getAttribute("aria-selected")).toBe("true");
    expect(q('[data-suggestion="m72"]')?.getAttribute("data-suggestion-open")).toBe("true");
    expect(q('[data-suggestion="s1"]')?.getAttribute("data-suggestion-open")).toBeNull();
  });

  it("sets how many, how fixes are applied, where it reads and the skipped lines in ⚙ (B5, B13, B16, B19)", async () => {
    const project = suggestionsProject({ ai: "cannot_investigate" });
    // Not on GitHub, with Memory and the turn-end analysis off: GitHub reads nothing, the session records read all but corrections and improvements.
    project.settings.signals = project.settings.signals.map((row) => (
      row.signal === "check_failed" || row.signal === "change_request" ? { ...row, enabled: false, unavailable: "no_github" as const }
        : row.signal === "correction" ? { ...row, enabled: false, unavailable: "memory_off" as const }
        : row.signal === "improvement" ? { ...row, enabled: false, unavailable: "analysis_off" as const } : row
    ));
    project.settings.dismissed = [{ id: "s9", discomfort: "Tab names come out wrong", basis: null, may_return: true }, { id: "s8", discomfort: "Bell too loud", basis: null, may_return: false }];
    await render(project);
    await press("[data-hide-ai-settings-open]");
    const sheet = q("[data-hide-ai-settings]");
    expect(sheet?.querySelector("[data-hide-ai-cannot-investigate]")?.textContent).toBe("This AI can't investigate");
    expect(sheet?.querySelector("[data-hide-ai-provider-order]")?.textContent).toBe("Follows the order in Settings › Hide AI. At a usage limit, the next AI");
    expect(sheet?.querySelector('[data-hide-ai-choice="quantity"]')?.getAttribute("data-hide-ai-choice-value")).toBe("normal");
    expect(sheet?.textContent).toContain("Pressing the button opens a PR or an issue");
    await click("#hide-ai-quantity-few");
    await click("#hide-ai-apply_mode-auto_pr");
    expect([...sheet!.querySelectorAll("[data-hide-ai-signal-group]")].map((row) => row.getAttribute("data-hide-ai-signal-group"))).toEqual(["session", "github"]);
    const session = sheet?.querySelector('[data-hide-ai-signal-group="session"]');
    expect(session?.textContent).toContain("Failed commands · agents stuck · my corrections · problems left over in sessions · things to improve");
    expect(session?.querySelector("[data-hide-ai-correction-off]")?.textContent).toBe("Memory is off, so my corrections aren't read");
    expect(session?.querySelector("[data-hide-ai-analysis-off]")?.textContent).toBe("Things to improve are recorded only while the turn-end analysis is on");
    expect(session?.querySelector("button")?.getAttribute("aria-checked")).toBe("true");
    const github = sheet?.querySelector('[data-hide-ai-signal-group="github"]');
    expect(github?.textContent).toContain("Not a GitHub repository");
    expect(github?.querySelector("button")?.hasAttribute("disabled")).toBe(true);
    expect(github?.querySelector("button")?.getAttribute("aria-checked")).toBe("false");
    await click("#hide-ai-read-p-session");
    await click("[data-hide-ai-dismissed] button");
    expect(sheet?.querySelector("[data-hide-ai-dismissed] > button")?.textContent).toBe("2 skipped suggestions");
    expect([...sheet!.querySelectorAll("[data-hide-ai-dismissed-may-return]")].map((line) => line.textContent)).toEqual(["Comes back once if it piles up again", "Won't come back"]);
    await click('[data-hide-ai-restore="s9"]');
    expect(sent("suggestion_settings")).toEqual([
      { workspace_id: "p", quantity: "few" },
      { workspace_id: "p", apply_mode: "auto_pr" },
      { workspace_id: "p", signal_group: { group: "session", enabled: false } },
    ]);
    expect(sent("suggestion_action")).toEqual([{ workspace_id: "p", id: "s9", action: "restore" }]);
    await click("[data-hide-ai-provider-chip]");
    expect(useUiStore.getState()).toMatchObject({ overlay: "settings", settingsTab: "hideAi" });
  });

  it("names the agent standing in under ⚙ while Runs on is past the limit, and nothing in the head", async () => {
    await render(suggestionsProject(), { ...rest.status!.background_ai!, refusal: { provider: "claude", reason: "past_limit", retry_at_ms: null, using: "codex", used_percent: 93, limit: 90 } });
    expect(q("[data-lane-header]")?.textContent).not.toContain("Codex");
    await press("[data-hide-ai-settings-open]");
    expect(q("[data-hide-ai-provider-chip]")?.textContent).toBe("Codex");
  });
});
