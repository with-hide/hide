// What a What's new scene is: a feature's own short film, drawn at the
// screen's real size from the screen's own parts, that a release names as a
// highlight's `drawing` (`RELEASE_DRAWINGS` in `web/src/releases.ts`). A scene
// is a pure drawing of one moment of its loop, so the gallery and a test can
// draw any beat, and `useSceneTime` plays it.

import { useEffect, useState } from "react";
import { useReducedMotion } from "../../lib/useReducedMotion";

// Every scene's parts come in as the screen would draw them: a row or a line
// rising into place, a count or a mark popping in, a press ringing out, the
// pointer gliding. Lengths are in em so they follow the interface scale;
// reduced motion draws each part in place.
export const SCENE_CSS = `
@keyframes wns-rise { from { opacity: 0; transform: translateY(0.6em); } }
@keyframes wns-pop { from { opacity: 0; transform: scale(0.4); } }
@keyframes wns-fade { from { opacity: 0; } }
@keyframes wns-grow { from { transform: scaleX(0); } }
@keyframes wns-draw { from { stroke-dashoffset: 1; } to { stroke-dashoffset: 0; } }
@keyframes wns-press { from { transform: scale(0.3); opacity: 0.9; } to { transform: scale(1.8); opacity: 0; } }
.wns-rise { animation: wns-rise 0.4s cubic-bezier(0.2, 1, 0.3, 1) both; }
.wns-pop { animation: wns-pop 0.45s cubic-bezier(0.3, 1.6, 0.5, 1) both; }
.wns-fade { animation: wns-fade 0.4s ease-out both; }
.wns-press { animation: wns-press 0.5s ease-out both; }
.wns-grow { transform-origin: left; animation: wns-grow 0.7s cubic-bezier(0.2, 1, 0.3, 1) both; }
.wns-draw { stroke-dasharray: 1; animation: wns-draw 1.2s ease-out both; }
.wns-glide { transition: left 0.6s cubic-bezier(0.4, 0, 0.2, 1), top 0.6s cubic-bezier(0.4, 0, 0.2, 1), width 0.6s cubic-bezier(0.4, 0, 0.2, 1), height 0.6s cubic-bezier(0.4, 0, 0.2, 1), opacity 0.4s, transform 0.6s cubic-bezier(0.4, 0, 0.2, 1); }
@media (prefers-reduced-motion: reduce) { .wns-rise, .wns-pop, .wns-fade, .wns-press, .wns-grow, .wns-draw { animation: none; } .wns-glide { transition: none; } }
`;

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
