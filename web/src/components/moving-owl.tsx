// Brand artwork that animates itself, decorative only. Switching to the still
// also stops the SVG's private animation timeline while the scene cannot be
// seen. It holds no store, so a page outside the shell (the phone's start
// page) draws it too; the caller decides where a failure is recorded.
import { useEffect, useRef, useState } from "react";
import { cn } from "../lib/utils";

export type OwlUnavailable = "motion_capability" | "motion" | "still";

export function MovingOwl({ motion, still, scene, onUnavailable, className }: {
  motion: string;
  still: string;
  scene: string;
  onUnavailable: (asset: OwlUnavailable) => void;
  className?: string;
}) {
  const image = useRef<HTMLImageElement>(null);
  const [moving, setMoving] = useState(false);
  const [failed, setFailed] = useState<"motion" | "all" | null>(null);
  const report = useRef(onUnavailable);
  report.current = onUnavailable;
  useEffect(() => {
    if (!window.matchMedia || !window.IntersectionObserver) {
      report.current("motion_capability");
      return;
    }
    const reduced = window.matchMedia("(prefers-reduced-motion: reduce)");
    let onscreen = false;
    const sync = () => setMoving(onscreen && !document.hidden && !reduced.matches);
    const observer = new IntersectionObserver(([entry]) => { onscreen = entry!.isIntersecting; sync(); });
    observer.observe(image.current!);
    reduced.addEventListener("change", sync);
    document.addEventListener("visibilitychange", sync);
    return () => {
      observer.disconnect();
      reduced.removeEventListener("change", sync);
      document.removeEventListener("visibilitychange", sync);
    };
  }, [scene]);
  const playing = moving && failed === null;
  return <img ref={image} src={playing ? motion : still} alt="" aria-hidden="true" draggable={false}
    data-brand-owl={scene} data-owl-motion={playing ? "playing" : "paused"}
    className={cn("pointer-events-none shrink-0 object-contain select-none", failed === "all" && "invisible", className)}
    onError={() => {
      report.current(playing ? "motion" : "still");
      setFailed(playing ? "motion" : "all");
    }} />;
}
