/* global document */
// Record sections remain inside a narrow lane. Dialog/palette
// portals are measured independently of the scrollable record beneath them.
export async function measure(page) {
  return page.evaluate(() => {
    const root = document.querySelector('[data-gallery-scene="session-record"]');
    if (!root) throw new Error('Session record scene is missing');
    const overflow=[],partProblems=[],rowsFit=[];
    for (const box of root.querySelectorAll('[data-record-content], [data-session-record], [data-record-flow], [data-record-request], [data-record-ask]')) {
      if (!box.getClientRects().length) continue;
      if (box.scrollWidth > box.clientWidth+1) overflow.push(`${box.getAttribute('data-record-flow') ?? box.getAttribute('data-record-request') ?? box.tagName} scrolls sideways`);
    }
    const sections=[...root.querySelectorAll('[data-record-content] > section')].filter(section=>section.getClientRects().length);
    for (const [i,a] of sections.entries()) for (const b of sections.slice(i+1)) {
      const ra=a.getBoundingClientRect(),rb=b.getBoundingClientRect();
      if (Math.min(ra.bottom,rb.bottom)-Math.max(ra.top,rb.top)>1) partProblems.push('Record sections overlap');
    }
    return {overflow,partProblems,rowsFit};
  });
}
