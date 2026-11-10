// The docs pages' head script (site/src/embed-script.mjs) runs here as it is shipped, in a Node VM whose page,
// frames and observers are stand-ins this file drives by hand, so the order of every event is fixed by the test:
// which document a frame loaded and when, and what the browser does with a `location.replace`.
// Expected answers come from the behavior the script's comment states: a frame ends on the page's theme, a
// refused navigation or a document that stays wrong never throws or loops, a later theme switch repairs again.
import assert from "node:assert/strict";
import { test } from "node:test";
import vm from "node:vm";
import { EMBED_SCRIPT } from "../../site/src/embed-script.mjs";

const ORIGIN = "https://withhide.dev";
const part = (theme) => `/embed/docs-embed.html?part=task-row&theme=${theme}&lang=en`;

class FakeFrame {
  constructor(src) {
    this.attributes = { src };
    this.replaces = [];
    this.docUrl = "about:blank";
    // What a `location.replace(url)` leaves the frame's document at: the new address, unless a test says otherwise.
    this.onReplace = (url) => url;
    this.crossOrigin = false;
    const frame = this;
    this.contentWindow = {
      location: {
        get href() {
          if (frame.crossOrigin) throw new Error("SecurityError: blocked a frame with a different origin");
          return frame.docUrl;
        },
        replace(url) {
          frame.replaces.push(url);
          frame.docUrl = frame.onReplace(url);
        },
      },
    };
    this.style = {};
  }
  getAttribute(name) { return this.attributes[name] ?? null; }
  setAttribute(name, value) { this.attributes[name] = value; }
  matches(selector) {
    assert.equal(selector, '[src*="/embed/"]');
    return String(this.attributes.src).includes("/embed/");
  }
}

/** A page with the script installed; `theme` is the page's data-theme. */
function page(theme) {
  const frames = [];
  const loadListeners = [];
  const attributeObservers = [];
  const childObservers = [];
  const readyListeners = [];
  const document = {
    documentElement: { dataset: { theme } },
    querySelectorAll: (selector) => (selector === 'iframe[src*="/embed/"]' ? frames.filter((frame) => String(frame.attributes.src).includes("/embed/")) : frames),
    addEventListener(type, listener) {
      if (type === "load") loadListeners.push(listener);
      else if (type === "DOMContentLoaded") readyListeners.push(listener);
    },
  };
  class FakeObserver {
    constructor(callback) { this.callback = callback; }
    observe(_target, options) { (options.attributes ? attributeObservers : childObservers).push(this); this.active = true; }
    disconnect() { this.active = false; }
  }
  const context = vm.createContext({
    document,
    location: { href: `${ORIGIN}/docs/start/concepts/`, origin: ORIGIN },
    window: { addEventListener() {} },
    HTMLIFrameElement: FakeFrame,
    MutationObserver: FakeObserver,
    URL,
    Number, Math, isFinite,
  });
  vm.runInContext(EMBED_SCRIPT, context);
  const run = (observers) => observers.filter((observer) => observer.active).forEach((observer) => observer.callback());
  return {
    /** The parser adds a frame: its `src` is what the page was written with. */
    add(src) {
      const frame = new FakeFrame(src);
      frames.push(frame);
      run(childObservers);
      return frame;
    },
    /** DOMContentLoaded: the parser's observer stops and the page syncs once more. */
    ready() { readyListeners.forEach((listener) => listener()); },
    /** The reader flips Starlight's theme switch. */
    switchTo(next) {
      document.documentElement.dataset.theme = next;
      run(attributeObservers);
    },
    /** A frame finished loading the document at `url`; the capture-phase listener hears it. */
    load(frame, url = frame.attributes.src) {
      frame.docUrl = new URL(url, ORIGIN).href;
      loadListeners.forEach((listener) => listener({ target: frame }));
    },
  };
}

const themeOf = (frame) => new URL(frame.docUrl).searchParams.get("theme");

test("a frame whose loaded document already has the page's theme is left alone", () => {
  const p = page("light");
  const frame = p.add(part("light"));
  p.ready();
  p.load(frame);
  assert.deepEqual(frame.replaces, []);
  assert.equal(frame.getAttribute("src"), part("light"));
});

