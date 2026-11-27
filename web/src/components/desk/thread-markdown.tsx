import { createContext, useContext, useLayoutEffect, useMemo, useRef, useState, type ComponentProps, type ReactNode, type RefObject } from "react";
import { MarkdownText } from "../../MarkdownText";

type Watch = { intersecting: boolean; show: (shown: boolean) => void };

/** One observer for the conversation, rather than one for every past answer. */
function markdownWindow() {
  const watched = new Map<HTMLElement, Watch>();
  let observer: IntersectionObserver | null = null;
  let selected = new Set<HTMLElement>();
  let withoutObserver = false;
  const shown = (element: HTMLElement, watch: Watch) => watch.show(withoutObserver || watch.intersecting || selected.has(element));
  return {
    watch(element: HTMLElement, show: Watch["show"]) {
      const watch = { intersecting: false, show };
      watched.set(element, watch);
      observer?.observe(element);
      if (withoutObserver) show(true);
      return () => {
        observer?.unobserve(element);
        watched.delete(element);
        selected.delete(element);
      };
    },
    connect(root: HTMLElement) {
      // Supported desktop and phone browsers have IntersectionObserver. A host
      // without it keeps the existing readable Markdown instead of hiding it.
      withoutObserver = typeof IntersectionObserver === "undefined";
      if (!withoutObserver) {
        // Prepare one viewport above and below the one initially shown. The
        // expensive views follow the viewport, plus at most two selection
        // endpoints and the one question sentence currently lit. Finished
        // offscreen views are destroyed; their words and last height remain.
        observer = new IntersectionObserver((entries) => {
          for (const entry of entries) {
            const element = entry.target as HTMLElement;
            const watch = watched.get(element);
            if (!watch) continue;
            watch.intersecting = entry.isIntersecting;
            shown(element, watch);
          }
        }, { root, rootMargin: `${root.clientHeight}px 0%` });
      }
      for (const [element, watch] of watched) {
        observer?.observe(element);
        if (withoutObserver) watch.show(true);
      }
      // Keep a selection's endpoints alive while it is copied, including
      // after scrolling away. Only the two endpoints can pin a view.
      const selectionChanged = () => {
        const selection = root.ownerDocument.getSelection();
        const next = new Set<HTMLElement>();
        if (selection && !selection.isCollapsed) {
          for (const node of [selection.anchorNode, selection.focusNode]) {
            const element = node instanceof Element ? node : node?.parentElement;
            const text = element?.closest<HTMLElement>("[data-thread-markdown]");
            if (text && watched.has(text)) next.add(text);
          }
        }
        const changed = new Set([...selected, ...next]);
        selected = next;
        for (const element of changed) {
          const watch = watched.get(element);
          if (watch) shown(element, watch);
        }
      };
      root.ownerDocument.addEventListener("selectionchange", selectionChanged);
      return () => {
        root.ownerDocument.removeEventListener("selectionchange", selectionChanged);
        observer?.disconnect();
        observer = null;
        withoutObserver = false;
        selected.clear();
      };
    },
  };
}

const MarkdownWindow = createContext<ReturnType<typeof markdownWindow> | null>(null);

/** The conversation owns its visibility subscription and releases it on every unmount. */
export function ThreadMarkdownWindow({ root, children }: { root: RefObject<HTMLDivElement | null>; children: ReactNode }) {
  const window = useMemo(markdownWindow, []);
  useLayoutEffect(() => {
    if (!root.current) throw new Error("A Markdown window needs its conversation scroll element");
    return window.connect(root.current);
  }, [window, root]);
  return <MarkdownWindow.Provider value={window}>{children}</MarkdownWindow.Provider>;
}

/**
 * Past answers keep their words and scroll anchors, but only nearby answers
 * own a Markdown editor. This also bounds a single long turn containing many
 * agent messages, without discarding history or resetting its work folds.
 */
export function ThreadMarkdown(props: ComponentProps<typeof MarkdownText>) {
  const window = useContext(MarkdownWindow);
  if (!window) throw new Error("Thread Markdown needs its conversation window");
  const host = useRef<HTMLDivElement>(null);
  const [visible, setVisible] = useState(false);
  const measured = useRef<{ text: string; height: number } | null>(null);
  const latest = useRef(props.text);
  latest.current = props.text;
  useLayoutEffect(() => {
    const element = host.current;
    if (!element) return;
    return window.watch(element, (shown) => {
      if (!shown) {
        const height = element.getBoundingClientRect().height;
        measured.current = height > 0 ? { text: latest.current, height } : null;
      }
      setVisible(shown);
    });
  }, [window]);
  // A question's target is drawn before its scroll/highlight effect runs.
  const live = visible || props.mark != null;
  const height = measured.current?.text === props.text ? measured.current.height : undefined;
  return (
    <div ref={host} className="min-w-0" data-thread-markdown={live ? "live" : "parked"}>
      {live ? <MarkdownText {...props} /> : (
        <div className="whitespace-pre-wrap break-words text-subhead" style={{ height, overflow: height === undefined ? undefined : "hidden", lineHeight: "calc(var(--text-subhead) + var(--spacing-md))" }}>{props.text}</div>
      )}
    </div>
  );
}
