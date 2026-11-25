// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { createActions } from "./actions";
import { TooltipProvider } from "./components/ui/tooltip";
import { clientI18n } from "./i18n/translator";
import { NoticeToasts } from "./Overlays";
import type { StoreAsk } from "./snapshot";
import { storeAskNotice } from "./storeAskNotice";
import { useUiStore } from "./ui";

// An agent's skill or MCP ask stands in the corner with 확인하기, which opens
// that ask's confirm and takes the notice away (docs/UI_BEHAVIOR.md, Notices).
afterEach(() => {
  vi.unstubAllGlobals();
  useUiStore.setState({ notices: [], overlay: "none", storeAskRequested: null });
  document.body.innerHTML = "";
});

const ask: StoreAsk = { id: "ask-1", device_id: "mac", pane_id: "w1:p1", since_unix_ms: 1, what: "install", preview: "p", names: ["structure-check"] };

it("opens the ask's confirm from the notice and takes the notice away", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  await clientI18n.changeLanguage("en");
  useUiStore.getState().showNotice(storeAskNotice(ask, clientI18n.getFixedT(null, "translation")));
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => root.render(<TooltipProvider><NoticeToasts actions={createActions((() => {}) as never)} /></TooltipProvider>));
  try {
    expect(container.querySelector("[data-notice]")?.textContent).toContain("structure-check");
    await act(async () => container.querySelector<HTMLButtonElement>("[data-notice-store-ask='ask-1']")!.click());
    expect(useUiStore.getState().storeAskRequested).toBe("ask-1");
    expect(useUiStore.getState().overlay).toBe("settings");
    expect(useUiStore.getState().notices).toHaveLength(0);
  } finally {
    await act(async () => root.unmount());
  }
});
