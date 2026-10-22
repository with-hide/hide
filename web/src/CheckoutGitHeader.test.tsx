// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { CheckoutCommit, CheckoutFileCount, CheckoutUpstream } from "./CheckoutGitHeader";
import type { Actions } from "./actions";
import { TooltipProvider } from "./components/ui/tooltip";
import { clientI18n } from "./i18n/translator";
import { useShellStore } from "./store";
import type { CheckoutGitHeader, WorkspaceView } from "./workspace";

vi.hoisted(() => { HTMLCanvasElement.prototype.getContext = () => null; });
const savedShell = useShellStore.getState();
const savedLanguage = clientI18n.language;
afterEach(async () => {
  useShellStore.setState(savedShell, true);
  await clientI18n.changeLanguage(savedLanguage);
  vi.unstubAllGlobals();
  document.body.innerHTML = "";
});

it("removes retained upstream claims offline, restores them live, and names the whole conflict count", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  await clientI18n.changeLanguage("en");
  const header: CheckoutGitHeader = { state: "fresh", reason: null, ci: null, facts: {
    head_sha: "a".repeat(40), subject: "Fixture", commit_unix_seconds: 1,
    changed_files: 5, conflicts: 2, upstream: "origin/main", upstream_state: "tracked", ahead: 3, behind: 1, last_fetch_unix_ms: 1,
  } };
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const showTool = vi.fn();
  try {
    useShellStore.setState({ connection: "live" });
    await act(async () => root.render(<TooltipProvider><CheckoutFileCount header={header} actions={{ showTool } as unknown as Actions} /><CheckoutUpstream header={header} /></TooltipProvider>));
    expect(container.querySelector('[data-checkout-upstream]')?.getAttribute('aria-label')).toBe('origin/main: 3 to send, 1 to receive');
    const count = container.querySelector<HTMLButtonElement>('[data-checkout-files]')!;
    expect(count.textContent).toBe('5');
    expect(count.getAttribute('aria-label')).toContain('5 uncommitted file records · 2 conflicted files');
    await act(async () => count.click());
    expect(showTool).toHaveBeenCalledWith('changes');
    await act(async () => useShellStore.getState().setConnection('reconnecting'));
    expect(container.querySelector('[data-checkout-upstream]')).toBeNull();
    expect(count.getAttribute('aria-label')).toContain('Git state is out of date');
    await act(async () => useShellStore.getState().setConnection('live'));
    expect(container.querySelector('[data-checkout-upstream]')?.getAttribute('aria-label')).toBe('origin/main: 3 to send, 1 to receive');
  } finally {
    await act(async () => root.unmount());
  }
});

const sha = (character: string) => character.repeat(40);

function facts(overrides: Partial<NonNullable<CheckoutGitHeader["facts"]>>): NonNullable<CheckoutGitHeader["facts"]> {
  return { head_sha: sha("a"), subject: "Local work", commit_unix_seconds: 1, changed_files: 0, conflicts: 0, upstream: "origin/main", upstream_state: "tracked", ahead: 0, behind: 0, last_fetch_unix_ms: 5, ...overrides };
}

async function render(node: React.ReactNode) {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  await clientI18n.changeLanguage("en");
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => root.render(<TooltipProvider>{node}</TooltipProvider>));
  return { container, root };
}

it("opens the upstream detail at 0/0 and says when the remote was checked, dimming a failing fetch", async () => {
  useShellStore.setState({ connection: "live" });
  const now = Date.now();
  const header: CheckoutGitHeader = { state: "fresh", reason: null, ci: null, facts: facts({}), remote: { auto: true, observed: null } };
  const { container, root } = await render(<CheckoutUpstream header={header} />);
  try {
    const detail = container.querySelector<HTMLButtonElement>("[data-checkout-upstream]")!;
    expect(detail).not.toBeNull();
    expect(detail.textContent).toBe("");
    await act(async () => detail.click());
    expect(document.querySelector("[data-checkout-remote-checked]")?.textContent).toBe("Remote not checked yet");

    const failing: CheckoutGitHeader = { ...header, facts: facts({ behind: 2 }), remote: { auto: true, observed: { freshness: "failed", read_at_unix_ms: now - 3 * 60_000 } } };
    await act(async () => root.render(<TooltipProvider><CheckoutUpstream header={failing} /></TooltipProvider>));
    const dimmed = container.querySelector<HTMLButtonElement>("[data-checkout-upstream]")!;
    expect(dimmed.getAttribute("data-checkout-remote")).toBe("failing");
    expect(dimmed.className).toContain("opacity-50");
    expect(dimmed.getAttribute("aria-label")).toBe("origin/main: 0 to send, 2 to receive · Couldn't reach the remote · as of 3m ago");

    // With the fetch turned off, the time is the operator's own fetch.
    const off: CheckoutGitHeader = { ...header, remote: { auto: false, observed: null } };
    await act(async () => root.render(<TooltipProvider><CheckoutUpstream header={off} /></TooltipProvider>));
    expect(container.querySelector("[data-checkout-upstream]")?.getAttribute("data-checkout-remote")).toBe("ok");
    expect(document.querySelector("[data-checkout-remote-checked]")?.textContent).toContain("Last remote fetch:");
  } finally {
    await act(async () => root.unmount());
  }
});

