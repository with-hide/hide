// A small mission map: the plan a Conductor made, the workers on it and the pull request one of them opened,
// drawn by the product's own `MissionMap` over an invented mission.
import { useLayoutEffect, useMemo, useRef, useState, type RefObject } from "react";
import { createActions } from "../../actions";
import { MissionMap } from "../../MissionMap";
import { useShellStore } from "../../store";
import { useUiStore } from "../../ui";
import { MISSION_ID, missionFixture } from "../fixtures/mission";
import { PartFrame } from "../frame";

/** The least height the map's frame has; it grows from here by what the map's page would scroll. */
const MIN_HEIGHT = 640;

/**
 * The product's map page is a box of a given height that scrolls whole while it is narrow (`MapPage`), so a frame
 * of a fixed height cuts the summary under the map at some line. A docs frame shows all of it: the box grows by
 * what the page scrolls, once the map has drawn and again whenever it redraws (a width change moves the stacked
 * parts). The map fits itself into the page's height and is never drawn larger than whole, so a taller page never
 * makes it taller and the height settles; it only ever grows, so it cannot flicker between two heights.
 */
function useWholePage(box: RefObject<HTMLDivElement | null>): number {
  const [height, setHeight] = useState(MIN_HEIGHT);
  useLayoutEffect(() => {
    const element = box.current;
    if (!element) return undefined;
    const grow = () => {
      const scroller = element.querySelector<HTMLElement>("[data-mission-scroll]");
      const hidden = scroller ? scroller.scrollHeight - scroller.clientHeight : 0;
      if (hidden > 0) setHeight((now) => now + hidden);
    };
    grow();
    const resized = new ResizeObserver(grow);
    resized.observe(element);
    const changed = new MutationObserver(grow);
    changed.observe(element, { subtree: true, childList: true, attributes: true });
    return () => {
      resized.disconnect();
      changed.disconnect();
    };
  }, [box]);
  return height;
}

export default function MissionMapPart() {
  const box = useRef<HTMLDivElement>(null);
  const height = useWholePage(box);
  const { rest } = useMemo(missionFixture, []);
  const actions = useMemo(() => createActions(() => true), []);
  useLayoutEffect(() => {
    useShellStore.setState({ rest, agents: [], connection: "gone" });
    useUiStore.setState({ screen: { kind: "missions" } });
  }, [rest]);
  return (
    <PartFrame part="mission-map" height={height}>
      <div ref={box} className="flex h-full min-h-0 min-w-0 flex-col rounded-md border border-border"><MissionMap missionId={MISSION_ID} actions={actions} /></div>
    </PartFrame>
  );
}
