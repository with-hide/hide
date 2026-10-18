// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { renderToStaticMarkup } from "react-dom/server";
import { afterAll, expect, it, vi } from "vitest";
import { createActions } from "./actions";
import { AgentMark } from "./AgentMark";
import { AGENT_ADAPTERS, agentAdapter, agentProductName } from "./agentAdapters";
import { monogram } from "./agentLogos";
import manifest from "./assets/agents/manifest.json";
import { PROVIDER_KINDS } from "./agentPicker";
import { TooltipProvider } from "./components/ui/tooltip";
import { LinkTimeline } from "./LinkSessions";
import { disarmKeyTarget } from "./keyTarget";
import { resumeProvider } from "./linkPanel";
import { AgentHead } from "./mobile/parts";
import type { PhoneAgent } from "./mobile/protocol";
import { NO_CHOICE, START_KINDS, selectionOf } from "./mobile/start";
import type { Checkout, LinkPanel, Workspace } from "./snapshot";
import { useShellStore } from "./store";
import type { DispatchFn } from "./ws";
import { emptyScope } from "../test/legacyAgentScope";

// Match the browser's lack of a canvas capability in jsdom, without a probe log.
const canvas = vi.hoisted(() => {
  const original = HTMLCanvasElement.prototype.getContext;
  HTMLCanvasElement.prototype.getContext = () => null;
  return { restore: () => { HTMLCanvasElement.prototype.getContext = original; } };
});
afterAll(() => canvas.restore());

it.each([
  [" CLAUDE-CODE ", "claude"], [" CLAUDE ", "claude"], [" CLAUDE_CODE ", "claude"], [" CODEX ", "codex"],
])("renders %s with its existing branded mark and canonical resume provider", (kind, canonical) => {
  expect(renderToStaticMarkup(<AgentMark kind={kind} />)).toContain(`data-agent-mark="${canonical}"`);
  expect(resumeProvider(kind)).toBe(canonical);
});

// An agent whose mark is withdrawn moves to the manifest's `monogram` list and draws its initials there (docs/BRAND.md); today every adapter has a bundled mark.
const MONOGRAMS: ReadonlySet<string> = new Set((manifest.monogram as { id: string }[]).map((row) => row.id));
it.each(AGENT_ADAPTERS.map((row) => [row.id, row.herdr_kind, row.label]))("draws the logo or monogram of %s, never the neutral mark", (id, herdrKind, label) => {
  const markup = renderToStaticMarkup(<AgentMark kind={herdrKind} />);
  expect(markup).toContain(`data-agent-mark="${herdrKind}"`);
  if (MONOGRAMS.has(id)) expect(markup).toContain(`>${monogram(label)}<`);
  else expect(markup).toContain("<img");
  expect(renderToStaticMarkup(<AgentMark kind={` ${id.toUpperCase()} `} />)).toContain(`data-agent-mark="${herdrKind}"`);
});

// A plain shell has no kind, and a bundled logo that no adapter lists (Gemini CLI's, kept for Hide AI) names no agent Hide supports.
it.each(["future-agent", "gemini-cli", "", null, undefined])("keeps %s neutral", (kind) => {
  const markup = renderToStaticMarkup(<AgentMark kind={kind} />);
  expect(markup).toContain('data-agent-mark="neutral"');
  expect(markup).not.toContain("<img");
});

// A row without a logo names its agent by product, never by the raw Herdr kind (PRD agent-mark-quiet B6, B7).
it.each([["claude", "Claude Code"], [" CLAUDE-CODE ", "Claude Code"], ["codex", "Codex"], ["future-agent", "future-agent"]])("names %s as %s", (kind, name) => {
  expect(agentProductName(kind)).toBe(name);
});
it.each([null, undefined])("has no product name for %s", (kind) => {
  expect(agentProductName(kind)).toBeNull();
});

// The phone's list row leads with the status mark alone; only the detail screen's head draws the logo (agent-mark-quiet B12).
it("draws the provider logo in the phone detail head and not in a list row", () => {
  const agent = { mark: "going", tone: "working", agent_kind: "codex", title: "작업", emphasized: false } as unknown as PhoneAgent;
  expect(renderToStaticMarkup(<AgentHead agent={agent} large={false} />)).not.toContain("data-agent-mark");
  expect(renderToStaticMarkup(<AgentHead agent={agent} large />)).toContain('data-agent-mark="codex"');
});

