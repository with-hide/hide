// Page motion: one-time reveals and the owls' loops, the hero peek's and the closing owl's.
// Under reduced motion the reveals show at once and every owl holds its still frame.

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

// A link to a folded answer opens it.
function openLinkedAnswer() {
  const target = location.hash ? document.getElementById(location.hash.slice(1)) : null;
  if (target instanceof HTMLDetailsElement) target.open = true;
}

mountReveals();
mountMotion();
openLinkedAnswer();
window.addEventListener("hashchange", openLinkedAnswer);
