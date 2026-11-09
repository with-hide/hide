#!/usr/bin/env python3
"""Stand-in for the installed `claude` CLI used by hide-ai tests.

Answers `auth status --json` and prints one print-mode result frame. The
frames are the shapes measured from claude 2.1.267 and recorded in
`agents/runs/hide-ai-claude-backend/claude-print-mode-measurements.md`; the
real CLI is never called from a test.

FAKE_MODE selects the frame: ok (default), no_structured_output,
null_structured_output, api_401, api_403, api_429, api_500, api_400,
structured_output_retries, context_limit, no_result_frame, init_frame_only,
slow, no_account, auth_broken, auth_without_field, thinking_leak (the usage
reports thinking tokens), fenced_result (a plain answer in a code fence, no
structured_output), prose_result (a plain answer that holds no JSON) and
verbose_array (the ok answer inside the array of events a user config with
verbose on makes `--output-format json` print).
FAKE_ARGS_FILE records the argument vector, FAKE_STDIN_FILE the prompt body,
FAKE_ENV_FILE the account and gateway variables a model list read received
(the `ENV_SEEN` names, null for one unset),
FAKE_THINKING_FILE the MAX_THINKING_TOKENS value the child received ("unset"
when it had none). FAKE_STARTED_FILE is created once a slow turn has started
waiting.

A `/usage` run receives a whitelisted environment, so it reads no FAKE_*
variable. It takes its mode from a `usage-mode` file in its working directory
(text, cost, slow, exit, is_error, not_json, verbose_array) and writes what
it received - argv to `usage-args.json`, the environment's key set to `usage-env.json` -
into that same directory; a slow read creates `usage-started` there once it
has started waiting.
"""
import json
import os
import sys
import time

MODE = os.environ.get("FAKE_MODE", "ok")
ARGS = sys.argv[1:]
# The shape `--output-format json` has when the user's config has verbose on:
# every event of the run in one array, the result frame last.
VERBOSE_ARRAY = MODE == "verbose_array"

if path := os.environ.get("FAKE_ARGS_FILE"):
    with open(path, "w", encoding="utf-8") as handle:
        json.dump(ARGS, handle)

if path := os.environ.get("FAKE_THINKING_FILE"):
    with open(path, "w", encoding="utf-8") as handle:
        handle.write(os.environ.get("MAX_THINKING_TOKENS", "unset"))


def emit(obj, code=0):
    if VERBOSE_ARRAY and obj.get("type") == "result":
        obj = [{"type": "system", "subtype": "init", "session_id": "s-1"},
               {"type": "assistant", "message": {"content": []}},
               obj]
    sys.stdout.write(json.dumps(obj) + "\n")
    sys.stdout.flush()
    sys.exit(code)


USAGE_TEXT = (
    "You are currently using your subscription to power your Claude Code usage\n\n"
    "Current session: 4% used · resets Sep 17 at 9pm (Asia/Seoul)\n"
    "Current week (all models): 1% used · resets Sep 24 at 1pm (Asia/Seoul)\n"
    "Current week (Fable): 0% used · resets Sep 24 at 1pm (Asia/Seoul)\n"
)
COST_TEXT = "Total cost:            $0.0000\nTotal duration (API):  0s\n"

if ARGS[:2] == ["-p", "/usage"]:
    with open("usage-args.json", "w", encoding="utf-8") as handle:
        json.dump(ARGS, handle)
    with open("usage-env.json", "w", encoding="utf-8") as handle:
        json.dump(sorted(os.environ), handle)
    try:
        with open("usage-mode", encoding="utf-8") as handle:
            usage_mode = handle.read().strip()
    except FileNotFoundError:
        usage_mode = "text"
    local = {"type": "result", "subtype": "success", "is_error": False,
             "duration_api_ms": 0, "num_turns": 0, "total_cost_usd": 0,
             "local_command": "usage", "session_id": "s-usage"}
    VERBOSE_ARRAY = usage_mode == "verbose_array"
    if usage_mode == "slow":
        open("usage-started", "w").close()
        time.sleep(60)
    if usage_mode == "exit":
        sys.stdout.write("fatal: something\n")
        sys.exit(2)
    if usage_mode == "not_json":
        sys.stdout.write("Loading...\n")
        sys.exit(0)
    if usage_mode == "is_error":
        emit({**local, "is_error": True, "result": "usage failed"})
    emit({**local, "result": COST_TEXT if usage_mode == "cost" else USAGE_TEXT})

if ARGS[:2] == ["auth", "status"]:
    if MODE == "auth_broken":
        sys.stdout.write("not json at all\n")
        sys.exit(3)
    if MODE == "auth_without_field":
        emit({"authMethod": "none"})
    emit({"loggedIn": MODE != "no_account", "authMethod": "claude.ai"})

# The variables a model list read records: the account's folder, the
# gateway's, and the one a wrapper sets to say it ran.
ENV_SEEN = [
    "CLAUDE_CONFIG_DIR",
    "ANTHROPIC_BASE_URL",
    "ANTHROPIC_DEFAULT_OPUS_MODEL",
    "ANTHROPIC_DEFAULT_SONNET_MODEL",
    "ANTHROPIC_CUSTOM_MODEL_OPTION",
    "FAKE_WRAPPED",
]

