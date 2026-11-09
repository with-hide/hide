// One decoration belongs to this status document: the blinking owl, an SVG
// that animates itself. It shows its still while it cannot be seen (reduced
// motion, a hidden document, offscreen) or once connecting ends, so its own
// timeline stops. No polling or host/core state.
const MOTION = "owl-blink.svg";
const STILL = "owl-blink-still.svg";

export function startOwlMotion() {
  const owl = document.getElementById("owl");
  const reduced = window.matchMedia("(prefers-reduced-motion: reduce)");
  let active = true;
  let onscreen = false;
  let disposed = false;
  // null: both drawings load; "motion": only the still does; "all": neither.
  let failed = null;
  let reported = false;
  function report() {
    if (reported || disposed) return;
    reported = true;
    window.console.error(JSON.stringify({ event: "brand_owl.unavailable", scene: "startup", asset: "owl-blink" }));
  }
  function sync() {
    const playing = active && onscreen && !document.hidden && !reduced.matches && failed === null;
    const src = playing ? MOTION : STILL;
    if (failed !== "all" && owl.getAttribute("src") !== src) owl.setAttribute("src", src);
    owl.hidden = failed === "all";
    document.body.dataset.owlMotion = playing ? "playing" : "paused";
  }
  function fail() {
    if (disposed) return;
    failed = failed === null && owl.getAttribute("src") === MOTION ? "motion" : "all";
    sync();
    report();
  }
  owl.addEventListener("error", fail);
  if (owl.complete && owl.naturalWidth === 0) fail();
  const observer = new window.IntersectionObserver(([entry]) => { onscreen = entry.isIntersecting; sync(); });
  observer.observe(owl);
  reduced.addEventListener("change", sync);
  document.addEventListener("visibilitychange", sync);
  window.addEventListener("pagehide", () => {
    disposed = true;
    observer.disconnect();
    owl.removeEventListener("error", fail);
    reduced.removeEventListener("change", sync);
    document.removeEventListener("visibilitychange", sync);
  }, { once: true });
  sync();
  return { connecting(value) { active = value; sync(); } };
}
