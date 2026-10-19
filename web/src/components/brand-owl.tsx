// Canonical brand artwork, decorative only. Switching to the still also stops
// the SVG's private animation timeline while the scene cannot be seen.
import { useEffect, useRef, useState } from "react";
import agents from "../../../site/assets/owls/owl-agents.svg";
import agentsStill from "../../../site/assets/owls/owl-agents-still.svg";
import devices from "../../../site/assets/owls/owl-devices.svg";
import devicesStill from "../../../site/assets/owls/owl-devices-still.svg";
import ai from "../../../site/assets/owls/owl-ai.svg";
import aiStill from "../../../site/assets/owls/owl-ai-still.svg";
import factory from "../../../site/assets/owls/owl-factory.svg";
import factoryStill from "../../../site/assets/owls/owl-factory-still.svg";
import { cn } from "../lib/utils";
import { useShellStore } from "../store";

const ART = { agents: [agents, agentsStill], devices: [devices, devicesStill], ai: [ai, aiStill], factory: [factory, factoryStill] } as const;
export type OwlPose = keyof typeof ART;

export function BrandOwl({ pose, className }: { pose: OwlPose; className?: string }) {
  const image = useRef<HTMLImageElement>(null);
  const [moving, setMoving] = useState(false);
  const [failed, setFailed] = useState<"motion" | "all" | null>(null);
  useEffect(() => {
    if (!window.matchMedia || !window.IntersectionObserver) {
      useShellStore.getState().noteDiagnostic(JSON.stringify({ event: "brand_owl.unavailable", scene: pose, asset: "motion_capability" }));
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
  }, [pose]);
  const playing = moving && failed === null;
  return <img ref={image} src={playing ? ART[pose][0] : ART[pose][1]} alt="" aria-hidden="true" draggable={false}
    data-brand-owl={pose} data-owl-motion={playing ? "playing" : "paused"}
    className={cn("pointer-events-none shrink-0 object-contain select-none", failed === "all" && "invisible", className)}
    onError={() => {
      useShellStore.getState().noteDiagnostic(JSON.stringify({ event: "brand_owl.unavailable", scene: pose, asset: playing ? "motion" : "still" }));
      setFailed(playing ? "motion" : "all");
    }} />;
}
