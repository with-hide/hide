/* global document, CSS, DOMRect, Node, getComputedStyle */
// Where the sidebar's rows and their parts are on screen, measured in the
// page. The e2e specs assert on these (through wire.ts), and the private design
// review tooling, kept outside this checkout, reads them against a baseline's
// rules, so both judge one geometry. Plain JavaScript because that tooling is
// a Node script, not a Playwright spec.

/** Every row the sidebar lists draw, by a stable key. */
const ROWS = [
  ["agent", "li[data-pane]", "data-pane"],
  ["checkout", "[data-checkout-menu]", "data-checkout-menu"],
  ["project", "[data-project-menu]", "data-project-menu"],
  ["no-agent-projects", "[data-no-agent-projects]", "data-no-agent-projects"],
  ["inactive-checkouts", "[data-inactive-checkouts]", "data-inactive-checkouts"],
  ["inactive-projects", "[data-inactive-projects]", "data-inactive-projects"],
];

/** What ends or fills one line of a row: names, chips, badges, times and the fold slot. */
const PARTS = [
  "[data-agent-title]",
  "[data-branch-chip]",
  "[data-device-chip]",
  "[data-descendant-mark]",
  "[data-agent-pr]",
  "[data-agent-elapsed]",
  "[data-fold-slot]",
  "[data-row-name]",
  "[data-checkout-age]",
  "[data-checkout-status]",
  "[data-checkout-toggle]",
  "[data-project-toggle]",
].join(", ");

/**
 * Each visible row's box by key (`agent:a1`, `checkout:<id>`, ...), in the
 * coordinates of its list's scrolled content and rounded to a tenth of a
 * pixel, so scrolling a row into view is not read as the rows moving. Two
 * measurements that differ moved a row.
 */
export async function rowBoxes(page) {
  return page.evaluate((rows) => {
    const boxes = {};
    const round = (value) => Math.round(value * 10) / 10;
    for (const [kind, selector, attribute] of rows) {
      for (const row of document.querySelectorAll(`nav[data-sidebar] ${selector}`)) {
        if (row.getClientRects().length === 0) continue;
        const box = row.getBoundingClientRect();
        const list = row.closest("[data-project-list], [data-raised-list]");
        const origin = list ? list.getBoundingClientRect() : { left: 0, top: 0 };
        const left = box.x - origin.left + (list?.scrollLeft ?? 0);
        const top = box.y - origin.top + (list?.scrollTop ?? 0);
        boxes[`${kind}:${row.getAttribute(attribute)}`] = { x: round(left), y: round(top), width: round(box.width), height: round(box.height) };
      }
    }
    return boxes;
  }, ROWS);
}

/**
 * Each visible row's key, a selector for the row, and a selector for the
 * control a pointer or Tab reaches on it (the button under the whole row, or
 * the row itself when it is the button), so a caller can hover and focus rows
 * one at a time.
 */
