// How a System layer (menu, dialog, popover, select) joins the shell's one
// Escape owner. `keyboard.ts` answers Escape in the window's capture phase,
// before Radix's own document listener and before xterm could send the byte,
// and closes the innermost registered layer first; so every open layer
// registers its close here, and nested layers close innermost-first.

import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { restoreFocus } from "../../terminals";
import { useUiStore } from "../../ui";
import { returnKeyboardFocus } from "../../viewFocus";

/** While `open`, Escape runs `close` before any layer opened earlier. */
export function useEscapeLayer(open: boolean, close: () => void, base = false) {
  const latest = useRef(close);
  useEffect(() => {
    latest.current = close;
  });
  useEffect(() => {
    if (!open) return;
    return useUiStore.getState().pushEscape(() => latest.current(), base);
  }, [open, base]);
}

/**
 * A Radix root's open state, controlled or not, that also registers the
 * Escape layer. A controlled caller keeps its own state; an uncontrolled one
 * gets it here.
 */
export function useLayerOpen(open: boolean | undefined, defaultOpen: boolean | undefined, onOpenChange: ((open: boolean) => void) | undefined, base = false) {
  const [own, setOwn] = useState(defaultOpen ?? false);
  const current = open ?? own;
  const change = useCallback(
    (next: boolean) => {
      if (open === undefined) setOwn(next);
      onOpenChange?.(next);
    },
    [open, onOpenChange],
  );
  useEscapeLayer(current, () => change(false), base);
  return [current, change] as const;
}

/**
 * Layers opened so far. Radix hands focus back from a timer after a layer
 * closes, and under load that timer runs after the next layer opened and closed;
 * comparing this count at close and at hand-back tells a late hand-back that a
 * newer layer owns the keyboard now.
 */
let layersOpened = 0;

/**
 * The element that held the keyboard when a layer opened, so closing it can
 * hand the keyboard back through `restoreFocus`, which knows that a terminal
 * is focused through its pane rather than through a stale textarea.
 * A layer whose owner names where the keyboard goes (Overview's own return
 * target or a context-menu target) passes `named`. An optional retained surface
 * still holding focus does not count as the keyboard having moved on, and the
 * same late hand-back guard applies.
 */
export function useReturnFocus(open: boolean, named?: { target: () => HTMLElement | null; within?: () => HTMLElement | null }) {
  const previous = useRef<HTMLElement | null>(null);
  const wasOpen = useRef(false);
  const openedAtClose = useRef<number | null>(null);
  // Read on the render that opens the layer, before Radix moves focus into it;
  // Radix hands focus back after a timeout, so the value must outlive the close.
  if (open && !wasOpen.current && typeof document !== "undefined") {
    previous.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
  }
  wasOpen.current = open;
  // Layout cleanups of what a commit removes run before the layout effects of what it adds, so a layer
  // replacing another in one commit counts as opened after the other closed. A content part is mounted
  // only while its layer is open, so it closes by unmounting; a menu closes by `open` turning false.
  useLayoutEffect(() => {
    if (!open) return;
    layersOpened += 1;
    openedAtClose.current = null;
    return () => {
      openedAtClose.current = layersOpened;
    };
  }, [open]);
  return useCallback((event: Event) => {
    const target = named ? named.target() : previous.current;
    if (!target && !named) return;
    event.preventDefault();
    if (!target) return;
    // A layer opened after this one closed holds the keyboard, or hands it back
    // itself; taking it here would undo that, whichever timer ran first.
    if (openedAtClose.current !== null && openedAtClose.current !== layersOpened) return;
    // Radix hands focus back a tick after the layer closed; an item that
    // moved the keyboard on purpose in the meantime (Rename's inline field)
    // keeps it, since taking it back would blur and commit that field.
    const now = document.activeElement;
    if (now instanceof HTMLElement && now !== document.body && now.isConnected && !named?.within?.()?.contains(now)) return;
    returnKeyboardFocus(() => restoreFocus(target));
  }, []);
}
