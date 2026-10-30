// Throwaway diagnosis for with-hide/hide-private issue 354, never merged.
// The "global" variant: vitest's own process builds the shell's Intl objects once,
// before any worker starts, and records how long each took (wall against cpu).
import { appendFileSync, mkdirSync } from "node:fs";
import { join } from "node:path";
import { formatDateTime, formatNumber, formatPercent, formatRelativeTime, formatUnit } from "../src/i18n/format";
import { INTERFACE_LANGUAGES } from "../src/i18n/locale";

export default function setup() {
  const directory = process.env.DIAG_DIR;
  if (!directory) throw new Error("DIAG_DIR is required for the diagnosis global setup");
  mkdirSync(directory, { recursive: true });
  const sink = join(directory, `main-${process.pid}.log`);
  const cpuMs = () => { const usage = process.cpuUsage(); return (usage.user + usage.system) / 1000; };
  const start = performance.now();
  const startCpu = cpuMs();
  const startFaults = process.resourceUsage().majorPageFault;
  const step = (name: string, run: () => unknown) => {
    const wall = performance.now();
    const cpu = cpuMs();
    const faults = process.resourceUsage().majorPageFault;
    run();
    appendFileSync(sink, `${new Date().toISOString()} global ${name}: wall ${(performance.now() - wall).toFixed(0)} cpu ${(cpuMs() - cpu).toFixed(0)} faults ${process.resourceUsage().majorPageFault - faults}\n`);
  };
  for (const language of INTERFACE_LANGUAGES) {
    step(`${language} number`, () => formatNumber(language, 1));
    step(`${language} percent`, () => formatPercent(language, 1));
    step(`${language} relative`, () => formatRelativeTime(language, -1, "minute"));
    step(`${language} units`, () => { for (const unit of ["second", "minute", "hour", "day"] as const) formatUnit(language, 1, unit); });
    step(`${language} date`, () => formatDateTime(language, 0, { dateStyle: "medium", timeStyle: "short" }));
    step(`${language} plural`, () => new Intl.PluralRules(language).select(1));
  }
  step("localeCompare", () => "a".localeCompare("b"));
  appendFileSync(sink, `${new Date().toISOString()} global total: wall ${(performance.now() - start).toFixed(0)} cpu ${(cpuMs() - startCpu).toFixed(0)} faults ${process.resourceUsage().majorPageFault - startFaults} uptime ${(process.uptime() * 1000).toFixed(0)}\n`);
}
