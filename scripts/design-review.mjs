#!/usr/bin/env node
// The design review run (docs/DESIGN_WORKFLOW.md, Reference bundle and review
// run). `baseline` keeps a chosen Pen design as a portable, never-overwritten
// bundle; `review` checks the production screen against that bundle's rules
// in a real browser and lays the reference, the current Pen and the actual
// screen side by side under the same width, theme, content and state.
// Everything it writes is under the ignored agents/runs/<slug>/design/.
import fs from 'node:fs';
import path from 'node:path';
import {createHash, randomUUID} from 'node:crypto';
import {createRequire} from 'node:module';
import {execFileSync, spawnSync} from 'node:child_process';
import {pathToFileURL} from 'node:url';
import {PenError, pen, requirePen} from './pen-cli.mjs';
import {evaluate} from './design-review-rules.mjs';

const SCHEMA = 'hide.design-baseline.v1';
const PIXEL_RATIO = 2;
const EXPORT_TIMEOUT_MS = 180_000;
const EXIT = {pass: 0, fail: 1, usage: 2, incomplete: 3};

const USAGE = `Usage:
  node scripts/design-review.mjs baseline <slug> --target <target> --approval user|delegated --reference "<where the decision is recorded>" [--name <name>] [--from <file.pen>] [--rules <rules.json>]
  node scripts/design-review.mjs review <slug> --baseline <bundle-dir> [--theme light,dark] [--width 292,240] [--content reference,long] [--scale 1,1.25] [--state rest,hover-parent,...] [--no-pen]
  node scripts/design-review.mjs show <bundle-dir>

baseline  Exports the target's Pen frames and keeps them, the Pen file, its libraries and
          image assets, and the layout rules in agents/runs/<slug>/design/baseline/<name>/.
          An existing bundle is never overwritten. Needs pen and a Pen login.
review    Runs the static design checks, measures the production screen in Chromium against
          the bundle's rules, exports the same frames from the current Pen file, and writes
          comparisons and report.md under agents/runs/<slug>/design/review/<run>/.
          Exit 0 PASS, 1 FAIL, 3 INCOMPLETE (something that has to be seen was not rendered).
show      Prints a bundle's approval, conditions and rules; needs neither Pen nor a browser.

Targets are listed in design/review-targets.json.`;

class UsageError extends Error {}

// -- shared ----------------------------------------------------------------------

function repositoryRoot() {
  return fs.realpathSync(execFileSync('git', ['rev-parse', '--show-toplevel'], {encoding: 'utf8'}).trim());
}

function sha256(file) {
  return createHash('sha256').update(fs.readFileSync(file)).digest('hex');
}

function parseArgs(args, flags) {
  const positional = [], options = {};
  for (let i = 0; i < args.length; i += 1) {
    const arg = args[i];
    if (!arg.startsWith('--')) { positional.push(arg); continue; }
    const name = arg.slice(2);
    if (!(name in flags)) throw new UsageError(`Unknown option --${name}`);
    if (flags[name] === 'boolean') options[name] = true;
    else {
      const value = args[i + 1];
      if (value === undefined || value.startsWith('--')) throw new UsageError(`--${name} needs a value`);
      options[name] = value;
      i += 1;
    }
  }
  return {positional, options};
}

function checkSlug(slug) {
  if (!slug || !/^[a-z0-9]+(?:-[a-z0-9]+)*$/.test(slug) || slug.length > 80) throw new UsageError('Use a lowercase, hyphen-separated slug (at most 80 characters).');
}

function targets(root) {
  return JSON.parse(fs.readFileSync(path.join(root, 'design/review-targets.json'), 'utf8'));
}

/** A directory for this run's output inside the ignored harness tree, refusing a symlinked path. */
function ignoredDirectory(root, relative) {
  execFileSync('git', ['check-ignore', '--quiet', '--', relative], {cwd: root});
  let directory = root;
  for (const part of relative.split('/')) {
    directory = path.join(directory, part);
    try { fs.mkdirSync(directory); }
    catch (error) { if (error.code !== 'EEXIST') throw error; }
    if (!fs.lstatSync(directory).isDirectory()) throw new Error(`Refusing non-directory or symlink: ${directory}`);
  }
  return directory;
}

/** Width and height of a PNG, read from its header. */
function pngSize(file) {
  const bytes = fs.readFileSync(file);
  if (bytes.length < 24 || bytes.toString('latin1', 1, 4) !== 'PNG') throw new Error(`Not a PNG: ${file}`);
  return {width: bytes.readUInt32BE(16), height: bytes.readUInt32BE(20)};
}

