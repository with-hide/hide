#!/usr/bin/env python3
"""Stand-in for `codex app-server` over stdio, used by the trust tests.

It reads the hooks the way Codex lists them from `$CODEX_HOME/hooks.json`,
keeps trust in `$CODEX_HOME/fake-trust.json` (a table per key, like
`hooks.state` in config.toml, so `enabled` is a field the upsert must keep),
and records what happened beside them: `fake-calls.log` (one method per line)
and `fake-pid` (its own pid). `$CODEX_HOME/fake-mode` selects the behaviour:

  ok            (default) answers everything
  unsupported   hooks/list is an unknown method, answered the way codex-cli
                0.160.0 answers one (-32600, "unknown variant")
  invalid       hooks/list is refused for its parameters (-32600, not unknown)
  hang          never answers hooks/list, and keeps a child running
  noisy         sends a notification and a request of its own (with the id
                of Hide's next request) before it answers hooks/list
  flood         makes more requests of the client than any answer needs
  big_line      prints one line longer than any caller reads
  errors        lists no hooks and reports config.toml as unusable, the way
                codex-cli 0.160.0 does (the error names the Codex home)
  unparsable    lists no hooks and warns "failed to parse hooks config <path>",
                the way codex-cli 0.160.0 reports a hooks.json it cannot read
  warning       answers normally with a warning about another tool's hook
  escape        answers normally and leaves a process outside its tree
                holding its output open
  no_server     ends at once, before it reads the handshake, the way a
                program with no app-server does
  exit          ends right after initialize
  ignore_write  accepts config/batchWrite and stores nothing
  refuse_write  answers config/batchWrite with an error
  chatty        prints far more than any caller reads
  project       also lists an identical project-layer hook
  overridden    answers a projects write `okOverridden`, as when a managed
                layer decides the value
  slow_write    takes 2.5 s over a projects write, longer than any wait a
                second change of the same trust once had

Folder trust (`projects."<folder>".trust_level`) lives in
`$CODEX_HOME/fake-projects.json`: `config/read` answers it as the user layer's
`projects`, with the file's hash as the layer's version, and
`config/batchWrite` refuses a write whose `expectedVersion` is not it.
"""
import hashlib
import json
import os
import subprocess
import sys
import time

HOME = os.environ["CODEX_HOME"]


def path(name):
    return os.path.join(HOME, name)


def mode():
    try:
        return open(path("fake-mode")).read().strip()
    except OSError:
        return "ok"


def load_trust():
    try:
        return json.load(open(path("fake-trust.json")))
    except OSError:
        return {}


def save_trust(tables):
    with open(path("fake-trust.json"), "w") as f:
        json.dump(tables, f)


SNAKE = {
    "SessionStart": "session_start",
    "UserPromptSubmit": "user_prompt_submit",
    "SubagentStart": "subagent_start",
    "SubagentStop": "subagent_stop",
    "Stop": "stop",
    "PreToolUse": "pre_tool_use",
}


def wire(event):
    return event[0].lower() + event[1:]


def listed():
    hooks_json = path("hooks.json")
    try:
        document = json.load(open(hooks_json))
    except OSError:
        return []
    trust = load_trust()
    out = []
    order = 0
    for event, groups in document.get("hooks", {}).items():
        for g, group in enumerate(groups):
            for h, hook in enumerate(group.get("hooks", [])):
                key = f"{hooks_json}:{SNAKE.get(event, event)}:{g}:{h}"
                current = "sha256:" + hashlib.sha256(
                    json.dumps([event, group.get("matcher"), hook.get("command")]).encode()
                ).hexdigest()
                held = trust.get(key, {}).get("trusted_hash")
                status = "untrusted" if held is None else ("trusted" if held == current else "modified")
                out.append({
                    "key": key, "eventName": wire(event), "handlerType": "command",
                    "command": hook.get("command"), "async": False,
                    "matcher": group.get("matcher"), "timeoutSec": hook.get("timeout", 600),
                    "statusMessage": None, "additionalContextLimit": None,
                    "sourcePath": hooks_json, "source": "user", "pluginId": None,
                    "displayOrder": order, "enabled": trust.get(key, {}).get("enabled", True),
                    "isManaged": False, "currentHash": current, "trustStatus": status,
                })
                order += 1
    return out


def load_projects():
    try:
        return json.load(open(path("fake-projects.json")))
    except OSError:
        return None


def projects_version():
    try:
        return "sha256:" + hashlib.sha256(open(path("fake-projects.json"), "rb").read()).hexdigest()
    except OSError:
        return None


def key_path(text):
    """Codex's keyPath: dot-separated, a quoted segment keeps its dots."""
    segments, segment, quoted, i = [], "", False, 0
    while i < len(text):
        ch = text[i]
        if ch == '"' and not quoted and segment == "":
            quoted = True
        elif ch == '"' and quoted:
            quoted = False
        elif ch == "\\" and quoted:
            i += 1
            segment += text[i]
        elif ch == "." and not quoted:
            segments.append(segment)
            segment = ""
        else:
            segment += ch
        i += 1
    segments.append(segment)
    return segments


def send(obj):
    sys.stdout.write(json.dumps(obj) + "\n")
    sys.stdout.flush()


def reply(rid, result):
    send({"jsonrpc": "2.0", "id": rid, "result": result})


def error(rid, code, message):
    send({"jsonrpc": "2.0", "id": rid, "error": {"code": code, "message": message}})


if sys.argv[1:] != ["app-server"]:
    sys.stderr.write("unexpected arguments: %r\n" % sys.argv[1:])
    sys.exit(2)

