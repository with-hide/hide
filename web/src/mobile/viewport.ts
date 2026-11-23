// The phone app's screen height follows what the keyboard leaves of it. The iOS app's web view shrinks by itself when the
// keyboard shows; a browser or a Home Screen app keeps its page as tall and shrinks only the visual viewport, so the page
// takes that height here (`--phone-viewport-height`, read by mobile.css) and a detail's reply bar and last line stay above
// the keyboard on both.
//
// The visual viewport also changes while a list scrolls: Safari and Chrome fold their toolbar away as the page moves and
// bring it back as it returns. That scroll is the reader's, and only the keyboard's own move of the page is taken back.
//
// While a field being written in has the keyboard, the page stays at its top whenever the browser moves it: the page is as
// tall as what shows, so any move is the browser bringing the field up, and it lands before, with or after the resize (Safari
// often pans the visual viewport a few hundred milliseconds after it), so every move is answered, not only the resize.

export const VIEWPORT_HEIGHT = "--phone-viewport-height";

// The fields a phone writes in with its keyboard; a check box, a button or a range raises none.
const WRITING = "textarea, [contenteditable]:not([contenteditable='false']), input:not([type]), input[type='text'], input[type='search'], input[type='email'], input[type='url'], input[type='tel'], input[type='password'], input[type='number']";

/** Whether the keyboard is up for a field of this page: one it writes in has the focus. */
function writing(page: Document): boolean {
  return page.activeElement instanceof Element && page.activeElement.matches(WRITING);
}

type View = Pick<Window, "visualViewport" | "scrollTo" | "addEventListener" | "removeEventListener">;

/** Follows the visual viewport's height until the returned stop is called; nothing where the browser has none. */
export function followVisualViewport(view: View = window, root: HTMLElement = document.documentElement): () => void {
  const viewport = view.visualViewport;
  if (!viewport) return () => {};
  // With no field being written in, the page's scroll is the reader's and stays.
  const holdTop = () => {
    if (writing(root.ownerDocument)) view.scrollTo(0, 0);
  };
  const follow = () => {
    root.style.setProperty(VIEWPORT_HEIGHT, `${viewport.height}px`);
    holdTop();
  };
  follow();
  viewport.addEventListener("resize", follow);
  // Safari pans the visual viewport without scrolling the page, and Chrome scrolls the page: each says so on its own target.
  viewport.addEventListener("scroll", holdTop);
  view.addEventListener("scroll", holdTop);
  return () => {
    viewport.removeEventListener("resize", follow);
    viewport.removeEventListener("scroll", holdTop);
    view.removeEventListener("scroll", holdTop);
    root.style.removeProperty(VIEWPORT_HEIGHT);
  };
}
