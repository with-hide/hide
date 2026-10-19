/* global document */
// The card and its visible controls must fit the lane without horizontal scrolling.
export async function measure(page) {
  return page.evaluate(() => {
    const root = document.querySelector('[data-gallery-scene="pr-card"]');
    if (!root) throw new Error('PR card scene is missing');
    const overflow = [], partProblems = [];
    for (const box of root.querySelectorAll('article, header, section, [data-pr-card-checks], [data-pr-card-buttons]')) {
      if (box.getClientRects().length && box.scrollWidth > box.clientWidth + 1) overflow.push(`${box.tagName} scrolls sideways`);
    }
    const sections = [...root.querySelectorAll('[data-pr-card] > div > *')].filter((node) => node.getClientRects().length);
    for (let i = 1; i < sections.length; i++) if (sections[i].getBoundingClientRect().top < sections[i - 1].getBoundingClientRect().bottom - 1) partProblems.push('PR sections overlap');
    return { overflow, partProblems, rowsFit: [] };
  });
}
