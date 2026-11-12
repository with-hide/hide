// The Markdown Live plugin (PRD B6, D-07): it turns the pure plan into
// CodeMirror decorations, so the source stays the document while the drawn
// text carries the styles and hides the markup. Recomputing is whole-document
// per edit, which is why Live is off above the byte limit.
//
// The decorations live in a state field, not a view plugin, because an HTML
// block is drawn as a block widget and CodeMirror takes block decorations
// only from state.

import { syntaxTree } from "@codemirror/language";
import type { SyntaxNode } from "@lezer/common";
import { StateField, type EditorState, type Extension, type Range } from "@codemirror/state";
import { Decoration, EditorView, ViewPlugin, WidgetType, type DecorationSet } from "@codemirror/view";
import { createElement, Fragment, useLayoutEffect, type ReactElement, type ReactNode } from "react";
import { createRoot, type Root } from "react-dom/client";
import type { FileSource } from "../fileBytes";
import { translate } from "../i18n/client";
import { useShellStore } from "../store";
import { hasVisibleHtml, imageTarget, liveLinkAddress, resolveInCheckout, sanitizeHtml, type ImageBase } from "./liveHtml";
import { LiveImages, readImageFile } from "./liveImages";
import { markdownLivePlan, MARKDOWN_LIVE_BYTE_LIMIT } from "./markdownLive";
import { drawMermaid, showMermaidSvg, watchPalette } from "../markdownMermaid";
import { renderMarkdown } from "../mobile/markdown";

/**
 * What a Live text can reach outside itself: where its relative paths start
 * (null for a text that is not a file, such as an issue body) and what a
 * modifier-click on a link does.
 */
export type LiveFiles = {
  base: () => (ImageBase & { source: FileSource }) | null;
  openUrl: (url: string) => void;
  openPath: (path: string) => void;
};

const OUTSIDE: LiveFiles = {
  base: () => null,
  openUrl: (url) => {
    window.open(url, "_blank", "noopener,noreferrer");
  },
  openPath: () => {},
};

/** How long an image may stay pending before it falls back to its alt text. */
const IMAGE_TIMEOUT_MS = 8_000;

/** What a widget draws with: the view's image cache and the files it may name. */
type Drawing = { images: LiveImages; files: LiveFiles };

class BulletWidget extends WidgetType {
  toDOM(): HTMLElement {
    const span = document.createElement("span");
    span.className = "cm-md-bullet";
    span.textContent = "•";
    return span;
  }
  eq(): boolean {
    return true;
  }
  ignoreEvent(): boolean {
    return false;
  }
}

class CheckboxWidget extends WidgetType {
  constructor(
    readonly checked: boolean,
    readonly from: number,
    readonly to: number,
  ) {
    super();
  }
  eq(other: CheckboxWidget): boolean {
    return other.checked === this.checked && other.from === this.from;
  }
  toDOM(view: EditorView): HTMLElement {
    const input = document.createElement("input");
    input.type = "checkbox";
    input.checked = this.checked;
    input.className = "cm-md-checkbox";
    input.setAttribute("aria-label", translate(this.checked ? "documents.completedTask" : "documents.openTask"));
    input.addEventListener("mousedown", (event) => event.preventDefault());
    input.addEventListener("click", (event) => {
      event.preventDefault();
      view.dispatch({
        changes: { from: this.from, to: this.to, insert: this.checked ? "[ ]" : "[x]" },
        // The toggle is an operator edit like a keystroke, so the draft event
        // reaches the core and the tab turns dirty.
        userEvent: "input",
      });
    });
    return input;
  }
  ignoreEvent(): boolean {
    return false;
  }
}