# The model list: the stream-json control protocol with one `initialize`
# request on stdin and no model turn. The answer carries the account too, which
# the backend must never return; FAKE_MODE init_broken answers an error. Behind
# a gateway (`ANTHROPIC_BASE_URL`) an alias shows the model its
# `ANTHROPIC_DEFAULT_*_MODEL` names, and `ANTHROPIC_CUSTOM_MODEL_OPTION` adds
# one, as claude 2.1.319 answered on 2026-11-10.
if "--input-format" in ARGS:
    request = sys.stdin.read()
    if path := os.environ.get("FAKE_STDIN_FILE"):
        with open(path, "w", encoding="utf-8") as handle:
            handle.write(request)
    if path := os.environ.get("FAKE_ENV_FILE"):
        with open(path, "w", encoding="utf-8") as handle:
            json.dump({name: os.environ.get(name) for name in ENV_SEEN}, handle)
    if MODE == "init_broken":
        emit({"type": "control_response", "response": {
            "subtype": "error", "request_id": "hide-models", "error": "x"}})
    gateway = "ANTHROPIC_BASE_URL" in os.environ

    def alias(value, variable):
        model = {"value": value, "resolvedModel": f"claude-{value}-5-5"}
        if gateway and variable in os.environ:
            model["displayName"] = os.environ[variable]
        return model

    models = [
        {"value": "default", "resolvedModel": "claude-opus-5-5"},
        alias("opus", "ANTHROPIC_DEFAULT_OPUS_MODEL"),
        alias("sonnet", "ANTHROPIC_DEFAULT_SONNET_MODEL"),
    ]
    if custom := os.environ.get("ANTHROPIC_CUSTOM_MODEL_OPTION"):
        models.append({"value": custom, "displayName": custom})
    emit({"type": "control_response", "response": {
        "subtype": "success", "request_id": "hide-models",
        "response": {
            "models": models,
            "account": {"email": "someone@example.invalid", "organization": "Example"},
        }}})

# Print mode. The prompt body arrives on stdin, never in argv.
prompt = sys.stdin.read()
if path := os.environ.get("FAKE_STDIN_FILE"):
    with open(path, "w", encoding="utf-8") as handle:
        handle.write(prompt)

if MODE == "slow":
    if path := os.environ.get("FAKE_STARTED_FILE"):
        open(path, "w").close()
    time.sleep(60)

if MODE == "no_result_frame":
    sys.stdout.write("Killed before the turn produced a frame\n")
    sys.exit(1)

if MODE == "init_frame_only":
    emit({"type": "system", "subtype": "init", "session_id": "s-1"}, code=1)

USAGE = {"input_tokens": 1188, "output_tokens": 468,
         "cache_creation_input_tokens": 600, "cache_read_input_tokens": 4000,
         "cache_creation": {"ephemeral_1h_input_tokens": 100, "ephemeral_5m_input_tokens": 500}}
MODEL_USAGE = {"claude-haiku-4-5-20251001": {
    "inputTokens": 1188, "outputTokens": 468, "cacheReadInputTokens": 4000,
    "cacheCreationInputTokens": 600, "costUSD": 0.0045}}
BASE = {
    "type": "result",
    "session_id": "s-1",
    "num_turns": 3,
    "duration_ms": 5100,
    "total_cost_usd": 0.0045,
    "usage": USAGE,
    "modelUsage": MODEL_USAGE,
    "permission_denials": [],
}

API_ERRORS = {
    "api_401": (401, "Failed to authenticate. API Error: 401"),
    "api_403": (403, "Failed to authenticate. API Error: 403"),
    "api_429": (429, "You've hit your session limit · resets 6:42pm"),
    "api_500": (500, "API Error: 500 This is a server-side issue"),
    "api_400": (400, "API Error: 400 bad request"),
}

if MODE in API_ERRORS:
    status, message = API_ERRORS[MODE]
    emit({**BASE, "subtype": "success", "is_error": True,
          "terminal_reason": "api_error", "api_error_status": status,
          "result": message, "usage": {"input_tokens": 0, "output_tokens": 0}}, code=1)

if MODE == "structured_output_retries":
    emit({**BASE, "subtype": "error_max_structured_output_retries", "is_error": True,
          "terminal_reason": "structured_output_retry_exhausted", "errors": []}, code=1)

if MODE == "context_limit":
    emit({**BASE, "subtype": "success", "is_error": True,
          "terminal_reason": "prompt_too_long", "api_error_status": None,
          "result": "Context limit reached"}, code=1)

ANSWER = json.loads(os.environ.get(
    "FAKE_REPLY", '{"summary":"fixture","attention":"none"}'))
FRAME = {**BASE, "subtype": "success", "is_error": False,
         "api_error_status": None, "terminal_reason": "completed",
         "result": json.dumps(ANSWER)}

if MODE == "fenced_result":
    emit({**FRAME, "result": "```json\n" + json.dumps(ANSWER) + "\n```"})

if MODE == "prose_result":
    emit({**FRAME, "result": "I could not classify that."})

if MODE == "thinking_leak":
    emit({**FRAME, "structured_output": ANSWER,
          "usage": {**USAGE, "output_tokens_details": {"thinking_tokens": 384}}})

if MODE == "no_structured_output":
    # The answer text is there and schema-shaped; the schema-bound field is
    # not. Only the field counts.
    emit(FRAME)

if MODE == "null_structured_output":
    emit({**FRAME, "structured_output": None})

emit({**FRAME, "structured_output": ANSWER})