it("lists commits not pulled above local ones and opens a pushed commit's GitHub page", async () => {
  const view = { device_id: "device", path: "/repo" } as WorkspaceView;
  const header: CheckoutGitHeader = {
    state: "fresh", reason: null, ci: null, recent_ci: [],
    facts: facts({
      ahead: 1, behind: 7,
      recent_commits: [{ sha: sha("a"), subject: "Local work", commit_unix_seconds: 1 }, { sha: sha("b"), subject: "Pushed work", commit_unix_seconds: 1 }],
      unpushed_commits: [sha("a")],
      incoming_commits: ["c", "d", "e", "f", "1"].map((character) => ({ sha: sha(character), subject: `Theirs ${character}`, commit_unix_seconds: 1 })),
    }),
    remote: { auto: true, observed: { freshness: "fresh", read_at_unix_ms: Date.now() } },
    commit_links: [sha("b"), sha("c")].map((commit) => ({ sha: commit, url: `https://github.com/example/fixture/commit/${commit}` })),
    commit_pages: true,
  };
  useShellStore.setState({ connection: "live", rest: { workspace_view: { ...view, git: header } } as never });
  const openBrowser = vi.fn();
  const { container, root } = await render(<CheckoutCommit header={header} view={view} actions={{ openBrowser } as unknown as Actions} title time={false} />);
  try {
    await act(async () => container.querySelector<HTMLButtonElement>("[data-checkout-commit]")!.click());
    const items = [...document.querySelectorAll<HTMLElement>("[data-commit-history-item]")];
    expect(items.map((item) => item.dataset.commitIncoming === "true")).toEqual([true, true, true, true, true, false, false]);
    expect(document.querySelector("[data-commit-incoming-more]")?.textContent).toBe("+2 more not pulled");
    expect(items[0]!.textContent).toContain("Not pulled");
    expect(items[5]!.textContent).toContain("Not pushed");
    expect(items[5]!.querySelector("[data-commit-link]")).toBeNull();
    expect(items[1]!.querySelector("[data-commit-link]")).toBeNull();
    const pushed = items[6]!.querySelector<HTMLButtonElement>("[data-commit-link]")!;
    expect(pushed.getAttribute("aria-label")).toBe("Pushed work · Open commit on GitHub");
    await act(async () => pushed.click());
    expect(openBrowser).toHaveBeenCalledWith(`https://github.com/example/fixture/commit/${sha("b")}`, { device_id: "device", path: "/repo" });
    await act(async () => items[0]!.querySelector<HTMLButtonElement>("[data-commit-link]")!.click());
    expect(openBrowser).toHaveBeenLastCalledWith(`https://github.com/example/fixture/commit/${sha("c")}`, { device_id: "device", path: "/repo" });
  } finally {
    await act(async () => root.unmount());
  }
});

it("keeps the last CI answer and ↑↓ counts dimmed with their age when a read fails (B22, D-09)", async () => {
  const view = { device_id: "device", path: "/repo" } as WorkspaceView;
  const failed = { freshness: "failed" as const, read_at_unix_ms: Date.now() - 6 * 60_000 };
  const ci = { sha: sha("a"), state: "passing" as const, observed: failed, url: `https://github.com/example/fixture/commit/${sha("a")}` };
  const header: CheckoutGitHeader = { state: "fresh", reason: null, ci, recent_ci: [ci], facts: facts({ behind: 2 }), observed: { freshness: "fresh", read_at_unix_ms: Date.now() } };
  useShellStore.setState({ connection: "live", rest: { workspace_view: { ...view, git: header } } as never });
  const { container, root } = await render(<CheckoutCommit header={header} view={view} actions={{} as Actions} title time={false} />);
  try {
    const mark = container.querySelector<HTMLElement>("[data-checkout-ci]")!;
    expect(mark.dataset.checkoutCi).toBe("passing");
    expect(mark.dataset.checkoutCiCurrent).toBe("false");
    expect(mark.getAttribute("aria-label")).toBe("Commit checks passed · GitHub read failed · value from 6m ago");
    await act(async () => mark.click());
    const row = document.querySelector<HTMLButtonElement>("[data-commit-ci]")!;
    expect(row.dataset.commitCi).toBe("passing");
    expect(row.disabled).toBe(true);
  } finally {
    await act(async () => root.unmount());
  }
  const stale: CheckoutGitHeader = { ...header, state: "stale", observed: { freshness: "failed", read_at_unix_ms: Date.now() - 2 * 60_000 } };
  const upstream = await render(<CheckoutUpstream header={stale} />);
  try {
    const counts = upstream.container.querySelector<HTMLElement>("[data-checkout-upstream]")!;
    expect(counts.className).toContain("opacity-50");
    expect(counts.getAttribute("aria-label")).toContain("Git read failed · value from 2m ago");
  } finally {
    await act(async () => upstream.root.unmount());
  }
});
