/* global document, getComputedStyle */
// Where the core move's screens put their parts, measured in the page for the
// private design review tooling, kept outside this checkout, which judges them against
// the rules a baseline states: the Devices rows and their menus, the move
// dialog, the strip above the window and the device rail. Expected values come
// from the baseline, never from these measurements. Plain JavaScript because
// that tooling is a Node script, not a Playwright spec.

const SCENE = "[data-gallery-scene^=\"core-move\"]";

/** The boxes whose direct children share one line: a part of one must not overlap another or pass the box's edge. */
const LINES = "[data-device-row] > div, [data-core-move-check], [data-core-move-step], [data-connection], [role=menuitem]";

const describe = (element) => {
  const own = [...element.attributes].find((attribute) => attribute.name.startsWith("data-") || attribute.name === "aria-label");
  return own ? `${own.name}${own.value && own.value !== "true" ? `=${own.value}` : ""}` : element.tagName.toLowerCase();
};

/** Parts of one line that overlap each other or run past the line's right edge. */
async function partProblems(page) {
  return page.evaluate(
    ({ lines, describeSource }) => {
      const describe = new Function(`return ${describeSource}`)();
      const problems = [];
      for (const line of document.querySelectorAll(lines)) {
        if (line.getClientRects().length === 0) continue;
        const edge = line.getBoundingClientRect().right;
        const parts = [...line.children].filter((part) => part.getClientRects().length > 0);
        for (const [i, a] of parts.entries()) {
          const ra = a.getBoundingClientRect();
          if (ra.right > edge + 0.5) problems.push(`${describe(line)}: ${describe(a)} runs ${Math.round(ra.right - edge)}px past the line`);
          for (const b of parts.slice(i + 1)) {
            const rb = b.getBoundingClientRect();
            const across = Math.min(ra.right, rb.right) - Math.max(ra.left, rb.left);
            const down = Math.min(ra.bottom, rb.bottom) - Math.max(ra.top, rb.top);
            if (across > 0.5 && down > 0.5) problems.push(`${describe(line)}: ${describe(a)} overlaps ${describe(b)} by ${Math.round(across * 10) / 10}px`);
          }
        }
      }
      return problems;
    },
    { lines: LINES, describeSource: describe.toString() },
  );
}

/** Text that is cut off without the ellipsis a truncating box draws, so it is lost rather than shortened. */
async function textSpills(page) {
  return page.evaluate((scene) => {
    const problems = [];
    for (const part of document.querySelectorAll(`${scene} *, [role=dialog] *, [role=menu] *`)) {
      if (part.getClientRects().length === 0 || part.children.length > 0) continue;
      if (!part.textContent?.trim()) continue;
      const style = getComputedStyle(part);
      const truncates = style.textOverflow === "ellipsis" || style.webkitLineClamp !== "none";
      if (part.scrollWidth > part.clientWidth + 1 && !truncates && style.overflowX !== "visible") problems.push(`${part.textContent.trim().slice(0, 40)} is cut off`);
    }
    return problems;
  }, SCENE);
}

/** Whether the scene scrolls sideways, or a control, a dialog or a menu sits past the window's right edge. */
async function overflow(page) {
  return page.evaluate((scene) => {
    const root = document.querySelector(scene);
    if (!root) return ["no core move scene"];
    const problems = [];
    if (document.documentElement.scrollWidth > document.documentElement.clientWidth + 1) problems.push(`the page scrolls sideways ${document.documentElement.scrollWidth} > ${document.documentElement.clientWidth}`);
    const edge = root.getBoundingClientRect().right;
    for (const part of document.querySelectorAll(`${scene} button, [role=dialog], [role=menu]`)) {
      if (part.getClientRects().length > 0 && part.getBoundingClientRect().right > edge + 0.5) problems.push(`${part.textContent?.trim().slice(0, 30) || part.tagName} passes the window's edge`);
    }
    return problems;
  }, SCENE);
}

export async function measure(page) {
  return { partProblems: await partProblems(page), rowsFit: await textSpills(page), overflow: await overflow(page) };
}
