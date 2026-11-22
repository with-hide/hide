// @vitest-environment jsdom
import { act, Profiler } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { createActions } from "../actions";
import { TooltipProvider } from "../components/ui/tooltip";
import { useShellStore } from "../store";
import { RemoteRepositoriesTab } from "./RemoteRepositoriesTab";

// The shell's modules reach xterm, which asks jsdom for a canvas it lacks.
vi.hoisted(() => {
  HTMLCanvasElement.prototype.getContext = () => null;
});

// The declarations the core's units make (`code_host::declaration`).
const GITHUB = { key: "github", name: "GitHub", operations: ["merge", "update_branch", "issues", "handover", "search", "watch"], access: { program: "gh", minimum_version: "2.40.0", sign_in: "gh auth login" } };
const GITLAB = { key: "gitlab", name: "GitLab", operations: ["commit_checks"], access: { program: "glab", minimum_version: "1.78.0", sign_in: "glab auth login --hostname gitlab.com" } };

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
          remote_target_id: null,
          checkouts: [{ id: "c1", github: { failure_category: "not_logged_in", available: false, loading: false, stale: false, last_success_at_unix_ms: null, host: GITHUB } }],
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
  const actions = createActions(() => true, { write: async (text) => { copied.push(text); return true; }, read: async () => null });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const saved = useShellStore.getState();
  await act(async () => {
    useShellStore.setState(next as never);
    root.render(<TooltipProvider><RemoteRepositoriesTab actions={actions} /></TooltipProvider>);
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
  // gh's own words stay in the core's log; the row says the state.
  expect(q("[data-issue-github-reason]")).toBeNull();
  await unmount();
});

it("offers the sign-in command on the core's own machine too, a node dialing in or not (B1)", async () => {
  const { q, copied, unmount } = await mount(signedOut());
  expect(q("[data-issue-source-github]")?.textContent).toMatch(/Not signed in to gh$/);
  expect(q("[data-github-fix-on]")?.textContent).toContain("Run in a terminal");
  expect(q("[data-github-fix-command] code")?.textContent).toBe("gh auth login");
  await act(async () => { q("[data-github-fix-command] button")?.click(); });
  expect(copied).toEqual(["gh auth login"]);
  expect(q("[data-issue-github-reason]")).toBeNull();
  await unmount();
});

it("draws GitLab's row beside GitHub's only once a gitlab.com project was read, each with its own state and sign-in command", async () => {
  const github = signedOut();
  github.rest.navigator.devices = [{ id: "local", kind: "local", label: null, machine_name: "Mac mini" }] as never;
  github.rest.navigator.workspaces[0]!.checkouts[0]!.github = { failure_category: null, available: true, loading: false, stale: false, last_success_at_unix_ms: 1, host: GITHUB } as never;
  const alone = await mount(github);
  expect([...document.querySelectorAll("[data-settings-host]")].map((row) => row.getAttribute("data-settings-host"))).toEqual(["github"]);
  await alone.unmount();
  document.body.innerHTML = "";
  const both = signedOut();
  both.rest.navigator.devices = github.rest.navigator.devices;
  both.rest.navigator.workspaces = [
    github.rest.navigator.workspaces[0]!,
    { id: "w2", is_git: true, remote_target_id: null, checkouts: [{ id: "c2", github: { failure_category: "not_logged_in", available: false, loading: false, stale: false, last_success_at_unix_ms: null, host: GITLAB } }] },
  ] as never;
  const { q, copied, unmount } = await mount(both);
  expect(q("[data-settings-host=github] [data-issue-source-github]")?.textContent).toContain("Connected");
  expect(q("[data-settings-host=gitlab]")?.textContent).toContain("GitLab");
  expect(q("[data-settings-host=gitlab] [data-issue-source-github]")?.textContent).toMatch(/Not signed in to glab$/);
  expect(q("[data-settings-host=gitlab] [data-github-fix-command] code")?.textContent).toBe("glab auth login --hostname gitlab.com");
  await act(async () => q("[data-settings-host=gitlab] [data-github-fix-command] button")!.click());
  expect(copied).toEqual(["glab auth login --hostname gitlab.com"]);
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
        <TooltipProvider><RemoteRepositoriesTab actions={createActions(() => true)} /></TooltipProvider>
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