/** Exports Pen nodes to `<directory>/<node>.png` from `file`, through a session whose output is thrown away. */
async function exportFrames(file, nodes, directory, cwd, cleanup) {
  fs.mkdirSync(directory, {recursive: true});
  const session = path.join(directory, `.session-${randomUUID()}.pen`);
  const call = `Export(${JSON.stringify(nodes)}, "png", ${JSON.stringify(directory)}, {scale: ${PIXEL_RATIO}})`;
  try {
    const output = await pen(['interactive', '-i', file, '-o', session], {cwd, input: `execute({ input: ${JSON.stringify(call)} })\nexit()\n`, timeoutMs: EXPORT_TIMEOUT_MS, action: 'the Pen export', cleanup});
    const missing = nodes.filter(node => !fs.existsSync(path.join(directory, `${node}.png`)) || fs.statSync(path.join(directory, `${node}.png`)).size === 0);
    if (missing.length) throw new Error(`Pen exported no image for ${missing.join(', ')}; is the node id in ${path.basename(file)}?\n${output.slice(-2000)}`);
  } finally {
    fs.rmSync(session, {force: true});
  }
  return Object.fromEntries(nodes.map(node => [node, path.join(directory, `${node}.png`)]));
}

// -- baseline --------------------------------------------------------------------

/**
 * Copies a Pen file and every library it imports into `bundle`, and every
 * image a node's `url` names into `bundle/assets/`, rewriting the references
 * so the bundle opens from any directory on any machine.
 */
function copyPortable(sourceFile, bundle) {
  const assets = new Map();
  // Every document lands flat in the bundle under its own file name, so two
  // different files with one name cannot both be kept; a file imported twice
  // (or in a cycle) is copied once.
  const documents = new Map();
  const copyDocument = (file, name) => {
    file = fs.realpathSync(file);
    const claimed = documents.get(name);
    if (claimed === file) return;
    if (claimed) throw new Error(`Two Pen files would both be ${name} in the bundle: ${claimed} and ${file}; rename one`);
    documents.set(name, file);
    const document = JSON.parse(fs.readFileSync(file, 'utf8'));
    const from = path.dirname(file);
    for (const [alias, relative] of Object.entries(document.imports ?? {})) {
      const library = path.resolve(from, relative);
      if (!fs.existsSync(library)) throw new Error(`${path.basename(file)} imports ${relative} (${alias}), which does not exist`);
      const target = path.basename(library);
      copyDocument(library, target);
      document.imports[alias] = `./${target}`;
    }
    (function walk(node) {
      if (Array.isArray(node)) return node.forEach(walk);
      if (!node || typeof node !== 'object') return;
      if (typeof node.url === 'string' && !/^[a-z]+:/i.test(node.url)) {
        const asset = path.resolve(from, node.url);
        if (!fs.existsSync(asset)) throw new Error(`${path.basename(file)} uses image ${node.url}, which does not exist`);
        const hash = sha256(asset);
        if (!assets.has(hash)) assets.set(hash, {source: asset, name: `assets/${hash.slice(0, 12)}-${path.basename(asset)}`});
        node.url = assets.get(hash).name;
      }
      Object.values(node).forEach(walk);
    })(document.children ?? []);
    fs.writeFileSync(path.join(bundle, name), `${JSON.stringify(document, null, 2)}\n`);
  };
  copyDocument(sourceFile, path.basename(sourceFile));
  if (assets.size) fs.mkdirSync(path.join(bundle, 'assets'));
  for (const {source, name} of assets.values()) fs.copyFileSync(source, path.join(bundle, name));
}

function listFiles(directory, base = directory) {
  return fs.readdirSync(directory, {withFileTypes: true}).flatMap(entry => {
    const full = path.join(directory, entry.name);
    return entry.isDirectory() ? listFiles(full, base) : [path.relative(base, full)];
  }).sort();
}

