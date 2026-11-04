// The local measurement CLI uses the existing compiled providers on the real
// pinned Herdr. This is tool plumbing, never authenticated model acceptance.
import { test, expect } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";
import { herdrBinary } from "./herdr-fixture";
import { copyFixtureShim } from "./shims/build";
import { runPython, startPython } from "./live-check-process";

const originalConfig = '{"fixture":"before-run"}\n';
const originalCodexConfig = 'model = "fixture-original"\n';
// Fault injection stays in the test runner. Production has no hidden route to
// mutate operator configuration or fake successful native delivery.
const toolRunner = `
import os,sys
from pathlib import Path
from unittest.mock import patch
sys.path.insert(0,str(Path('../scripts').resolve()))
from agent_live_check.cli import main
from agent_live_check import cli
from agent_live_check.runtime import Runtime
from agent_live_check.processes import ProcessError,ProcessSafetyError
from agent_live_check.integration import prepare,observe
from agent_live_check.protection import private_directory,write_private
original=Runtime.__init__
def initialize(self,*args,**kwargs):
 original(self,*args,**kwargs)
 file=self.home/'.claude.json'
 file.write_bytes(${JSON.stringify(originalConfig)}.encode())
 file.chmod(0o640)
 codex=self.home/'.codex/config.toml'
 codex.parent.mkdir(mode=0o700,parents=True,exist_ok=True)
 codex.write_bytes(${JSON.stringify(originalCodexConfig)}.encode())
 codex.chmod(0o640)
 peer=self.home/'.claude/settings.local.json'
 peer.parent.mkdir(mode=0o700,parents=True,exist_ok=True)
 peer.write_bytes(${JSON.stringify(originalConfig)}.encode())
def fail(self,*args,**kwargs):
 original_workspace(self,*args,**kwargs)
 if os.environ.get('LIVE_CHECK_FIXTURE_CASE')=='server-loss':
  child=next(child for child,output,label in self.servers if label=='herdr')
  self.owner.end(child)
 if os.environ.get('LIVE_CHECK_FIXTURE_CASE')=='configuration':
  (self.home/'.claude/settings.local.json').write_bytes(b'owned temporary settings')
  os.link(self.home/'.claude.json',self.home/'.claude/unexpected-link')
 raise RuntimeError('injected_after_registered_private_workspace')
original_workspace=Runtime.new_workspace
original_command=Runtime.command
original_prepare=cli.prepare_integration
original_close=Runtime.close_workspace
integration_cases=('integrity','integrity-fatal','integrity-native-error')
def integration_preparation(self,recipe,overlay):
 if os.environ.get('LIVE_CHECK_FIXTURE_CASE') not in integration_cases: return original_prepare(self,recipe,overlay)
 # Real pinned installer bytes, but a deliberately synthetic native emitter.
 # This tests CLI evidence retention and makes no authenticated-load claim.
 overlay['env']['CODEX_HOME']=str(self.probe/'fixture-codex-config')
 private_directory(Path(overlay['env']['CODEX_HOME']))
 fixture=self.fixture_bin
 try:
  self.fixture_bin=None
  plan=prepare(self,recipe,overlay)
 finally: self.fixture_bin=fixture
 plan['synthetic']=True
 self.integration_fixture=(recipe,plan)
 self.integration_observations=[]
 self.integration_closes=0
 return plan
def integration_close(self,workspace):
 scenario=os.environ.get('LIVE_CHECK_FIXTURE_CASE')
 if scenario not in integration_cases: return original_close(self,workspace)
 recipe,plan=self.integration_fixture
 self.integration_closes+=1
 if self.integration_closes==1 or (scenario=='integrity' and self.integration_closes<=3):
  self.integration_observations.append(observe(self,self.integration_pane,recipe,plan))
 original_close(self,workspace)
 if scenario in ('integrity-fatal','integrity-native-error'):
  if self.integration_closes==1:
   import json
   write_private(self.run/'integration-observations.json',json.dumps(self.integration_observations).encode())
   first,later=plan['artifacts'][:2]
   first['file'].write_bytes(b'changed private fixture integration')
   if scenario=='integrity-fatal':
    later['file'].unlink()
    later['file'].symlink_to(self.home/'.codex/config.toml')
  return
 if self.integration_closes==1:
  self.changed_artifact=next(item for item in plan['artifacts']
   if item['version'] is None and item['file'].is_relative_to(self.probe/'fixture-codex-config'))
  self.changed_artifact['file'].write_bytes(b'changed private fixture configuration')
 elif self.integration_closes==2:
  self.changed_artifact['file'].write_bytes(self.changed_artifact['content'])
 elif self.integration_closes==3:
  import json
  write_private(self.run/'integration-observations.json',json.dumps(self.integration_observations).encode())
def integration_observation(self,pane,recipe,plan):
 if os.environ.get('LIVE_CHECK_FIXTURE_CASE')=='integrity-native-error' and self.integration_closes>=1:
  with patch.object(self,'agent',side_effect=ProcessError('injected_native_query_after_integrity_loss')):
   return observe(self,pane,recipe,plan)
 return observe(self,pane,recipe,plan)
def guardian_after_workspace(self,args,**kwargs):
 if os.environ.get('LIVE_CHECK_FIXTURE_CASE')=='guardian' and args[:2]==['agent','start']:
  try:
   self.owner.run([sys.executable,'-c','raise SystemExit(125)'],env=self.env,check=False)
  except ProcessSafetyError:
   child=next(child for child,output,label in self.servers if label=='herdr')
   self.owner.end(child)
   raise
 if os.environ.get('LIVE_CHECK_FIXTURE_CASE')=='integrity-native-error' and args[:2]==['agent','start'] and args[2].endswith('-rest'):
  # The case is the integration read in this scene's finalization, after the
  # integrity loss; ending the scene here skips its observation windows.
  raise ProcessError('fixture_rest_scene_not_observed')
 result=original_command(self,args,**kwargs)
 if os.environ.get('LIVE_CHECK_FIXTURE_CASE') in integration_cases and args[:2]==['agent','start']:
  self.integration_pane=args[args.index('--pane')+1]
  if args[2].endswith('-startup'):
   assert self.agent(self.integration_pane)['agent']=='codex'
   original_command(self,['pane','report-agent-session',self.integration_pane,'--source','herdr:codex',
    '--agent','codex','--agent-session-id','live-check-synthetic-emitter','--seq','1'])
 return result
with patch.object(Runtime,'__init__',initialize),patch.object(cli,'prepare_integration',integration_preparation),patch.object(cli,'observe_integration',integration_observation),patch.object(Runtime,'close_workspace',integration_close):
 if os.environ.get('LIVE_CHECK_FIXTURE_CASE')=='guardian':
  with patch.object(Runtime,'command',guardian_after_workspace): raise SystemExit(main(sys.argv[1:]))
 if os.environ.get('LIVE_CHECK_FIXTURE_CASE') in ('failure','configuration','server-loss'):
  with patch.object(Runtime,'new_workspace',fail): raise SystemExit(main(sys.argv[1:]))
 with patch.object(Runtime,'command',guardian_after_workspace): raise SystemExit(main(sys.argv[1:]))
`;

