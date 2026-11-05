// One terminal: a lead pane starts eleven workers, which tile the screen and ring; in hide they fold into
// the pane that started them, its second row and the sidebar carry them, a child's question climbs to its
// parent and the results come home. Drawn by the page over invented work; it plays only while on screen
// and the tab is visible, a chapter press plays from that chapter, and under reduced motion the stage holds
// a chapter's last frame (Filed until a press). Without script the stage is not shown.
(() => {
const stageEl = document.getElementById("ot-stage");
if (!stageEl) return;
const W = 1280, H = 686, SIDE = 320, G = 6, NS = "http://www.w3.org/2000/svg";
const LOGO = {
  "Claude Code": "./assets/agents/claude-code.png",
  "Codex": "./assets/agents/codex.png",
  "OpenCode": "./assets/agents/opencode.svg",
  "Antigravity": "./assets/agents/antigravity.svg",
};
const img = (a, cls = "logo") => `<img class="${cls}" src="${LOGO[a]}" alt="">`;
const KIDS = [
  ["Codex", "db migration", "writing 0042", "Allow this command?", "psql staging -c \"ALTER TABLE invoices DROP COLUMN legacy_id\" (y/n)", "needs your approval"],
  ["Claude Code", "flaky e2e", "rerun 3/5 passed", "Question", "Fails 2 of 5 on Linux only. Quarantine it, or keep digging?", "has a question"],
  ["OpenCode", "install docs", "editing INSTALL.md", "Finished", "INSTALL.md updated for 1.2. Open a PR?", "finished · waiting for you"],
  ["Antigravity", "invoices page", "rendering /invoices"],
  ["Codex", "windows ConPTY", "cargo build", "Allow this command?", "rm -rf target/x86_64-pc-windows-msvc (y/n)", "needs your approval"],
  ["Claude Code", "relay keys", "reading noise.rs"],
  ["OpenCode", "changelog", "collecting PRs", "Question", "Group the 23 PRs by area or by date?", "has a question"],
  ["Codex", "linux .rpm", "packaging"],
  ["Antigravity", "onboarding copy", "drafting chapter 0", "Finished", "Chapter 0 drafted. Review the copy?", "finished · waiting for you"],
  ["Claude Code", "pane resize", "fixing grid jitter"],
  ["Codex", "tab latency", "measuring p50"],
].map(([agent, task, line, rt, rb, toast]) => ({ agent, task, line, toast,
  ring: rt && `<div class="box"><small>${rt}</small>${rb}</div>`,
  boot: [["dim", `${agent} · ~/hide`], ["", `Reading the ${task} task`], ["dim", "Thinking…"], ["", line]] }));
const N = KIDS.length, ASK = 1;
const OTHER_TASKS = [
  { a: "Codex", n: "billing webhooks", p: "acme-api", t: ["retrying 3 events", "verifying signatures", "running 41 tests", "writing the replay job"] },
  { a: "Antigravity", n: "landing films", p: "hide-site", t: ["rendering", "checking frame 214", "encoding webm", "comparing stills"] },
  { a: "OpenCode", n: "push tokens", p: "mobile-app", t: ["reading APNs docs", "editing PushService.kt", "running the emulator", "writing token refresh"] },
];
const PROJECTS = [
  ["acme-api", [["Codex", "billing webhooks", 0], ["Claude Code", "rate limits", "<i>done</i> · PR #418"], ["Antigravity", "error pages", "drafting 404 copy"]]],
  ["hide-site", [["Antigravity", "landing films", 1]]],
  ["mobile-app", [["OpenCode", "push tokens", 2], ["Codex", "iOS crash", "<i>done</i> · fixed in 2.4.1"]]],
];
const NOISE = [["", "Reading src/runtime/mod.rs"], ["dim", "Thinking…"], ["", "git diff --stat"], ["ok", "✓ 38 passed"], ["", "Applying patch"], ["dim", "Searching 1,204 files"], ["", "rg legacy_id"]];
const FORK = `<svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"><circle cx="4" cy="3" r="1.6"/><circle cx="4" cy="13" r="1.6"/><circle cx="12" cy="8" r="1.6"/><path d="M4 4.6v6.8M4 8h3.5a3 3 0 0 0 3-3"/></svg>`;

const $ = (id) => document.getElementById("ot-" + id);
const layer = $("layer");
const BASE = 0.68;
const reduceMotion = matchMedia("(prefers-reduced-motion: reduce)").matches;
let holdAfter = -1;
let speed = BASE, gen = 0, ffTo = -1, chapter = -1, chT0 = 0, chDur = [7600, 4600, 4800, 6800, 5600];
let panes, rows, prow, poprows, branches, tick, otick, quiet, timers = [];

class Stale extends Error {}
const sleep = (ms) => { const g = gen; return new Promise((r, j) => setTimeout(() => (g === gen ? r() : j(new Stale())), ms * speed)); };
const frame = () => { const g = gen; return new Promise((r, j) => requestAnimationFrame((t) => (g === gen ? r(t) : j(new Stale())))); };

function fit() { const s = stageEl.clientWidth / 1280; $("scaler").style.transform = `scale(${s})`; }
addEventListener("resize", fit); fit();

/* ---------- chapters ---------- */
const tabs = [...$("tabs").querySelectorAll("button")];
function setChapter(i) {
  if (holdAfter >= 0 && i > holdAfter) { hold(holdAfter); throw new Stale(); }
  const now = performance.now();
  if (chapter >= 0 && chapter < i && ffTo < 0) chDur[chapter] = now - chT0;
  chapter = i; chT0 = now;
  if (ffTo >= 0 && i >= ffTo) { ffTo = -1; speed = BASE; $("stage").classList.remove("ff"); }
  tabs.forEach((b, k) => { b.setAttribute("aria-current", k === i ? "true" : "false"); b.querySelector(".ch-bar").style.transform = `scaleX(${k < i ? 1 : 0})`; });
}
(function bar() {
  if (chapter >= 0 && ffTo < 0) tabs[chapter].querySelector(".ch-bar").style.transform = `scaleX(${Math.min(1, (performance.now() - chT0) / chDur[chapter])})`;
  requestAnimationFrame(bar);
})();
// A press plays from that chapter; under reduced motion it shows the chapter's last frame and holds it.
function jump(k) {
  gen++; ffTo = k; speed = 0.004; $("stage").classList.add("ff");
  if (reduceMotion) { holdAfter = k; ffTo = 99; }
  setTimeout(loop, 30);
}
tabs.forEach((b, k) => b.addEventListener("click", () => jump(k)));

/* ---------- geometry ---------- */
const rect = (el, r) => Object.assign(el.style, { left: r.x + "px", top: r.y + "px", width: r.w + "px", height: r.h + "px" });
function rel(el) { const b = $("body").getBoundingClientRect(), r = el.getBoundingClientRect(), s = b.width / W;
  return { x: (r.left - b.left) / s, y: (r.top - b.top) / s, w: r.width / s, h: r.height / s }; }
const mid = (r) => ({ x: r.x + r.w / 2, y: r.y + r.h / 2 });

/* ---------- building ---------- */
function rowEl(cls, agent, n, l) { const r = document.createElement("div"); r.className = "row " + cls; r.innerHTML = `<i class="d"></i>${img(agent)}<span class="n">${n}</span><span class="l">${l}</span>`; return r; }
function makeTree(host, lead, kids) {
  host.innerHTML = ""; const svg = document.createElementNS(NS, "svg"); host.appendChild(svg);
  const list = [lead ? rowEl("lead", ...lead) : null, ...kids.map((k) => rowEl("kid", ...k))];
  list.forEach((r) => r && host.appendChild(r)); return { svg, list, host };
}
function build() {
  timers.forEach(clearTimeout); timers = [];
  layer.innerHTML = ""; $("toasts").innerHTML = ""; $("needsRows").innerHTML = "";
  const lead = document.createElement("div"); lead.className = "pane lead";
  lead.innerHTML = `<div class="h"><i class="d"></i>${img("Claude Code")}<b>Claude Code</b> · ~/hide · release-1.2</div>
    <div class="r2" id="ot-r2"><span class="kidbtn" id="ot-kidbtn">${FORK}<span id="ot-kidn">0</span></span><span id="ot-r2txt"></span></div><div class="t"></div>`;
  layer.appendChild(lead);
  panes = [lead, ...KIDS.map((k) => {
    const p = document.createElement("div"); p.className = "pane";
    p.innerHTML = `<div class="h"><i class="d"></i>${img(k.agent)}<b>${k.agent}</b> · ${k.task}</div><div class="t"></div>`;
    p.style.opacity = 0; layer.appendChild(p); return p; })];
  rows = makeTree($("tree"), ["Claude Code", "release 1.2", `waiting on ${N} children`], KIDS.map((k) => [k.agent, k.task, k.line]));
  prow = makeTree($("ptree"), ["Claude Code", "Claude Code · lead", "release 1.2"], KIDS.map((k) => [k.agent, k.agent, k.task]));
  poprows = makeTree($("poptree"), null, KIDS.map((k) => [k.agent, `${k.agent} · ${k.task}`, k.line]));
  branches = { tasks: [], projects: [], pop: [] };
  const to = $("tasksOther"); to.innerHTML = "";
  OTHER_TASKS.forEach((o, i) => { const r = rowEl("", o.a, o.n, `${o.p} · ${o.t[0]}`); r.dataset.tick = i; r.dataset.pre = o.p + " · "; to.appendChild(r); });
  const po = $("projectsOther"); po.innerHTML = "";
  PROJECTS.forEach(([name, list]) => {
    po.insertAdjacentHTML("beforeend", `<div class="prj">${name} <small>${list.length} agent${list.length > 1 ? "s" : ""}</small></div>`);
    list.forEach(([a, n, l]) => { const live = typeof l === "number"; const r = rowEl(live ? "" : "unread", a, `${a} · ${n}`, live ? OTHER_TASKS[l].t[0] : l); if (live) { r.dataset.tick = l; r.dataset.pre = ""; } po.appendChild(r); });
  });
  $("tabTasks").classList.add("on"); $("tabProjects").classList.remove("on"); $("listTasks").classList.remove("off"); $("listProjects").classList.add("off");
  rows.list.slice(1).forEach((r) => r.classList.add("hid")); prow.list.slice(1).forEach((r) => r.classList.add("hid"));
}
function dotOf(r) { const d = r.querySelector(".d"); return { x: r.offsetLeft + d.offsetLeft + 4, y: r.offsetTop + d.offsetTop + 4 }; }
function branchD(t, i) { const c = dotOf(t.list[i]); const p = t.list[0] ? dotOf(t.list[0]) : { x: c.x - 22, y: -6 }; const R = 6;
  return `M ${p.x} ${p.y + 7} V ${c.y - R} Q ${p.x} ${c.y} ${p.x + R} ${c.y} H ${c.x - 7}`; }
function drawBranch(t, key, i) {
  const path = document.createElementNS(NS, "path"); path.setAttribute("d", branchD(t, i)); t.svg.prepend(path);
  const len = path.getTotalLength(); path.style.strokeDasharray = len; path.style.strokeDashoffset = len;
  path.getBoundingClientRect(); path.style.transition = "stroke-dashoffset .45s var(--ease), stroke .3s"; path.style.strokeDashoffset = 0;
  branches[key][i] = path; t.list[i].classList.remove("hid");
}
async function runBranch(t, i, green, up) {
  const tr = document.createElementNS(NS, "path"); tr.setAttribute("class", "trail " + (green ? "cool" : "hot")); tr.setAttribute("d", branchD(t, i)); t.svg.appendChild(tr);
  const len = tr.getTotalLength(); tr.style.strokeDasharray = len; tr.style.strokeDashoffset = up ? -len : len;
  const d = document.createElement("div"); d.className = "tdot" + (green ? " g" : ""); t.host.appendChild(d);
  const T = 820 * speed, t0 = performance.now();
  for (;;) {
    const now = await frame(); const k = Math.min(1, (now - t0) / T), e = k < .5 ? 2 * k * k : 1 - Math.pow(-2 * k + 2, 2) / 2;
    const pt = tr.getPointAtLength(up ? len * (1 - e) : len * e); d.style.left = pt.x + "px"; d.style.top = pt.y + "px";
    tr.style.strokeDashoffset = up ? -len * (1 - e) : len * (1 - e);
    if (k >= 1) break;
  }
  d.remove(); setTimeout(() => { tr.style.transition = "opacity .5s"; tr.style.opacity = 0; setTimeout(() => tr.remove(), 500); }, 250 * speed);
}

/* ---------- terminal text ---------- */
function push(p, cls, text, html) {
  const t = p.querySelector(".t"); t.querySelectorAll(".caret").forEach((c) => c.classList.remove("caret"));
  const d = document.createElement("div"); if (cls) d.className = cls;
  if (html) d.innerHTML = text; else d.textContent = text || " ";
  t.appendChild(d); while (t.children.length > 40) t.firstChild.remove(); return d;
}
const say = (cls, text, html) => push(panes[0], cls, text, html);
async function type(el, text, ms = 55) { for (const ch of text) { el.textContent += ch; await sleep(ms); } }
function startTicks() {
  clearInterval(tick); clearInterval(otick);
  tick = setInterval(() => panes.forEach((p, i) => {
    if (!i || quiet || p.style.opacity === "0" || p.dataset.ringing) return;
    if (Math.random() < .35) { const [c, t] = NOISE[(Math.random() * NOISE.length) | 0]; push(p, c, t); }
  }), 200);
  otick = setInterval(() => stageEl.querySelectorAll("[data-tick]").forEach((r) => {
    if (Math.random() < .3) { const L = OTHER_TASKS[r.dataset.tick].t; r.querySelector(".l").textContent = r.dataset.pre + L[(Math.random() * L.length) | 0]; }
  }), 900);
}
function toast(a, m) { const t = document.createElement("div"); t.className = "toast"; t.innerHTML = `${img(a, "")}<b>${a}</b><span>${m}</span>`; $("toasts").prepend(t); }
function cursorTo(x, y, show = true) { const c = $("cursor"); c.style.left = x + "px"; c.style.top = y + "px"; c.style.opacity = show ? 1 : 0; }
async function clickAt(el) { const m = mid(rel(el)); cursorTo(m.x - 4, m.y - 4); await sleep(620);
  const r = document.createElement("div"); r.className = "ripple"; r.style.left = m.x + "px"; r.style.top = m.y + "px"; $("body").appendChild(r); setTimeout(() => r.remove(), 700); }
function r2(html, cls) { const r = $("r2"); r.classList.add("on"); r.classList.toggle("ask", cls === "ask"); $("r2txt").innerHTML = html; }
const waiting = (n) => `<span class="arrow">ship 1.2 → </span><span class="w">Waiting on children · ${n} working</span>`;
function kidState(i, state) { [rows, prow, poprows].forEach((t) => { const r = t.list[i + 1]; r.classList.toggle("ask", state === "ask"); r.classList.toggle("done", state === "done"); }); }
function setMode(hide, title) { $("ttl").classList.toggle("hide", hide); $("title").textContent = title; }

/* ---------- the film ---------- */
function grid(n) {
  const cols = n <= 2 ? 2 : n <= 4 ? 2 : n <= 9 ? 3 : 4, rn = Math.ceil(n / cols);
  const cw = (W - G * (cols + 1)) / cols, ch = (H - G * (rn + 1)) / rn;
  return Array.from({ length: n }, (_, i) => ({ x: G + (i % cols) * (cw + G), y: G + Math.floor(i / cols) * (ch + G), w: cw, h: ch }));
}
const all = (n) => Array.from({ length: n }, (_, i) => i + 1);
async function run() {
  quiet = false; build(); layer.classList.remove("tiny");
  $("side").classList.remove("on"); $("needs").classList.remove("on"); $("pop").classList.remove("on"); cursorTo(700, 420, false);
  const L = panes[0]; $("r2").className = "r2";
  L.style.transition = "none"; rect(L, { x: G, y: G, w: W - 2 * G, h: H - 2 * G }); L.offsetWidth; L.style.transition = "";
  setMode(false, "Terminal"); startTicks();

  // ---- 1. Tiled
  setChapter(0);
  say("dim", "Claude Code · ~/hide"); say("", "");
  const ask = say("me caret", "› "); await sleep(450);
  await type(ask, "ship 1.2. split it up and run everything in parallel", 36);
  await sleep(300); say("", ""); say("ac", `Splitting 1.2 into ${N} jobs, one worker each.`); await sleep(550);
  let shown = 1;
  for (const n of [2, 4, 6, 9, 12]) {
    const g = grid(n);
    for (let i = shown; i < n; i++) say("", `→ starting ${KIDS[i - 1].agent} · ${KIDS[i - 1].task}`);
    panes.slice(0, n).forEach((p, i) => {
      if (i >= shown) {
        const s = g[Math.max(0, i - 1)];
        p.style.transition = "none"; rect(p, { x: s.x + s.w, y: s.y, w: 0, h: s.h }); p.offsetWidth; p.style.transition = ""; p.style.opacity = 1;
        KIDS[i - 1].boot.forEach(([c, t], j) => timers.push(setTimeout(() => push(p, c, t), (200 + j * 150) * speed / BASE)));
      }
      rect(p, g[i]);
    });
    shown = n; layer.classList.toggle("tiny", n > 4);
    setMode(false, `Terminal — ${n} panes`);
    await sleep(560);
  }
  say("ok", `✓ ${N} workers started. Waiting on them.`);
  await sleep(700);
  const ringers = KIDS.map((k, i) => (k.ring ? i : -1)).filter((i) => i >= 0);
  for (const i of ringers) {
    const p = panes[i + 1]; p.dataset.ringing = 1; push(p, "", KIDS[i].ring, true);
    p.classList.remove("ring"); p.offsetWidth; p.classList.add("ring"); toast(KIDS[i].agent, KIDS[i].toast);
    while ($("toasts").children.length > 5) $("toasts").lastChild.remove();
    await sleep(380);
  }
  cursorTo(1040, 180); await sleep(380); cursorTo(260, 560); await sleep(380); cursorTo(700, 320); await sleep(380); cursorTo(1000, 560); await sleep(380);
  for (const i of ringers) { const p = panes[i + 1]; p.classList.remove("ring"); p.offsetWidth; p.classList.add("ring"); }
  await sleep(1400); cursorTo(700, 400, false);

  // ---- 2. Folded
  setChapter(1);
  $("toasts").querySelectorAll(".toast").forEach((t, i) => setTimeout(() => t.classList.add("gone"), i * 50));
  setMode(true, "hide · release-1.2");
  quiet = true;
  rect(L, { x: G, y: G, w: W - 2 * G, h: H - 2 * G }); L.classList.add("big");
  KIDS.forEach((_, i) => { const p = panes[i + 1]; p.classList.remove("ring"); delete p.dataset.ringing; });
  r2(waiting(0));
  await sleep(520);
  const kbr = rel($("kidbtn"));
  for (let i = 0; i < N; i++) {
    const p = panes[i + 1];
    p.style.transitionDuration = ".42s"; rect(p, kbr); p.style.opacity = 0;
    timers.push(setTimeout(() => {
      $("kidn").textContent = i + 1; $("kidbtn").classList.add("bump"); setTimeout(() => $("kidbtn").classList.remove("bump"), 140);
      $("r2txt").innerHTML = waiting(i + 1);
    }, 380 * speed / BASE));
    await sleep(95);
  }
  await sleep(500); layer.classList.remove("tiny");
  $("toasts").innerHTML = "";
  say("", ""); say("", `Waiting on ${N} children. I'll bring you what they need.`);
  await sleep(800);
  await clickAt($("kidbtn")); $("kidbtn").classList.add("press");
  const kb = rel($("kidbtn")); Object.assign($("pop").style, { left: kb.x + "px", top: kb.y + kb.h + 6 + "px" }); $("pop").classList.add("on");
  for (const i of all(N)) { drawBranch(poprows, "pop", i); await sleep(60); }
  await sleep(1700); $("pop").classList.remove("on"); $("kidbtn").classList.remove("press"); cursorTo(800, 400, false);
  await sleep(300);

  // ---- 3. Filed
  setChapter(2);
  $("side").classList.add("on"); rect(L, { x: SIDE + 8, y: 8, w: W - SIDE - 16, h: H - 16 });
  await sleep(750);
  for (const i of all(N)) { drawBranch(rows, "tasks", i); await sleep(70); }
  await sleep(1000);
  await clickAt($("tabProjects"));
  $("tabProjects").classList.add("on"); $("tabTasks").classList.remove("on"); $("listTasks").classList.add("off"); $("listProjects").classList.remove("off");
  await sleep(150); for (const i of all(N)) { drawBranch(prow, "projects", i); await sleep(60); }
  await sleep(1400);
  await clickAt($("tabTasks"));
  $("tabTasks").classList.add("on"); $("tabProjects").classList.remove("on"); $("listProjects").classList.add("off"); $("listTasks").classList.remove("off");
  cursorTo(800, 420, false); await sleep(500);

  // ---- 4. Asked
  setChapter(3);
  const A = ASK + 1;
  kidState(ASK, "ask"); rows.list[A].querySelector(".l").innerHTML = "<b>asks</b>"; branches.tasks[A].classList.add("hot");
  await sleep(600);
  await runBranch(rows, A, false, true);
  rows.list[0].classList.add("raised"); $("needs").classList.add("on");
  const nr = rows.list[0].cloneNode(true); nr.querySelector(".l").innerHTML = "<b>flaky e2e (child)</b> · quarantine the test?"; $("needsRows").innerHTML = ""; $("needsRows").appendChild(nr);
  const hp = document.createElement("div"); hp.className = "hop"; const a = mid(rel(rows.list[0].querySelector(".d")));
  Object.assign(hp.style, { left: a.x + "px", top: a.y + "px" }); $("body").appendChild(hp); hp.offsetWidth;
  const t2 = rel($("r2")); Object.assign(hp.style, { left: t2.x + 90 + "px", top: t2.y + t2.h / 2 + "px" }); await sleep(520); hp.remove();
  r2(`<span class="who">${img("Claude Code")}flaky e2e (child)</span><span>Fails 2 of 5 on Linux. Quarantine it?</span><span class="open">Open</span>`, "ask");
  quiet = false; say("", ""); say("bell", "🦉 letter from my child · flaky e2e"); await sleep(400);
  say("", `<div class="box"><small>flaky e2e asks you, through me · ${N - 1} others still working</small>pane-resize.spec fails 2 of 5 runs on Linux only. Quarantine it and keep going?</div>`, true);
  const inp = say("me caret", "› "); await sleep(950);
  await type(inp, "yes, quarantine it", 65); await sleep(300);
  say("ok", "✓ replied to flaky e2e");
  rows.list[0].classList.remove("raised"); $("needs").classList.remove("on"); await sleep(480);
  branches.tasks.forEach((p, i) => p && p.setAttribute("d", branchD(rows, i)));
  branches.tasks[A].classList.remove("hot");
  await runBranch(rows, A, true, false);
  kidState(ASK); rows.list[A].querySelector(".l").textContent = "quarantining";
  r2(waiting(N));
  await sleep(1400);

  // ---- 5. Done
  setChapter(4);
  await Promise.all(all(N).map((i, k) => sleep(k * 110).then(() => {
    kidState(i - 1, "done"); rows.list[i].querySelector(".l").innerHTML = `<i>done</i> · #${2090 + i}`;
    return runBranch(rows, i, true, true);
  })));
  rows.list[0].classList.add("unread"); rows.list[0].querySelector(".l").textContent = `${N} PRs ready`;
  say("", ""); say("bell", `🦉 ${N} letters from my children`);
  say("ok", "✓ db migration · 0042 applied on staging · #2091");
  say("ok", "✓ flaky e2e · spec quarantined · #2092");
  say("ok", "✓ windows ConPTY · builds on x64 · #2095");
  say("ok", "✓ tab latency · p50 41 → 18 ms · #2101");
  say("dim", `  … and ${N - 4} more`);
  r2(`<span class="arrow">ship 1.2 → </span><span class="g">Done</span><span>${N} PRs ready to review</span>`);
  await sleep(4200);
  layer.style.transition = "opacity .5s"; layer.style.opacity = 0; await sleep(500); layer.style.opacity = 1; layer.style.transition = "";
}
// The held frame: the stage stops where it is and the pressed chapter's bar stands full.
function hold(k) {
  gen++; clearInterval(tick); clearInterval(otick); ffTo = -1; speed = BASE;
  $("stage").classList.remove("ff"); chapter = -1;
  tabs.forEach((b, j) => { b.setAttribute("aria-current", j === k ? "true" : "false"); b.querySelector(".ch-bar").style.transform = `scaleX(${j <= k ? 1 : 0})`; });
}
async function loop() {
  const g = gen;
  try {
    for (;;) {
      await run();
      if (holdAfter >= 0) { hold(holdAfter); return; }
      if (g !== gen) return;
    }
  } catch (e) { if (!(e instanceof Stale)) throw e; }
}

// Plays only while the stage is on screen and the tab is visible, resuming at the chapter it left.
if (reduceMotion) { jump(2); return; }
let seen = false, playing = false;
const sync = () => {
  const want = seen && !document.hidden;
  if (want && !playing) { playing = true; const at = Math.max(0, chapter); if (at > 0) jump(at); else { gen++; loop(); } }
  if (!want && playing) { playing = false; gen++; clearInterval(tick); clearInterval(otick); }
};
if ("IntersectionObserver" in window) new IntersectionObserver((e) => { seen = e[0].isIntersecting; sync(); }, { threshold: 0.2 }).observe(stageEl);
else seen = true;
document.addEventListener("visibilitychange", sync);
sync();
})();
