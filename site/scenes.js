// The page's drawn scenes: the team's letters, the setup layers, the terminal and the memory timeline.
// The markup already holds each scene's resting frame, which is what reduced motion and a page without
// script show. A scene plays only while it is on screen and the tab is visible, and stops every timer
// it started when it leaves.

(() => {
  // A phone shows the team's drawing at its own size, scrolled to the middle where the lead and the person are.
  const teamFrame = document.querySelector(".team-canvas");
  if (teamFrame) teamFrame.scrollLeft = (teamFrame.scrollWidth - teamFrame.clientWidth) / 2;

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

  // ---- The team: letters between parents and children, and only decisions over the line. ----
  const team = document.getElementById("team-tree");
  if (team) {
    const NS = "http://www.w3.org/2000/svg";
    const node = id => team.querySelector(`.t-node[data-id="${id}"]`);
    const edge = id => team.querySelector(`[data-edge="${id}"]`);
    const up = id => team.querySelector(`[data-up="${id}"]`);
    const letters = team.querySelector(".t-letters");
    const rows = document.getElementById("you-rows");
    const count = document.getElementById("you-count");
    const calm = document.getElementById("you-calm");
    const kids = [...team.querySelectorAll("[data-edge]")].map(path => path.dataset.edge);
    const resting = [...team.querySelectorAll(".t-node")].map(element => [element, element.dataset.state]);
    const restingRows = rows.innerHTML;
    const asks = [];
    let flying = [];

    const svg = (tag, attrs, parent) => {
      const element = document.createElementNS(NS, tag);
      for (const [key, value] of Object.entries(attrs)) element.setAttribute(key, value);
      parent?.appendChild(element);
      return element;
    };
    const setState = (id, state) => { node(id).dataset.state = state; };

    function drawYou() {
      rows.replaceChildren();
      asks.forEach((ask, at) => {
        const row = svg("g", { transform: `translate(14 ${46 + at * 36})` }, rows);
        svg("rect", { class: "t-row", width: 412, height: 30, rx: 8 }, row);
        svg("circle", { class: "t-row-dot", cx: 16, cy: 15, r: 5 }, row);
        svg("text", { class: "t-row-title", x: 30, y: 19.5 }, row).textContent = ask.title;
        svg("text", { class: "t-row-agent", x: 398, y: 19.5, "text-anchor": "end" }, row).textContent = node(ask.id).querySelector(".t-name").textContent;
      });
      count.textContent = `NEEDS YOU · ${asks.length}`;
      calm.setAttribute("display", asks.length ? "none" : "inline");
    }

    function send(path, reverse, ask, ms, done) {
      const letter = svg("use", { href: ask ? "#t-envq" : "#t-env" }, letters);
      flying.push({ path, length: path.getTotalLength(), reverse, start: performance.now(), ms, letter, done });
    }

    let frame = 0;
    function fly(now) {
      flying = flying.filter(item => {
        const p = Math.min(1, (now - item.start) / item.ms);
        const eased = p < 0.5 ? 2 * p * p : 1 - (-2 * p + 2) ** 2 / 2;
        const point = item.path.getPointAtLength(item.length * (item.reverse ? 1 - eased : eased));
        item.letter.setAttribute("transform", `translate(${point.x} ${point.y})`);
        if (p < 1) return true;
        item.letter.remove();
        item.done?.();
        return false;
      });
      frame = requestAnimationFrame(fly);
    }

    const toYou = (id, title) => { setState(id, "blocked"); send(up(id), false, true, 1500, () => { asks.push({ id, title }); drawYou(); }); };
    const answer = id => {
      const at = asks.findIndex(ask => ask.id === id);
      if (at >= 0) asks.splice(at, 1);
      drawYou();
      send(up(id), true, false, 1400, () => setState(id, "working"));
    };
    const askParent = (id, time) => {
      setState(id, "ask");
      send(edge(id), true, true, 1500, () => time.after(500, () => send(edge(id), false, false, 1300, () => setState(id, "working"))));
    };
    const report = id => { setState(id, "done"); send(edge(id), true, false, 1400); };

    whileSeen(team, () => {
      const time = clock();
      resting.forEach(([element]) => { element.dataset.state = "working"; });
      asks.length = 0;
      drawYou();
      frame = requestAnimationFrame(fly);
      time.every(700, () => {
        const id = kids[Math.floor(Math.random() * kids.length)];
        send(edge(id), Math.random() < 0.5, false, 1700 + Math.random() * 900);
      });
      const story = () => {
        time.after(400, () => askParent("c3", time));
        time.after(2600, () => toYou("g3", "Blocked: allow a push to main?"));
        time.after(4600, () => report("g1"));
        time.after(6200, () => askParent("g2", time));
        time.after(7600, () => toYou("lead", "Approve #431 and #436?"));
        time.after(10200, () => answer("g3"));
        time.after(12000, () => answer("lead"));
        time.after(12600, () => setState("g1", "working"));
      };
      story();
      time.every(14000, story);
      return () => {
        time.stop();
        cancelAnimationFrame(frame);
        flying.forEach(item => item.letter.remove());
        flying = [];
        asks.length = 0;
        resting.forEach(([element, state]) => { element.dataset.state = state; });
        rows.innerHTML = restingRows;
        count.textContent = "NEEDS YOU · 2";
        calm.setAttribute("display", "none");
      };
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

  // ---- Say it once: the sessions light up left to right, then the whole row rests. ----
  const timeline = document.getElementById("timeline");
  if (timeline) {
    const cells = [...timeline.querySelectorAll(".cell")];
    const columns = Math.max(...cells.map(cell => Number(cell.dataset.col))) + 1;
    const frame = timeline.parentElement;
    // The timeline follows the lit session only until the visitor scrolls it themselves.
    let follow = true;
    for (const type of ["pointerdown", "wheel", "keydown"]) frame.addEventListener(type, () => { follow = false; }, { passive: true });
    whileSeen(timeline, () => {
      const time = clock();
      let step = 0;
      timeline.classList.add("playing");
      const tick = () => {
        if (step === 0) cells.forEach(cell => cell.classList.remove("on", "now"));
        cells.forEach(cell => cell.classList.remove("now"));
        if (step < columns) {
          const lit = cells.filter(cell => Number(cell.dataset.col) === step);
          lit.forEach(cell => cell.classList.add("on", "now"));
          if (follow && frame.scrollWidth > frame.clientWidth) frame.scrollTo({ left: lit[0].offsetLeft - 96, behavior: "smooth" });
        }
        step = (step + 1) % (columns + 2);
      };
      tick();
      time.every(1700, tick);
      return () => { time.stop(); timeline.classList.remove("playing"); cells.forEach(cell => cell.classList.remove("on", "now")); };
    });
  }
})();
