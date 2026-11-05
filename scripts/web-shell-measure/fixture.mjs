#!/usr/bin/env node
// The measured topology made the way the web shell makes it (PRD
// engine-bundle-3-core-owns D-15, D-24): every workspace, tab and split is one
// typed event on the daemon's WebSocket (`create_workspace`, `create_tab`,
// `create_pane`), and the ids come back from the snapshot frames, so two
// builds start from the same fixture, measured in a primary Git checkout.
// Scales (PRD instant-pane-topology D-18):
//   keys:     the measured checkout with five panes in one tab
//   none:     the measured checkout, its pane and the tab switch target
//   operator: 43 workspaces, 62 tabs, 66 panes, 5 printing
//   double:   86 workspaces, 124 tabs, 132 panes, 10 printing
// No pane reports an agent: a report has no daemon event, so the fixture has
// none. A printing pane is a split whose folder holds the `.measure-go` FIFO
// the private `.zshrc` (run.sh) blocks on; `drive` writes to each.
// usage (from run.sh, in the isolated environment):
//   fixture.mjs build <none|operator|double|keys>  -> fixture.json, scale.json
//   fixture.mjs drive                         -> starts the printers
// Reads MEASURE_RUN_DIR, MEASURE_FIXTURE and MEASURE_HIDE_STATE.
import { execFileSync } from "node:child_process";
import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import { connectStateDir } from "../../web/e2e/hided-control.mjs";

const runDir = process.env.MEASURE_RUN_DIR;
// How long a step waits for the snapshot to answer.
const STEP_MS = 30_000;
const stateDir = process.env.MEASURE_HIDE_STATE;
if (!runDir || !stateDir) throw new Error("MEASURE_RUN_DIR and MEASURE_HIDE_STATE are required");

const SCALES = {
  none: { workspaces: 1, tabs: 2, panes: 2, printing: 0 },
  operator: { workspaces: 43, tabs: 62, panes: 66, printing: 5 },
  double: { workspaces: 86, tabs: 124, panes: 132, printing: 10 },
};

const panesOf = (node) => (node.type === "pane" ? [node.pane_id] : [...panesOf(node.first), ...panesOf(node.second)]);
const layouts = (rest) => rest?.pane_layouts ?? [];
/** The checkout at `folder` with its project: a linked worktree is a checkout under its repository's row. */
const checkoutAt = (rest, folder) => {
  for (const workspace of rest?.navigator?.workspaces ?? []) {
    const checkout = (workspace.checkouts ?? []).find((row) => row.path === folder);
    if (checkout) return { workspace, checkout };
  }
  return undefined;
};
const checkoutById = (rest, id) => (rest?.navigator?.workspaces ?? []).flatMap((workspace) => workspace.checkouts ?? []).find((row) => row.id === id);
const tabsOf = (checkout) => checkout?.agent_groups?.groups?.flatMap((group) => group.tabs) ?? [];
const requestId = () => `measure-${crypto.randomUUID()}`;

/**
 * Registers `folder` as a checkout, confirming it outside home as Add project
 * does, and returns its workspace, first tab and pane. A folder inside a
 * project outside home is answered with the project's root to confirm in
 * its place, and is sent once more with that root, as the dialog does.
 */
async function addCheckout(control, folder, label) {
  const ask = (confirmed) => control.send("create_workspace", { path: folder, label, initialize_git: false, confirmed_outside_home: confirmed });
  const seen = control.refusals.length;
  ask(folder);
  let asked = false;
  return control.until((rest) => {
    const refusal = control.refusals.slice(seen).find((row) => row.path === folder);
    if (refusal) {
      if (refusal.reason !== "needs_confirmation" || !refusal.confirm || asked) throw new Error(`the daemon refused ${folder}: ${JSON.stringify(refusal)}`);
      asked = true;
      control.refusals.splice(seen);
      ask(refusal.confirm);
    }
    const found = checkoutAt(rest, folder);
    const tab = tabsOf(found?.checkout)[0];
    const layout = layouts(rest).find((row) => row.tab_id === tab);
    return layout && { workspace: found.workspace.id, checkout: found.checkout.id, tab, pane: panesOf(layout.root)[0] };
  }, `the checkout ${folder}`);
}