async function baseline(root, args) {
  const {positional: [slug, ...extra], options} = parseArgs(args, {target: 'value', approval: 'value', reference: 'value', name: 'value', from: 'value', rules: 'value'});
  checkSlug(slug);
  if (extra.length) throw new UsageError(`Unexpected ${extra.join(' ')}`);
  const target = targets(root)[options.target];
  if (!target) throw new UsageError(`--target must be one of: ${Object.keys(targets(root)).join(', ')}`);
  if (!['user', 'delegated'].includes(options.approval)) throw new UsageError('--approval must be user (the operator chose this design) or delegated (a proposal made under delegated authority)');
  if (!options.reference?.trim()) throw new UsageError('--reference must say where the decision is recorded, e.g. "agents/prd/<slug>/prd.md D-08"');
  const name = options.name ?? new Date().toISOString().replace(/[-:]/g, '').replace(/\..+/, '').replace('T', '-');
  checkSlug(name);
  const from = path.resolve(root, options.from ?? target.file);
  if (!fs.existsSync(from)) throw new UsageError(`No Pen file at ${from}`);
  const rules = options.rules ? JSON.parse(fs.readFileSync(path.resolve(options.rules), 'utf8')) : target.rules;

  const parent = ignoredDirectory(root, `agents/runs/${slug}/design/baseline`);
  const destination = path.join(parent, name);
  if (fs.existsSync(destination)) throw new Error(`Baseline ${name} already exists and is never overwritten: ${destination}\nChoose another --name.`);
  const version = await requirePen(root);
  const temporary = path.join(parent, `.${name}-${randomUUID()}`);
  fs.mkdirSync(temporary);
  try {
    copyPortable(from, temporary);
    const document = path.join(temporary, path.basename(from));
    // Export from the bundle itself: the images prove the bundle opens on its own.
    const exported = await exportFrames(document, target.frames.map(frame => frame.referenceNode ?? frame.node), path.join(temporary, 'png'), temporary);
    const frames = target.frames.map(frame => {
      const size = pngSize(exported[frame.referenceNode ?? frame.node]);
      if (size.width / PIXEL_RATIO !== frame.width) throw new Error(`Frame ${frame.node} is ${size.width / PIXEL_RATIO}px wide, but the target declares ${frame.width}px; fix design/review-targets.json or the frame`);
      return {...frame, png: `png/${frame.referenceNode ?? frame.node}.png`, height: size.height / PIXEL_RATIO};
    });
    const head = execFileSync('git', ['rev-parse', 'HEAD'], {cwd: root, encoding: 'utf8'}).trim();
    const relativeSource = path.relative(root, from);
    const inRepository = relativeSource !== '..' && !relativeSource.startsWith(`..${path.sep}`) && !path.isAbsolute(relativeSource);
    // An approved proposal may belong to another checkout. Its bytes are
    // sealed below; this checkout cannot attest to that file's Git state.
    const dirty = inRepository
      ? execFileSync('git', ['status', '--porcelain', '--', relativeSource], {cwd: root, encoding: 'utf8'}).trim() !== ''
      : null;
    const manifest = {
      schema: SCHEMA,
      name,
      createdAt: new Date().toISOString(),
      target: options.target,
      approval: {kind: options.approval, reference: options.reference.trim()},
      source: {file: inRepository ? relativeSource : from, gitHead: inRepository ? head : null, uncommittedChanges: dirty, sha256: sha256(from)},
      pen: version,
      document: path.basename(from),
      pixelRatio: PIXEL_RATIO,
      frames,
      rules,
      files: {},
    };
    for (const file of listFiles(temporary)) manifest.files[file] = sha256(path.join(temporary, file));
    fs.writeFileSync(path.join(temporary, 'manifest.json'), `${JSON.stringify(manifest, null, 2)}\n`);
    // A rename publishes the whole bundle at once and fails if another run won the name.
    fs.renameSync(temporary, destination);
  } catch (error) {
    fs.rmSync(temporary, {recursive: true, force: true});
    throw error;
  }
  console.log(`Baseline ${name}: ${destination}`);
  console.log(`Approval: ${options.approval} (${options.reference.trim()})`);
  console.log(`Open it in Pen from any checkout: open -a Pen ${JSON.stringify(path.join(destination, path.basename(from)))}`);
  console.log(`Review against it: node scripts/design-review.mjs review ${slug} --baseline ${JSON.stringify(path.relative(root, destination))}`);
}

// -- bundle reading ------------------------------------------------------------

/** A bundle's manifest, with every file it lists checked against its hash. */
function readBundle(directory) {
  const file = path.join(directory, 'manifest.json');
  if (!fs.existsSync(file)) throw new UsageError(`No manifest.json in ${directory}; is it a baseline bundle?`);
  const manifest = JSON.parse(fs.readFileSync(file, 'utf8'));
  if (manifest.schema !== SCHEMA) throw new UsageError(`Unsupported bundle schema ${manifest.schema}; expected ${SCHEMA}`);
  const problems = [];
  for (const [name, hash] of Object.entries(manifest.files)) {
    const full = path.join(directory, name);
    if (!fs.existsSync(full)) problems.push(`${name} is missing`);
    else if (sha256(full) !== hash) problems.push(`${name} changed since the bundle was made`);
  }
  return {manifest, problems};
}

function show(args) {
  const {positional: [directory, ...extra]} = parseArgs(args, {});
  if (!directory || extra.length) throw new UsageError('show takes one bundle directory');
  const {manifest, problems} = readBundle(path.resolve(directory));
  console.log(`${manifest.name}: ${manifest.target}, ${manifest.approval.kind} (${manifest.approval.reference})`);
  console.log(`From ${manifest.source.file} at ${(manifest.source.gitHead === null ? "unversioned design" : manifest.source.gitHead.slice(0, 12))}${manifest.source.uncommittedChanges ? ' with uncommitted changes' : ''}, ${manifest.pen}, ${manifest.createdAt}`);
  for (const frame of manifest.frames) console.log(`Frame ${frame.node}: ${frame.theme}, ${frame.width}px, scale ${frame.scale}, ${frame.content} content, ${frame.state} -> ${frame.png}`);
  console.log(`Rules: ${JSON.stringify(manifest.rules)}`);
  console.log(problems.length ? `Integrity: ${problems.join('; ')}` : 'Integrity: every file matches its hash');
  process.exitCode = problems.length ? EXIT.fail : EXIT.pass;
}

// -- review ----------------------------------------------------------------------

function list(value, fallback, parse = String) {
  return value ? value.split(',').map(item => parse(item.trim())) : fallback;
}

