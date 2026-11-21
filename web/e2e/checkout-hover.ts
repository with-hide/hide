// Read a failed card hover before teardown adds a banner and moves the sidebar.
import type { Locator } from "@playwright/test";
import { dumpOnFailure } from "./failure-dump";

export async function withCheckoutHoverDump(row: Locator, step: () => Promise<void>): Promise<void> {
  const observation = await row.evaluateHandle((original) => {
    const key = original.getAttribute("data-checkout");
    const scope = original.closest("nav[data-sidebar]");
    if (!key || !scope) throw new Error("checkout hover observation needs a sidebar checkout key");
    const seen = new WeakSet<Element>([original]);
    let mounts = 1;
    let pointer: { x: number; y: number } | null = null;
    let previous = "";
    let past = 0;
    const events: unknown[] = [];
    const describe = (node: Element | null) => node && {
      tag: node.tagName,
      checkout: node.closest("[data-checkout]")?.getAttribute("data-checkout") ?? null,
      slot: node.getAttribute("data-slot"),
    };
    const read = () => {
      const target = document.querySelector('[data-checkout="' + CSS.escape(key) + '"]');
      if (target && !seen.has(target)) { seen.add(target); mounts += 1; }
      const rect = target?.getBoundingClientRect();
      return {
        key, mounts, connected: target !== null,
        kind: target?.getAttribute("data-checkout-kind"),
        hint: target?.getAttribute("data-state"), hovered: target?.matches(":hover"),
        rect: rect && { x: rect.x, y: rect.y, width: rect.width, height: rect.height },
        pointer, hit: pointer && describe(document.elementFromPoint(pointer.x, pointer.y)),
        focused: describe(document.activeElement), documentFocused: document.hasFocus(),
        visibility: document.visibilityState, theme: document.documentElement.className,
        dragging: ["data-view-drag", "data-agent-drag"].map((name) => document.documentElement.getAttribute(name)),
        settings: document.querySelector("[data-settings]") !== null,
        card: document.querySelector("[data-checkout-card]")?.getAttribute("data-checkout-card"),
      };
    };
    const capture = (event: string) => {
      const state = read();
      const value = JSON.stringify({ event, ...state });
      if (value === previous) return;
      previous = value;
      events.push({ event, at: performance.now(), wallTime: Date.now(), ...state });
      if (events.length > 32) { events.shift(); past += 1; }
    };
    const changed = (event: Event) => {
      if (event instanceof PointerEvent) pointer = { x: event.clientX, y: event.clientY };
      capture(event.type);
    };
    const types = ["pointermove", "pointerover", "pointerout", "pointerdown", "focusin", "focusout"];
    for (const type of types) document.addEventListener(type, changed, true);
    const observer = new MutationObserver(() => capture("row mutation"));
    observer.observe(scope, {
      subtree: true, childList: true, attributes: true,
      attributeFilter: ["data-state", "data-checkout-kind", "aria-current", "aria-expanded"],
    });
    const theme = new MutationObserver(() => capture("theme mutation"));
    theme.observe(document.documentElement, { attributes: true, attributeFilter: ["class"] });
    capture("begin");
    return {
      read: () => ({ current: read(), past, events }),
      stop: () => {
        observer.disconnect(); theme.disconnect();
        for (const type of types) document.removeEventListener(type, changed, true);
      },
    };
  });
  let failed = false;
  let failure: unknown;
  try {
    await dumpOnFailure("checkout card hover", () => observation.evaluate((state) => state.read()), step);
  } catch (error) {
    failed = true;
    failure = error;
  }
  // Preserve the assertion's error while reporting any independent cleanup failure.
  for (const cleanup of [() => observation.evaluate((state) => state.stop()), () => observation.dispose()]) {
    try { await cleanup(); }
    catch (error) {
      if (failed) console.log("[checkout hover cleanup] " + String(error));
      else { failed = true; failure = error; }
    }
  }
  if (failed) throw failure;
}
