// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { initializeInterfaceI18n } from "./i18n/instance";
import { PaneNoticeBody } from "./PaneNotice";
import type { PaneNotice } from "./snapshot";

afterEach(() => {
  vi.unstubAllGlobals();
  document.body.innerHTML = "";
});

async function draw(notice: PaneNotice) {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  initializeInterfaceI18n("en");
  const container = document.body.appendChild(document.createElement("div"));
  const root = createRoot(container);
  await act(async () => root.render(<PaneNoticeBody notice={notice} />));
  return { container, unmount: async () => { await act(async () => root.unmount()); } };
}

const notice = (kind: string, tone: PaneNotice["tone"] = "muted", extra: Partial<PaneNotice> = {}): PaneNotice => ({ kind, tone, reason: null, since_unix_ms: null, exit_code: null, ...extra });

// docs/UI_BEHAVIOR.md, Agent panes and the Agents explorer: why a terminal is not live is said
// where the terminal would be, never on the second row.
it.each([
  [notice("reconnecting"), "reconnecting…"],
  [notice("helper_not_allowed", "warning"), "helper not allowed · allow it in Settings > Devices"],
  [notice("device_offline", "muted", { reason: "Mac mini" }), "disconnected · Mac mini"],
  [notice("exit", "error", { exit_code: 7 }), "Exit code 7"],
])("says why the terminal is not live: %o", async (value, text) => {
  const { container, unmount } = await draw(value);
  try {
    const body = container.querySelector(`[data-pane-notice="${value.kind}"]`)!;
    expect(body.textContent).toBe(text);
    expect(body.textContent).not.toContain("unavailable");
  } finally { await unmount(); }
});

it("keeps a live terminal readable while another client holds its wheel", async () => {
  const { container, unmount } = await draw(notice("controlled_elsewhere"));
  try {
    const caption = container.querySelector('[data-pane-notice="controlled_elsewhere"]')!;
    expect(caption.textContent).toBe("scroll held by another client");
    // A corner caption, not a cover over the terminal.
    expect(caption.className).not.toContain("inset-0");
  } finally { await unmount(); }
});

it("refuses a notice the core never publishes", async () => {
  vi.spyOn(console, "error").mockImplementation(() => {});
  await expect(draw(notice("result"))).rejects.toThrow("Unknown core pane notice: result");
});