/** Starts this checkout's web dev server and a headless Chromium, both owned by this process. */
async function openBrowser(root, cleanup) {
  const web = path.join(root, 'web');
  const require = createRequire(path.join(web, 'package.json'));
  if (!fs.existsSync(path.join(web, 'node_modules'))) throw new Error('web dependencies are not installed; run: pnpm --dir web install');
  const {createServer} = await import(pathToFileURL(require.resolve('vite')).href);
  const {chromium} = require('@playwright/test');
  const server = await createServer({root: web, configFile: path.join(web, 'vite.config.ts'), logLevel: 'error', server: {host: '127.0.0.1', port: 0, strictPort: false, hmr: false}});
  cleanup.push(() => server.close());
  await server.listen();
  const origin = server.resolvedUrls?.local?.[0]?.replace(/\/$/, '');
  if (!origin) throw new Error('The web dev server did not report an address');
  const browser = await chromium.launch();
  cleanup.push(() => browser.close());
  return {origin, browser};
}

function sceneUrl(origin, target, condition) {
  const query = new URLSearchParams({scene: target.scene, theme: condition.theme, width: String(condition.width), scale: String(condition.scale), content: condition.content});
  if (target.language) query.set('lang', target.language);
  return `${origin}/gallery?${query}`;
}

const conditionName = condition => `${condition.theme}-${condition.width}-${condition.content}-x${condition.scale}`;
const conditionWords = condition => `${condition.theme === 'light' ? 'Light' : 'Dark'} · ${condition.width}px · ${condition.content === 'long' ? 'long Korean titles' : 'reference content'} · text x${condition.scale}`;

/** Puts one scene into a named state through the product's own controls. */
async function enterState(page, state, target) {
  await page.mouse.move(0, 0);
  await page.evaluate(() => document.activeElement?.blur());
  if (state === 'hover-parent') await page.locator('nav[data-sidebar] li[data-pane="a1"]').first().hover();
  if (state === 'hover-project') await page.locator('nav[data-sidebar] [data-project-row="hide"]').hover();
  if (state === 'focus-project') await page.locator('nav[data-sidebar] [data-row-settings="hide"]').focus();
  if (state === 'settings-open') await page.locator('nav[data-sidebar] [data-row-settings="hide"]').click();
  if (state === 'focus-parent') {
    await page.keyboard.press('Shift');
    await page.locator('nav[data-sidebar] [data-agent-open="a1"]').focus();
  }
  if (state === 'checkout-closed') await page.locator('nav[data-sidebar] [data-checkout-toggle="hide:main"]').click();
  if (state === 'checkout-closed') await page.mouse.move(0, 0);
  // Sessions opens a row a level through its chevron; a pane header opens the tree popover from its tree button.
  if (state === 'children') await page.locator('[data-session-chevron], [data-pane-tree]').first().click();
  if (state === 'resolved') await page.locator('[data-session-group=resolved] [data-session-focus=group]').click();
  if (state === 'server-picker') await page.getByRole('button', {name:'Open server', exact:true}).click();
  // The Factory screen's other views, and the states a target's scene sets up from its address:
  // `urlStates` lists states that set `state=<name>`, or maps each state to the query it sets.
  const urlStates = Array.isArray(target.urlStates) ? Object.fromEntries(target.urlStates.map(name => [name, {state: name}])) : target.urlStates ?? {};
  if (['board', 'graph', 'sizes'].includes(state) || Object.hasOwn(urlStates, state)) {
    const url = new URL(page.url());
    for (const [key, value] of Object.entries(urlStates[state] ?? {state})) url.searchParams.set(key, value);
    await page.goto(url.href);
    await page.locator(target.readySelector ?? target.selector).waitFor();
    await page.evaluate(() => document.fonts.ready);
  }
  if (state === 'edge' && target.scene === 'pane-header') await page.locator('[data-pane-header-case="popover"] [data-pane-tree]').click();
  // The Observer frames that show a disclosure or a menu open.
  if (state === 'obs-direct') {
    await page.locator('[data-factory-settings-group="advanced"] > summary').click();
    await page.mouse.move(0, 0);
  }
  if (state === 'obs-pick') {
    await page.locator('[data-factory-worker-select]').click();
    await page.locator('[data-factory-worker-option]').first().waitFor();
    await page.locator('[data-factory-worker-option="2"]').hover();
  }
  // The Task page frame shows one of Factory AI's decisions with 다른 답 open and an answer typed.
  if (state === 'task') {
    await page.locator('[data-factory-task-page]').waitFor();
    await page.locator('[data-factory-decision-override="R2"]').click();
    await page.locator('[data-factory-override-text]').fill('작은 기록으로 잰 결과로 충분하다');
    await page.mouse.move(0, 0);
  }
  if (state === 'content-match') {
    await page.getByRole('searchbox', {name:'Search sessions'}).fill('화검');
    await page.locator('[data-content-match]').waitFor();
    await page.locator('[data-session-row]').first().click();
    await page.locator('[data-search-match="true"]').waitFor();
  }
  await page.waitForTimeout(120);
}