function fixtureRoot(agents = "claude-code,codex") {
  const runs = path.resolve("..", "agents", "runs");
  fs.mkdirSync(runs, { recursive: true });
  const root = fs.mkdtempSync(path.join(runs, "live-check-e2e-"));
  fs.chmodSync(root, 0o700);
  const bin = path.join(root, "bin");
  fs.mkdirSync(bin, { mode: 0o700 });
  for (const name of ["claude", "codex"]) {
    const program = path.join(bin, name);
    copyFixtureShim("claude-shim", program);
    fs.chmodSync(program, 0o700);
  }
  const run = path.join(root, "measurement");
  return { root, run, args: ["-c", toolRunner, "--agents", agents,
    "--fixture-bin", bin, "--herdr-bin", herdrBinary(), "--run-dir", run, "--scene-seconds", "1"] };
}

function assertEnded(pid: number) {
  expect(() => process.kill(pid, 0), `owned process ${pid} survived cleanup`).toThrow();
}

function assertClean(run: string, configurationFailure = false, serverLoss = false) {
  const report = JSON.parse(fs.readFileSync(path.join(run, "report.json"), "utf8"));
  if (configurationFailure) {
    expect(report.configuration.failures).toContainEqual({ path: path.join(run, "daemon-home", ".claude.json"),
      reason: "config_not_private_regular_file" });
    // Letter 2686: a known-file byte failure does not invalidate metadata.
    expect(report.configuration.inventory_checked).toBe(true);
    expect(report.configuration.directory_changes.some((row: { kind: string; path: string }) =>
      row.kind === "added" && row.path.endsWith("unexpected-link"))).toBe(true);
    expect(report.configuration.restored).toContainEqual({ path: path.join(run, "daemon-home", ".claude", "settings.local.json"),
      result: "restored" });
  } else expect(report.configuration.failures).toEqual([]);
  expect(report.cleanup).toMatchObject({ confirmed: !serverLoss, processes_confirmed: true,
    probe_removed: true, socket_removed: true, credential_copies_removed: true });
  if (serverLoss) expect(report.cleanup.failures.length).toBeGreaterThan(0);
  expect(fs.existsSync(path.join(run, "probe"))).toBe(false);
  expect(fs.readFileSync(path.join(run, "daemon-home", ".claude.json"), "utf8")).toBe(originalConfig);
  expect(fs.statSync(path.join(run, "daemon-home", ".claude.json")).mode & 0o777).toBe(0o640);
  expect(fs.readFileSync(path.join(run, "daemon-home", ".codex", "config.toml"), "utf8")).toBe(originalCodexConfig);
  expect(fs.statSync(path.join(run, "daemon-home", ".codex", "config.toml")).mode & 0o777).toBe(0o640);
  expect(fs.readFileSync(path.join(run, "daemon-home", ".claude", "settings.local.json"), "utf8")).toBe(originalConfig);
  return report;
}

