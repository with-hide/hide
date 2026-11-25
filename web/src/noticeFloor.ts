// How high the window's corner notices stand (`ToastStack`, UI_BEHAVIOR.md
// Notices): above every input box a screen keeps at the window's foot, so a
// notice that waits for the operator never covers where they type. Desk's
// input boxes and the sleeping session's `에이전트 깨우기` in their place
// (PRD desk-2-ui B43) each hold the corner above their top while they are
// drawn; the screen under the corner never moves.

import { useLayoutEffect, type RefObject } from "react";
import { useUiStore } from "./ui";

/** Each drawn box's height above the window's foot, by the hook that drew it. */
const floors = new Map<symbol, number>();

function publish() {
  const floor = Math.max(0, ...floors.values());
  if (useUiStore.getState().noticeFloor !== floor) useUiStore.getState().setNoticeFloor(floor);
}

/** Holds the corner's notices above `ref`'s box while it is drawn, following its height and the window's. */
export function useNoticeFloor(ref: RefObject<HTMLElement | null>): void {
  useLayoutEffect(() => {
    const node = ref.current;
    if (!node) return undefined;
    const id = Symbol("notice-floor");
    const measure = () => {
      // From the window's foot to the box's top, in CSS pixels.
      floors.set(id, Math.max(0, Math.ceil(window.innerHeight - node.getBoundingClientRect().top)));
      publish();
    };
    measure();
    const resized = new ResizeObserver(measure);
    resized.observe(node);
    window.addEventListener("resize", measure);
    return () => {
      resized.disconnect();
      window.removeEventListener("resize", measure);
      floors.delete(id);
      publish();
    };
  }, [ref]);
}
