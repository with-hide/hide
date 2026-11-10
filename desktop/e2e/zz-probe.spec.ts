import http from "node:http";
import type { AddressInfo } from "node:net";
import { launchShell, startStack, test } from "./fixture";

// Temporary diagnosis of the first page load of a new session on Windows.
test("zz-probe: the first load of a new page session", { tag: "@platform" }, async () => {
  const server = http.createServer((_request, response) => {
    response.writeHead(200, { "content-type": "text/html; charset=utf-8" });
    response.end("<!doctype html><title>probe</title><h1>probe</h1>");
  });
  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  const origin = `http://127.0.0.1:${(server.address() as AddressInfo).port}/`;
  const stack = await startStack("zz-probe", { agents: false });
  try {
    const { app } = await launchShell(stack.run.env);
    const timings = await app.evaluate(async ({ session, BrowserWindow }, target) => {
      const out: Record<string, number | string> = {};
      for (const name of ["persist:probe-a", "persist:probe-b"]) {
        const probe = session.fromPartition(name);
        const proxyStart = Date.now();
        out[`${name}.proxy`] = await probe.resolveProxy(target).then((answer) => answer, (error: unknown) => String(error));
        out[`${name}.proxyMs`] = Date.now() - proxyStart;
        const window = new BrowserWindow({ show: false, webPreferences: { session: probe } });
        const loadStart = Date.now();
        await window.loadURL(target);
        out[`${name}.loadMs`] = Date.now() - loadStart;
        window.destroy();
      }
      return out;
    }, origin);
    console.log(`[zz-probe] ${JSON.stringify(timings)}`);
    await app.close();
  } finally {
    stack.stop();
    await new Promise((resolve) => server.close(resolve));
  }
});
