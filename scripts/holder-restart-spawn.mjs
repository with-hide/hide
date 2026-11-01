// Starts the holder restart lane's node the way the desktop app starts its
// daemon on Windows: through Node's `child_process.spawn`, which puts the
// child in libuv's job (kill on close, breakaway allowed, children silently
// outside it), as Electron's main process does (docs/TESTING.md, The holder
// restart lane). The node's holders break away from that job; killing this
// process closes the job and ends the node, the way the app's end does.
import { spawn } from "node:child_process";

const [program, ...args] = process.argv.slice(2);
const child = spawn(program, args, { stdio: "inherit", windowsHide: true });
child.on("error", (error) => {
  process.stderr.write(`holder-restart-spawn: ${error.message}\n`);
  process.exit(127);
});
child.on("exit", (code) => process.exit(code ?? 1));
