// Page motion: one-time reveals, the owls' loops, the hero peek's and the section owls', and the hero's tabs.
// Under reduced motion the reveals show at once, every owl holds its still frame and the tabs never turn by themselves.

const reduceMotion = window.matchMedia("(prefers-reduced-motion: reduce)").matches;

function mountReveals() {
  const items = document.querySelectorAll(".reveal");
  if (reduceMotion || !("IntersectionObserver" in window)) {
    items.forEach(item => item.classList.add("in"));
    return;
  }
  const observer = new IntersectionObserver(entries => {
    for (const entry of entries) {
      if (!entry.isIntersecting) continue;
      entry.target.classList.add("in");
      observer.unobserve(entry.target);
    }
  }, { rootMargin: "0px 0px -8% 0px" });
  items.forEach(item => observer.observe(item));
}

// An animated owl plays its loop only while it is on screen and the tab is visible; otherwise, and
// under reduced motion, the image holds its still frame. Its src is the still and data-motion the
// loop; a section owl's loop begins on its still frame, so the swap does not show.
function mountMotion() {
  const images = document.querySelectorAll("img[data-motion]");
  if (!images.length || reduceMotion) return;
  const watch = "IntersectionObserver" in window;
  const owls = new Map([...images].map(img => [img, { still: img.getAttribute("src"), moving: img.dataset.motion, inView: !watch }]));
  const sync = (owl, img) => {
    const want = owl.inView && !document.hidden ? owl.moving : owl.still;
    if (img.getAttribute("src") !== want) img.setAttribute("src", want);
  };
  if (watch) {
    const observer = new IntersectionObserver(entries => {
      for (const entry of entries) {
        const owl = owls.get(entry.target);
        owl.inView = entry.isIntersecting;
        sync(owl, entry.target);
      }
    }, { threshold: 0.1 });
    owls.forEach((_, img) => observer.observe(img));
  }
  document.addEventListener("visibilitychange", () => owls.forEach(sync));
  owls.forEach(sync);
}

// The hero's tabs: one window each. With motion allowed they hand over every few seconds while the stage is on
// screen and the tab is visible; the first press, click or key, leaves the choice to the visitor for good.
const TAB_MS = 7000;

function mountTabs() {
  const stage = document.querySelector(".stage");
  const tabs = [...stage.querySelectorAll('[role="tab"]')];
  const caption = document.getElementById("stage-caption");
  let index = 0;
  let timer = null;
  let manual = reduceMotion;
  let inView = false;

  function select(next, focus) {
    index = (next + tabs.length) % tabs.length;
    tabs.forEach((tab, at) => {
      const on = at === index;
      tab.setAttribute("aria-selected", String(on));
      tab.tabIndex = on ? 0 : -1;
      document.getElementById(tab.getAttribute("aria-controls")).hidden = !on;
    });
    const panel = document.getElementById(tabs[index].getAttribute("aria-controls"));
    caption.textContent = panel.dataset.caption;
    stage.dataset.tab = tabs[index].id.replace("tab-", "");
    if (focus) tabs[index].focus();
    schedule();
  }

  // The fill under the selected tab restarts with each hand-over, so it is drawn again from zero.
  function schedule() {
    clearTimeout(timer);
    const running = !manual && inView && !document.hidden;
    stage.classList.remove("auto");
    void stage.offsetWidth;
    stage.classList.toggle("auto", !manual);
    stage.classList.toggle("paused", !running);
    if (running) timer = setTimeout(() => select(index + 1, false), TAB_MS);
  }

  function take() {
    manual = true;
    clearTimeout(timer);
    stage.classList.remove("auto", "paused");
  }

  stage.style.setProperty("--tab-ms", `${TAB_MS}ms`);
  tabs.forEach((tab, at) => tab.addEventListener("click", () => { take(); select(at, false); }));
  stage.querySelector('[role="tablist"]').addEventListener("keydown", event => {
    const step = { ArrowRight: 1, ArrowLeft: -1 }[event.key];
    const jump = { Home: 0, End: tabs.length - 1 }[event.key];
    if (step === undefined && jump === undefined) return;
    event.preventDefault();
    take();
    select(jump ?? index + step, true);
  });
  if ("IntersectionObserver" in window) {
    new IntersectionObserver(entries => {
      inView = entries[0].isIntersecting;
      schedule();
    }, { threshold: 0.35 }).observe(stage);
  }
  document.addEventListener("visibilitychange", schedule);
  select(0, false);
}

mountReveals();
mountMotion();
mountTabs();
