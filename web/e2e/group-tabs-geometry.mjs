/* global document, getComputedStyle */
// Production control bounds for design-review. Tabs may scroll inside their
// own strip; the start form and the enclosing workspace must fit the window.
export async function measure(page) {
  return page.evaluate(() => {
    const scene = document.querySelector('[data-gallery-scene="group-tabs"]');
    if (!scene) throw new Error('Group tabs scene is missing');
    const partProblems = [], rowsFit = [], overflow = [];
    const visible = node => node.getClientRects().length > 0;
    const name = node => node.getAttribute('aria-label') || node.textContent?.trim().slice(0, 50) || node.tagName;
    const form = scene.querySelector('[data-agent-tab-start]');
    if (form) {
      const edge = form.getBoundingClientRect();
      const controls = [...form.querySelectorAll('button, textarea')].filter(visible);
      for (const [index, control] of controls.entries()) {
        const box = control.getBoundingClientRect();
        if (box.left < edge.left - 1 || box.right > edge.right + 1) overflow.push(`${name(control)} passes the form edge`);
        if (box.width < 1 || box.height < 1) rowsFit.push(`${name(control)} has no visible bounds`);
        for (const other of controls.slice(index + 1)) {
          const next = other.getBoundingClientRect();
          if (Math.min(box.right, next.right) - Math.max(box.left, next.left) > 1 && Math.min(box.bottom, next.bottom) - Math.max(box.top, next.top) > 1) partProblems.push(`${name(control)} overlaps ${name(other)}`);
        }
      }
      const terminal = form.querySelector('[data-group-start-terminal]');
      if (!terminal || !visible(terminal)) rowsFit.push('Terminal start is not visible');
    }
    for (const selector of ['[data-agent-group-selector]', '[data-agent-tab-start]']) {
      for (const node of scene.querySelectorAll(selector)) {
        if (!visible(node)) continue;
        if (node.scrollWidth > node.clientWidth + 1 && getComputedStyle(node).overflowX !== 'auto') overflow.push(`${selector} scrolls sideways`);
      }
    }
    const box = scene.getBoundingClientRect();
    if (scene.scrollWidth > scene.clientWidth + 1) overflow.push('Workspace scrolls sideways');
    if (box.left < -1 || box.right > document.documentElement.clientWidth + 1) overflow.push('Workspace passes the window edge');
    return { partProblems, rowsFit, overflow };
  });
}
