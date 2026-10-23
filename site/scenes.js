// The page's moving parts: the Hide AI switch, which flips by itself until the visitor presses it; the browser
// story, where the spec opens, then the page, then the agent's pointer presses its button; a letter crossing the
// machine map; and the Overview map's recording over its still. Under reduced motion, and without script, the
// switch rests on, the browser story shows its last step, the map of machines holds its first letter and the
// Overview map shows its still; each plays only while it is on screen and the tab is visible.

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

  // ---- The browser: the spec opens, then the page, then the agent's pointer presses Export CSV, as hide draws it. ----
  const stage = document.querySelector(".agent-stage");
  if (stage) {
    const page = stage.querySelector(".agent-page");
    const pointer = page.querySelector(".agent-pointer");
    // The button on the captured window, as fractions of its width and height: where the pointer lands and what flashes.
    const BUTTON = { x: 0.879, y: 0.21, flash: [0.825, 0.181, 0.108, 0.06] };
    const mark = (kind, style, ms) => {
      const element = document.createElement("span");
      element.className = kind;
      Object.assign(element.style, style);
      page.append(element);
      setTimeout(() => element.remove(), ms);
    };
    const pct = (value) => `${(value * 100).toFixed(2)}%`;
    whileSeen(stage, () => {
      let timer = 0;
      const later = (ms, next) => { timer = setTimeout(next, ms); };
      const show = (step) => { stage.dataset.step = String(step); };
      const run = () => {
        pointer.classList.remove("shown");
        pointer.style.left = "62%";
        pointer.style.top = "80%";
        show(1);
        later(2600, () => {
          show(2);
          later(1400, () => {
            pointer.classList.add("shown");
            pointer.style.left = pct(BUTTON.x);
            pointer.style.top = pct(BUTTON.y);
            later(900, () => {
              pointer.classList.add("pressed");
              mark("agent-ripple", { left: pct(BUTTON.x), top: pct(BUTTON.y) }, 600);
              const [left, top, width, height] = BUTTON.flash;
              mark("agent-flash", { left: pct(left), top: pct(top), width: pct(width), height: pct(height) }, 900);
              later(140, () => {
                pointer.classList.remove("pressed");
                later(700, () => {
                  show(3);
                  // hide takes its overlay away two seconds after the last action.
                  later(2000, () => { pointer.classList.remove("shown"); later(2600, run); });
                });
              });
            });
          });
        });
      };
      run();
      return () => { clearTimeout(timer); pointer.classList.remove("shown"); show(3); };
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
