// Temporary observation for Windows repetition; removed before delivery.
import { test } from "@playwright/test";
let records: unknown[] = [];
test.beforeEach(async ({ page }) => {
  if (!test.info().title.startsWith("a checkout's pull request:")) return;
  records = [];
  await page.exposeFunction("recordCheckoutHover", (state: unknown) => { records.push(state); if (records.length > 128) records.shift(); });
  await page.addInitScript(() => {
    let previous = "";
    let sequence = 0;
    const identities = new WeakMap<Element, number>();
    let pointer = { x: 0, y: 0 };
    const describe = (node: Element | null) => node && { tag: node.tagName, checkout: node.closest("[data-checkout]")?.getAttribute("data-checkout"), slot: node.getAttribute("data-slot"), state: node.getAttribute("data-state"), themeOption: node.getAttribute("data-theme-option") };
    const capture = (event: string) => {
      const row = document.querySelector('[data-checkout][aria-label^="feature/pr-card"]');
      if (row && !identities.has(row)) identities.set(row, ++sequence);
      const rect = row?.getBoundingClientRect();
      const state = { theme: document.documentElement.className, mount: row && identities.get(row), mounts: sequence, key: row?.getAttribute("data-checkout"), kind: row?.getAttribute("data-checkout-kind"), hint: row?.getAttribute("data-state"), hovered: row?.matches(":hover"), rect: rect && { x: rect.x, y: rect.y, width: rect.width, height: rect.height }, pointer, hit: describe(document.elementFromPoint(pointer.x, pointer.y)), focused: describe(document.activeElement), card: document.querySelector("[data-checkout-card]")?.getAttribute("data-checkout-card"), settings: document.querySelector("[data-settings]")!==null };
      const key = JSON.stringify(state);
      if (key !== previous) { previous = key; (window as Window & { recordCheckoutHover: (state: unknown) => void }).recordCheckoutHover({event, t: performance.now(), ...state}); }
    };
    (window as Window & { captureCheckoutHover?: () => void }).captureCheckoutHover = () => capture("read");
    for (const type of ["pointermove", "pointerover", "pointerout", "pointerdown", "focusin", "focusout"]) document.addEventListener(type, (event) => { if (event instanceof PointerEvent) pointer = { x: event.clientX, y: event.clientY }; capture(type); }, true);
    const bootstrap = new MutationObserver(() => {
      const nav = document.querySelector("nav[data-sidebar]");
      if (!nav) return;
      bootstrap.disconnect();
      new MutationObserver(() => capture("row mutation")).observe(nav, {subtree:true,childList:true,attributes:true,attributeFilter:["data-state","data-checkout-kind","aria-current","aria-expanded"]});
      new MutationObserver(() => capture("theme mutation")).observe(document.documentElement,{attributes:true,attributeFilter:["class"]});
      capture("mounted");
    });
    bootstrap.observe(document,{subtree:true,childList:true});
  });
});
test.afterEach(async ({ page }) => {
  if (!test.info().title.startsWith("a checkout's pull request:")) return;
  await page.evaluate(() => (window as Window & { captureCheckoutHover?: () => void }).captureCheckoutHover?.());
  console.log("[checkout hover observation] " + JSON.stringify(records));
  if (test.info().status !== test.info().expectedStatus) await test.info().attach("checkout hover observation", { body: JSON.stringify(records), contentType: "application/json" });
});
