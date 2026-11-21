// The page's moving parts: the Hide AI switch, which flips by itself until the visitor presses it; the browser
// story, where the spec opens, then the page, then the agent's pointer presses its button; a letter crossing the
// machine map; and the three films over their stills. Under reduced motion, and without script, the switch rests
// on, the browser story shows its last step, the map of machines holds its first letter and each film shows its
// still; each plays only while it is on screen and the tab is visible.

(() => {
  const ai = document.querySelector(".ai-switch");
  const toggle = ai?.querySelector(".toggle");
  let pressed = false;
  const flip = (on) => { ai.dataset.on = String(on); toggle.setAttribute("aria-checked", String(on)); };
  toggle?.addEventListener("click", () => { pressed = true; flip(ai.dataset.on !== "true"); });

  // ---- Hide AI's slides: a press shows that slide and stops the turning for good. ----
  const slides = document.querySelector(".slides");
  const slideButtons = slides ? [...slides.querySelectorAll(".sl-tabs button")] : [];
  let slidesPressed = false;
  let onSlide = () => {};
  const showSlide = (index) => {
    slides.dataset.slide = String(index);
    slideButtons.forEach((button, i) => button.setAttribute("aria-pressed", String(i === index)));
    onSlide(index);
  };
  slideButtons.forEach((button, i) => button.addEventListener("click", () => { slidesPressed = true; showSlide(i); }));

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
    // The stylesheet places each machine (one layout wide, another on a phone), so the letter reads a node's place from it.
    const at = (name) => { const style = getComputedStyle(mesh.querySelector(`.node.${name}`)); return [style.left, style.top]; };
    // `data-step` is the index plus one; the stylesheet closes the laptop on 3 and 4, shows the phone's ask on 4 and its reply on 5.
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
      const place = ([x, y]) => { letter.style.left = x; letter.style.top = y; };
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
          place(at(now.path[0]));
          letter.getBoundingClientRect();
          letter.style.transition = "";
          letter.classList.add("moving");
          let leg = 0;
          const next = () => {
            if (leg === legs.length) { finish(); return; }
            place(at(legs[leg++]));
            later(LEG, next);
          };
          later(120, next);
        });
      };
      step();
      return () => { clearTimeout(timer); letter.classList.remove("moving"); light(""); };
    });
  }

  // ---- The films: each plays over its still while it is on screen and the tab is visible. ----
  // H.264 where the browser has it, VP9 where it does not (Chromium builds without the licensed codecs).
  const load = (film) => {
    const source = film.dataset.src + (film.canPlayType('video/mp4; codecs="avc1.640028"') ? ".mp4" : ".webm");
    if (film.getAttribute("src") !== source) film.src = source;
  };
  document.querySelectorAll(".film-video").forEach((film) => {
    film.addEventListener("playing", () => { film.hidden = false; });
    if (film.dataset.follow) return;
    whileSeen(film.parentElement, () => {
      load(film);
      film.play().catch(() => {});
      return () => film.pause();
    });
  });

  // ---- Chapters: the playing chapter fills its bar; a press jumps the film to that chapter. ----
  document.querySelectorAll(".chapters").forEach((chapters) => {
    const film = chapters.querySelector(".film-video");
    const tabs = [...chapters.querySelectorAll(".ch-tabs button")].map((button) => ({ button, bar: button.querySelector(".ch-bar"), at: Number(button.dataset.at), to: Number(button.dataset.to) }));
    tabs.forEach(({ button, at }) => button.addEventListener("click", () => {
      load(film);
      film.currentTime = at;
      film.play().catch(() => {});
    }));
    whileSeen(chapters, () => {
      let frame = 0;
      const draw = () => {
        const now = film.currentTime;
        tabs.forEach(({ button, bar, at, to }) => {
          const inside = now >= at && now < to;
          button.setAttribute("aria-current", String(inside));
          bar.style.transform = `scaleX(${inside ? (now - at) / (to - at) : 0})`;
        });
        frame = requestAnimationFrame(draw);
      };
      draw();
      return () => cancelAnimationFrame(frame);
    });
  });

  // ---- Hide AI's slides turn by themselves while on screen until the first press; the memory film plays on its slide. ----
  if (slides) {
    const memory = slides.querySelector(".film-video");
    const HOLD = [6500, 6500, 6500, 16000];
    let seen = false;
    onSlide = (index) => {
      const mine = index === 3 && seen;
      if (mine) { load(memory); memory.currentTime = 0; memory.play().catch(() => {}); } else memory.pause();
    };
    whileSeen(slides, () => {
      seen = true;
      let index = Number(slides.dataset.slide);
      let frame = 0;
      let since = performance.now();
      onSlide(index);
      const tick = (now) => {
        const bar = slideButtons[index].querySelector(".ch-bar");
        if (slidesPressed) { slideButtons.forEach((b) => { b.querySelector(".ch-bar").style.transform = "scaleX(0)"; }); index = Number(slides.dataset.slide); }
        else {
          const done = (now - since) / HOLD[index];
          bar.style.transform = `scaleX(${Math.min(done, 1)})`;
          if (done >= 1) { bar.style.transform = "scaleX(0)"; index = (index + 1) % slideButtons.length; since = now; showSlide(index); }
        }
        frame = requestAnimationFrame(tick);
      };
      frame = requestAnimationFrame(tick);
      return () => { seen = false; cancelAnimationFrame(frame); memory.pause(); };
    });
  }
})();