test("live check guards preserve bytes, refuse aliases, and end detached children", async () => {
  test.skip(process.platform === "win32", "The local measurement runs on Unix hosts");
  const result = await runPython(["-m", "unittest", "discover", "-s", "../scripts/tests", "-p", "test_agent_live_*.py"],
    { timeout: 30_000 });
  expect(result.error, result.stderr).toBeUndefined();
  expect(result.status, result.stdout + result.stderr).toBe(0);
});

// Each provider keeps all ten scenes and its own bounded native lifecycle.
// Mixed-provider report/exit aggregation is covered at the report boundary;
// authenticated acceptance still requires one invocation with all adapters.
// Herdr only: the live check measures the agents on the pinned Herdr it is handed (`--herdr-bin`).
for (const provider of ["claude-code", "codex"]) test(`live check retains every ${provider} scene and rejects unsafe picker or plan input @herdr`, async () => {
  test.skip(process.platform === "win32", "Native measurement's process guardian supports Unix hosts");
  test.setTimeout(240_000);
  const { root, run, args } = fixtureRoot(provider);
  let clean = false;
  const env = provider === "codex" ? { ...process.env, LIVE_CHECK_FIXTURE_CASE: "integrity" } : process.env;
  const result = await runPython(args, { env, timeout: 220_000 });
  try {
    expect(result.error, result.stderr).toBeUndefined();
    expect(result.status, result.stdout + result.stderr).toBe(1);
    const report = assertClean(run);
    expect(report.fixture).toBe(true);
    expect(report.herdr.manifests.length).toBeGreaterThan(0);
    expect(report.configuration.failures).toEqual([]);
    const configuration = provider === "codex" ? path.join(".codex", "config.toml") : ".claude.json";
    expect(report.configuration.restored).toContainEqual({ path: path.join(run, "daemon-home", configuration), result: "restored" });
    expect(report.cleanup).toMatchObject({ confirmed: true, probe_removed: true, socket_removed: true });
    expect(fs.existsSync(path.join(run, "probe"))).toBe(false);
    expect(report.agents).toHaveLength(1);
    expect(report.agents[0].id).toBe(provider);
    if (provider === "codex") {
      const observations = JSON.parse(fs.readFileSync(path.join(run, "integration-observations.json"), "utf8"));
      expect(observations).toHaveLength(3);
      expect(observations[0].status).toBe("loaded_version_observed");
      expect(observations[0].loaded_version.length).toBe(1);
      for (const observation of observations.slice(1)) {
        expect(observation).toMatchObject({ status: "integrity_unproven", integrity: "unproven",
          native_source: null, loaded_version: null });
      }
      expect(report.agents[0].integration).toMatchObject({ status: "integrity_unproven", loaded_version: null });
      expect(report.failures).toEqual([]);
      expect(fs.readFileSync(path.join(run, "report.md"), "utf8")).toContain('"status": "integrity_unproven"');
    }
    for (const agent of report.agents) {
      expect(agent.scenes.map((row: { scene: string }) => row.scene).sort()).toEqual([
        "rest", "working", "shell_approval", "file_approval", "question", "plan_approval",
        "model_picker", "resume_picker", "mcp_approval", "startup"].sort());
      expect(agent.scenes.every((row: { arrival: string }) => row.arrival === "reached")).toBe(true);
      for (const row of agent.scenes) {
        const evidence = JSON.parse(fs.readFileSync(path.join(run, row.evidence), "utf8"));
        expect(evidence.samples.length, `${provider}/${row.scene} has no screen evidence`).toBeGreaterThan(0);
      }
      expect(agent.verdict).toBe("unsafe");
      expect(agent.scenes.some((row: { scene: string; effect: string }) =>
        ["model_picker", "resume_picker", "plan_approval"].includes(row.scene)
        && ["selection", "approval", "resumed_session"].includes(row.effect))).toBe(true);
    }
    expect(fs.readFileSync(path.join(run, "report.md"), "utf8")).toContain("unsafe");
    clean = true;
  } catch (error) {
    // Keep the private tool report and its screen reads beside other CI logs.
    const evidence = process.env.HIDE_E2E_SCREENSHOT_DIR;
    if (evidence && fs.existsSync(run)) fs.cpSync(run, path.join(evidence, path.basename(root)), { recursive: true });
    throw error;
  } finally {
    if (clean) fs.rmSync(root, { recursive: true, force: true });
  }
});

