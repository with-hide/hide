/* global document, getComputedStyle */
// Inspect the real Overview shell. Scrollable tables and the graph canvas
// own their local overflow; neither may widen the page or cover controls.
export async function measure(page) {
  return page.evaluate(() => {
    const root = document.querySelector('[data-overview-page]');
    if (!root) throw new Error('Overview did not render');
    const partProblems = [], rowsFit = [], overflow = [];
    if (root.scrollWidth > root.clientWidth + 1) overflow.push('Overview widens its page');
    const tabs = [...root.querySelectorAll('[data-main-tab]')];
    if (tabs.length !== 5) throw new Error('Overview must render all five tabs');
    const visible = node => node.getClientRects().length && getComputedStyle(node).visibility !== 'hidden';
    const overlap = (a, b) => Math.min(a.right, b.right) - Math.max(a.left, b.left) > .5 && Math.min(a.bottom, b.bottom) - Math.max(a.top, b.top) > .5;
    for (const row of [root.firstElementChild, root.querySelector('[data-overview-navigation]')]) {
      if (!row) continue;
      const controls = [...row.querySelectorAll('button, [role=tab]')].filter(visible);
      controls.forEach((control, index) => {
        const a = control.getBoundingClientRect(), edge = root.getBoundingClientRect();
        if (a.left < edge.left - 1 || a.right > edge.right + 1) rowsFit.push('An Overview header control escapes the page');
        if (controls.slice(index + 1).some(other => overlap(a, other.getBoundingClientRect()))) partProblems.push('Overview header controls overlap');
      });
    }
    const nodes = [...root.querySelectorAll('.react-flow__node-session')].filter(visible);
    nodes.forEach((node, index) => {
      if (nodes.slice(index + 1).some(other => overlap(node.getBoundingClientRect(), other.getBoundingClientRect()))) partProblems.push('Graph sessions overlap');
    });
    for (const section of root.querySelectorAll('[data-overview-project-home], [data-overview-backlog], [data-overview-history]')) {
      if (visible(section) && section.scrollWidth > section.clientWidth + 1) overflow.push('Overview content escapes its local scroll region');
    }
    return { partProblems, rowsFit, overflow };
  });
}
