/* global document, getComputedStyle */
// The graph owns pan overflow. Its surrounding controls and each node's text must fit.
export async function measure(page) {
  return page.evaluate(() => {
    const root = document.querySelector('[data-overview-graph]');
    if (!root) throw new Error('Overview graph is missing');
    const overflow = [], partProblems = [], rowsFit = [];
    for (const box of [root, ...root.querySelectorAll('.og-filters, .og-toolbar, .og-omitted, .og-details')]) {
      if (box.scrollWidth > box.clientWidth + 1) overflow.push(`${box.className} scrolls sideways`);
    }
    for (const node of root.querySelectorAll('.og-session-target')) {
      const box = node.getBoundingClientRect();
      for (const part of node.children) {
        const rect = part.getBoundingClientRect();
        if (rect.bottom > box.bottom + 1) rowsFit.push(`${node.getAttribute('aria-label')}: text exceeds node height`);
      }
    }
    for (const orb of root.querySelectorAll('.og-orb')) {
      const rect = orb.getBoundingClientRect();
      if (Math.abs(rect.width - rect.height) > 1 || getComputedStyle(orb).borderRadius !== '50%') partProblems.push('Provider node lost its circle');
    }
    return { overflow, partProblems, rowsFit };
  });
}
