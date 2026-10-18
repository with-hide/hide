"""Local mode: how each agent reaches the candidate's private configuration.

This table is the only per-agent CLI knowledge the journey has (PRD
agent-journey-check D-24). Everything else an agent is started with comes from
the product's own spawn. The report prints each agent's route beside its spawn
arguments, so what the journey added is visible.

The candidate daemon's kit installs Hide's hooks and Herdr's integration into
the private daemon home as on a new machine. Claude keeps the operator's HOME
for its keychain login and reads only the candidate's settings; Codex reads a
private CODEX_HOME holding the candidate's hooks, a copy of the login and one
trust entry for the disposable repository.
"""

import datetime
import json
import os
from pathlib import Path
import re
import shlex
import tomllib

from agent_live_check.protection import ProtectionError, private_directory, write_private
from agent_live_check.system import WINDOWS

# A Codex login copy older than this is not used: a refresh inside the copy
# would rotate the operator's token (PRD D-32; Codex was seen refreshing every
# 8 days, which it does not document).
CODEX_LOGIN_DAYS = 7

ROUTES = {
    "claude": {
        # Claude's own setting sources, with the user's left out, so the
        # operator's installed hooks and plugins never run in the journey.
        "args": lambda home: ["--settings", str(home / ".claude/settings.json"),
                              "--setting-sources", "project,local"],
        "env": lambda home: {},
        # Keychain login: the operator's own HOME, which is also the folder
        # trust the disposable repository under it inherits.
        "unset": ["CLAUDE_CONFIG_DIR"],
        # Where its sessions are written, for Hide's reader: the operator's HOME.
        "sessions": lambda operator, home: operator,
        "known": [".claude.json", ".claude/settings.json", ".claude/settings.local.json"],
        "shared": {".claude.json": "json"},
        "roots": [".claude.json", ".claude"],
        "untrusted": "not measured: it would leave an entry in the operator's shared ~/.claude.json; the CI mode measures it",
    },
    "codex": {
        "args": lambda home: [],
        "env": lambda home: {"CODEX_HOME": str(home / ".codex")},
        "unset": [],
        "sessions": lambda operator, home: home,
        "known": [".codex/config.toml", ".codex/hooks.json"],
        "shared": {".codex/config.toml": "toml"},
        "roots": [".codex"],
        "untrusted": None,
    },
}

# Telemetry and self-update stay off in every measured agent.
QUIET = {"DISABLE_AUTOUPDATER": "1", "DISABLE_TELEMETRY": "1", "DISABLE_ERROR_REPORTING": "1"}


def route(kind, home):
    """What the journey adds to an agent of `kind`: arguments and environment."""
    if kind not in ROUTES:
        raise ProtectionError("no_local_route:" + kind)
    entry = ROUTES[kind]
    return {"args": entry["args"](home), "env": entry["env"](home), "unset": list(entry["unset"])}


def wrapper(directory, executable, operator, state, kind, home, sandbox, path):
    """The program a pane's `claude` or `codex` runs: the agent with the
    operator's HOME, the operator's PATH behind the candidate's `hide`, and
    its route, inside the write sandbox. A spawned pane has the private
    Herdr's environment, whose PATH reaches neither node nor the account's tools."""
    private_directory(directory)
    target = directory / Path(executable).name
    # The route's arguments go on every start, the product's own restarts
    # (Wake's `--resume`, Fork's `--fork-session`) included, not only the spawn.
    command = [str(executable), *route(kind, home)["args"]]
    settings = home / ".claude/settings.json"
    if kind == "claude" and not settings.exists():
        # Claude refuses a `--settings` file that is not there, and the kit
        # writes its entries into this one only once hided runs.
        private_directory(settings.parent)
        write_private(settings, b"{}\n")
    if sandbox:
        command = sandbox.command(command)
    environment = {"HOME": str(operator), "HIDE_STATE_DIR": str(state), "PATH": os.pathsep.join((str(directory), path)),
                   **QUIET, **route(kind, home)["env"]}
    lines = ["#!/bin/sh"]
    lines.extend("unset " + name for name in route(kind, home)["unset"])
    lines.append("export " + " ".join(f"{key}={shlex.quote(value)}" for key, value in environment.items()))
    lines.append("exec " + " ".join(shlex.quote(value) for value in command) + ' "$@"')
    write_private(target, ("\n".join(lines) + "\n").encode())
    target.chmod(0o700)
    return target


def disable_hide_ai(home):
    """The candidate's Hide AI switched off in its private home: on, it would
    run the operator's signed-in CLI with the private HOME and spend requests
    outside the agent's budget (docs/AI_PROVIDERS.md, `enabled`)."""
    folder = home / "Library/Application Support/hide"
    private_directory(folder)
    write_private(folder / "ai.json", b'{"enabled": false}\n')


