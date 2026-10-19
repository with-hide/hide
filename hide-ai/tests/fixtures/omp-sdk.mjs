// Real vendor auth and inference libraries, a localhost catalog and provider.
// No coding-agent module is imported. The synthetic credential never leaves
// the loopback endpoint. A separate process owns this server in the Rust test.
import { mkdir, readFile, writeFile, symlink } from "node:fs/promises";
import { join, dirname } from "node:path";
import { pathToFileURL } from "node:url";

const [installed, fixture] = process.argv.slice(2);
const meta = JSON.parse(await readFile(join(installed, "package.json"), "utf8"));
const load = name => import(pathToFileURL(Bun.resolveSync(name, installed)).href);
const logger = await load("@oh-my-pi/pi-utils/logger");
logger.setTransports({ file: false, console: false });
const { AuthStorage } = await load("@oh-my-pi/pi-ai");
const { getAgentDbPath } = await load("@oh-my-pi/pi-utils");
const { getBundledModels } = await load("@oh-my-pi/pi-catalog/models");
await mkdir(dirname(getAgentDbPath()), { recursive: true });
const auth = await AuthStorage.create(getAgentDbPath());
await auth.credentials.set("openai", { type: "api_key", key: "synthetic-local-only", source: "login" });
await auth.credentials.set("anthropic", { type: "oauth", access: "sk-ant-oat01-synthetic-local-only", refresh: "synthetic-refresh-never-used", expires: Date.now() + 24 * 60 * 60 * 1000 });
await auth.close();

const record = join(fixture, "requests.jsonl");
const server = Bun.serve({ hostname: "127.0.0.1", port: 0, async fetch(request) {
  const body = await request.json();
  await writeFile(record, JSON.stringify(body) + "\n", { flag: "a" });
  console.log(JSON.stringify({ submitted: body.model }));
  if (body.model === "hang") return new Promise(() => {});
  if (body.model.startsWith("stream-http")) return new Response(`event: error\ndata: ${JSON.stringify({type:"error",error:{type:"invalid_request_error",message:`HTTP ${body.model.slice(11)} synthetic`}})}\n\n`, { headers: { "content-type": "text/event-stream" } });
  if (body.model === "terminal-error") return new Response('data: {"error":{"message":"synthetic terminal error","type":"api_error"}}\n\n', { headers: { "content-type": "text/event-stream" } });
  if (body.model === "oversize") return new Response("x".repeat(8 * 1024 * 1024 + 1), { headers: { "content-type": "text/event-stream" } });
  if (body.model.startsWith("http")) return Response.json({ error: { message: "synthetic refusal" } }, { status: Number(body.model.slice(4)) });
  const text = body.model === "bad-json" ? "not JSON" : body.model === "bad-schema" ? '{"value":42}' : '{"value":"fixture"}';
  if (body.model === "api-anthropic") {
    const events = [
      ["message_start", { message: { id: "fixture", type: "message", role: "assistant", model: body.model, content: [], stop_reason: null, usage: { input_tokens: 4, output_tokens: 0 } } }],
      ["content_block_start", { index: 0, content_block: { type: "text", text: "" } }],
      ["content_block_delta", { index: 0, delta: { type: "text_delta", text } }],
      ["content_block_stop", { index: 0 }],
      ["message_delta", { delta: { stop_reason: "end_turn" }, usage: { output_tokens: 3 } }],
      ["message_stop", {}],
    ];
    return new Response(events.map(([type, data]) => `event: ${type}\ndata: ${JSON.stringify({ type, ...data })}\n\n`).join(""), { headers: { "content-type": "text/event-stream" } });
  }
  if (["api-responses", "api-codex"].includes(body.model)) {
    const item = { type: "message", id: "msg", role: "assistant", status: "completed", content: [{ type: "output_text", text, annotations: [] }] };
    const response = { id: "resp", object: "response", created_at: 0, model: body.model, status: "completed", output: [item], usage: { input_tokens: 4, output_tokens: 3, total_tokens: 7, input_tokens_details: { cached_tokens: 0 }, output_tokens_details: { reasoning_tokens: 0 } } };
    const events = [
      { type: "response.created", response: { ...response, status: "in_progress", output: [] } },
      { type: "response.output_item.added", output_index: 0, item: { ...item, status: "in_progress", content: [] } },
      { type: "response.content_part.added", item_id: "msg", output_index: 0, content_index: 0, part: { type: "output_text", text: "", annotations: [] } },
      { type: "response.output_text.delta", item_id: "msg", output_index: 0, content_index: 0, delta: text },
      { type: "response.output_text.done", item_id: "msg", output_index: 0, content_index: 0, text },
      { type: "response.output_item.done", output_index: 0, item },
      { type: "response.completed", response },
    ];
    return new Response(events.map(event => `event: ${event.type}\ndata: ${JSON.stringify(event)}\n\n`).join(""), { headers: { "content-type": "text/event-stream" } });
  }
  const frame = { id: "fixture", object: "chat.completion.chunk", choices: [{ index: 0, delta: { content: text }, finish_reason: null }] };
  const end = { ...frame, choices: [{ index: 0, delta: {}, finish_reason: "stop" }], usage: { prompt_tokens: 4, completion_tokens: 3, total_tokens: 7 } };
  return new Response(`data: ${JSON.stringify(frame)}\n\ndata: ${JSON.stringify(end)}\n\ndata: [DONE]\n\n`, { headers: { "content-type": "text/event-stream" } });
} });

