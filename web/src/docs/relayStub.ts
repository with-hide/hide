// The phone part draws the phone app's list from invented rows and never opens a relay, but the phone's modules
// import the relay protocol's WASM package, which only `prepare:relay` (cargo) builds. `docs-embed.config.ts`
// points those imports here so the docs build needs no cargo; any call that would reach a relay fails loudly.
const unavailable = (name: string): never => {
  throw new Error(`The docs embed has no relay: ${name} was called`);
};

export default function init(): Promise<never> {
  return unavailable("init");
}
export const decodeQr = () => unavailable("decodeQr");
export const decodeServer = () => unavailable("decodeServer");
export const encodeClient = () => unavailable("encodeClient");
export class NoiseSession {
  constructor() {
    unavailable("NoiseSession");
  }
}
