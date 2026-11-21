// What a What's new scene is: a feature's own short film, drawn at the
// screen's real size from the screen's own parts, that a release names as a
// highlight's `drawing` (`RELEASE_DRAWINGS` in `web/src/releases.ts`). A scene
// is a pure drawing of one moment of its loop, so the gallery and a test can
// draw any beat, and `useSceneTime` plays it.

import { useEffect, useState } from "react";
import { useReducedMotion } from "../../lib/useReducedMotion";

export type SceneProps = {
  /** The moment of the loop to draw, in ms from its start. */
  at: number;
};

export type Scene = {
  Draw: (props: SceneProps) => React.ReactNode;
  /** One loop, in ms; the window moves to the next feature after one. */
  loopMs: number;
  /** The moment a still picture shows: the gallery, a paused window, reduced motion. */
  stillAt: number;
};

/** How often a playing scene draws its next moment. */
const FRAME_MS = 100;

/** The moment of `scene`'s loop to draw now: its still moment unless it plays and motion is allowed. */
export function useSceneTime(scene: Scene, playing: boolean): number {
  const reduced = useReducedMotion();
  const moving = playing && !reduced;
  const [at, setAt] = useState(moving ? 0 : scene.stillAt);
  useEffect(() => {
    if (!moving) {
      setAt(scene.stillAt);
      return;
    }
    const began = performance.now();
    setAt(0);
    const timer = window.setInterval(() => setAt((performance.now() - began) % scene.loopMs), FRAME_MS);
    return () => window.clearInterval(timer);
  }, [moving, scene]);
  return at;
}