// Herdr only: the live check measures the agents on the pinned Herdr it is handed (`--herdr-bin`).
test("live check refuses operator socket and state before a server starts @herdr", async () => {
  test.skip(process.platform === "win32", "Unix local tool");
  const { root, run, args } = fixtureRoot();
  let clean = false;
  try {
    for (const [flag, value] of [["--socket", path.join(process.env.HOME!, ".config", "herdr", "herdr.sock")],
      ["--state-dir", path.join(process.env.HOME!, ".hide", "state")]]) {
      const destination = run + flag;
      const modified = [...args];
      modified[modified.indexOf("--run-dir") + 1] = destination;
      const result = await runPython([...modified, flag, value], { timeout: 10_000 });
      expect(result.error, result.stderr).toBeUndefined();
      expect(result.status, result.stdout + result.stderr).toBe(2);
      const report = JSON.parse(fs.readFileSync(path.join(destination, "report.json"), "utf8"));
      expect(report.herdr).toEqual({});
      expect(report.agents).toEqual([]);
      expect(report.failures).toHaveLength(1);
      expect(fs.existsSync(path.join(destination, "state", "hided.json"))).toBe(false);
      expect(fs.existsSync(path.join(destination, "probe"))).toBe(false);
    }
    clean = true;
  } finally { if (clean) fs.rmSync(root, { recursive: true, force: true }); }
});