def codex_login_age(auth, now=None):
    """Days since the Codex login was last refreshed, or None when it cannot be read."""
    try:
        refreshed = json.loads(auth.read_text()).get("last_refresh")
        when = datetime.datetime.fromisoformat(refreshed.replace("Z", "+00:00"))
    except (OSError, ValueError, AttributeError, TypeError):
        return None
    now = now or datetime.datetime.now(datetime.timezone.utc)
    return (now - when).total_seconds() / 86400


def codex_login_refusal(auth, now=None):
    """Why the Codex login copy must not be used, or None."""
    if not auth.is_file():
        return "not logged in: run codex login yourself; the journey never logs in"
    age = codex_login_age(auth, now)
    if age is None:
        return "login refresh time unreadable; run Codex once, then rerun"
    if age > CODEX_LOGIN_DAYS:
        return f"login near refresh ({age:.1f} days old); run Codex once, then rerun"
    return None


def trust_codex(config, repository):
    """One trust entry for the disposable repository in the private Codex config.

    Codex reads a linked worktree under its main repository's trust, so the
    worktrees the spawns make start at the composer. The resolved path is the
    one Codex compares (/private/... on macOS, in lower case on Windows,
    where Codex compares folders without case; docs/agent-hooks.md).
    """
    text = config.read_text() if config.is_file() else ""
    path = str(repository.resolve())
    entry = f'\n[projects.{json.dumps(path.lower() if WINDOWS else path)}]\ntrust_level = "trusted"\n'
    if entry in text:
        return
    replacement = config.with_name(config.name + ".journey")
    replacement.unlink(missing_ok=True)
    write_private(replacement, (text + entry).encode())
    replacement.replace(config)


def allow_one_shot_references(sandbox, runtime):
    """Let a `hide` command inside the sandbox read the one-shot credential the
    candidate daemon writes for it.

    A spawned agent's pane holds no persistent reference, so each `hide` it
    runs (its hook's `hide inbox --confirm`, for one) bootstraps a one-shot
    reference that the daemon writes as `<nonce>.json` under the private
    state and the command then reads. The bell check's profile hides the
    whole run, so without this every such command answers credential_expired.
    The rule opens those reference files of this run's own candidate and
    nothing else of its state; the operator's daemon stays denied.
    """
    folder = (runtime.state / "pane-capabilities").resolve()
    # The profile's regular expressions take a backslash before a
    # metacharacter and no other escape and no counted repetition.
    escaped = "".join("\\" + char if char in ".^$*+?()[]{}|\\" else char for char in str(folder))
    pattern = "^" + escaped + "/" + "[0-9a-f]" * 32 + "[.]json$"
    sandbox.rules.append(f"(allow file-read* (regex {json.dumps(pattern)}))")
    replacement = sandbox.profile.with_suffix(".next")
    write_private(replacement, ("\n".join(sandbox.rules) + "\n").encode())
    replacement.replace(sandbox.profile)


def operator_trust(operator, run):
    """The folder trust the operator's own agent files hold for a path of this
    run, by file and key (PRD B14): the candidate records trust for each
    worktree it spawns into, and must record Codex's only in the private
    CODEX_HOME the agent reads. A file that is there and cannot be read is a
    failure, never an empty answer."""
    run = run.resolve()

    def ours(key):
        try:
            return Path(key).resolve().is_relative_to(run)
        except (OSError, ValueError):
            return False
    found = {}
    codex = operator / ".codex/config.toml"
    if codex.is_file():
        try:
            projects = tomllib.loads(codex.read_text()).get("projects", {})
        except (OSError, ValueError) as error:
            raise ProtectionError(f"operator_codex_config_unreadable: {error}", path=codex) from error
        found[str(codex)] = [f"projects.{json.dumps(key)}" for key in projects if ours(key)]
    claude = operator / ".claude.json"
    if claude.is_file():
        try:
            projects = json.loads(claude.read_text()).get("projects", {})
        except (OSError, ValueError) as error:
            raise ProtectionError(f"operator_claude_json_unreadable: {error}", path=claude) from error
        found[str(claude)] = [f"projects.{json.dumps(key)}.hasTrustDialogAccepted" for key, entry in projects.items()
                              if ours(key) and isinstance(entry, dict) and "hasTrustDialogAccepted" in entry]
    return found


def claude_session_folders(operator, run):
    """The folders Claude keeps in the operator's ~/.claude/projects for a
    path of this run, by exact path: Claude names each after its working
    folder with every character but a letter or digit as `-`, and an agent
    works in the run's probe."""
    projects = operator / ".claude/projects"
    if not projects.is_dir():
        return []
    prefixes = {re.sub(r"[^A-Za-z0-9]", "-", str(path)) for path in (run.absolute(), run.resolve())}
    return sorted(str(entry) for entry in projects.iterdir()
                  if any(entry.name == prefix or entry.name.startswith(prefix + "-probe-") for prefix in prefixes))
