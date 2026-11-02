#!/usr/bin/env node
// The measured topology made the way the web shell makes it, on either engine
// (PRD engine-bundle-3-core-owns D-15, D-24): every workspace, tab and split
// is one typed event on the daemon's WebSocket (`create_workspace`,
// `create_tab`, `create_pane`), and the ids come back from the snapshot
// frames, so a Herdr run and an own-engine run of the same build start from
// the same fixture. Scales (PRD instant-pane-topology D-18):
//   none:     the measured checkout, its pane and the tab switch target
//   operator: 43 workspaces, 62 tabs, 66 panes, 5 printing
//   double:   86 workspaces, 124 tabs, 132 panes, 10 printing
// No pane reports an agent: a report is Herdr's own call (`pane
// report-agent`) and has no daemon event, so neither engine's fixture has
// one. A printing pane is a split whose folder holds the `.measure-go` FIFO
// the private `.zshrc` (run.sh) blocks on; `drive` writes to each.
// usage (from run.sh, in the isolated environment):
//   fixture.mjs build <none|operator|double>  -> fixture.json, scale.json
//   fixture.mjs drive                         -> starts the printers
// Reads MEASURE_RUN_DIR, MEASURE_FIXTURE and MEASURE_HIDE_STATE.
import { execFileSync } from "node:child_process";
import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
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
async function split(control, tab, pane, cwd) {
  const count = (rest) => panesOf(layouts(rest).find((row) => row.tab_id === tab).root);
  control.send("focus_pane", { pane_id: pane, origin: "operator" });
  const before = count(await control.until((rest) => rest?.terminal?.pane_id === pane && rest, `the focus on ${pane}`));
  control.send("create_pane", { tab_id: tab, cwd, command: null, direction: "right", request_id: requestId() });
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
  const scale = SCALES[scaleName];
  if (!scale) throw new Error("scale must be none, operator or double");
  const fixture = fs.realpathSync(process.env.MEASURE_FIXTURE ?? "");
  const control = await connectStateDir(stateDir, STEP_MS);
  try {
    const measured = await addCheckout(control, fixture, "measure");
    const switchTab = await addTab(control, measured, "switch");
    const tabPanes = [];
    const others = [];
    const root = path.join(runDir, "scale");
    for (let n = 1; n < scale.workspaces; n += 1) {
      const repo = path.join(root, `project-${n}`);
      gitProject(repo, n);
      const made = await addCheckout(control, fs.realpathSync(repo), `project-${n}`);
      others.push({ ...made, repo: fs.realpathSync(repo) });
      tabPanes.push({ tab: made.tab, pane: made.pane, cwd: fs.realpathSync(repo) });
    }
    const extraTabs = scale.tabs - 2 - others.length;
    for (let n = 0; n < extraTabs; n += 1) await addTab(control, others[n % others.length], `extra-${n}`);
    const printing = [];
    for (let n = 0; n < scale.panes - scale.tabs; n += 1) {
      const target = tabPanes[n];
      let cwd = target.cwd;
      if (n < scale.printing) {
        // The printer waits in its shell on this FIFO until `drive`.
        cwd = path.join(root, `print-${n}`);
        fs.mkdirSync(cwd, { recursive: true });
        if (!fs.existsSync(path.join(cwd, ".measure-go"))) execFileSync("mkfifo", [path.join(cwd, ".measure-go")]);
        cwd = fs.realpathSync(cwd);
      }
      const pane = await split(control, target.tab, target.pane, cwd);
      if (n < scale.printing) printing.push({ pane, fifo: path.join(cwd, ".measure-go") });
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
      return front && all.length >= scale.tabs && panes.length >= scale.panes && { tabs: all.length, panes: panes.length, workspaces };
    }, `${scale.tabs} tabs and ${scale.panes} panes with the measured pane in front`);
    const out = { scale: scaleName, pane: measured.pane, tab: measured.tab, workspace: measured.workspace, switch_tab: switchTab.tab, switch_pane: switchTab.pane, printing };
    fs.writeFileSync(path.join(runDir, "fixture.json"), JSON.stringify(out, null, 2));
    fs.writeFileSync(path.join(runDir, "scale.json"), JSON.stringify({ ...counted, scale: scaleName, agents_reported: 0, printing_targets: scale.printing, target: scale }));
    console.log(JSON.stringify(out));
  } finally {
    control.close();
  }
}

function drive() {
  const { printing } = JSON.parse(fs.readFileSync(path.join(runDir, "fixture.json"), "utf8"));
  // Opening a FIFO for writing waits for its reader: the pane's shell.
  for (const { fifo } of printing) fs.writeFileSync(fifo, "go\n");
}

const [mode, ...args] = process.argv.slice(2);
if (mode === "build") await build(args[0] ?? "none");
else if (mode === "drive") drive();
else throw new Error("usage: fixture.mjs build <none|operator|double> | drive");