with open(path("fake-pid"), "w") as f:
    f.write(str(os.getpid()))
with open(path("fake-cwd"), "w") as f:
    f.write(os.getcwd())
MODE = mode()
if MODE == "no_server":
    sys.exit(1)
if MODE == "hang":
    # The child starts before any request, so a check that times out before
    # `hooks/list` arrives (a slow start under load) still has one to stop.
    child = subprocess.Popen(["sleep", "600"])
    with open(path("fake-child-pid"), "w") as f:
        f.write(str(child.pid))

for raw in sys.stdin:
    raw = raw.strip()
    if not raw:
        continue
    message = json.loads(raw)
    method = message.get("method")
    rid = message.get("id")
    params = message.get("params") or {}
    with open(path("fake-calls.log"), "a") as log:
        log.write(f"{method if method else 'client-response'}\n")
    if method == "initialize":
        reply(rid, {"userAgent": "fake", "codexHome": HOME})
        if MODE == "exit":
            sys.exit(0)
    elif method == "initialized":
        pass
    elif method == "hooks/list":
        if MODE == "unsupported":
            error(rid, -32600, "Invalid request: unknown variant `hooks/list`, expected one of `initialize`, `thread/start`")
        elif MODE == "invalid":
            error(rid, -32600, "Invalid request: missing field `cwds`")
        elif MODE == "hang":
            while True:
                time.sleep(0.2)
        elif MODE == "flood":
            for n in range(50):
                send({"jsonrpc": "2.0", "id": 1000 + n, "method": "item/tool/requestUserInput", "params": {}})
            while True:
                time.sleep(0.2)
        elif MODE == "big_line":
            sys.stdout.write("y" * (2 * 1024 * 1024) + "\n")
            sys.stdout.flush()
        elif MODE == "errors":
            reply(rid, {"data": [{"cwd": os.getcwd(), "hooks": [], "warnings": [],
                                  "errors": [{"path": HOME, "message": path("config.toml") + ":1:10: unclosed array"}]}]})
        elif MODE == "unparsable":
            reply(rid, {"data": [{"cwd": os.getcwd(), "hooks": [], "errors": [],
                                  "warnings": ["failed to parse hooks config %s: key must be a string at line 1 column 3" % path("hooks.json")]}]})
        elif MODE == "chatty":
            for _ in range(80):
                sys.stdout.write("x" * 65536 + "\n")
            sys.stdout.flush()
        else:
            if MODE == "noisy":
                send({"jsonrpc": "2.0", "method": "remoteControl/status/changed", "params": {"status": "disabled"}})
                send({"jsonrpc": "2.0", "id": rid, "method": "item/tool/requestUserInput", "params": {}})
            if MODE == "escape":
                if os.fork() == 0:
                    os.setsid()
                    if os.fork() == 0:
                        with open(path("fake-escaped-pid"), "w") as f:
                            f.write(str(os.getpid()))
                        time.sleep(60)
                    os._exit(0)
                os.wait()
                while not os.path.exists(path("fake-escaped-pid")):
                    time.sleep(0.01)
            hooks = listed()
            if MODE == "project":
                hooks += [dict(h, source="project", key=h["key"] + ":p") for h in hooks]
            warnings = ["skipping prompt hook in /somewhere/else/hooks.json"] if MODE == "warning" else []
            reply(rid, {"data": [{"cwd": os.getcwd(), "hooks": hooks, "warnings": warnings, "errors": []}]})
    elif method == "config/read":
        projects = load_projects()
        layers = [{"name": {"type": "system", "file": "/etc/codex/config.toml"}, "version": "s", "config": {}}]
        if projects is not None:
            layers.insert(0, {"name": {"type": "user", "file": path("config.toml")},
                              "version": projects_version(), "config": {"projects": projects}})
        reply(rid, {"config": {}, "origins": {}, "layers": layers})
    elif method == "config/batchWrite" and params["edits"][0]["keyPath"].startswith("projects."):
        if params.get("expectedVersion") != projects_version():
            error(rid, -32600, "Configuration was modified since last read. Fetch latest version and retry.")
            continue
        if MODE == "slow_write":
            time.sleep(2.5)
        projects = load_projects() or {}
        for edit in params["edits"]:
            assert edit["mergeStrategy"] == "replace"
            segments = key_path(edit["keyPath"])
            assert segments[0] == "projects"
            if edit["value"] is None:
                if len(segments) == 2:
                    projects.pop(segments[1], None)
                else:
                    projects.get(segments[1], {}).pop(segments[2], None)
            else:
                projects.setdefault(segments[1], {})[segments[2]] = edit["value"]
        if MODE != "ignore_write":
            with open(path("fake-projects.json"), "w") as f:
                json.dump(projects, f)
        status = "okOverridden" if MODE == "overridden" else "ok"
        reply(rid, {"status": status, "version": projects_version(), "filePath": path("config.toml")})
    elif method == "config/batchWrite":
        if MODE == "refuse_write":
            error(rid, -32600, "config is read-only")
        else:
            if MODE != "ignore_write":
                tables = load_trust()
                for edit in params["edits"]:
                    assert edit["keyPath"] == "hooks.state" and edit["mergeStrategy"] == "upsert"
                    for key, table in edit["value"].items():
                        tables.setdefault(key, {}).update(table)
                save_trust(tables)
            reply(rid, {"status": "ok", "version": "v", "filePath": path("config.toml")})
    elif rid is not None:
        error(rid, -32601, "Method not found")
