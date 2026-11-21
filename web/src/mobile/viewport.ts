// The phone app's screen height follows what the keyboard leaves of it. The iOS app's web view shrinks by itself when the
// keyboard shows; a browser or a Home Screen app keeps its page as tall and shrinks only the visual viewport, so the page
// takes that height here (`--phone-viewport-height`, read by mobile.css) and a detail's reply bar and last line stay above
// the keyboard on both.

export const VIEWPORT_HEIGHT = "--phone-viewport-height";

/** Follows the visual viewport's height until the returned stop is called; nothing where the browser has none. */
export function followVisualViewport(view: { visualViewport: VisualViewport | null; scrollTo: (x: number, y: number) => void } = window, root: HTMLElement = document.documentElement): () => void {
  const viewport = view.visualViewport;
  if (!viewport) return () => {};
  const follow = () => {
    root.style.setProperty(VIEWPORT_HEIGHT, `${viewport.height}px`);
    // The page is now as tall as what shows, so a browser that scrolled it to bring the field up has nothing left to scroll.
    view.scrollTo(0, 0);
  };
  follow();
  viewport.addEventListener("resize", follow);
  return () => {
    viewport.removeEventListener("resize", follow);
    root.style.removeProperty(VIEWPORT_HEIGHT);
  };
}