// Herdr only: the live check measures the agents on the pinned Herdr it is handed (`--herdr-bin`).
test("live check retains integrity loss before a later fatal artifact refusal @herdr", async () => {
  test.skip(process.platform === "win32", "Unix private integration and aliases");
  test.setTimeout(120_000);
  for (const scenario of ["integrity-fatal", "integrity-native-error"]) {
    const { root, run, args } = fixtureRoot("codex");
    let clean = false;
    try {
      const result = await runPython(args, { env: { ...process.env, LIVE_CHECK_FIXTURE_CASE: scenario }, timeout: 45_000 });
      expect(result.error, result.stderr).toBeUndefined();
      expect(result.status, result.stdout + result.stderr).toBe(2);
      const report = assertClean(run);
      const observations = JSON.parse(fs.readFileSync(path.join(run, "integration-observations.json"), "utf8"));
      expect(observations[0].status).toBe("loaded_version_observed");
      if (scenario === "integrity-fatal") {
        expect(report.failures[0]).toMatchObject({ type: "ProtectionError", reason: "config_not_private_regular_file" });
        expect(report.failures).toEqual(expect.arrayContaining([expect.objectContaining({
          type: "ProtectionError", reason: "prepared_integration_path_refused", phase: "scene_integration",
        })]));
      } else {
        expect(report.failures[0]).toMatchObject({ type: "ProcessError", reason: "injected_native_query_after_integrity_loss" });
      }
      expect(report.agents[0].integration).toMatchObject({ status: "integrity_unproven", integrity: "unproven", loaded_version: null });
      expect(report.agents[0].integration.integrity_changes).toHaveLength(1);
      expect(report.agents[0].verdict).toBe("unsafe");
      expect(fs.readFileSync(path.join(run, "report.md"), "utf8")).toContain('"status": "integrity_unproven"');
      clean = true;
    } catch (error) {
      // Keep the private tool report and its screen reads beside other CI logs.
      const evidence = process.env.HIDE_E2E_SCREENSHOT_DIR;
      if (evidence && fs.existsSync(run)) fs.cpSync(run, path.join(evidence, `${scenario}-${path.basename(root)}`), { recursive: true });
      throw error;
    } finally { if (clean) fs.rmSync(root, { recursive: true, force: true }); }
  }
});

// Herdr only: the live check measures the agents on the pinned Herdr it is handed (`--herdr-bin`).
test("live check tears down a started runtime on failure and Ctrl-C @herdr", async () => {
  test.skip(process.platform === "win32", "Unix signal and private socket contract");
  test.setTimeout(120_000);
  for (const scenario of ["failure", "configuration", "server-loss", "guardian", "hold"]) {
    const { root, run, args } = fixtureRoot();
    let running: ReturnType<typeof startPython> | undefined;
    let clean = false;
    try {
      const env = { ...process.env, LIVE_CHECK_FIXTURE_CASE: scenario };
      if (scenario !== "hold") {
        const result = await runPython(args, { env, timeout: 45_000 });
        expect(result.error, result.stderr).toBeUndefined();
        expect(result.status, result.stdout + result.stderr).toBe(2);
        const report = assertClean(run, scenario === "configuration", ["server-loss", "guardian"].includes(scenario));
        expect(report.herdr.version).toContain("herdr");
        if (scenario === "guardian") {
          expect(report.failures[0]).toMatchObject({ type: "ProcessSafetyError",
            reason: "guardian_cleanup_or_resource_failure" });
          expect(report.agents).toHaveLength(1);
          expect(report.agents[0].scenes).toEqual([]);
          expect(report.failures.slice(1)).toEqual(expect.arrayContaining([
            expect.objectContaining({ phase: "scene_integration", agent: "claude-code", scene: "startup" }),
            expect.objectContaining({ phase: "scene_workspace_close", agent: "claude-code", scene: "startup" }),
          ]));
        } else expect(report.failures[0].reason).toBe("injected_after_registered_private_workspace");
        clean = true;
      } else {
        args[args.indexOf("--scene-seconds") + 1] = "120";
        running = startPython(args, { env, timeout: 110_000 });
        const identity = path.join(run, "daemon-home", ".live-check-child.json");
        await expect.poll(() => fs.existsSync(identity), { timeout: 40_000 }).toBe(true);
        const fixture = JSON.parse(fs.readFileSync(identity, "utf8"));
        const state = path.join(run, "state", "hided.json");
        expect(fs.existsSync(state)).toBe(true);
        const daemon = JSON.parse(fs.readFileSync(state, "utf8"));
        running.child.kill("SIGINT");
        const result = await running.completed;
        expect(result.error, result.stderr).toBeUndefined();
        expect(result.status, result.stdout + result.stderr).toBe(2);
        assertClean(run);
        assertEnded(fixture.pid);
        assertEnded(daemon.pid);
        expect(fixture.socket.length).toBeGreaterThan(0);
        expect(fs.existsSync(fixture.socket)).toBe(false);
        clean = true;
      }
    } finally {
      if (running) await running.stop();
      if (clean) fs.rmSync(root, { recursive: true, force: true });
    }
  }
});
