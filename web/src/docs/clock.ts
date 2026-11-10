// The docs' pictures are the same every time they are drawn: the page's clock stands at one fixed instant, so
// every elapsed time, "so far" and age the product's components compute from `Date.now()` is a constant, and a
// capture taken now is byte-identical to one taken two minutes later. Import this first in the entry, before any
// module reads the clock. The product's components take no `now` to pass, so the clock itself is what stands still.

import { NOW } from "./now";

const frozen = new Proxy(Date, {
  construct: (target, args: unknown[]) => Reflect.construct(target, args.length === 0 ? [NOW] : args),
  get: (target, property, receiver) => (property === "now" ? () => NOW : Reflect.get(target, property, receiver)),
});
globalThis.Date = frozen;
