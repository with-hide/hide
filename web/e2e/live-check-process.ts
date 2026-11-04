// The measurement driver owns its runtime; this fixture owns the driver.
// Its stdin is an owner channel kept open only by this Playwright worker.
import { spawn } from "node:child_process";
import { ownUntilWorkerExit } from "./worker-owned";

const ownerRunner = `
import os,runpy,signal,sys,threading
def lost_owner():
 try:
  while os.read(0,1): pass
 except OSError: pass
 os.kill(os.getpid(),signal.SIGTERM)
threading.Thread(target=lost_owner,daemon=True).start()
mode,subject,*arguments=sys.argv[1:]
sys.argv=[subject if mode=='-m' else '-c',*arguments]
if mode=='-m': runpy.run_module(subject,run_name='__main__')
elif mode=='-c': exec(compile(subject,'<live-check-fixture>','exec'))
else: raise SystemExit(2)
`;

function startPython(args: string[], options: { timeout: number; env?: NodeJS.ProcessEnv }) {
  const child = spawn("python3", ["-c", ownerRunner, ...args],
    { env: options.env, stdio: ["pipe", "pipe", "pipe"] });
  let stdout = "", stderr = "", error: Error | undefined;
  let killDeadline: NodeJS.Timeout | undefined;
  const owned = ownUntilWorkerExit(() => {
    child.stdin.destroy();
    if (child.exitCode === null && child.signalCode === null) {
      child.kill("SIGCONT");
      child.kill("SIGTERM");
    }
  });
  const stop = () => {
    owned.stop();
    if (!killDeadline && child.exitCode === null && child.signalCode === null) {
      killDeadline = setTimeout(() => child.kill("SIGKILL"), 10_000);
    }
  };
  const deadline = setTimeout(() => { error = new Error("python_fixture_timeout"); stop(); }, options.timeout);
  const append = (stream: "stdout" | "stderr", chunk: Buffer) => {
    if (Buffer.byteLength(stdout) + Buffer.byteLength(stderr) + chunk.length > 1024 * 1024) {
      error = new Error("python_fixture_output_over_budget");
      stop();
      return;
    }
    if (stream === "stdout") stdout += chunk.toString();
    else stderr += chunk.toString();
  };
  child.stdout.on("data", chunk => append("stdout", chunk));
  child.stderr.on("data", chunk => append("stderr", chunk));
  child.once("error", cause => { error = cause; });
  const completed = new Promise<{ status: number | null; stdout: string; stderr: string; error?: Error }>(resolve => {
    child.once("close", status => {
      clearTimeout(deadline);
      clearTimeout(killDeadline);
      owned.disown();
      child.stdin.destroy();
      resolve({ status, stdout, stderr, error });
    });
  });
  return { child, completed, stop: async () => { stop(); return completed; } };
}

export async function runPython(args: string[], options: { timeout: number; env?: NodeJS.ProcessEnv }) {
  const running = startPython(args, options);
  try { return await running.completed; }
  finally { await running.stop(); }
}