export async function rowTargets(page) {
  return page.evaluate((rows) => {
    const targets = [];
    const controls = ["[data-agent-open]", "[data-checkout]", "[data-project-row]"];
    for (const [kind, selector, attribute] of rows) {
      for (const row of document.querySelectorAll(`nav[data-sidebar] ${selector}`)) {
        if (row.getClientRects().length === 0) continue;
        const value = row.getAttribute(attribute) ?? "";
        const base = `nav[data-sidebar] ${selector}[${attribute}="${CSS.escape(value)}"]`;
        // An agent can be drawn twice (its Needs You row and its tree row), so each draw is addressed by its order.
        const index = [...document.querySelectorAll(base)].indexOf(row);
        const rowSelector = `${base} >> nth=${index}`;
        const inner = controls.find((control) => row.querySelector(control));
        targets.push({ key: `${kind}:${value}${index ? `#${index}` : ""}`, row: rowSelector, control: inner ? `${rowSelector} >> ${inner}` : row.matches("button") ? rowSelector : null });
      }
    }
    return targets;
  }, ROWS);
}

/**
 * The status mark and title of every agent row under an opened checkout, with
 * the row's depth: the child step is the mark's (and the title's) distance
 * from the roots of the same list, divided by the depth.
 */
export async function agentColumns(page) {
  return page.evaluate(() => {
    const lists = [];
    for (const list of document.querySelectorAll("nav[data-sidebar] [data-checkout-agents-open]")) {
      const rows = [];
      for (const row of list.querySelectorAll("li[data-pane]")) {
        if (row.getClientRects().length === 0) continue;
        const mark = row.querySelector("[data-agent-status-mark]")?.getBoundingClientRect();
        const title = row.querySelector("[data-agent-title]")?.getBoundingClientRect();
        if (!mark || !title) continue;
        rows.push({ pane: row.getAttribute("data-pane"), depth: Number(row.getAttribute("data-depth") ?? "0"), mark: Math.round(mark.x * 10) / 10, title: Math.round(title.x * 10) / 10 });
      }
      lists.push({ list: list.getAttribute("data-checkout-agents-open") ?? "agents", rows });
    }
    return lists;
  });
}

/**
 * Parts of one row that overlap each other or run past the row's right edge,
 * each as a sentence naming the row and the parts.
 */
export async function rowPartProblems(page) {
  return page.evaluate(
    ({ rows, parts }) => {
      const problems = [];
      const name = (part) =>
        [...part.attributes].find((attribute) => attribute.name.startsWith("data-") && attribute.name !== "data-slot")?.name.slice(5) ?? part.tagName.toLowerCase();
      for (const [kind, selector, attribute] of rows) {
        for (const row of document.querySelectorAll(`nav[data-sidebar] ${selector}`)) {
          if (row.getClientRects().length === 0) continue;
          const edge = row.getBoundingClientRect().right;
          // A part belongs to the nearest row around it, so a checkout's agents are not its parts.
          // A part drawn transparent or hidden (the badge under a hovered row's + and ⋯) shows nothing to overlap.
          const own = [...row.querySelectorAll(parts)].filter(
            (part) => part.checkVisibility({ opacityProperty: true, visibilityProperty: true }) && part.closest(rows.map(([, s]) => s).join(", ")) === row,
          );
          const key = `${kind}:${row.getAttribute(attribute)}`;
          for (const [i, a] of own.entries()) {
            const ra = a.getBoundingClientRect();
            if (ra.right > edge + 0.5) problems.push(`${key}: ${name(a)} runs ${Math.round(ra.right - edge)}px past the row`);
            for (const b of own.slice(i + 1)) {
              if (a.contains(b) || b.contains(a)) continue;
              const rb = b.getBoundingClientRect();
              const across = Math.min(ra.right, rb.right) - Math.max(ra.left, rb.left);
              const down = Math.min(ra.bottom, rb.bottom) - Math.max(ra.top, rb.top);
              if (across > 0.5 && down > 0.5) problems.push(`${key}: ${name(a)} overlaps ${name(b)} by ${Math.round(across * 10) / 10}px`);
            }
          }
        }
      }
      return problems;
    },
    { rows: ROWS, parts: PARTS },
  );
}

/**
 * Where the rows of the list on screen end (PRD sidebar-readability D-3,
 * D-14): the distinct right edges of what ends each line before its fold slot
 * (an agent's elapsed, a checkout's age on either line, a project's or a
 * checkout's status badge with no age after it) and the distinct centres of
 * the fold chevrons, shown or waiting for the pointer. One of each means every
 * row ends on the same grid.
 */
export async function sidebarColumns(page) {
  return page.evaluate(() => {
    const list = "nav[data-sidebar] :is([data-project-list], [data-raised-list])";
    const boxes = (parts) =>
      Array.from(document.querySelectorAll(`${list} :is(${parts})`))
        .filter((part) => part.getClientRects().length > 0)
        .map((part) => part.getBoundingClientRect());
    const distinct = (values) => [...new Set(values.map((value) => Math.round(value)))].sort((a, b) => a - b);
    return {
      times: distinct(
        [
          ...boxes("[data-agent-elapsed], [data-checkout-age]"),
          ...Array.from(document.querySelectorAll(`${list} :is([data-checkout-status])`))
            .filter((badge) => badge.getClientRects().length > 0 && !badge.nextElementSibling?.matches("[data-checkout-age]"))
            .map((badge) => badge.getBoundingClientRect()),
        ].map((box) => box.right),
      ),
      chevrons: distinct(boxes("[data-checkout-toggle], [data-project-toggle]").map((box) => box.left + box.width / 2)),
    };
  });
}

/**
 * The sidebar at a given total width (the content column plus the device rail), and whether anything in it overflows
 * sideways: the list scrolling horizontally, or a row's time or control
 * running past the row's right edge (PRD sidebar-readability B25).
 */
export async function sidebarOverflow(page, width) {
  return page.evaluate((value) => {
    const nav = document.querySelector("nav[data-sidebar]");
    if (!nav) return ["no sidebar"];
    if (value !== null) nav.style.width = value;
    const problems = [];
    for (const list of Array.from(document.querySelectorAll("[data-project-list], [data-raised-list]"))) {
      if (list.scrollWidth > list.clientWidth) problems.push(`list scrolls sideways ${list.scrollWidth} > ${list.clientWidth}`);
    }
    for (const row of Array.from(document.querySelectorAll("[data-pane], [data-checkout-row] > *, [data-project] > *"))) {
      const right = row.getBoundingClientRect().right;
      for (const part of Array.from(row.querySelectorAll("[data-agent-elapsed], [data-checkout-age], [data-checkout-status], button"))) {
        if (part.getBoundingClientRect().right > right + 0.5) problems.push(`${part.textContent ?? part.tagName} passes its row`);
      }
    }
    // A line's leading word and its background tail are never cut; only the sentence between them is (status-marks B27, B29).
    for (const part of Array.from(document.querySelectorAll('nav[data-sidebar] [data-line-part="lead"], nav[data-sidebar] [data-line-part="tail"]'))) {
      const line = part.parentElement;
      if (part.scrollWidth > part.clientWidth + 0.5 || (line && part.getBoundingClientRect().right > line.getBoundingClientRect().right + 0.5)) problems.push(`${part.textContent} is cut`);
    }
    return problems;
  }, width);
}

/**
 * Whether every sidebar row still holds its text at the interface font size
 * in force (PRD sidebar-readability B26): no text runs past the bottom of
 * any box around it up to its list item, which is where a fixed-height row
 * spills, unless that box clips it, and no two of them overlap, which is how
 * a spill shows on screen. A clipping box's own glyphs are judged by eye.
 */
export async function sidebarRowsFit(page) {
  return page.evaluate(() => {
    const problems = [];
    // Every element in the lists that holds text itself (names, lines, places,
    // times, chips), every icon, and every status and provider mark, so a
    // line drawn only in marks is measured too. One drawn transparent or
    // hidden shows nothing: a row's + and ⋯ wait at opacity 0 in its badge's
    // cell until the pointer or the keyboard comes (PRD sidebar-titlebar-refresh D-04).
    const parts = Array.from(document.querySelectorAll("nav[data-sidebar] :is([data-project-list], [data-raised-list]) *")).filter(
      (part) =>
        part.checkVisibility({ opacityProperty: true, visibilityProperty: true }) &&
        (part.matches("svg, [data-mark], [data-hide-mark]") || Array.from(part.childNodes).some((node) => node.nodeType === Node.TEXT_NODE && node.textContent?.trim())),
    );
    const name = (part) => part.textContent?.trim() || part.getAttribute("data-mark") || (part.hasAttribute("data-hide-mark") ? "hide-mark" : null) || part.getAttribute("class") || part.tagName;
    // What of a part can show: a box that clips its overflow (a badge, a
    // truncated label) is the visible edge of the text inside it.
    const shown = new Map();
    for (const part of parts) {
      const rect = DOMRect.fromRect(part.getBoundingClientRect());
      for (let box = part.parentElement; box && box.tagName !== "NAV"; box = box.parentElement) {
        const edge = box.getBoundingClientRect().bottom;
        if (rect.bottom > edge + 0.5) {
          if (getComputedStyle(box).overflowY === "visible") problems.push(`${name(part)} spills below its ${box.tagName.toLowerCase()}`);
          else rect.height = Math.max(0, edge - rect.top);
        }
        if (box.tagName === "LI") break;
      }
      shown.set(part, rect);
    }
    const boxes = [...shown];
    // A section header stuck to the list's top covers the rows that scroll
    // under it by design (D-05), so a header's parts and a row's are not an
    // overlap. In the flow a header sits above its rows, so a row meets one
    // only while it is stuck; its `data-stuck` mark follows a scroll event a
    // frame later, which a measurement straight after a scroll would miss.
    // A header's own text spilling is the spill check's above.
    const inHeader = (part) => part.closest("[data-section-sticky]") !== null;
    for (const [i, [a, ra]] of boxes.entries()) {
      for (const [b, rb] of boxes.slice(i + 1)) {
        if (a.contains(b) || b.contains(a)) continue;
        if (inHeader(a) !== inHeader(b)) continue;
        const across = Math.min(ra.right, rb.right) - Math.max(ra.left, rb.left);
        const down = Math.min(ra.bottom, rb.bottom) - Math.max(ra.top, rb.top);
        if (across > 1 && down > 1) problems.push(`${name(a)} overlaps ${name(b)}`);
      }
    }
    return problems;
  });
}
