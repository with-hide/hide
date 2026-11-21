// The shell's brand poses; a failed drawing goes to the shell's diagnostic log.
import agents from "../../../site/public/assets/owls/owl-agents.svg";
import agentsStill from "../../../site/public/assets/owls/owl-agents-still.svg";
import devices from "../../../site/public/assets/owls/owl-devices.svg";
import devicesStill from "../../../site/public/assets/owls/owl-devices-still.svg";
import ai from "../../../site/public/assets/owls/owl-ai.svg";
import aiStill from "../../../site/public/assets/owls/owl-ai-still.svg";
import factory from "../../../site/public/assets/owls/owl-factory.svg";
import factoryStill from "../../../site/public/assets/owls/owl-factory-still.svg";
import { useShellStore } from "../store";
import { MovingOwl } from "./moving-owl";

const ART = { agents: [agents, agentsStill], devices: [devices, devicesStill], ai: [ai, aiStill], factory: [factory, factoryStill] } as const;
export type OwlPose = keyof typeof ART;

export function BrandOwl({ pose, className }: { pose: OwlPose; className?: string }) {
  return <MovingOwl motion={ART[pose][0]} still={ART[pose][1]} scene={pose} className={className}
    onUnavailable={(asset) => useShellStore.getState().noteDiagnostic(JSON.stringify({ event: "brand_owl.unavailable", scene: pose, asset }))} />;
}
