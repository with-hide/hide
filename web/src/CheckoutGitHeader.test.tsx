// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { CheckoutFileCount, CheckoutUpstream } from "./CheckoutGitHeader";
import type { Actions } from "./actions";
import { TooltipProvider } from "./components/ui/tooltip";
import { clientI18n } from "./i18n/translator";
import { useShellStore } from "./store";
import type { CheckoutGitHeader } from "./workspace";

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
