// The page's moving parts: the Hide AI switch, which flips by itself until the visitor presses it, and the
// Overview map's recording over its still. Under reduced motion, and without script, the switch rests on
// and the map shows its still; each plays only while it is on screen and the tab is visible.

(() => {
  const ai = document.querySelector(".ai-switch");
  const toggle = ai?.querySelector(".toggle");
  let pressed = false;
  const flip = (on) => { ai.dataset.on = String(on); toggle.setAttribute("aria-checked", String(on)); };
  toggle?.addEventListener("click", () => { pressed = true; flip(ai.dataset.on !== "true"); });

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

  // ---- Hide AI: off, then on, every few seconds, until the first press. ----
  if (ai) {
    whileSeen(ai, () => {
      if (pressed) return () => {};
      flip(false);
      const timer = setInterval(() => { if (pressed) clearInterval(timer); else flip(ai.dataset.on !== "true"); }, 2600);
      return () => { clearInterval(timer); if (!pressed) flip(true); };
    });
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