it.each(["future-agent"])("keeps %s ineligible for resume", (kind) => {
  expect(resumeProvider(kind)).toBeNull();
});

it.each(["grok", "pi", "omp", "cursor"])("enables the completed exact resume reader of %s", (kind) => {
  expect(resumeProvider(` ${kind.toUpperCase()} `)).toBe(kind);
});

it("resumes OpenCode by its proven session id", () => {
  expect(resumeProvider(" OPENCODE ")).toBe("opencode");
});

it("derives all eight supported phone starts and the existing default from the generated projection", () => {
  expect(START_KINDS).toEqual([
    { id: "claude", label: "Claude" }, { id: "codex", label: "Codex" },
    { id: "grok", label: "Grok" }, { id: "opencode", label: "OpenCode" },
    { id: "pi", label: "Pi" }, { id: "omp", label: "omp" },
    { id: "cursor", label: "Cursor" }, { id: "agy", label: "Antigravity" },
  ]);
  expect(START_KINDS.map((row) => row.id)).toEqual(PROVIDER_KINDS);
  expect(START_KINDS).toEqual(AGENT_ADAPTERS.filter((row) => row.can_start).map((row) => ({ id: row.herdr_kind, label: row.picker_label })));
  expect(selectionOf(null, NO_CHOICE).kind).toBe("claude");
  expect(agentAdapter("future-agent")).toBeUndefined();
});

it.each([[" CLAUDE_CODE ", "claude", "Claude Code"], [" CODEX ", "codex", "Codex"], [" GROK ", "grok", "Grok"], [" PI ", "pi", "Pi"], [" OMP ", "omp", "omp"], [" CURSOR ", "cursor", "Cursor"], [" OPENCODE ", "opencode", "OpenCode"], [" AGY ", "agy", "Antigravity"]])("dispatches a canonical resume from the actual %s session button", async (kind, canonical, product) => {
  const saved = useShellStore.getState();
  const checkout = { id: "checkout", workspace_id: "project", path: "/project/task", branch: "task", exists: true } as Checkout;
  const project = { id: "project", device_id: "local", path: "/project", label: "Project", checkouts: [checkout] } as Workspace;
  const panel: LinkPanel = {
    workspace_id: project.id, target: { kind: "pr", number: 1 }, loading: false, failure: null,
    pr: null, prs: [], total: 1, sessions: [{
      agent: kind, id: "session", ids: ["session"], device_id: "local", role: "worked", pr: 1,
      request: "Resume this work", started_at_unix_ms: null, ended_at_unix_ms: null,
      path: "/sessions/session", cwd: checkout.path, file: "present", parent: null,
    }],
  };
  const events: Parameters<DispatchFn>[0][] = [];
  const actions = createActions((event) => { events.push(event); return true; });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  try {
    useShellStore.setState({ rest: { navigator: { workspaces: [project], devices: [{ agent_scope: emptyScope(), id: "local", kind: "local", label: "Local", state: "local", message: null, ssh_alias: null, agent_count: 0, test: null }] } } });
    await act(async () => root.render(<TooltipProvider><LinkTimeline panel={panel} project={project} pr={null} milestones={[]} branchOf={() => "task"} onRetry={() => undefined} actions={actions} /></TooltipProvider>));
    // The row leads with its status mark and the agent's logo, and names the product in its line (PRD record-card-readability D-09, B17).
    expect(container.querySelector("[data-session-row] [data-agent-mark]")).not.toBeNull();
    expect(container.querySelector("[data-session-row-line]")?.textContent).toBe(`${product} · Done`);
    const button = container.querySelector<HTMLButtonElement>('[data-link-button="resume"]');
    expect(button).not.toBeNull();
    expect(button?.getAttribute("aria-disabled")).not.toBe("true");
    await act(async () => button!.click());
    expect(events.filter((event) => event.kind === "agent_start_in_checkout")).toHaveLength(1);
    expect(events.find((event) => event.kind === "agent_start_in_checkout")?.payload).toMatchObject({
      provider: canonical, checkout_path: checkout.path, resume_session_id: "session",
      resume_session_path: "/sessions/session",
    });
  } finally {
    await act(async () => root.unmount());
    container.remove();
    useShellStore.setState(saved, true);
    disarmKeyTarget();
    vi.unstubAllGlobals();
  }
});