/** A new tab in `made`'s checkout; returns it with its pane. */
async function addTab(control, made, label) {
  const before = new Set(layouts(await control.until((rest) => rest, "a snapshot")).map((row) => row.tab_id));
  control.send("create_tab", { workspace_id: made.workspace, checkout_id: made.checkout, label, request_id: requestId() });
  return control.until((rest) => {
    const layout = layouts(rest).find((row) => !before.has(row.tab_id) && tabsOf(checkoutById(rest, made.checkout)).includes(row.tab_id));
    return layout && { tab: layout.tab_id, pane: panesOf(layout.root)[0] };
  }, `a new tab in ${made.workspace}`);
}

/**
 * Splits `pane` of `tab` to the right with the shell in `cwd`, as ⌘D does:
 * a split acts on the pane the core holds as the current terminal, so the
 * pane is focused first. Returns the new pane.
 */
async function split(control, tab, pane, cwd, direction = "right") {
  const count = (rest) => {
    const layout = layouts(rest).find((row) => row.tab_id === tab);
    return layout ? panesOf(layout.root) : [];
  };
  control.send("focus_pane", { pane_id: pane, origin: "operator" });
  const before = count(await control.until((rest) => rest?.terminal?.pane_id === pane && count(rest).includes(pane) && rest, `the focus on ${pane}`));
  control.send("create_pane", { tab_id: tab, cwd, command: null, direction, request_id: requestId() });
  return control.until((rest) => count(rest).find((id) => !before.includes(id)), `the split of ${pane}`);
}

function gitProject(dir, n) {
  if (fs.existsSync(path.join(dir, ".git"))) return;
  fs.mkdirSync(dir, { recursive: true });
  const git = (...args) => execFileSync("git", ["-C", dir, ...args], { stdio: "ignore" });
  git("init", "--quiet");
  fs.writeFileSync(path.join(dir, "README"), `scale project ${n}\n`);
  git("add", "README");
  git("-c", "commit.gpgsign=false", "-c", "user.email=measure@example.invalid", "-c", "user.name=measure", "commit", "--quiet", "-m", "scale project");
}

