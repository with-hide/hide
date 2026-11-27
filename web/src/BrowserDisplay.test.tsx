// @vitest-environment jsdom
// A browser display in another computer's screen (docs/BROWSER_DISPLAYS.md,
// A page in another computer's screen): with an older desktop app showing
// the screen, which draws no page and opens no window, the notice asks for
// that app's update and offers no button that would do nothing.
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import type { Actions } from "./actions";
import { BrowserDisplay } from "./BrowserDisplay";
import { TooltipProvider } from "./components/ui/tooltip";
import { clientI18n } from "./i18n/translator";
import type { ViewDisplaySnapshot } from "./snapshot";

vi.hoisted(() => { HTMLCanvasElement.prototype.getContext = () => null; });

afterEach(() => {
  vi.unstubAllGlobals();
  document.body.innerHTML = "";
});

async function render(): Promise<HTMLElement> {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  await clientI18n.changeLanguage("en");
  const container = document.createElement("div");
  document.body.append(container);
  const display = { id: "d1", kind: "browser", url: "https://github.com/with-hide/hide/pulls", load: 1 } as unknown as ViewDisplaySnapshot;
  await act(async () => createRoot(container).render(<TooltipProvider><BrowserDisplay display={display} workspace={{ device_id: "mini", path: "/web" }} actions={{} as Actions} /></TooltipProvider>));
  return container;
}

it("asks an older app in another computer's screen for its update, with no dead button", async () => {
  vi.stubGlobal("hideRemote", { bridge: 1, socket: {}, clipboardWrite: async () => true, onMenuCommand: () => {}, onVisible: () => {} });
  const container = await render();
  expect(container.textContent).toContain("Update Hide on this computer to open pages here.");
  expect(container.querySelector("[data-browser-open-external]")).toBeNull();
});

it("keeps the plain tab's notice and its button in a browser tab", async () => {
  const container = await render();
  expect(container.textContent).toContain("Pages open in the Hide desktop app.");
  expect(container.querySelector("[data-browser-open-external]")).not.toBeNull();
});
