#!/usr/bin/env python3
"""Stand-in for `codex app-server --listen stdio://` used by hide-ai tests.

Speaks the subset of the JSON-RPC protocol the backend uses. Behaviour is
selected with FAKE_MODE: ok (default), slow, usage_limit, garbage,
exit_at_thread_start, exit (at turn/start, before answering),
exit_after_turn_start, no_account, no_model, and child_per_thread. The last
one models the process leak this change exists to cap: it starts a long-lived
child process on every thread/start and answers the turn normally, and it ends
all of those children when its stdin reaches EOF (the owner's shutdown signal)
so a killed owner leaves no survivors. FAKE_ARGS_FILE records the argument
vector when set, FAKE_INSTRUCTIONS_FILE the base instructions of the last
thread/start, FAKE_TURN_FILE the parameters of the last turn/start, and
FAKE_STARTED_FILE is created once a slow turn has
started and is waiting.
interrupted_after_usage, stream_disconnected and stream_disconnected_500 end
an already-started turn after it reported tokens without a usable answer.
generic_failure and unknown_failure do the same with missing or unfamiliar
error information; bad_request and connection_failed refuse before usage.
In every mode an `outputSchema` that uses a keyword the Responses API's strict
mode refuses fails the turn the way codex-cli 0.163.0 was measured to: an
`other` error whose message is the API's own 400 body.
"""
import atexit
import json
import os
import subprocess
import sys
import time

MODE = os.environ.get("FAKE_MODE", "ok")
REPLY = os.environ.get("FAKE_REPLY", '{"summary":"fixture","attention":"none"}')
if path := os.environ.get("FAKE_ARGS_FILE"):
    with open(path, "w", encoding="utf-8") as f:
        json.dump(sys.argv[1:], f)

# Children started per thread, ended when this process does (atexit) or when
# stdin reaches EOF - the same ownership the real app-server has over its tree.
CHILDREN = []


def spawn_child():
    CHILDREN.append(subprocess.Popen(["sleep", "600"]))


def reap_children():
    for child in CHILDREN:
        try:
            child.kill()
        except Exception:
            pass


atexit.register(reap_children)


def send(obj):
    sys.stdout.write(json.dumps(obj) + "\n")
    sys.stdout.flush()


def result(rid, value):
    send({"jsonrpc": "2.0", "id": rid, "result": value})


def notify(method, params):
    send({"jsonrpc": "2.0", "method": method, "params": params})


STRICT_REFUSED = ("oneOf", "uniqueItems")


def strict_refusal(schema):
    """The first keyword strict mode refuses, as the API names its place."""
    def walk(node, place):
        if isinstance(node, dict):
            for key, value in node.items():
                if key in STRICT_REFUSED:
                    return f"{place}.{key}"
                found = walk(value, f"{place}.{key}")
                if found:
                    return found
        elif isinstance(node, list):
            for item in node:
                found = walk(item, place)
                if found:
                    return found
        return None
    return walk(schema, "schema") if schema is not None else None


