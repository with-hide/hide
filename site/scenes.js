// The page's moving parts: the Hide AI switch, which flips by itself until the visitor presses it, the agent's
// pointer driving the browser card's page, a letter crossing the machine map, and the Overview map's recording over its still. Under reduced motion,
// and without script, the switch rests on, the pointer stays away and the map shows its still; each plays only
// while it is on screen and the tab is visible.

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

  // ---- The browser: the agent's pointer runs the commands the bubble names, as hide draws them on the page. ----
  const page = document.querySelector(".agent-page");
  if (page) {
    const pointer = page.querySelector(".agent-pointer");
    const command = page.closest(".card").querySelector(".agent-command");
    // Places on the captured page, as fractions of its width and height.
    const STEPS = [
      { say: "hide browser snapshot browser-1", wait: 1400 },
      { say: 'hide browser click browser-1 --text "Total"', x: 0.725, y: 0.385, flash: [0.68, 0.355, 0.27, 0.06] },
      { say: 'hide browser click browser-1 --text "Export CSV"', x: 0.86, y: 0.215, flash: [0.746, 0.177, 0.211, 0.059] },
    ];
    const mark = (kind, style, ms) => {
      const element = document.createElement("span");
      element.className = kind;
      Object.assign(element.style, style);
      page.append(element);
      setTimeout(() => element.remove(), ms);
    };
    const pct = (value) => `${(value * 100).toFixed(2)}%`;
    whileSeen(page, () => {
      let timer = 0;
      let index = 0;
      const later = (ms, next) => { timer = setTimeout(next, ms); };
      const say = (text) => {
        command.classList.add("swapping");
        later(200, () => { command.textContent = text; command.classList.remove("swapping"); });
      };
      const step = () => {
        if (index === STEPS.length) {
          // hide takes its overlay away two seconds after the last action.
          later(2000, () => { pointer.classList.remove("shown"); index = 0; later(1400, step); });
          return;
        }
        const now = STEPS[index++];
        say(now.say);
        if (now.x === undefined) { later(now.wait, step); return; }
        later(260, () => {
          pointer.classList.add("shown");
          pointer.style.left = pct(now.x);
          pointer.style.top = pct(now.y);
          later(820, () => {
            if (!now.flash) { later(now.wait ?? 600, step); return; }
            pointer.classList.add("pressed");
            mark("agent-ripple", { left: pct(now.x), top: pct(now.y) }, 600);
            const [left, top, width, height] = now.flash;
            mark("agent-flash", { left: pct(left), top: pct(top), width: pct(width), height: pct(height) }, 900);
            later(140, () => { pointer.classList.remove("pressed"); later(1300, step); });
          });
        });
      };
      step();
      return () => {
        clearTimeout(timer);
        pointer.classList.remove("shown");
        command.classList.remove("swapping");
        command.textContent = STEPS[STEPS.length - 1].say;
      };
    });
  }

  // ---- Every machine: a letter runs between agents on different machines, always through the core. ----
  const mesh = document.querySelector(".mesh");
  if (mesh) {
    const letter = mesh.querySelector(".letter");
    const caption = document.querySelector(".mesh-caption");
    const route = caption.querySelector(".mesh-route");
    const text = caption.querySelector(".mesh-text");
    const AT = { laptop: [24, 17], server: [76, 17], core: [50, 52], phone: [50, 87] };
    // `data-step` is the index plus one; the stylesheet closes the laptop on 3 and 4 and colors the phone's ask on 4 and 5.
    const STEPS = [
      { route: "Claude Code on MacBook → Codex on Linux server", say: "Run the export tests on the Linux box.", path: ["laptop", "core", "server"] },
      { route: "Codex on Linux server → Claude Code on MacBook", say: "All 128 tests pass. #431 is ready.", path: ["server", "core", "laptop"] },
      { route: "MacBook", say: "The laptop closes. The Mac mini keeps every agent going.", path: [], lit: "core" },
      { route: "hide on Mac mini → your phone", say: "Approve #431 and #436?", path: ["core", "phone"] },
      { route: "You → Claude Code on MacBook", say: "Merge both.", path: ["phone", "core", "laptop"] },
    ];
    const LEG = 700;
    whileSeen(mesh, () => {
      let timer = 0;
      let index = 0;
      const later = (ms, next) => { timer = setTimeout(next, ms); };
      const light = (name) => mesh.querySelectorAll(".node").forEach((node) => node.classList.toggle("lit", node.classList.contains(name)));
      const place = ([x, y]) => { letter.style.left = `${x}%`; letter.style.top = `${y}%`; };
      const step = () => {
        const now = STEPS[index];
        mesh.dataset.step = String(index + 1);
        caption.classList.add("swapping");
        later(250, () => {
          route.textContent = now.route;
          text.textContent = now.say;
          caption.classList.remove("swapping");
          const legs = now.path.slice(1);
          const finish = () => { light(now.lit ?? now.path.at(-1) ?? ""); letter.classList.remove("moving"); index = (index + 1) % STEPS.length; later(2200, step); };
          if (!legs.length) { finish(); return; }
          letter.style.transition = "none";
          place(AT[now.path[0]]);
          letter.getBoundingClientRect();
          letter.style.transition = "";
          letter.classList.add("moving");
          let leg = 0;
          const next = () => {
            if (leg === legs.length) { finish(); return; }
            place(AT[legs[leg++]]);
            later(LEG, next);
          };
          later(120, next);
        });
      };
      step();
      return () => { clearTimeout(timer); letter.classList.remove("moving"); light(""); };
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
