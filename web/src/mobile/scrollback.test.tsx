// @vitest-environment jsdom
// The phone explains a full-screen terminal only when its own rows frame
// says so; neither the agent's kind nor a missing field is that evidence.
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { clientI18n } from "../i18n/translator";
import { Detail } from "./Detail";
import { applyFrame, patch, usePhone } from "./store";
import type { ServerFrame } from "./protocol";

afterEach(() => vi.unstubAllGlobals());

it("explains alternate rows, opens their conversation, and leaves primary and legacy rows alone", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  await clientI18n.changeLanguage("en");
  const container = document.createElement("div");
  const root = createRoot(container);
  const key = { device_id: "local", pane_id: "w1:p1" };
  const frame = { type: "rows", ...key, state: "ok", text: "Current screen", lines: 200, more: false } satisfies ServerFrame;
  const hello = { type: "hello", mac_name: "Mac", phone_id: "p", name: null, vapid_public_key: "", notifications: "off" } satisfies ServerFrame;
  const hint = () => container.querySelector<HTMLElement>("[data-phone-screen-history]");
  try {
    patch({ connected: true, pageKeys: false, detail: { key, view: "terminal", tab: "terminal", rows: null, moreAskedAt: null, conversation: { messages: [{ id: 1, who: "agent", text: "Earlier answer", truncated: false, at_ms: 0 }], before: null }, olderAskedAt: null, looks: null, look: null } });
    await act(async () => { applyFrame(frame); root.render(<Detail onBack={() => {}} />); });
    expect(hint()).toBeNull(); // An older computer has no screen-mode field.
    expect(container.querySelector('[data-phone-key="page_up"]')).toBeNull();
    await act(async () => applyFrame({ ...hello, page_keys: true }));
    expect(container.querySelector('[data-phone-key="page_up"]')).not.toBeNull();
    expect(container.querySelector('[data-phone-key="page_down"]')).not.toBeNull();
    await act(async () => applyFrame(hello));
    expect(container.querySelector('[data-phone-key="page_up"]')).toBeNull();
    expect(container.querySelector('[data-phone-key="enter"]')).not.toBeNull();
    await act(async () => applyFrame({ ...frame, alternate_screen: false }));
    expect(hint()).toBeNull();
    await act(async () => applyFrame({ ...frame, alternate_screen: true, more: true }));
    expect(hint()).toBeNull(); // There are still rows to load above these.
    await act(async () => applyFrame({ ...frame, alternate_screen: true }));
    expect(hint()?.textContent).toBe("This full-screen program keeps no earlier lines here. See Conversation for earlier content.");
    expect(hint()?.tagName).toBe("BUTTON");
    await act(async () => hint()!.click());
    expect(container.querySelector("[data-phone-conversation]")?.textContent).toContain("Earlier answer");
    await act(async () => {
      patch({ detail: { ...usePhone.getState().detail!, tab: "terminal", view: "terminal", conversation: "none" } });
      applyFrame({ ...frame, alternate_screen: true });
    });
    expect(hint()?.tagName).toBe("P");
    await act(async () => applyFrame(frame));
    expect(hint()).toBeNull(); // A reconnect to an older computer clears it.
  } finally {
    await act(async () => root.unmount());
    patch({ detail: null });
  }
});