/** One image: shown once its address is decided and its bytes are in, and as its alt text in the smallest muted form when it is refused or fails. */
function drawImage(img: HTMLImageElement, src: string, drawing: Drawing, view: EditorView): void {
  const base = drawing.files.base();
  const target = imageTarget(src, base);
  const alt = img.getAttribute("alt") ?? "";
  const name = src.split(/[?#]/)[0]?.split("/").pop() ?? "";
  const fallback = document.createElement("span");
  fallback.className = "cm-md-alt";
  fallback.dataset.state = "loading";
  fallback.textContent = alt || name;
  const fail = () => {
    fallback.dataset.state = "failed";
    view.requestMeasure();
  };
  img.replaceWith(fallback);
  if (target.kind === "refused") {
    fail();
    return;
  }
  img.addEventListener(
    "load",
    () => {
      fallback.replaceWith(img);
      view.requestMeasure();
    },
    { once: true },
  );
  img.addEventListener("error", fail, { once: true });
  // An address that neither answers nor refuses (offline behind a proxy) ends
  // in its alt text too, and the request is cancelled.
  const giveUp = window.setTimeout(() => {
    if (img.complete && img.naturalWidth > 0) return;
    img.removeAttribute("src");
    fail();
  }, IMAGE_TIMEOUT_MS);
  img.addEventListener("load", () => window.clearTimeout(giveUp), { once: true });
  img.addEventListener("error", () => window.clearTimeout(giveUp), { once: true });
  img.referrerPolicy = "no-referrer";
  img.decoding = "async";
  void drawing.images.load(target, base?.source ?? null).then((url) => {
    if (url === null) fail();
    else img.src = url;
  });
}

/** A link's target: http(s) and mailto in the browser, a relative file inside the checkout as a tab, anything else a diagnostic. A plain click only places the caret; a modifier-click opens it. */
function openLink(href: string, files: LiveFiles): void {
  if (href === "" || href.startsWith("#")) return;
  if (/^(?:https?|mailto):/i.test(href)) {
    files.openUrl(href);
    return;
  }
  const base = files.base();
  const path = base ? resolveInCheckout(href, base) : null;
  if (path === null) {
    useShellStore.getState().noteDiagnostic(`markdown live: link ${href} is not inside the checkout`);
    return;
  }
  files.openPath(path);
}

/** Sanitized HTML, drawn as a block (a whole HTML block) or inline (one `<img>` tag). */
class HtmlWidget extends WidgetType {
  constructor(
    readonly html: string,
    readonly block: boolean,
    readonly key: string,
    private readonly drawing: Drawing,
  ) {
    super();
  }
  eq(other: HtmlWidget): boolean {
    return other.html === this.html && other.block === this.block && other.key === this.key;
  }
  toDOM(view: EditorView): HTMLElement {
    const root: HTMLElement = document.createElement(this.block ? "div" : "span");
    root.className = this.block ? "cm-md-html cm-md-html-block" : "cm-md-html";
    const clean = sanitizeHtml(this.html);
    // The plan only draws a block that has something to show.
    if (!clean) return root;
    root.append(clean.fragment);
    for (const { element, src } of clean.images) drawImage(element, src, this.drawing, view);
    const anchorOf = (event: Event) => (event.target instanceof Element ? event.target.closest("a") : null);
    // The modifier-click opens the link before the editor sees the press, so
    // the caret stays and the source does not open; every other press is the
    // editor's, which places the caret and shows the source.
    root.addEventListener("mousedown", (event) => {
      const anchor = anchorOf(event);
      if (!anchor || !(event.metaKey || event.ctrlKey)) return;
      event.preventDefault();
      event.stopPropagation();
      openLink(anchor.getAttribute("href") ?? "", this.drawing.files);
    });
    // A link never navigates the page itself, whichever button pressed it.
    for (const type of ["click", "auxclick"]) {
      root.addEventListener(type, (event) => {
        if (anchorOf(event)) event.preventDefault();
      });
    }
    return root;
  }
  ignoreEvent(): boolean {
    return false;
  }
}

/** A Markdown image `![alt](src "title")`. */
class ImageWidget extends WidgetType {
  constructor(
    readonly src: string,
    readonly alt: string,
    readonly title: string | undefined,
    readonly key: string,
    private readonly drawing: Drawing,
  ) {
    super();
  }
  eq(other: ImageWidget): boolean {
    return other.src === this.src && other.alt === this.alt && other.title === this.title && other.key === this.key;
  }
  toDOM(view: EditorView): HTMLElement {
    const root = document.createElement("span");
    root.className = "cm-md-html";
    const img = document.createElement("img");
    img.alt = this.alt;
    if (this.title) img.title = this.title;
    root.append(img);
    drawImage(img, this.src, this.drawing, view);
    return root;
  }
  ignoreEvent(): boolean {
    return false;
  }
}

/** What each drawn diagram must let go of when CodeMirror drops its DOM. */
const diagramWatches = new WeakMap<HTMLElement, () => void>();

/**
 * A fenced `mermaid` block drawn as its diagram. It takes no room until the
 * picture is in; a source Mermaid cannot read shows as code under one muted
 * line saying why, so the operator sees what to fix, and a press on either
 * shows the source.
 */
class MermaidWidget extends WidgetType {
  constructor(readonly source: string) {
    super();
  }
  eq(other: MermaidWidget): boolean {
    return other.source === this.source;
  }
  toDOM(view: EditorView): HTMLElement {
    const root = document.createElement("div");
    root.className = "cm-md-mermaid";
    let current = 0;
    const draw = () => {
      const attempt = ++current;
      void drawMermaid(this.source).then((drawing) => {
        if (attempt !== current) return;
        if (drawing.kind === "svg") {
          root.dataset.state = "drawn";
          showMermaidSvg(root, drawing.svg);
        } else {
          root.dataset.state = "failed";
          const reason = document.createElement("div");
          reason.className = "cm-md-alt";
          reason.dataset.state = "failed";
          reason.textContent = translate("documents.mermaidUnavailable", { reason: drawing.reason });
          const code = document.createElement("pre");
          code.className = "cm-md-mermaid-source";
          code.textContent = this.source;
          root.replaceChildren(reason, code);
          useShellStore.getState().noteDiagnostic(`markdown live: mermaid diagram not drawn: ${drawing.reason}`);
        }
        view.requestMeasure();
      });
    };
    root.dataset.state = "loading";
    draw();
    diagramWatches.set(root, watchPalette(draw));
    // A press on a picture has no text position under it, and CodeMirror
    // would put the caret after the block; the diagram puts it at its own
    // start, which shows its source.
    root.addEventListener("mousedown", (event) => {
      if (event.button !== 0) return;
      event.preventDefault();
      view.dispatch({ selection: { anchor: view.posAtDOM(root) }, userEvent: "select.pointer" });
      view.focus();
    });
    return root;
  }
  destroy(dom: HTMLElement): void {
    diagramWatches.get(dom)?.();
    diagramWatches.delete(dom);
  }
  ignoreEvent(event: Event): boolean {
    return event.type === "mousedown";
  }
}

/** Each drawn table's React root, unmounted after the editor lets it go, since that can happen while React commits. */
const tableRoots = new WeakMap<HTMLElement, Root>();

/** Runs `drawn` once React has put the table in the page, so the editor measures the height it takes. */
function Drawn({ drawn, children }: { drawn: () => void; children: ReactNode }): ReactElement {
  useLayoutEffect(drawn, [drawn]);
  return createElement(Fragment, null, children);
}

/**
 * A top-level GFM table drawn by the renderer the phone and the pull request
 * card use (`mobile/markdown.tsx`, its HTML on), read-only. Its links keep
 * Live's addresses, a relative file among them, and open on modifier-click
 * like a drawn HTML block's, never navigating the page; any other press puts
 * the caret at the table's start, which shows its source.
 */
class TableWidget extends WidgetType {
  constructor(
    readonly source: string,
    private readonly files: LiveFiles,
  ) {
    super();
  }
  eq(other: TableWidget): boolean {
    return other.source === this.source;
  }
  toDOM(view: EditorView): HTMLElement {
    const root = document.createElement("div");
    root.className = "cm-md-table";
    const react = createRoot(root);
    react.render(createElement(Drawn, { drawn: () => view.requestMeasure(), children: renderMarkdown(this.source, { html: true, link: liveLinkAddress }) }));
    tableRoots.set(root, react);
    // A picture in a cell changes the table's height when it arrives or gives up.
    for (const type of ["load", "error"]) root.addEventListener(type, () => view.requestMeasure(), true);
    const anchorOf = (event: Event) => (event.target instanceof Element ? event.target.closest("a") : null);
    root.addEventListener("mousedown", (event) => {
      if (event.button !== 0) return;
      event.preventDefault();
      const anchor = anchorOf(event);
      if (anchor && (event.metaKey || event.ctrlKey)) {
        openLink(anchor.getAttribute("href") ?? "", this.files);
        return;
      }
      view.dispatch({ selection: { anchor: view.posAtDOM(root) }, userEvent: "select.pointer" });
      view.focus();
    });
    for (const type of ["click", "auxclick"]) {
      root.addEventListener(type, (event) => {
        if (anchorOf(event)) event.preventDefault();
      });
    }
    return root;
  }
  destroy(dom: HTMLElement): void {
    const react = tableRoots.get(dom);
    tableRoots.delete(dom);
    if (react) queueMicrotask(() => react.unmount());
  }
  ignoreEvent(event: Event): boolean {
    return event.type === "mousedown";
  }
}

/**
 * Whether the operator has moved the caret or edited in this view. A file
 * opens with the caret at 0, which the operator did not put there, so a drawn
 * block or image waits for the first move before a selection shows its source.
 */
const engaged = StateField.define<boolean>({
  create: () => false,
  update(value, tr) {
    if (value) return true;
    return ["select", "input", "delete", "move", "undo", "redo"].some((event) => tr.isUserEvent(event));
  },
});

/** No selection anywhere in the document, so no mark is revealed. */
const NOWHERE = { from: -1, to: -1 };

function build(state: EditorState, reveal: boolean, drawing: Drawing): DecorationSet {
  const doc = state.doc;
  if (doc.length > MARKDOWN_LIVE_BYTE_LIMIT) return Decoration.none;
  const plan = markdownLivePlan(syntaxTree(state), doc, reveal ? state.selection.main : NOWHERE, state.field(engaged, false) === true);
  const ranges: Range<Decoration>[] = [];
  const base = drawing.files.base();
  // The widgets of a different document or checkout are not the same drawing.
  const key = base ? `${base.docPath}|${base.root}|${base.source?.device ?? ""}` : "";
  for (const range of plan) {
    switch (range.kind) {
      case "hide":
        ranges.push(Decoration.replace({}).range(range.from, range.to));
        break;
      case "bullet":
        ranges.push(Decoration.replace({ widget: new BulletWidget() }).range(range.from, range.to));
        break;
      case "checkbox":
        ranges.push(
          Decoration.replace({ widget: new CheckboxWidget(range.checked === true, range.from, range.to) }).range(
            range.from,
            range.to,
          ),
        );
        break;
      case "heading":
        ranges.push(Decoration.line({ class: `cm-md-heading-${range.level ?? 1}` }).range(range.from));
        break;
      case "codeBlock":
        ranges.push(Decoration.line({ class: "cm-md-code" }).range(range.from));
        break;
      case "fenceLine":
        ranges.push(Decoration.line({ class: "cm-md-hidden-line" }).range(range.from));
        break;
      case "quote":
        ranges.push(Decoration.line({ class: "cm-md-quote" }).range(range.from));
        break;
      case "link":
        ranges.push(Decoration.mark({ class: "cm-md-link" }).range(range.from, range.to));
        break;
      case "codeInline":
        ranges.push(Decoration.mark({ class: "cm-md-code-inline" }).range(range.from, range.to));
        break;
      case "html":
        if (!hasVisibleHtml(range.html ?? "")) break;
        ranges.push(
          Decoration.replace({ widget: new HtmlWidget(range.html ?? "", range.block === true, key, drawing), block: range.block === true }).range(
            range.from,
            range.to,
          ),
        );
        break;
      case "mermaid":
        ranges.push(Decoration.replace({ widget: new MermaidWidget(range.source ?? ""), block: true }).range(range.from, range.to));
        break;
      case "table":
        ranges.push(Decoration.replace({ widget: new TableWidget(range.source ?? "", drawing.files), block: true }).range(range.from, range.to));
        break;
      case "image":
        ranges.push(
          Decoration.replace({ widget: new ImageWidget(range.src ?? "", range.alt ?? "", range.title, key, drawing) }).range(range.from, range.to),
        );
        break;
    }
  }
  return Decoration.set(ranges, true);
}

/**
 * `reveal` shows the marks of the lines the selection touches, as an editor
 * does; a read-only text (an issue's body) has no caret to edit at, so it
 * never reveals them. `files` says where a document's relative images and
 * links start; a text with none loads web images only.
 *
 * The field rebuilds when the document, the selection or the parse changed, so
 * a lazily loaded Markdown language lights the view up as soon as its first
 * tree exists. A widget whose source and base are unchanged keeps its DOM, so
 * an edit outside an HTML block neither redraws it nor reads its images again.
 */
export function markdownLive({ reveal = true, files = OUTSIDE }: { reveal?: boolean; files?: LiveFiles } = {}): Extension {
  const drawing: Drawing = { images: new LiveImages(readImageFile, (message) => useShellStore.getState().noteDiagnostic(message)), files };
  const field = StateField.define<DecorationSet>({
    create: (state) => build(state, reveal, drawing),
    update(value, tr) {
      if (tr.docChanged || tr.selection || tr.state.field(engaged, false) !== tr.startState.field(engaged, false) || syntaxTree(tr.state) !== syntaxTree(tr.startState)) {
        return build(tr.state, reveal, drawing);
      }
      return value;
    },
    provide: (f) => EditorView.decorations.from(f),
  });
  // The image cache lives as long as the view that draws with it.
  const owner = ViewPlugin.define((view) => {
    // A block or image at the top of a file changes height once it is drawn
    // and again when its image arrives, and CodeMirror then keeps the line
    // below it in place by scrolling the first one away. A view that opened
    // at the top stays there until the operator scrolls, moves the caret or
    // types.
    const scroller = view.scrollDOM;
    let atTop = scroller.scrollTop === 0;
    const leave = () => {
      atTop = false;
    };
    const keep = () => {
      if (atTop && scroller.scrollTop !== 0) scroller.scrollTop = 0;
    };
    const operator = ["wheel", "touchstart", "pointerdown", "keydown"];
    for (const type of operator) scroller.addEventListener(type, leave, { passive: true });
    scroller.addEventListener("scroll", keep);
    return {
      update(update) {
        if (update.docChanged || update.selectionSet) leave();
      },
      destroy() {
        for (const type of operator) scroller.removeEventListener(type, leave);
        scroller.removeEventListener("scroll", keep);
        drawing.images.dispose();
      },
    };
  });
  const links = EditorView.domEventHandlers({
    // A Markdown link opens on modifier-click only; a plain click places the caret.
    mousedown(event, view) {
      if (!(event.metaKey || event.ctrlKey) || event.button !== 0) return false;
      const pos = view.posAtCoords({ x: event.clientX, y: event.clientY });
      if (pos === null) return false;
      for (let node: SyntaxNode | null = syntaxTree(view.state).resolveInner(pos, 1); node; node = node.parent) {
        if (node.name !== "Link" && node.name !== "Autolink") continue;
        const url = node.getChild("URL");
        if (!url) return false;
        event.preventDefault();
        openLink(view.state.sliceDoc(url.from, url.to), drawing.files);
        return true;
      }
      return false;
    },
  });
  return [engaged, field, owner, links];
}
