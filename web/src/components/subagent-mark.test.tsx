// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { SubagentMark } from "./subagent-mark";
import { TooltipProvider } from "./ui/tooltip";

// The subagent mark after a row's title (docs/UI_BEHAVIOR.md, Subagents):
// the orbit icon and the count, nothing for none, and the words said once.
afterEach(() => {
  vi.unstubAllGlobals();
  document.body.innerHTML = "";
});

async function render(node: React.ReactNode) {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => root.render(<TooltipProvider>{node}</TooltipProvider>));
  return container;
}

it("draws the count after the orbit icon, one mark whatever the count", async () => {
  const container = await render(<SubagentMark count={12} />);
  const mark = container.querySelector("[data-subagent-mark]")!;
  expect(mark.textContent).toBe("12");
  expect(mark.querySelectorAll("svg")).toHaveLength(1);
  expect(mark.getAttribute("aria-label")).toBe("12 subagents working");
});

it("draws nothing for none, which is also what an unreadable count reaches it as", async () => {
  for (const count of [0, undefined]) {
    const container = await render(<SubagentMark count={count} />);
    expect(container.querySelector("[data-subagent-mark]")).toBeNull();
  }
});

it("leaves the words to a row's own name inside a row", async () => {
  const container = await render(<SubagentMark count={2} inRow />);
  const mark = container.querySelector("[data-subagent-mark]")!;
  expect(mark.getAttribute("aria-hidden")).toBe("true");
  expect(mark.getAttribute("aria-label")).toBeNull();
});