const root = join(fixture, "sdk");
await mkdir(join(root, "node_modules/@oh-my-pi"), { recursive: true });
await mkdir(join(root, "bin"));
await writeFile(join(root, "bin/omp"), "CLI must never run");
await writeFile(join(root, "package.json"), JSON.stringify({ name: meta.name, version: meta.version, engines: meta.engines }));
for (const name of ["pi-ai", "pi-utils"]) {
  // A junction on Windows needs no developer-mode symlink permission.
  let actual = dirname(Bun.resolveSync(`@oh-my-pi/${name}`, installed));
  for (;;) {
    try { if (JSON.parse(await readFile(join(actual, "package.json"), "utf8")).name === `@oh-my-pi/${name}`) break; } catch {}
    if (dirname(actual) === actual) throw new Error("fixture dependency missing");
    actual = dirname(actual);
  }
  await symlink(actual, join(root, `node_modules/@oh-my-pi/${name}`), process.platform === "win32" ? "junction" : "dir");
}
const catalog = join(root, "node_modules/@oh-my-pi/pi-catalog");
await mkdir(catalog);
await writeFile(join(catalog, "package.json"), JSON.stringify({ name: "@oh-my-pi/pi-catalog", version: meta.version, type: "module", exports: { "./models": "./models.mjs" } }));
const base = { ...getBundledModels("openai").find(m => !m.kind || m.kind === "chat"), provider: "openai", api: "openai-completions", reasoning: false, baseUrl: `${server.url}v1` };
const ids = ["ok", "bad-json", "bad-schema", "http400", "http413", "http401", "http429", "http500", "hang", "terminal-error", "oversize"];
const models = ids.map(id => ({ ...base, id, name: id }));
models.push({ ...base, id: "image-must-not-be-listed", kind: "image" });
for (const [id, api] of [["api-anthropic", "anthropic-messages"], ["api-responses", "openai-responses"], ["api-codex", "openai-codex-responses"], ["stream-http400", "anthropic-messages"], ["stream-http413", "anthropic-messages"]]) models.push({ ...base, id, api, provider: api === "anthropic-messages" ? "anthropic" : "openai" });
await writeFile(join(catalog, "models.mjs"), `import { appendFileSync } from "node:fs";
appendFileSync(${JSON.stringify(join(fixture, "children.jsonl"))}, JSON.stringify({pid:process.pid,cwd:process.cwd(),argv:process.argv})+"\\n");
const models = ${JSON.stringify(models)};
export const getBundledProviders = () => ["openai", "anthropic"]; export const getBundledModels = provider => models.filter(model => model.provider === provider);`);
console.log(JSON.stringify({ root, record, port: server.port, db: getAgentDbPath() }));
// stdin EOF is the fixture owner's deterministic shutdown signal.
await Bun.stdin.text();
server.stop(true);