active_turn = None
for raw in sys.stdin:
    raw = raw.strip()
    if not raw:
        continue
    msg = json.loads(raw)
    rid = msg.get("id")
    method = msg.get("method")
    params = msg.get("params") or {}
    if method == "initialize":
        result(rid, {"userAgent": "fake"})
    elif method == "initialized":
        pass
    elif method == "account/read":
        account = None if MODE == "no_account" else {"type": "chatgpt", "planType": "pro"}
        result(rid, {"account": account, "requiresOpenaiAuth": True})
    elif method == "model/list":
        models = [] if MODE == "no_model" else [{"id": "gpt-5.6-luna", "model": "gpt-5.6-luna"}]
        result(rid, {"data": models, "nextCursor": None})
    elif method == "account/rateLimits/read":
        result(rid, {"rateLimits": {"primary": {"usedPercent": 100, "resetsAt": int(time.time()) + 120}}})
    elif method == "thread/start":
        if MODE == "exit_at_thread_start":
            sys.exit(2)
        assert params.get("ephemeral") is True and params.get("baseInstructions")
        if path := os.environ.get("FAKE_INSTRUCTIONS_FILE"):
            with open(path, "w", encoding="utf-8") as f:
                f.write(params["baseInstructions"])
        if MODE == "child_per_thread":
            spawn_child()
        result(rid, {"thread": {"id": "thread-1"}, "model": params.get("model"), "instructionSources": []})
    elif method == "turn/start":
        if MODE == "exit":
            sys.exit(3)
        assert params.get("clientUserMessageId")
        if path := os.environ.get("FAKE_TURN_FILE"):
            with open(path, "w", encoding="utf-8") as f:
                json.dump(params, f)
        active_turn = "turn-1"
        result(rid, {"turn": {"id": active_turn, "status": "inProgress", "items": []}})
        notify("turn/started", {"threadId": "thread-1", "turn": {"id": active_turn, "status": "inProgress", "items": []}})
        if refused := strict_refusal(params.get("outputSchema")):
            body = {"type": "error", "status": 400, "error": {
                "message": f"Invalid schema for response_format 'codex_output_schema': '{refused}' is not permitted when strict is true.",
                "type": "invalid_request_error", "param": "text.format.schema", "code": "invalid_json_schema"}}
            notify("turn/completed", {"threadId": "thread-1", "turn": {"id": active_turn, "status": "failed", "items": [],
                    "error": {"message": json.dumps(body), "codexErrorInfo": "other"}}})
            continue
        if MODE == "exit_after_turn_start":
            sys.exit(4)
        if MODE == "slow":
            if path := os.environ.get("FAKE_STARTED_FILE"):
                open(path, "w").close()
            continue
        if MODE == "usage_limit":
            notify("turn/completed", {"threadId": "thread-1", "turn": {"id": active_turn, "status": "failed", "items": [],
                    "error": {"message": "limit", "codexErrorInfo": "usageLimitExceeded"}}})
            continue
        if MODE in ("bad_request", "connection_failed"):
            info = "badRequest" if MODE == "bad_request" else {"responseStreamConnectionFailed": {"httpStatusCode": None}}
            notify("turn/completed", {"threadId": "thread-1", "turn": {"id": active_turn, "status": "failed", "items": [],
                    "error": {"message": "refused", "codexErrorInfo": info}}})
            continue
        text = "not json at all" if MODE == "garbage" else REPLY
        notify("item/completed", {"threadId": "thread-1", "turnId": active_turn,
                                  "item": {"type": "agentMessage", "id": "msg-1", "text": text, "phase": "final_answer"}})
        notify("thread/tokenUsage/updated", {"threadId": "thread-1", "turnId": active_turn,
               "tokenUsage": {"last": {"inputTokens": 1234, "outputTokens": 56, "cachedInputTokens": 0, "reasoningOutputTokens": 0, "totalTokens": 1290},
                              "total": {"inputTokens": 1234, "outputTokens": 56, "cachedInputTokens": 200, "cacheWriteInputTokens": 34, "reasoningOutputTokens": 12, "totalTokens": 1290}}})
        if MODE == "interrupted_after_usage":
            notify("turn/completed", {"threadId": "thread-1", "turn": {"id": active_turn, "status": "interrupted", "items": []}})
            continue
        if MODE in ("stream_disconnected", "stream_disconnected_500"):
            status = 500 if MODE == "stream_disconnected_500" else None
            notify("turn/completed", {"threadId": "thread-1", "turn": {"id": active_turn, "status": "failed", "items": [],
                    "error": {"message": "stream ended", "codexErrorInfo": {"responseStreamDisconnected": {"httpStatusCode": status}}}}})
            continue
        if MODE in ("generic_failure", "unknown_failure"):
            info = None if MODE == "generic_failure" else "unfamiliarFailure"
            notify("turn/completed", {"threadId": "thread-1", "turn": {"id": active_turn, "status": "failed", "items": [],
                    "error": {"message": "failed", "codexErrorInfo": info}}})
            continue
        notify("turn/completed", {"threadId": "thread-1", "turn": {"id": active_turn, "status": "completed", "items": []}})
    elif method == "turn/interrupt":
        result(rid, {})
        notify("turn/completed", {"threadId": params.get("threadId"), "turn": {"id": params.get("turnId"), "status": "interrupted", "items": []}})
        active_turn = None
    elif rid is not None:
        send({"jsonrpc": "2.0", "id": rid, "error": {"code": -32601, "message": "unknown"}})
