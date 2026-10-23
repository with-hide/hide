/* global document, Node, getComputedStyle */
// Compare the visible parts of the sidebar 태스크 list's rows, including
// Korean text and fixed facts. Truncated text is clipped to its visible box;
// a part drawn transparent or not drawn (the row's buttons at rest) is not
// on screen. Under the pointer the buttons take the time's place in the
// line, so they are checked like any other part. Font and contrast remain a
// visual judgment.
export async function measure(page) {
  return page.evaluate(() => {
    const partProblems = [], rowsFit = [], overflow = [];
    const root = document.querySelector('[data-task-list]');
    if (!root) throw new Error('the 태스크 list is missing');
    const shown = (part, row) => {
      for (let node = part; node && node !== row; node = node.parentElement) if (getComputedStyle(node).opacity === '0') return false;
      return part.getClientRects().length > 0;
    };
    for (const row of root.querySelectorAll('[data-task-row], [data-task-child]')) {
      const edge = row.getBoundingClientRect();
      const name = row.dataset.taskRow ?? row.dataset.taskChild;
      const parts = [...row.querySelectorAll('button, [data-mark], [data-hide-mark], span')].filter(part => shown(part, row) && (part.matches('button, [data-mark], [data-hide-mark]') || [...part.childNodes].some(node => node.nodeType === Node.TEXT_NODE && node.textContent.trim())));
      for (const part of parts) {
        const box = part.getBoundingClientRect();
        if (box.right > edge.right + 0.5 || box.left < edge.left - 0.5) overflow.push(`${name}: ${part.textContent} passes its row`);
        if (box.bottom > edge.bottom + 0.5) rowsFit.push(`${name}: ${part.textContent} spills below its row`);
      }
      for (const [index, a] of parts.entries()) for (const b of parts.slice(index + 1)) {
        if (a.contains(b) || b.contains(a)) continue;
        const ra = a.getBoundingClientRect(), rb = b.getBoundingClientRect();
        if (Math.min(ra.right, rb.right) - Math.max(ra.left, rb.left) > 1 && Math.min(ra.bottom, rb.bottom) - Math.max(ra.top, rb.top) > 1) partProblems.push(`${name}: ${a.textContent} overlaps ${b.textContent}`);
      }
    }
    if (root.scrollWidth > root.clientWidth) overflow.push('the 태스크 list scrolls sideways');
    return { partProblems, rowsFit, overflow };
  });
}
