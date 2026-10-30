// Throwaway diagnosis for with-hide/hide-private issue 354, never merged.
// Counts the wall and processor time each test file and test spends building Intl
// objects (and in the first localeCompare), so a slow first-wave test says whether
// its time went to ICU and whether it waited (wall >> cpu) or computed.
import { appendFileSync, mkdirSync } from "node:fs";
import { join } from "node:path";
import { afterEach, beforeEach } from "vitest";

const directory = process.env.DIAG_DIR;
if (!directory) throw new Error("DIAG_DIR is required for the diagnosis setup");
mkdirSync(directory, { recursive: true });
const sink = join(directory, `worker-${process.pid}.log`);
const write = (line: string) => appendFileSync(sink, `${new Date().toISOString()} ${line}\n`);

type Totals = { wall: number; cpu: number; calls: number; slowest: string; slowestWall: number };
const totals: Totals = { wall: 0, cpu: 0, calls: 0, slowest: "", slowestWall: 0 };
const cpuMs = () => { const usage = process.cpuUsage(); return (usage.user + usage.system) / 1000; };

function measured<T>(name: string, run: () => T): T {
  const wall = performance.now();
  const cpu = cpuMs();
  try { return run(); } finally {
    const spent = performance.now() - wall;
    totals.wall += spent;
    totals.cpu += cpuMs() - cpu;
    totals.calls += 1;
    if (spent > totals.slowestWall) { totals.slowestWall = spent; totals.slowest = name; }
  }
}

const g = globalThis as unknown as { __diagIntl?: boolean };
if (!g.__diagIntl) {
  g.__diagIntl = true;
  const intl = Intl as unknown as Record<string, unknown>;
  for (const name of ["NumberFormat", "PluralRules", "RelativeTimeFormat", "DateTimeFormat", "Collator", "ListFormat", "Segmenter", "DisplayNames"]) {
    const original = intl[name] as (new (...args: unknown[]) => object) | undefined;
    if (!original) continue;
    intl[name] = new Proxy(original, {
      construct: (target, args) => measured(`${name}(${String(args[0] ?? "")})`, () => Reflect.construct(target, args)),
      apply: (target, self, args) => measured(`${name}(${String(args[0] ?? "")})`, () => Reflect.apply(target, self, args)),
    });
  }
  const prototypes: [object, string][] = [[Number.prototype, "toLocaleString"], [Date.prototype, "toLocaleString"], [Date.prototype, "toLocaleDateString"], [Date.prototype, "toLocaleTimeString"], [String.prototype, "localeCompare"]];
  for (const [prototype, name] of prototypes) {
    const original = (prototype as Record<string, (...args: unknown[]) => unknown>)[name];
    let first = true;
    (prototype as Record<string, unknown>)[name] = function (this: unknown, ...args: unknown[]) {
      if (!first) return original.apply(this, args);
      first = false;
      return measured(name, () => original.apply(this, args));
    };
  }
}

let file = String((globalThis as unknown as { __vitest_worker__?: { filepath?: string } }).__vitest_worker__?.filepath ?? "?").replace(/^.*[\\/]web[\\/]/, "");
const fileStart = performance.now();
let fileReported = false;
let testWall = 0;
let testCpu = 0;
let testIntl: Totals = { ...totals };
let faults = 0;
let index = 0;

let current = "(collect)";
let lastTick = performance.now();
let lastTickCpu = cpuMs();
const ticker = setInterval(() => {
  const now = performance.now();
  const cpu = cpuMs();
  if (now - lastTick > 400) write(`stall ${file} ${current.slice(0, 50)}: ${(now - lastTick - 100).toFixed(0)} ms, cpu ${(cpu - lastTickCpu).toFixed(0)} ms in it`);
  lastTick = now;
  lastTickCpu = cpu;
}, 100);
ticker.unref();

const snapshot = () => ({ ...totals });
const fmt = (value: number) => value.toFixed(0);

beforeEach((context) => {
  if (!fileReported) {
    fileReported = true;
    write(`file ${file}: uptime ${fmt(process.uptime() * 1000)} ms, before tests ${fmt(performance.now() - fileStart)} ms, intl so far wall ${fmt(totals.wall)} cpu ${fmt(totals.cpu)} calls ${totals.calls} slowest ${totals.slowest} ${fmt(totals.slowestWall)} ms`);
  }
  current = context.task.name;
  testIntl = snapshot();
  faults = process.resourceUsage().majorPageFault;
  testCpu = cpuMs();
  testWall = performance.now();
});

afterEach((context) => {
  const wall = performance.now() - testWall;
  const cpu = cpuMs() - testCpu;
  const intlWall = totals.wall - testIntl.wall;
  index += 1;
  current = "(between tests)";
  if (index > 2 && wall < 400) return;
  const slowest = totals.slowestWall > testIntl.slowestWall ? ` slowest ${totals.slowest} ${fmt(totals.slowestWall)} ms` : "";
  write(`test ${file} #${index} ${context.task.name.slice(0, 60)}: wall ${fmt(wall)} cpu ${fmt(cpu)} faults ${process.resourceUsage().majorPageFault - faults} intl wall ${fmt(intlWall)} cpu ${fmt(totals.cpu - testIntl.cpu)} calls ${totals.calls - testIntl.calls}${slowest}`);
});
