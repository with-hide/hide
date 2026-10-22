"""Local mode: how each agent reaches the candidate's private configuration.

This table is the only per-agent CLI knowledge the journey has (PRD
agent-journey-check D-24). Everything else an agent is started with comes from
the product's own spawn. The report prints each agent's route beside its spawn
arguments, so what the journey added is visible.

The candidate daemon's kit installs Hide's hooks and Herdr's integration into
the private daemon home as on a new machine. Every agent keeps the operator's
HOME and is pointed at its own folder in the private daemon home by the
variable it documents for that (CLAUDE_CONFIG_DIR, CODEX_HOME, GROK_HOME,
PI_CODING_AGENT_DIR, OpenCode's XDG folders), which holds the candidate's
hooks or extension and, but for Claude, whose login stays the operator's
keychain entry, a copy of the operator's login; agy has no such variable, so
it is not measured here.
"""

import datetime
import json
import os
from pathlib import Path
import re
import shlex
import sys
import time
import tomllib

from agent_live_check.protection import ProtectionError, private_directory, write_private
from agent_live_check.system import WINDOWS

from . import opencode_service

# A Codex login copy older than this is not used: a refresh inside the copy
# would rotate the operator's token (PRD D-32; Codex was seen refreshing every
# 8 days, which it does not document).
CODEX_LOGIN_DAYS = 7
# An OAuth login copy whose access token expires before the run's deadline,
# or within this lead of it, is not used, for the same reason: the agent
# would refresh it inside the copy, and a refresh that rotates the refresh
# token signs the operator's own agent out. Pi 1.0.4 refreshes five minutes
# before expiry (DEFAULT_OAUTH_MINIMUM_VALIDITY_MS); the lead doubles that for
# an agent whose window is not read. A fixed hour refused Pi's OpenAI login
# always, whose access token lasts one hour.
REFRESH_LEAD_SECONDS = 10 * 60

