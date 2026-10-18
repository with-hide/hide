// One player belongs to this status document. No polling or host/core state.
import animationData from "./owl-roam.json";

export function startOwlMotion() {
  const scene = document.getElementById("flight-scene");
  const container = document.getElementById("owl-flight");
  const still = document.getElementById("owl-still");
  const reduced = window.matchMedia("(prefers-reduced-motion: reduce)");
  let player = null;
  let ready = false;
  let active = true;
  let onscreen = false;
  let disposed = false;
  let failed = false;
  let stillFailed = false;
  let reported = false;
  function report() {
    if (reported || disposed) return;
    reported = true;
    window.console.error(JSON.stringify({ event: "brand_owl.unavailable", scene: "startup", asset: "owl-roam" }));
  }
  function sync() {
    const playing = ready && active && onscreen && !document.hidden && !reduced.matches && !failed;
    container.hidden = failed || !ready || reduced.matches;
    still.hidden = stillFailed || (ready && !failed && !reduced.matches);
    document.body.dataset.owlMotion = playing ? "playing" : "paused";
    if (playing) player.play();
    else player?.pause();
  }
  function fail() {
    if (failed || disposed) return;
    failed = true;
    player?.destroy();
    player = null;
    container.hidden = true;
    still.hidden = false;
    sync();
    report();
  }
  function failStill() { stillFailed = true; sync(); report(); }
  still.addEventListener("error", failStill, { once: true });
  if (still.complete && still.naturalWidth === 0) failStill();
  const observer = new window.IntersectionObserver(([entry]) => { onscreen = entry.isIntersecting; sync(); });
  observer.observe(scene);
  reduced.addEventListener("change", sync);
  document.addEventListener("visibilitychange", sync);
  // Keep vendor module initialization inside the illustration boundary too:
  // its canvas capability probe must never prevent status/recovery setup.
  void import("lottie-web/build/player/lottie_light.min.js").then(({ default: lottie }) => {
    if (disposed) return;
    player = lottie.loadAnimation({ container, renderer: "svg", loop: true, autoplay: false, animationData });
    player.addEventListener("data_failed", fail);
    player.addEventListener("error", fail);
    player.addEventListener("DOMLoaded", () => {
      if (disposed || failed) return;
      ready = true;
      player.goToAndStop(0, true);
      sync();
    });
    // This bundled vector has no images, fonts or async resources. Lottie
    // configures it synchronously and may emit a config error before returning.
    if (!player.isLoaded) fail();
  }).catch(fail);
  window.addEventListener("pagehide", () => {
    disposed = true;
    observer.disconnect();
    reduced.removeEventListener("change", sync);
    document.removeEventListener("visibilitychange", sync);
    still.removeEventListener("error", failStill);
    player?.destroy();
    player = null;
  }, { once: true });
  return { connecting(value) { active = value; sync(); } };
}
