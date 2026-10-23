// The page's one moving picture: the Overview map's recording, over the still that reduced motion
// and a page without script show. It plays only while it is on screen and the tab is visible.

(() => {
  if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) return;

  /** Runs `start` while `element` is on screen and the tab visible; `start` returns the function that stops it. */
  function whileSeen(element, start) {
    let stop = null;
    let inView = false;
    const sync = () => {
      const want = inView && !document.hidden;
      if (want && !stop) stop = start();
      if (!want && stop) { stop(); stop = null; }
    };
    if ("IntersectionObserver" in window) {
      new IntersectionObserver(entries => { inView = entries[0].isIntersecting; sync(); }, { threshold: 0.2 }).observe(element);
    } else {
      inView = true;
    }
    document.addEventListener("visibilitychange", sync);
    sync();
  }

  // ---- The team: the Overview map's recording plays over its still, in the visitor's color scheme. ----
  const map = document.querySelector(".map-video");
  if (map) {
    const light = window.matchMedia("(prefers-color-scheme: light)");
    // H.264 where the browser has it, VP9 where it does not (Chromium builds without the licensed codecs).
    const kind = map.canPlayType('video/mp4; codecs="avc1.640028"') ? ".mp4" : ".webm";
    map.addEventListener("playing", () => { map.hidden = false; });
    whileSeen(map.parentElement, () => {
      const source = (light.matches ? map.dataset.light : map.dataset.dark) + kind;
      if (map.getAttribute("src") !== source) { map.hidden = true; map.src = source; }
      map.play().catch(() => {});
      return () => map.pause();
    });
  }
})();