async function build(scaleName) {
  const keys = scaleName === "keys";
  const scale = keys ? { workspaces: 1, tabs: 1, panes: 5, printing: 0 } : SCALES[scaleName];
  if (!scale) throw new Error("scale must be none, operator, double or keys");
  const fixture = fs.realpathSync(process.env.MEASURE_FIXTURE ?? "");
  const extraTabs = keys ? 0 : scale.tabs - 2 - (scale.workspaces - 1);
  const rootPrinters = Math.max(0, scale.printing - (scale.panes - scale.tabs));
  // Extra tabs use the first projects. Root printers use the last ones so
  // their FIFO cannot also block an unlisted new tab in the same folder.
  if (rootPrinters && extraTabs + rootPrinters > scale.workspaces - 1) throw new Error("scale has too few projects without extra tabs for its root printers");
  const control = await connectStateDir(stateDir, STEP_MS);
  try {
    const measured = await addCheckout(control, fixture, "measure");
    const switchTab = keys ? null : await addTab(control, measured, "switch");
    const keyPanes = [];
    if (keys) {
      let from = measured.pane;
      for (const direction of ["right", "down", "right", "down"]) {
        from = await split(control, measured.tab, from, fixture, direction);
        keyPanes.push(from);
      }
    }
    const tabPanes = [];
    const others = [];
    const printing = [];
    const root = path.join(runDir, "scale");
    for (let n = 1; n < scale.workspaces; n += 1) {
      const repo = path.join(root, `project-${n}`);
      gitProject(repo, n);
      const rootPrinter = n >= scale.workspaces - rootPrinters;
      if (rootPrinter) execFileSync("mkfifo", [path.join(repo, ".measure-go")]);
      const made = await addCheckout(control, fs.realpathSync(repo), `project-${n}`);
      others.push({ ...made, repo: fs.realpathSync(repo) });
      tabPanes.push({ tab: made.tab, pane: made.pane, cwd: fs.realpathSync(repo) });
      if (rootPrinter) printing.push({ pane: made.pane, fifo: path.join(repo, ".measure-go") });
    }
    for (let n = 0; n < extraTabs; n += 1) await addTab(control, others[n % others.length], `extra-${n}`);
    for (let n = 0; !keys && n < scale.panes - scale.tabs; n += 1) {
      const target = tabPanes[n];
      let cwd = target.cwd;
      const printer = printing.length < scale.printing;
      if (printer) {
        // The printer waits in its shell on this FIFO until `drive`.
        cwd = path.join(root, `print-${n}`);
        fs.mkdirSync(cwd, { recursive: true });
        if (!fs.existsSync(path.join(cwd, ".measure-go"))) execFileSync("mkfifo", [path.join(cwd, ".measure-go")]);
        cwd = fs.realpathSync(cwd);
      }
      const pane = await split(control, target.tab, target.pane, cwd);
      if (printer) printing.push({ pane, fifo: path.join(cwd, ".measure-go") });
    }
    // The measured checkout in front again, as its row's click and its pane's
    // focus bring it: each registration brought its own checkout forward.
    control.send("focus_checkout", { workspace_id: measured.workspace, checkout_id: measured.checkout });
    control.send("focus_pane", { pane_id: measured.pane, origin: "operator" });
    const counted = await control.until((rest) => {
      const all = layouts(rest);
      const panes = all.flatMap((row) => panesOf(row.root));
      // A workspace is the engine's: a project row lists the session workspaces under it.
      const workspaces = new Set((rest.navigator?.workspaces ?? []).flatMap((row) => row.session_workspace_ids ?? [])).size;
      const front = rest.navigator?.focused_checkout_id === measured.checkout && rest.terminal?.pane_id === measured.pane;
      return front && workspaces >= scale.workspaces && all.length >= scale.tabs && panes.length >= scale.panes && { tabs: all.length, panes: panes.length, workspaces };
    }, `${scale.tabs} tabs and ${scale.panes} panes with the measured pane in front`);
    const valid = ["workspaces", "tabs", "panes"].every((key) => counted[key] === scale[key]) && printing.length === scale.printing;
    fs.writeFileSync(path.join(runDir, "scale.json"), JSON.stringify({ ...counted, scale: scaleName, agents_reported: 0, printing_targets: scale.printing, printing_panes: printing.length, target: scale, valid }));
    if (!valid) throw new Error(`fixture shape mismatch: expected ${JSON.stringify(scale)}, observed ${JSON.stringify({ ...counted, printing: printing.length })}`);
    const out = { scale: scaleName, pane: measured.pane, tab: measured.tab, workspace: measured.workspace, checkout: measured.checkout, switch_tab: switchTab?.tab ?? null, switch_pane: switchTab?.pane ?? null, printing, key_panes: keyPanes };
    fs.writeFileSync(path.join(runDir, "fixture.json"), JSON.stringify(out, null, 2));
    console.log(JSON.stringify(out));
  } finally {
    control.close();
  }
}

async function drive() {
  const { printing } = JSON.parse(fs.readFileSync(path.join(runDir, "fixture.json"), "utf8"));
  const deadline = Date.now() + STEP_MS;
  const started = [];
  let complete = false;
  try {
    for (const { pane, fifo } of printing) {
      let fd;
      while (fd === undefined) {
        try {
          fd = fs.openSync(fifo, fs.constants.O_WRONLY | fs.constants.O_NONBLOCK);
        } catch (error) {
          if (error.code !== "ENXIO" || Date.now() >= deadline) throw error;
          await delay(50);
        }
      }
      try { fs.writeSync(fd, "go\n"); } finally { fs.closeSync(fd); }
      // The private shell writes this once, after its first flushed output.
      // This is startup evidence outside the measured window, not a claim of
      // uninterrupted progress throughout the later measurement.
      const marker = path.join(path.dirname(fifo), ".measure-printing");
      let pid;
      while (pid === undefined) {
        const text = fs.existsSync(marker) ? fs.readFileSync(marker, "utf8") : "";
        if (/^[1-9][0-9]*\n$/.test(text)) pid = Number(text.trim());
        else if (Date.now() >= deadline) throw new Error(`printer ${pane} did not confirm its first output`);
        else await delay(50);
      }
      started.push({ pane, program_pid: pid });
    }
    complete = true;
  } finally {
    fs.writeFileSync(path.join(runDir, "printing-start.json"), JSON.stringify({ started, count: started.length, complete, boundary: "marker after first flushed line, before measurement" }));
  }
}

const [mode, ...args] = process.argv.slice(2);
if (mode === "build") await build(args[0] ?? "none");
else if (mode === "drive") await drive();
else throw new Error("usage: fixture.mjs build <none|operator|double|keys> | drive");
