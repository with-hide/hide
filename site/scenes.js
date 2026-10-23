// The page's moving parts: the Overview map's recording, the setup layers and the terminal.
// The markup already holds each one's resting frame, which is what reduced motion and a page without
// script show. A scene plays only while it is on screen and the tab is visible, and stops every timer
// it started when it leaves.

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

  /** Timers that all end together. */
  function clock() {
    const ids = new Set();
    return {
      after(ms, run) { const id = setTimeout(() => { ids.delete(id); run(); }, ms); ids.add(id); },
      every(ms, run) { const id = setInterval(run, ms); ids.add(id); },
      stop() { ids.forEach(id => { clearTimeout(id); clearInterval(id); }); ids.clear(); },
    };
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

  // ---- Whatever you run: one agent and one harness at a time pass through hide into the terminal. ----
  const stack = document.getElementById("stack");
  if (stack) {
    const agents = [...document.querySelectorAll("#opt-agents span")];
    const harness = [...document.querySelectorAll("#opt-harness span")];
    whileSeen(stack, () => {
      const time = clock();
      let turn = 0;
      const pick = () => {
        [...agents, ...harness].forEach(chip => chip.classList.remove("on"));
        agents[turn % agents.length].classList.add("on");
        harness[(turn * 3 + 1) % harness.length].classList.add("on");
        turn += 1;
      };
      pick();
      time.every(1800, pick);
      return () => { time.stop(); [...agents, ...harness].forEach(chip => chip.classList.remove("on")); };
    });
  }

  // ---- The terminal: the command typed as you type it today, and the lead starting its workers. ----
  const body = document.getElementById("term-body");
  if (body) {
    const title = document.getElementById("term-title");
    const full = body.textContent;
    const fullTitle = title.textContent;
    const [prompt, ...rest] = full.split("\n");
    const PROMPT = "~/billing-app (main) $ ";
    whileSeen(body, () => {
      const time = clock();
      const run = () => {
        body.textContent = PROMPT;
        title.textContent = "";
        const caret = Object.assign(document.createElement("span"), { className: "caret" });
        body.append(caret);
        const typed = prompt.slice(PROMPT.length);
        let at = 0;
        const type = () => {
          caret.before(typed[at]);
          at += 1;
          if (at < typed.length) return time.after(90, type);
          time.after(500, () => {
            caret.remove();
            title.textContent = fullTitle;
            rest.forEach((line, index) => time.after(700 * (index + 1), () => body.append(`\n${line}`)));
          });
        };
        time.after(600, type);
      };
      run();
      time.every(11000, run);
      return () => { time.stop(); body.textContent = full; title.textContent = fullTitle; };
    });
  }
})();
