// Two small behaviors for the live parts a page frames (`/embed/docs-embed.html?part=<id>&theme=...`):
// - each frame ends on the page's own theme, now and when the reader flips Starlight's theme switch, so a
//   dark picture never sits on a light page. A frame is set to the page's theme as soon as the parser adds it
//   and again at DOMContentLoaded, which is only a hint: changing `src` while the first navigation is still
//   running can lose the race and leave the frame's document on the old theme with the new `src` printed on
//   it. What the page trusts is the document a frame actually loaded, so every frame `load` (the capture
//   phase sees a load that does not bubble) reads that document's own URL and, when its theme is not the
//   page's, replaces the frame's location with the right one, once per frame and theme (a navigation the browser
//   refuses, or a document that stays wrong, is left alone rather than retried on every load);
// - a part tells its page how tall it is with `{ type: "hide-docs-embed-height", part, height }` posted to
//   `window.parent`; the page finds the frame that sent it (`event.source`), accepts only its own origin and
//   sets that frame's height, clamped so a wrong number cannot make a page unusable. A page whose part posts
//   nothing keeps the frame's `height` attribute, with the minimum set in `owl-field.css`.
export const EMBED_SCRIPT = `(function () {
  function pageTheme() {
    var theme = document.documentElement.dataset.theme;
    return theme === "light" || theme === "dark" ? theme : null;
  }
  function withTheme(href, theme) {
    var url = new URL(href, location.href);
    if (url.origin !== location.origin || url.searchParams.get("theme") === theme) return null;
    url.searchParams.set("theme", theme);
    return url;
  }
  function frames() {
    return document.querySelectorAll('iframe[src*="/embed/"]');
  }
  function sync() {
    var theme = pageTheme();
    if (!theme) return;
    frames().forEach(function (frame) {
      var url = withTheme(frame.getAttribute("src"), theme);
      if (url) frame.setAttribute("src", url.pathname + url.search);
    });
  }
  // The theme a frame was last repaired for: a frame is repaired once per theme, so a document that stays on the
  // wrong theme (a server answer that redirects back, a frame that keeps loading the same thing) is left as it is
  // instead of being navigated on every load. A load on the right theme clears the mark, so a later theme switch
  // that goes wrong again is repaired again.
  var repaired = new WeakMap();
  function settle(frame) {
    var theme = pageTheme();
    if (!theme || !(frame instanceof HTMLIFrameElement) || !frame.matches('[src*="/embed/"]')) return;
    var loaded;
    try { loaded = frame.contentWindow.location.href; } catch (error) { return; }
    var url = withTheme(loaded, theme);
    if (!url) { repaired.delete(frame); return; }
    if (loaded === "about:blank" || repaired.get(frame) === theme) return;
    repaired.set(frame, theme);
    try { frame.contentWindow.location.replace(url.href); } catch (error) { /* a refused navigation leaves the frame as it is */ }
  }
  document.addEventListener("load", function (event) { settle(event.target); }, true);
  var parsing = new MutationObserver(sync);
  parsing.observe(document.documentElement, { childList: true, subtree: true });
  document.addEventListener("DOMContentLoaded", function () {
    parsing.disconnect();
    sync();
  });
  new MutationObserver(sync).observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme"] });
  window.addEventListener("message", function (event) {
    var data = event.data;
    if (event.origin !== location.origin || !data || data.type !== "hide-docs-embed-height") return;
    var height = Number(data.height);
    if (!isFinite(height) || height <= 0) return;
    var frames = document.querySelectorAll("iframe");
    for (var i = 0; i < frames.length; i++) {
      if (frames[i].contentWindow === event.source) {
        frames[i].style.height = Math.min(Math.max(Math.round(height), 40), 2000) + "px";
        return;
      }
    }
  });
})();`;