test("the first document loads on the old theme while the src already says the new one: one repair, and it ends right", () => {
  const p = page("light");
  const frame = p.add(part("dark"));
  // The parser's observer has already set the page's theme on the src; the navigation that began with the old src
  // is the one that finishes first, so the document that loads is the dark one.
  assert.equal(frame.getAttribute("src"), part("light"));
  p.load(frame, part("dark"));
  assert.equal(frame.replaces.length, 1);
  assert.equal(themeOf(frame), "light");
  p.load(frame); // the repaired document loads
  assert.equal(frame.replaces.length, 1);
});

test("a frame the page adds later is handled the same way", () => {
  const p = page("dark");
  p.ready();
  // Added after DOMContentLoaded, so no observer rewrote its src: the load of its document is what repairs it.
  const frame = p.add(part("light"));
  p.load(frame, part("light"));
  assert.equal(frame.replaces.length, 1);
  assert.equal(themeOf(frame), "dark");
});

test("a frame of another origin or a blank frame is never navigated", () => {
  const p = page("light");
  const other = p.add("https://example.com/embed/x.html?theme=dark");
  p.load(other, "https://example.com/embed/x.html?theme=dark");
  const blank = p.add(part("light"));
  blank.docUrl = "about:blank";
  p.load(blank, "about:blank");
  assert.deepEqual([other.replaces, blank.replaces], [[], []]);
});

test("a cross-origin document does not throw and is not retried", () => {
  const p = page("light");
  const frame = p.add(part("light"));
  frame.crossOrigin = true;
  for (let i = 0; i < 5; i++) assert.doesNotThrow(() => p.load(frame, part("dark")));
  assert.deepEqual(frame.replaces, []);
});

test("a refused navigation does not throw out of the load handler", () => {
  const p = page("light");
  const frame = p.add(part("light"));
  frame.onReplace = () => { throw new Error("SecurityError: navigation refused"); };
  assert.doesNotThrow(() => p.load(frame, part("dark")));
  assert.doesNotThrow(() => p.load(frame, part("dark")));
  assert.equal(frame.replaces.length, 1);
});

test("the same wrong document on every load is repaired at most once", () => {
  const p = page("light");
  const frame = p.add(part("light"));
  // A server answer that sends the frame back to the dark address, every time.
  frame.onReplace = () => new URL(part("dark"), ORIGIN).href;
  for (let i = 0; i < 100; i++) p.load(frame, part("dark"));
  assert.equal(frame.replaces.length, 1);
});

test("a theme switch after a repair repairs again, also when the switch's own load is wrong", () => {
  const p = page("light");
  const frame = p.add(part("light"));
  p.load(frame, part("dark"));
  p.load(frame);
  assert.equal(frame.replaces.length, 1);
  p.switchTo("dark");
  assert.equal(frame.getAttribute("src"), part("dark"));
  p.load(frame, part("light")); // the switch loses its race: the document is the old one
  assert.equal(frame.replaces.length, 2);
  assert.equal(themeOf(frame), "dark");
  p.load(frame);
  p.switchTo("light");
  p.load(frame, part("dark"));
  assert.equal(frame.replaces.length, 3);
  assert.equal(themeOf(frame), "light");
});

test("a repair is not remembered past a load that came out right", () => {
  const p = page("light");
  const frame = p.add(part("light"));
  p.load(frame, part("dark"));
  p.load(frame);
  p.switchTo("dark");
  p.load(frame); // the switch's navigation wins its race: no repair
  p.switchTo("light");
  p.load(frame, part("dark")); // and now it loses it, on the theme that was repaired before
  assert.equal(frame.replaces.length, 2);
  assert.equal(themeOf(frame), "light");
});

test("a page with no theme leaves its frames as written", () => {
  const p = page(undefined);
  const frame = p.add(part("dark"));
  p.ready();
  p.load(frame, part("dark"));
  assert.equal(frame.getAttribute("src"), part("dark"));
  assert.deepEqual(frame.replaces, []);
});