async function measure(page, geometry) {
  const rest = await geometry.rowBoxes(page);
  const hover = {}, focus = {};
  for (const target of await geometry.rowTargets(page)) {
    await page.locator(target.row).first().hover();
    await page.waitForTimeout(30);
    hover[target.key] = await geometry.rowBoxes(page);
    if (target.control) {
      await page.mouse.move(0, 0);
      await page.keyboard.press('Shift');
      await page.locator(target.control).first().focus();
      await page.waitForTimeout(30);
      focus[target.key] = await geometry.rowBoxes(page);
      await page.evaluate(() => document.activeElement?.blur());
    }
  }
  await page.mouse.move(0, 0);
  return {
    rest,
    hover,
    focus,
    columns: await geometry.agentColumns(page),
    partProblems: await geometry.rowPartProblems(page),
    rowsFit: await geometry.sidebarRowsFit(page),
    columnsAtEnd: await geometry.sidebarColumns(page),
    overflow: await geometry.sidebarOverflow(page, null),
  };
}

const dataUri = file => `data:image/png;base64,${fs.readFileSync(file).toString('base64')}`;
const escapeHtml = text => String(text).replace(/[&<>"]/g, c => ({'&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;'})[c]);

/** Lays captures side by side with their labels and writes one PNG. */
async function compose(browser, file, {title, subtitle, columns}) {
  const page = await browser.newPage({deviceScaleFactor: PIXEL_RATIO, viewport: {width: 400, height: 300}});
  try {
    const cells = columns.map(column => `<figure><figcaption><b>${escapeHtml(column.label)}</b><span>${escapeHtml(column.detail ?? '')}</span></figcaption>${
      column.image ? `<img src="${dataUri(column.image)}" style="width:${pngSize(column.image).width / PIXEL_RATIO}px">` : `<div class="missing">${escapeHtml(column.missing)}</div>`}</figure>`).join('');
    await page.setContent(`<!doctype html><html><head><style>
      body{margin:0;font:13px -apple-system,BlinkMacSystemFont,sans-serif;background:#f4f4f5;color:#18181b}
      main{display:inline-flex;flex-direction:column;gap:12px;padding:16px}
      header b{font-size:15px} header span{display:block;color:#52525b;margin-top:2px}
      .row{display:flex;gap:16px;align-items:flex-start}
      figure{margin:0;display:flex;flex-direction:column;gap:6px}
      figcaption{display:flex;flex-direction:column;max-width:300px} figcaption span{color:#52525b;font-size:12px}
      img{display:block;border:1px solid #d4d4d8}
      .missing{width:292px;height:160px;display:flex;align-items:center;justify-content:center;text-align:center;padding:12px;box-sizing:border-box;border:1px dashed #a1a1aa;color:#b91c1c}
    </style></head><body><main><header><b>${escapeHtml(title)}</b><span>${escapeHtml(subtitle)}</span></header><div class="row">${cells}</div></main></body></html>`);
    await page.locator('main').screenshot({path: file});
  } finally {
    await page.close();
  }
}

async function review(root, args) {
  const {positional: [slug, ...extra], options} = parseArgs(args, {baseline: 'value', theme: 'value', width: 'value', content: 'value', scale: 'value', state: 'value', 'no-pen': 'boolean'});
  checkSlug(slug);
  if (extra.length) throw new UsageError(`Unexpected ${extra.join(' ')}`);
  if (!options.baseline) throw new UsageError('review needs --baseline <bundle-dir>');
  const bundle = path.resolve(root, options.baseline);
  const {manifest, problems: bundleProblems} = readBundle(bundle);
  const target = targets(root)[manifest.target];
  if (!target) throw new UsageError(`The bundle's target ${manifest.target} is not in design/review-targets.json`);
  const conditions = target.conditions;
  const themes = list(options.theme, conditions.themes);
  const widths = list(options.width, conditions.widths, Number);
  const contents = list(options.content, conditions.contents);
  // A target whose screen does not follow the interface scale lists no scales and is measured at 1.
  const scales = list(options.scale, conditions.scales ?? [1], Number);
  const states = list(options.state, Object.keys(target.states));
  for (const theme of themes) if (!['light', 'dark'].includes(theme)) throw new UsageError(`Unknown theme ${theme}`);
  for (const width of widths) if (!(width > 0)) throw new UsageError(`Bad width ${width}`);
  for (const scale of scales) if (!(scale > 0)) throw new UsageError(`Bad scale ${scale}`);
  for (const content of contents) if (!conditions.contents.includes(content)) throw new UsageError(`Unknown content ${content}`);
  for (const state of states) if (!(state in target.states)) throw new UsageError(`Unknown state ${state}; the target has ${Object.keys(target.states).join(', ')}`);

  const stamp = `${new Date().toISOString().replace(/[-:]/g, '').replace(/\..+/, '').replace('T', '-')}-${randomUUID().slice(0, 6)}`;
  const out = ignoredDirectory(root, `agents/runs/${slug}/design/review/${stamp}`);
  const report = {
    status: 'PASS',
    createdAt: new Date().toISOString(),
    gitHead: execFileSync('git', ['rev-parse', 'HEAD'], {cwd: root, encoding: 'utf8'}).trim(),
    uncommittedChanges: execFileSync('git', ['status', '--porcelain'], {cwd: root, encoding: 'utf8'}).trim() !== '',
    baseline: {directory: bundle, name: manifest.name, approval: manifest.approval, source: manifest.source, integrity: bundleProblems},
    target: manifest.target,
    selection: {themes, widths, contents, scales, states},
    static: null,
    pen: null,
    rules: [],
    pairs: [],
    stateSheets: [],
    incomplete: [],
    judgment: target.judgment,
  };
  const incomplete = (reason, next) => report.incomplete.push({reason, next});
  // The run owns its Pen session, dev server and browser. A cancel at any point
  // closes what is open and joins its Pen task before ending the command.
  const cleanup = [];
  let stopping, cancelled = 0;
  const stop = () => stopping ??= (async () => {
    for (const close of cleanup.splice(0).reverse()) {
      try { await close(); }
      catch (error) {
        console.error(`Design review could not close a process it started: ${error.message}`);
        incomplete(`A process this run started did not close: ${error.message.split('\n')[0]}`, 'Check for a leftover vite or Chromium process from this checkout and end it.');
      }
    }
  })();
  const interrupt = signal => () => {
    cancelled = 128 + (signal === 'SIGINT' ? 2 : 15);
    void stop().finally(() => process.exit(cancelled));
  };
  const onInt = interrupt('SIGINT'), onTerm = interrupt('SIGTERM');
  process.once('SIGINT', onInt);
  process.once('SIGTERM', onTerm);
  try {
    await reviewParts(root, {noPen: Boolean(options['no-pen']), slug, bundle, manifest, target, themes, widths, contents, scales, states, out, report, incomplete, cleanup, cancelled: () => cancelled !== 0});
  } finally {
    process.removeListener('SIGINT', onInt);
    process.removeListener('SIGTERM', onTerm);
    await stop();
  }
  if (cancelled) return;

  const failedRules = report.rules.filter(rule => rule.pass === false);
  if (report.static.exitCode !== 0 || failedRules.length) report.status = 'FAIL';
  if (report.incomplete.length) report.status = report.status === 'FAIL' ? 'FAIL' : 'INCOMPLETE';
  fs.writeFileSync(path.join(out, 'report.json'), `${JSON.stringify(report, null, 2)}\n`);
  fs.writeFileSync(path.join(out, 'report.md'), markdown(report, root));
  console.log(`${report.status}: ${path.join(out, 'report.md')}`);
  for (const rule of failedRules.slice(0, 12)) console.log(`  FAIL ${rule.rule} [${rule.condition}]: ${rule.problems[0]}${rule.problems.length > 1 ? ` (+${rule.problems.length - 1} more)` : ''}`);
  for (const item of report.incomplete) console.log(`  INCOMPLETE ${item.reason}\n    next: ${item.next}`);
  process.exitCode = report.status === 'PASS' ? EXIT.pass : report.status === 'FAIL' ? EXIT.fail : EXIT.incomplete;
}

/** The static contract, the current Pen frames, and the measured and captured screen. */
async function reviewParts(root, {noPen, slug, bundle, manifest, target, themes, widths, contents, scales, states, out, report, incomplete, cleanup, cancelled}) {
  if (report.baseline.integrity.length) incomplete(`The reference bundle does not match its manifest: ${report.baseline.integrity.join('; ')}`, 'Restore the bundle from where it was copied, or make a new baseline with another --name.');

  // 1. The static design contract, as CI runs it.
  const contract = spawnSync(process.execPath, [path.join(root, 'scripts/check-design-contract.mjs')], {cwd: root, encoding: 'utf8'});
  report.static = {command: 'node scripts/check-design-contract.mjs', exitCode: contract.status, output: `${contract.stdout}${contract.stderr}`.trim().split('\n').slice(-12)};

  // 2. The same frames from the target's current Pen file, when Pen can render here.
  const currentFile = path.join(root, target.file);
  let current = {};
  if (noPen) {
    report.pen = {status: 'NOT RENDERED', reason: '--no-pen was given'};
  } else {
    try {
      await requirePen(root, {cleanup});
      if (cancelled()) return;
      current = await exportFrames(currentFile, manifest.frames.map(frame => frame.node), path.join(out, 'current-pen'), root, cleanup);
      const hash = sha256(currentFile);
      // Only the same file can be said to have changed; a proposal's bundle came from a scratch copy.
      report.pen = {status: 'RENDERED', file: target.file, sha256: hash, changedSinceBaseline: target.file === manifest.source.file ? hash !== manifest.source.sha256 : null};
    } catch (error) {
      report.pen = {status: 'NOT RENDERED', reason: error instanceof PenError ? error.reason : error.message.split('\n')[0]};
    }
  }
  if (cancelled()) return;
  if (report.pen.status !== 'RENDERED') incomplete(`The current Pen frames were not rendered: ${report.pen.reason}`, `Rerun on a machine with pen and a Pen login: node scripts/design-review.mjs review ${slug} --baseline ${path.relative(root, bundle)}`);

  // 3. The production screen in Chromium, measured and captured per condition.
  try {
    const geometry = await import(pathToFileURL(path.join(root, target.measurement ?? 'web/e2e/sidebar-geometry.mjs')).href);
    const {origin, browser} = await openBrowser(root, cleanup);
    const wanted = [];
    for (const theme of themes) for (const width of widths) for (const content of contents) for (const scale of scales) wanted.push({theme, width, content, scale});
    // A frame the bundle pairs is always captured under its own conditions.
    for (const frame of manifest.frames) {
      if (!wanted.some(condition => ['theme', 'width', 'content', 'scale'].every(key => condition[key] === frame[key]))) wanted.push({theme: frame.theme, width: frame.width, content: frame.content, scale: frame.scale});
    }
    const captures = new Map();
    for (const condition of wanted) {
      const height = target.viewportHeight ?? manifest.frames.find(frame => frame.theme === condition.theme && frame.width === condition.width)?.height ?? 900;
      const context = await browser.newContext({viewport: {width: target.viewportWidth ?? condition.width, height: Math.ceil(height)}, deviceScaleFactor: PIXEL_RATIO});
      try {
        const page = await context.newPage();
        const errors = [];
        page.on('pageerror', error => errors.push(error.message));
        await page.goto(sceneUrl(origin, target, condition));
        await page.locator(target.readySelector ?? target.selector).waitFor();
        await page.waitForTimeout(200);
        if (errors.length) throw new Error(`The scene raised: ${errors.join('; ')}`);
        const measured = geometry.measure ? await geometry.measure(page) : await measure(page, geometry);
        for (const result of evaluate(manifest.rules, measured)) report.rules.push({condition: conditionName(condition), ...result});
        for (const state of new Set(['rest', ...states])) {
          await page.goto(sceneUrl(origin, target, condition));
          await page.locator(target.readySelector ?? target.selector).waitFor();
          await page.waitForTimeout(150);
          // Each state is captured at its own frame's height when the bundle has one.
          const own = manifest.frames.find(frame => frame.state === state && ['theme', 'width', 'content', 'scale'].every(key => frame[key] === condition[key]));
          await page.setViewportSize({width: target.viewportWidth ?? condition.width, height: Math.ceil(target.viewportHeight ?? own?.height ?? height)});
          await enterState(page, state, target);
          const file = path.join(out, 'actual', `${conditionName(condition)}-${state}.png`);
          fs.mkdirSync(path.dirname(file), {recursive: true});
          if (target.capturePadding !== undefined) {
            const padding = target.capturePadding;
            if (!Number.isFinite(padding) || padding < 0) throw new Error('capturePadding must be nonnegative');
            const box = await page.locator(target.selector).boundingBox();
            if (!box) throw new Error(`No visible capture target: ${target.selector}`);
            const clip = {x: box.x - padding, y: box.y - padding, width: box.width + 2 * padding, height: box.height + 2 * padding};
            if (clip.x < 0 || clip.y < 0) throw new Error('The padded capture reaches outside the viewport');
            await page.screenshot({path: file, clip});
          } else await page.locator(target.selector).screenshot({path: file});
          captures.set(`${conditionName(condition)}/${state}`, file);
        }
      } finally {
        await context.close();
      }
    }

    // 4. Reference, current Pen and actual, only where all conditions match.
    for (const frame of manifest.frames) {
      const condition = {theme: frame.theme, width: frame.width, content: frame.content, scale: frame.scale};
      const actual = captures.get(`${conditionName(condition)}/${frame.state}`);
      const reference = path.join(bundle, frame.png);
      const pair = {frame: frame.node, conditions: {...condition, state: frame.state}, reference, current: current[frame.node] ?? null, actual, widths: {}};
      for (const [name, file] of [['reference', reference], ['current', pair.current], ['actual', actual]]) {
        if (file && fs.existsSync(file)) pair.widths[name] = pngSize(file).width / PIXEL_RATIO;
      }
      for (const [name, width] of Object.entries(pair.widths)) if (width !== frame.width) incomplete(`${frame.node}: the ${name} image is ${width}px wide, not ${frame.width}px, so it is not the same condition`, 'Fix the frame or the target so both draw the declared width.');
      pair.image = path.join(out, 'compare', `${frame.node}.png`);
      fs.mkdirSync(path.dirname(pair.image), {recursive: true});
      await compose(browser, pair.image, {
        title: `${manifest.target} · ${frame.state} · ${conditionWords(condition)}`,
        subtitle: `Same width, theme, content and state in all three. Reference: bundle ${manifest.name}, ${manifest.approval.kind === 'user' ? 'chosen by the operator' : 'delegated proposal, not user-approved'} (${manifest.approval.reference}).`,
        columns: [
          {label: 'Reference', detail: `baseline bundle, Pen node ${frame.node}`, image: reference},
          {label: 'Current Pen', detail: `${target.file} in this checkout`, image: pair.current, missing: `NOT RENDERED: ${report.pen.reason ?? ''}`},
          {label: 'Actual', detail: 'production Sidebar on the gallery scene, Chromium', image: actual, missing: 'NOT CAPTURED'},
        ],
      });
      report.pairs.push(pair);
    }

    // 5. The states and conditions no Pen frame draws, as the actual screen only.
    for (const condition of wanted) {
      const columns = [...new Set(['rest', ...states])].map(state => ({label: state, detail: target.states[state], image: captures.get(`${conditionName(condition)}/${state}`), missing: 'NOT CAPTURED'}));
      const image = path.join(out, 'states', `${conditionName(condition)}.png`);
      fs.mkdirSync(path.dirname(image), {recursive: true});
      const paired = manifest.frames.filter(frame => ['theme', 'width', 'content', 'scale'].every(key => frame[key] === condition[key])).map(frame => frame.state);
      await compose(browser, image, {
        title: `${manifest.target} · actual screen · ${conditionWords(condition)}`,
        subtitle: `Production Sidebar on the gallery scene. No Pen frame draws ${paired.length ? `any state here but ${paired.join(', ')}` : 'this condition'}; these are captures of the real screen only.`,
        columns,
      });
      report.stateSheets.push({condition: conditionName(condition), image});
    }
  } catch (error) {
    incomplete(`The browser part did not finish: ${error.message.split('\n')[0]}`, 'Fix the cause above and rerun the same command.');
  }
}

/** The report a reviewer reads: automated facts first, then what is left to a person. */
function markdown(report, root) {
  const rel = file => (file ? path.relative(root, file) : 'none');
  const lines = [];
  lines.push(`# Design review: ${report.target} - ${report.status}`, '');
  lines.push(`Checkout ${report.gitHead.slice(0, 12)}${report.uncommittedChanges ? ' with uncommitted changes' : ''}, ${report.createdAt}.`);
  lines.push(`Baseline ${report.baseline.name}: ${report.baseline.approval.kind === 'user' ? 'chosen by the operator' : 'delegated proposal, not user-approved'} (${report.baseline.approval.reference}), from ${report.baseline.source.file} at ${(report.baseline.source.gitHead === null ? "unversioned design" : report.baseline.source.gitHead.slice(0, 12))}.`, '');
  lines.push('## Automated facts', '');
  lines.push(`- Static design contract: \`${report.static.command}\` exit ${report.static.exitCode}.`);
  lines.push(`- Current Pen: ${report.pen.status}${report.pen.reason ? ` (${report.pen.reason})` : ''}${report.pen.status !== 'RENDERED' ? '' : report.pen.changedSinceBaseline === null ? `; ${report.pen.file}, while the baseline was made from ${report.baseline.source.file}` : report.pen.changedSinceBaseline ? '; the Pen file changed since the baseline' : '; the Pen file is the baseline\'s'}.`);
  lines.push(`- Reference bundle integrity: ${report.baseline.integrity.length ? report.baseline.integrity.join('; ') : 'every file matches its hash'}.`);
  lines.push(`- Conditions measured: themes ${report.selection.themes.join('/')}, widths ${report.selection.widths.join('/')}px, content ${report.selection.contents.join('/')}, text scale ${report.selection.scales.join('/')}.`);
  lines.push('', '| Rule | Condition | Expected | Measured | Result |', '| --- | --- | --- | --- | --- |');
  for (const rule of report.rules) lines.push(`| ${rule.rule} | ${rule.condition} | ${rule.expected} | ${rule.measured} | ${rule.pass === null ? 'not judged' : rule.pass ? 'PASS' : 'FAIL'} |`);
  const failed = report.rules.filter(rule => rule.pass === false);
  if (failed.length) {
    lines.push('', '### Rule failures', '');
    for (const rule of failed) for (const problem of rule.problems.slice(0, 8)) lines.push(`- ${rule.rule} [${rule.condition}]: ${problem}`);
  }
  if (report.incomplete.length) {
    lines.push('', '### Not complete', '');
    for (const item of report.incomplete) lines.push(`- ${item.reason} Next: ${item.next}`);
  }
  lines.push('', '## Comparisons (same width, theme, content and state)', '');
  for (const pair of report.pairs) lines.push(`- ${pair.frame} (${pair.conditions.theme}, ${pair.conditions.width}px, ${pair.conditions.content}, x${pair.conditions.scale}, ${pair.conditions.state}): ${rel(pair.image)}`);
  lines.push('', '## Actual-only state captures (no matching Pen frame)', '');
  for (const sheet of report.stateSheets) lines.push(`- ${sheet.condition}: ${rel(sheet.image)}`);
  lines.push('', '## Left to human judgment', '');
  for (const item of report.judgment) lines.push(`- ${item}`);
  lines.push('- A PASS covers only the rules above; it is not a visual approval.');
  return `${lines.join('\n')}\n`;
}

// -- entry -----------------------------------------------------------------------

try {
  const [command, ...args] = process.argv.slice(2);
  const root = repositoryRoot();
  if (!command || command === '--help' || command === 'help') console.log(USAGE);
  else if (command === 'baseline') await baseline(root, args);
  else if (command === 'review') await review(root, args);
  else if (command === 'show') show(args);
  else throw new UsageError(`Unknown command ${command}`);
} catch (error) {
  if (error instanceof UsageError) {
    console.error(`${error.message}\n\n${USAGE}`);
    process.exitCode = EXIT.usage;
  } else {
    console.error(`Design review failed: ${error.message}`);
    process.exitCode = error instanceof PenError ? EXIT.incomplete : EXIT.fail;
  }
}
