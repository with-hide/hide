/** One pending geometry pass per list; the observer owns and cancels its frame. */
export function observeStuckHeaders(element: HTMLElement): () => void {
  let frame: number | null = null;
  const mark = () => {
    frame = null;
    const top = element.getBoundingClientRect().top;
    const marks = [...element.querySelectorAll<HTMLElement>("[data-section-sticky]")].map((header) => {
      const section = header.parentElement!.getBoundingClientRect();
      return { header, stuck: String(section.top < top - 0.5 && section.bottom > top) };
    });
    for (const { header, stuck } of marks) {
      if (header.dataset.stuck !== stuck) header.dataset.stuck = stuck;
    }
  };
  const invalidate = () => {
    frame ??= requestAnimationFrame(mark);
  };
  const sizes = new ResizeObserver(invalidate);
  const watch = () => {
    sizes.disconnect();
    sizes.observe(element);
    for (const child of element.children) sizes.observe(child);
    invalidate();
  };
  watch();
  const rows = new MutationObserver(watch);
  rows.observe(element, { childList: true });
  element.addEventListener("scroll", invalidate, { passive: true });
  return () => {
    element.removeEventListener("scroll", invalidate);
    if (frame !== null) cancelAnimationFrame(frame);
    rows.disconnect();
    sizes.disconnect();
  };
}