ROUTES = {
    "claude": {
        "args": lambda home: [],
        # Claude Code 2.1.307 reads its settings, plugins, sessions and global
        # config, the folder trust Hide records included, from
        # CLAUDE_CONFIG_DIR: the daemon home's `.claude`, where the
        # candidate's kit and trust write as they write `~/.claude` on a
        # machine. Its keychain entry is named after that folder too, unless
        # CLAUDE_SECURESTORAGE_CONFIG_DIR names another, which empty names the
        # default `~/.claude` (its secure-storage naming, read in the 2.1.307
        # bundle; `claude auth status` reads loggedIn through it), so the
        # login stays the operator's own entry and is never copied.
        "env": lambda home: {"CLAUDE_CONFIG_DIR": str(home / ".claude"), "CLAUDE_SECURESTORAGE_CONFIG_DIR": ""},
        "unset": [],
        "sessions": lambda operator, home: home,
        "known": [".claude.json", ".claude/settings.json", ".claude/settings.local.json"],
        "shared": {".claude.json": "json"},
        "roots": [".claude.json", ".claude"],
        "untrusted": None,
        "config": None,
        "copies": [],
        "login": lambda root, status, margin: claude_login_refusal(*status(["auth", "status"])),
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
        "config": ".codex",
        "copies": [[".codex/auth.json", "auth.json"]],
        "provenance": "A private CODEX_HOME holding a copy of the login (docs/AI_PROVIDERS.md); a refresh stays in the copy.",
        "login": lambda root, status, margin: codex_login_refusal(root / "auth.json"),
    },
    "grok": {
        "args": lambda home: [],
        # Grok 1.0.52 reads its configuration, hooks, folder trust, login and
        # sessions from GROK_HOME (Herdr's integration names it too).
        "env": lambda home: {"GROK_HOME": str(home / ".grok")},
        "unset": [],
        "sessions": lambda operator, home: home,
        "known": [".grok/config.toml", ".grok/trusted_folders.toml"],
        "shared": {},
        "roots": [".grok"],
        "untrusted": None,
        "config": ".grok",
        "copies": [[".grok/auth.json", "auth.json"]],
        "provenance": "A private GROK_HOME holding a copy of the login; a refresh stays in the copy.",
        "login": lambda root, status, margin: grok_login_refusal(root / "auth.json", margin),
    },
    "pi": {
        "args": lambda home: [],
        # Pi 1.0.4's agent folder (`pi --help`: PI_CODING_AGENT_DIR) holds its
        # login, settings, extensions and sessions.
        "env": lambda home: {"PI_CODING_AGENT_DIR": str(home / ".pi/agent")},
        "unset": ["PI_CODING_AGENT_SESSION_DIR"],
        "sessions": lambda operator, home: home,
        "known": [".pi/agent/settings.json"],
        "shared": {},
        "roots": [".pi/agent"],
        "untrusted": None,
        "config": ".pi/agent",
        "copies": [[".pi/agent/auth.json", "auth.json"]],
        "provenance": "A private PI_CODING_AGENT_DIR holding a copy of the login; a refresh stays in the copy.",
        "login": lambda root, status, margin: pi_login_refusal(root / "auth.json", margin),
    },
    "omp": {
        # omp 18.7.0 names a checkout's session folder from its own HOME,
        # which local mode keeps as the operator's, while the candidate's
        # reader names it from the daemon's private HOME.
        "unmeasured": ("omp names its session folder from its own HOME, and local mode's candidate daemon has a "
                       "private HOME, so Hide's reader names another folder for the same checkout and refuses the "
                       "session (label_session_default_directory_required); in the product hided and omp share one "
                       "HOME; the fake model and CI lanes measure it"),
    },
    "opencode": {
        "args": lambda home: [],
        # OpenCode 2.0.29 reads its configuration and plugins, its login and
        # sessions, its state and its cache from the XDG folders
        # (xdg-basedir): the probe's, where Hide's reader reads them too; the
        # mark names the background server it starts as the run's
        # (opencode_service.py).
        "env": opencode_service.environment,
        "unset": ["OPENCODE_CONFIG", "OPENCODE_CONFIG_DIR", "OPENCODE_CONFIG_CONTENT"],
        "sessions": lambda operator, home: home,
        "known": [".config/opencode/opencode.json", ".config/opencode/service.json"],
        "shared": {},
        "roots": [".config/opencode", ".local/share/opencode"],
        "untrusted": None,
        "config": str(opencode_service.DATA),
        "copies": [[".local/share/opencode/auth.json", "auth.json"]],
        "provenance": "Private XDG folders holding a copy of the login, the operator's model choice and a private "
                      "server port; a refresh stays in the copy.",
        "login": lambda root, status, margin: opencode_login_refusal(root / "auth.json", margin),
    },
    "agy": {
        # agy 1.4.0 has no variable or flag for its configuration folder and
        # signs in through the keychain of the HOME it runs in, so the
        # candidate's kit could reach it only through the operator's own
        # ~/.gemini/config, whose hooks would run instead.
        "unmeasured": ("agy reads its hooks only from ~/.gemini/config in the HOME its keychain sign-in needs, so the "
                       "candidate's kit cannot reach it without the operator's own hooks running; the CI mode measures "
                       "it; it closes once agy takes a configuration folder variable or a login kept in a private file"),
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


def wrapper(directory, executable, operator, state, kind, home, sandbox, path, extra=None, keys=None):
    """The program a pane's `claude` or `codex` runs: the agent with the
    operator's HOME, the operator's PATH behind the candidate's `hide`, and
    its route, inside the write sandbox. A spawned pane has the private
    Herdr's environment, whose PATH reaches neither node nor the account's tools.
    Each (variable, file) of `keys` is set to that file's content at each
    start, so no key is written into the program or the report."""
    private_directory(directory)
    target = directory / Path(executable).name
    # The route goes on every start, the product's own restarts (Wake's
    # `--resume`, Fork's `--fork-session`) included, not only the spawn.
    command = [str(executable), *route(kind, home)["args"]]
    if sandbox:
        command = sandbox.command(command)
    environment = {"HOME": str(operator), "HIDE_STATE_DIR": str(state), "PATH": os.pathsep.join((str(directory), path)),
                   **QUIET, **route(kind, home)["env"], **(extra or {})}
    lines = ["#!/bin/sh"]
    lines.extend("unset " + name for name in route(kind, home)["unset"])
    lines.append("export " + " ".join(f"{key}={shlex.quote(value)}" for key, value in environment.items()))
    lines.extend(f"{name}=$(cat {shlex.quote(str(file))}) && export {name}" for name, file in (keys or {}).items())
    lines.append("exec " + " ".join(shlex.quote(value) for value in command) + ' "$@"')
    write_private(target, ("\n".join(lines) + "\n").encode())
    target.chmod(0o700)
    return target


def disable_hide_ai(home):
    """The candidate's Hide AI switched off in its private home: on, it would
    run the operator's signed-in CLI with the private HOME and spend requests
    outside the agent's budget, or, with the fake model, ask it for labels
    the native titles are judged without (docs/AI_PROVIDERS.md, `enabled`).
    The folder is the system's state folder under that home
    (hide_platform::host::state_dir_under)."""
    state = ("Library/Application Support" if sys.platform == "darwin" else
             "AppData/Local" if os.name == "nt" else ".local/state")
    folder = home / state / "hide"
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


def claude_login_refusal(code, output):
    """Why Claude cannot be measured from `claude auth status`'s answer, or None."""
    try:
        logged_in = json.loads(output).get("loggedIn") is True
    except (ValueError, AttributeError):
        logged_in = False
    return None if not code and logged_in else "not logged in: run claude auth login yourself; the journey never logs in"


def expiry_refusal(cli, expiries, margin, now=None):
    """Why an OAuth login copy whose access tokens expire at `expiries` (Unix
    seconds) must not be used by a run that ends within `margin` seconds, or None."""
    if not expiries:
        return f"login expiry unreadable; run {cli} once yourself, then rerun"
    left = min(expiries) - (now if now is not None else time.time())
    if left >= margin + REFRESH_LEAD_SECONDS:
        return None
    state = "expired" if left <= 0 else f"near refresh ({left / 3600:.1f} h left, the run needs {margin / 3600:.1f} h)"
    return (f"login {state}: its copy would refresh inside the run and could sign your own {cli} out; "
            f"run {cli} once yourself (sign in again if it asks), then rerun")


def json_expiries(auth, field, scale):
    """The `field` of each entry of a JSON login file, in Unix seconds; None
    when the file is not there."""
    if not auth.is_file():
        return None
    try:
        entries = json.loads(auth.read_text()).values()
        values = [entry[field] for entry in entries if isinstance(entry, dict) and field in entry]
    except (OSError, ValueError, AttributeError):
        return []
    expiries = []
    for value in values:
        try:
            expiries.append(datetime.datetime.fromisoformat(value.replace("Z", "+00:00")).timestamp()
                            if isinstance(value, str) else float(value) / scale)
        except (ValueError, TypeError):
            return []
    return expiries


def grok_login_refusal(auth, margin, now=None):
    """Grok keeps one entry per account, each with an ISO `expires_at`."""
    expiries = json_expiries(auth, "expires_at", 1)
    if expiries is None:
        return "not logged in: run grok login yourself; the journey never logs in"
    return expiry_refusal("grok", expiries, margin, now)


def pi_login_refusal(auth, margin, now=None):
    """Pi keeps one entry per provider; an OAuth one has `expires` in ms."""
    expiries = json_expiries(auth, "expires", 1000)
    if expiries is None:
        return "not logged in: sign in with /login in pi yourself; the journey never logs in"
    return expiry_refusal("pi", expiries, margin, now)


def opencode_login_refusal(auth, margin, now=None):
    """OpenCode keeps one entry per provider: an API key, which never
    refreshes, or an OAuth login with `expires` in ms."""
    if not auth.is_file():
        return "not logged in: run opencode auth login yourself; the journey never logs in"
    try:
        entries = [entry for entry in json.loads(auth.read_text()).values() if isinstance(entry, dict)]
    except (OSError, ValueError, AttributeError):
        return expiry_refusal("opencode", [], margin, now)
    if not entries:
        return "not logged in: run opencode auth login yourself; the journey never logs in"
    if all(entry.get("type") == "api" for entry in entries):
        return None
    return expiry_refusal("opencode", json_expiries(auth, "expires", 1000), margin, now)


def opencode_model(operator, folder):
    """The private `opencode.json`: only the operator's model choice and
    provider settings, so their MCP servers and permissions stay theirs, and
    no update check. Returns the environment variables the chosen providers
    read their keys from (`opencode_key_names`)."""
    try:
        chosen = json.loads((operator / ".config/opencode/opencode.json").read_text())
    except (OSError, ValueError):
        chosen = {}
    keep = {key: chosen[key] for key in ("model", "small_model", "provider", "providers") if key in chosen}
    private_directory(folder)
    write_private(folder / "opencode.json", json.dumps({**keep, "update": "disable"}).encode())
    return opencode_key_names(keep)


# OpenCode's substitution of a variable into its configuration (`{env:NAME}`).
OPENCODE_ENV = re.compile(r"\{env:([A-Za-z_][A-Za-z0-9_]*)\}")


def opencode_key_names(config):
    """The variables named as `{env:NAME}` in the provider blocks of the
    providers `model` and `small_model` choose (`<provider>/<model>`), by
    provider: the only variables of the operator's shell an OpenCode of the
    journey receives, since the run's environment carries no `_API_KEY`
    (agent_live_check.runtime.clean_env)."""
    providers = {**(config.get("providers") or {}), **(config.get("provider") or {})}
    found = {}
    for choice in (config.get("model"), config.get("small_model")):
        name = choice.partition("/")[0] if isinstance(choice, str) and "/" in choice else None
        if name in providers:
            found[name] = sorted(set(OPENCODE_ENV.findall(json.dumps(providers[name]))))
    return found


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


def trust_grok(trusted, repository):
    """One trust entry for the measured repository in the private Grok
    folder trust (`trusted_folders.toml`), the form Grok 1.0.52 writes when
    the operator trusts a folder: the operator's own Grok trusts the
    measured repository the same way."""
    text = trusted.read_text() if trusted.is_file() else ""
    key = json.dumps(str(repository.resolve()))
    if f"[folders.{key}]" in text:
        return
    entry = f"\n[folders.{key}]\ntrusted = true\ndecided_at = {int(time.time())}\n"
    replacement = trusted.with_name(trusted.name + ".journey")
    replacement.unlink(missing_ok=True)
    write_private(replacement, (text + entry).encode())
    replacement.replace(trusted)


def claude_config(home, repository):
    """Claude's global config in its private folder, written before its first
    run: onboarding finished and the measured repository trusted, as the
    operator's own Claude trusts a project they work in. It is `.config.json`,
    the file Claude Code and Hide both take while it holds anything
    (hide-agent-hooks runtime.rs, `trust_path`), so the trust Hide records for
    each worktree it spawns into is the trust Claude reads."""
    folder = home / ".claude"
    private_directory(folder)
    write_private(folder / ".config.json", json.dumps({
        "hasCompletedOnboarding": True,
        "projects": {str(repository.resolve()): {"hasTrustDialogAccepted": True}}}).encode())


# The trust each agent's own file holds for the measured repository, written
# in the run's private folder of that agent in either mode (PRD D-25).
TRUST = {"codex": (".codex/config.toml", trust_codex), "grok": (".grok/trusted_folders.toml", trust_grok)}


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
