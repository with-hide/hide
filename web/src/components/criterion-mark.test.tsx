// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { clientI18n } from "../i18n/translator";
import type { MissionCriterionState } from "../snapshot";
import { CriterionMark } from "./criterion-mark";
import { TooltipProvider } from "./ui/tooltip";

// A finish line's mark as docs/UI_BEHAVIOR.md (Mission map, the panel)
// describes it: one glyph per state, whose hover and accessible name say the
// same state word.
afterEach(async () => {
  await clientI18n.changeLanguage("en");
  vi.unstubAllGlobals();
  document.body.innerHTML = "";
});

it("names each of the four states the same way to a pointer and to a screen reader", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  await clientI18n.changeLanguage("ko");
  const states: MissionCriterionState[] = ["pass", "open", "skipped", "fail"];
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => root.render(<TooltipProvider delayDuration={0}>{states.map((state) => <CriterionMark key={state} state={state} />)}</TooltipProvider>));
  try {
    const seen: [string, string | null, string | null | undefined][] = [];
    for (const mark of container.querySelectorAll<HTMLElement>("[data-criterion-mark]")) {
      await act(async () => {
        mark.dispatchEvent(new MouseEvent("pointermove", { bubbles: true }));
        await new Promise((done) => setTimeout(done, 0));
      });
      seen.push([mark.textContent!, mark.getAttribute("aria-label"), document.querySelector("[role=tooltip]")?.textContent]);
      await act(async () => mark.dispatchEvent(new MouseEvent("pointerleave", { bubbles: true })));
    }
    expect(seen).toEqual([["✓", "통과", "통과"], ["○", "아직", "아직"], ["⤳", "넘어감", "넘어감"], ["✕", "실패", "실패"]]);
  } finally { await act(async () => root.unmount()); }
});
