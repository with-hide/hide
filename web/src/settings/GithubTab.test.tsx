// @vitest-environment jsdom
import { act, Profiler } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { createActions } from "../actions";
import { TooltipProvider } from "../components/ui/tooltip";
import { useShellStore } from "../store";
import { GithubTab } from "./GithubTab";

// The shell's modules reach xterm, which asks jsdom for a canvas it lacks.
vi.hoisted(() => {
  HTMLCanvasElement.prototype.getContext = () => null;
});

/** A core on `Mac mini` whose `gh` is signed out, with a MacBook node dialing in. */
const signedOut = () => ({
  connection: "live" as const,
  rest: {
    navigator: {
      devices: [
        { id: "local", kind: "local", label: null, machine_name: "Mac mini" },
        { id: "mbp", kind: "remote", label: "MacBook Pro", dials_in: true },
      ],
      workspaces: [
        {
          id: "w1",
          is_git: true,
          is_home: false,
          remote_target_id: null,
          checkouts: [{ id: "c1", github: { failure_category: "not_logged_in", available: false, loading: false, stale: false, last_success_at_unix_ms: null, unavailable_reason: "run gh auth login" } }],
        },
      ],
    },
    status: {},
    ui_state: {},
  },
});

afterEach(() => {
  document.body.innerHTML = "";
  window.location.hash = "";
});

async function mount(next: ReturnType<typeof signedOut>) {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} unobserve() {} });
  const copied: string[] = [];
  const actions = createActions(() => true);
  actions.copyText = (text: string) => { copied.push(text); };
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const saved = useShellStore.getState();
  await act(async () => {
    useShellStore.setState(next as never);
    root.render(<TooltipProvider><GithubTab actions={actions} /></TooltipProvider>);
  });
  return {
    copied,
    q: (selector: string) => container.querySelector(selector) as HTMLElement | null,
    unmount: async () => {
      await act(async () => root.unmount());
      useShellStore.setState(saved, true);
    },
  };
}

it("names the core's machine and the command that signs gh in there, from a window on another machine (B15)", async () => {
  window.location.hash = "#token=t&node=mbp";
  const { q, copied, unmount } = await mount(signedOut());
  // The state reads as a symbol and its words (design 7).
  expect(q("[data-issue-source-github]")?.textContent).toMatch(/Not signed in to gh on Mac mini$/);
  expect(q("[data-github-fix-on]")?.textContent).toContain("Run on Mac mini");
  expect(q("[data-github-fix-command] code")?.textContent).toBe("gh auth login");
  await act(async () => { q("[data-github-fix-command] button")?.click(); });
  expect(copied).toEqual(["gh auth login"]);
  // The command replaces gh's own reason, which would say the same without the machine.
  expect(q("[data-issue-github-reason]")).toBeNull();
  await unmount();
});

it("keeps the gh row as it was on the core's own machine, a node dialing in or not (B1)", async () => {
  const { q, unmount } = await mount(signedOut());
  expect(q("[data-issue-source-github]")?.textContent).toMatch(/Not signed in to gh$/);
  expect(q("[data-github-fix-on]")).toBeNull();
  expect(q("[data-issue-github-reason]")?.textContent).toBe("run gh auth login");
  await unmount();
});

it("draws the page again for a project change, not for a snapshot change it does not read", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} unobserve() {} });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const saved = useShellStore.getState();
  let draws = 0;
  await act(async () => {
    useShellStore.setState(signedOut() as never);
    root.render(
      <Profiler id="github" onRender={() => { draws += 1; }}>
        <TooltipProvider><GithubTab actions={createActions(() => true)} /></TooltipProvider>
      </Profiler>,
    );
  });
  const settled = draws;
  const rest = useShellStore.getState().rest!;
  await act(async () => { useShellStore.setState({ rest: { ...rest, task_operation: { phase: "working" } } } as never); });
  expect(draws).toBe(settled);
  await act(async () => { useShellStore.setState({ rest: { ...rest, navigator: { ...rest.navigator, workspaces: [] } } } as never); });
  expect(draws).toBeGreaterThan(settled);
  await act(async () => root.unmount());
  useShellStore.setState(saved, true);
});
