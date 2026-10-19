// One request in an owned Bun child. Only public OMP library exports are used;
// importing the coding-agent entry point would initialize its ambient harness.
import { readFile, realpath, stat } from "node:fs/promises";
import { dirname, join } from "node:path";
import { pathToFileURL } from "node:url";

const versions = ["18.7.0", "18.12.2"];
const apis = new Set(["anthropic-messages", "openai-completions", "openai-responses", "openai-codex-responses"]);
class BridgeFailure extends Error {
  constructor(code, reason) { super(reason); this.code = code; }
}
const fail = (code, reason) => { throw new BridgeFailure(code, reason); };
const send = value => process.stdout.write(JSON.stringify({ protocol: 1, ...value }) + "\n");
let storage;
let submitted = false;
let refusal;
let deadlineSignal;

async function manifest(root) {
  const file = join(root, "package.json");
  if ((await stat(file)).size > 65536) fail("unavailable", "omp_package_manifest_too_large");
  const text = await readFile(file, "utf8");
  return JSON.parse(text);
}

// Resolve through the installation's public export map, then verify which
// package supplied it before executing any of its code.
async function exported(name, root, version) {
  const entry = await realpath(Bun.resolveSync(name, root));
  let parent = dirname(entry);
  for (let depth = 0; depth < 10; depth++, parent = dirname(parent)) {
    let meta;
    try { meta = await manifest(parent); } catch { continue; }
    if (meta.name !== name.split("/").slice(0, 2).join("/")) continue;
    if (meta.version !== version) fail("unavailable", "omp_sdk_dependency_version_mismatch");
    return import(pathToFileURL(entry).href);
  }
  fail("unavailable", "omp_sdk_export_identity_invalid");
}

try {
  const [root, requestPath] = process.argv.slice(2);
  const meta = await manifest(root);
  if (meta.name !== "@oh-my-pi/pi-coding-agent" || !versions.includes(meta.version))
    fail("unavailable", "omp_sdk_version_not_supported_install_a_documented_supported_version");
  if (typeof meta.engines?.bun !== "string" || !Bun.semver.satisfies(Bun.version, meta.engines.bun))
    fail("unavailable", "omp_bun_does_not_meet_package_engines");
  const request = JSON.parse(await readFile(requestPath, "utf8"));
  const logging = await exported("@oh-my-pi/pi-utils/logger", root, meta.version);
  logging.setTransports({ file: false, console: false });
  const ai = await exported("@oh-my-pi/pi-ai", root, meta.version);
  const auth = await exported("@oh-my-pi/pi-ai/auth-broker/discover", root, meta.version);
  const catalog = await exported("@oh-my-pi/pi-catalog/models", root, meta.version);
  if (typeof ai.completeSimple !== "function" || typeof auth.discoverAuthStorage !== "function" ||
      typeof catalog.getBundledModels !== "function" || typeof catalog.getBundledProviders !== "function")
    fail("unavailable", "omp_sdk_exports_incompatible");
  // Auth discovery belongs to OMP. No credential is parsed, copied or returned
  // by this bridge. Command-backed auth configuration is explicitly refused.
  storage = await auth.discoverAuthStorage({
    cachePath: request.auth_cache,
    configValueResolver: async value => {
      if (value.startsWith("!")) fail("unavailable", "omp_command_auth_configuration_not_supported");
      return process.env[value] || value;
    },
    authStorageOptions: {
      usageProviderResolver: () => undefined,
      rankingStrategyResolver: () => undefined,
    },
  });
  const models = catalog.getBundledProviders().flatMap(provider =>
    storage.keys.source(provider)?.concrete ? catalog.getBundledModels(provider).filter(model =>
      apis.has(model.api) && [undefined, "chat", "tiny"].includes(model.kind)) : []
  );
  if (request.operation === "catalog") {
    send({ kind: "catalog", models: models.map(model => `${model.provider}/${model.id}`) });
  } else if (request.operation === "execute") {
    const offered = models.find(model => `${model.provider}/${model.id}` === request.model);
    const model = offered && { ...offered, transport: "sse", preferWebsockets: false };
    if (!model) {
      if (!models.length) fail("auth", "omp_login_required");
      fail("rejected", "omp_model_not_offered_choose_a_listed_model");
    }
    deadlineSignal = AbortSignal.timeout(request.deadline_ms);
    const transport = new AbortController();
    const signal = AbortSignal.any([deadlineSignal, transport.signal]);
    let sends = 0;
    const result = await ai.completeSimple(model, {
      systemPrompt: request.system,
      messages: [{ role: "user", content: request.input, timestamp: 0 }],
      tools: [],
    }, {
      // Resolve only the initial credential. SDK auth retries cannot refresh
      // or rotate a failed inference into another charged attempt.
      apiKey: ({ error, signal }) => error ? Promise.resolve(undefined) :
        storage.keys.getWithCredential(model.provider, undefined, {
          modelId: model.id, baseUrl: model.baseUrl, signal,
        }),
      signal,
      disableReasoning: true,
      loopGuard: { enabled: false },
      acceptEmptyResponse: true,
      codexSseMaxAttempts: 1,
      preferWebsockets: false,
      cacheRetention: "none",
      maxInFlightRequests: {},
      maxTokens: 8192,
      providerRetryWait: async () => fail("unknown", "omp_inference_retry_refused"),
      fetch: async (url, options) => {
        if (++sends > 1) { transport.abort(); fail("unknown", "omp_inference_retry_refused"); }
        submitted = true;
        let response;
        try { response = await fetch(url, options); }
        catch (error) { transport.abort(); throw error; }
        if (!response.ok) {
          refusal = response.status;
          await response.body?.cancel();
          transport.abort();
          fail("unknown", "omp_request_refused");
        }
        if (!response.body) return response;
        const reader = response.body.getReader();
        let bytes = 0;
        return new Response(new ReadableStream({
          async pull(controller) {
            try {
              const next = await reader.read();
              if (next.done) { controller.close(); return; }
              bytes += next.value.byteLength;
              if (bytes > 8 * 1024 * 1024) {
                await reader.cancel();
                transport.abort();
                controller.error(new Error("omp_response_too_large"));
              } else controller.enqueue(next.value);
            } catch (error) { transport.abort(); controller.error(error); }
          },
          cancel(reason) { return reader.cancel(reason); },
        }), { status: response.status, headers: response.headers });
      },
    });
    if (["error", "aborted"].includes(result.stopReason))
      fail(submitted ? "unknown" : "auth", "omp_inference_failed");
    if (result.content.some(block => block.type === "toolCall")) fail("invalid", "omp_unexpected_tool_call");
    send({ kind: "answer", text: result.content.filter(block => block.type === "text").map(block => block.text).join("") });
  } else fail("unavailable", "omp_bridge_operation_invalid");
} catch (error) {
  const local = error instanceof BridgeFailure;
  const code = deadlineSignal?.aborted ? "timeout" : refusal === 401 || refusal === 403 ? "auth" : refusal === 429 ? "quota" :
    refusal >= 500 ? "transient" : refusal >= 400 ? "rejected" :
    local && error.code === "invalid" ? "invalid" : submitted ? "unknown" :
    local && ["auth", "rejected", "unavailable"].includes(error.code) ? error.code : "unavailable";
  send({ kind: "error", code, reason: local ? error.message : "omp_sdk_failed_check_package_dependencies_and_login_config" });
} finally {
  try { await storage?.close(); } catch { process.exitCode = 1; }
}
// Library transports may own timers. The one-shot child owns their lifetime.
process.exit(process.exitCode || 0);
