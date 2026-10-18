"""CI mode: each agent in the run's disposable HOME, signed in with an API key
from the environment, no write sandbox (PRD agent-journey-check D-31).

The candidate daemon's HOME is the agents' HOME, so the kit's hooks and
Herdr's integration are where each agent reads its own configuration, as on
a new machine. This table is CI mode's only per-agent CLI knowledge, as the
route table is local mode's. With `--fixture-bin` the compiled stand-ins of
`web/e2e/shims` run in place of the CLIs and no key is used.
"""

import json
import os
from pathlib import Path
import shutil

from agent_live_check.protection import ProtectionError, private_directory, write_private
from agent_live_check.runtime import XDG_FOLDERS, clean_env
from agent_live_check.system import WINDOWS, home_env, launcher, temp_env

from . import cells, omp_state, opencode_service
from .local import QUIET

LOGINS = {
    # Claude Code reads the key from ANTHROPIC_API_KEY; a fresh HOME first
    # asks to finish onboarding, to accept that key and to trust the folder,
    # which the disposable ~/.claude.json answers for the measured repository
    # only, so the never-trusted cell still meets Claude's folder question.
    "claude": {"secret": "HIDE_CLAUDE_API_KEY", "key_env": "ANTHROPIC_API_KEY", "env": {}},
    # Codex stores an API key login in its home with `codex login --with-api-key`.
    "codex": {"secret": "HIDE_CODEX_API_KEY", "key_env": None, "env": {"CODEX_HOME": ".codex"}},
    # agy 1.4.0 signs in with GEMINI_API_KEY only once its settings name
    # `"modelProvider": "gemini"` (docs/TESTING.md, A real agent on CI); the
    # model the key's free tier serves is named in the workflow.
    "agy": {"secret": "HIDE_AGY_API_KEY", "key_env": "GEMINI_API_KEY", "env": {}},
    # Grok 1.0.52 falls back to XAI_API_KEY with no `grok login`
    # (~/.grok/docs/user-guide/11-custom-models.md, Credential Resolution).
    "grok": {"secret": "HIDE_GROK_API_KEY", "key_env": "XAI_API_KEY", "env": {}},
    # Pi 1.0.4 and omp 18.7.0 read a provider's key from its environment
    # variable (`pi --help`, `omp --help`); OpenAI's is OPENAI_API_KEY.
    "pi": {"secret": "HIDE_PI_API_KEY", "key_env": "OPENAI_API_KEY", "env": {}},
    "omp": {"secret": "HIDE_OMP_API_KEY", "key_env": "OPENAI_API_KEY", "env": {}},
    # OpenCode 2.0.29 reads a provider's key from its environment variable;
    # its XDG folders are the probe's, named for every command so a `service
    # stop` run outside the pane stays in them, and the mark names the server
    # it starts as the run's (opencode_service.py).
    "opencode": {"secret": "HIDE_OPENCODE_API_KEY", "key_env": "OPENAI_API_KEY",
                 "env": {**XDG_FOLDERS, "XDG_CONFIG_HOME": str(opencode_service.CONFIG.parent),
                         opencode_service.MARK: "."}},
}

# A stand-in asks no folder question, so its start anywhere says nothing of one.
FIXTURE_UNTRUSTED = "not measured: a stand-in has no folder question; the login lane measures it"
# The agents the stand-ins imitate (web/e2e/shims/live-check.h) and the cells
# they pass, so the fixture lane passes only when each passed.
FIXTURE_AGENTS = ("claude-code", "codex")
# Why the stand-in lane leaves every other agent unstarted.
FIXTURE_ONLY = "a stand-in imitates Claude Code and Codex only; the fake model lane measures this agent"
# Cells the C stand-in does not imitate: it answers no question from its
# start's context, forks nothing, writes no title and no token usage. The
# keyless lane measures them once it runs the pinned CLIs on a fake model.
FIXTURE_UNIMITATED = {cell: "the C stand-in does not imitate it; the signed-in lane measures it"
                      for cell in ("guidance_memory", "fork", "title", "usage")}
FIXTURE_CELLS = tuple(name for name in cells.BUILT_CELLS
                      if name != "start_untrusted" and name not in FIXTURE_UNIMITATED)

# What a fixture stand-in cannot show, said in its report (PRD B27).
FIXTURE_LIMITS = [
    "A stand-in answers with the token its prompt or its hook context carries; no model reads the prompt.",
    "Herdr's integration runs from the hook commands the kit installed, fed the payload the stand-in writes, "
    "so a change in the real CLI's hook payload is not seen.",
    "The working and idle screens and titles and the session file format are the stand-in's copies of the CLI's.",
    "Codex's app-server, through which Hide records folder and hook trust, is the trust tests' stand-in "
    "(hide-agent-hooks/tests/fixtures/fake-codex.py), which keeps trust beside config.toml, not in it.",
    "A stand-in waits 5 s before its first turn, as a CLI's start takes seconds, so a first turn that ends "
    "sooner than Hide learns the session is not exercised.",
]


def redact(data):
    """`data` with each CI key, and the last 20 characters Claude's key
    question shows of it, replaced, for anything the run writes where an
    artifact may carry it."""
    for login in LOGINS.values():
        secret = os.environ.get(login["secret"], "").encode()
        for value in (secret, secret[-20:]) if len(secret) > 20 else (secret,):
            if value:
                data = data.replace(value, b"[redacted]")
    return data


def executable(fixture_bin, kind):
    """The program a pane's `claude` or `codex` runs, or None when it is not installed."""
    if fixture_bin:
        program = fixture_bin / (kind + (".exe" if os.name == "nt" else ""))
        if not program.is_file() or not os.access(program, os.X_OK):
            raise ProtectionError(f"fixture_bin_missing_{kind}: build web/e2e/shims/claude-shim.c into {fixture_bin}")
        return program
    found = shutil.which(kind)
    return Path(found) if found else None


def not_configured(kind, fixture_bin, fake=False):
    """The secret this agent's login needs, when the lane has none."""
    if fixture_bin or fake:
        return None
    secret = LOGINS[kind]["secret"]
    return None if os.environ.get(secret) else secret


def route_names(kind, fixture_bin, fake_env=None):
    """The environment CI mode adds to an agent, by name only (its values are paths or a key)."""
    names = [*home_env(Path()), "HIDE_STATE_DIR", "PATH", *QUIET, *LOGINS[kind]["env"]]
    if fixture_bin:
        names.append("HIDE_E2E_JOURNEY")
    elif fake_env is not None:
        names.extend(fake_env)
    elif LOGINS[kind]["key_env"]:
        names.append(LOGINS[kind]["key_env"])
    return names


def key_file(runtime, kind):
    """Where the run keeps the key for the agent's wrapper to read; under the
    probe, which the cleanup removes, so no report or artifact carries it."""
    return runtime.probe / "keys" / kind


def wrapper(runtime, program, kind, path, journey_config=None, fake_env=None):
    """The program a pane runs as `kind`: the agent in the disposable HOME,
    with the runner's PATH behind the candidate's `hide` and its key, or the
    fake model's environment (fake_lane.py) in place of a key."""
    private_directory(runtime.bin)
    environment = {**home_env(runtime.home), "HIDE_STATE_DIR": str(runtime.state),
                   "PATH": os.pathsep.join((str(runtime.bin), path)), **QUIET,
                   **{name: str(runtime.home / relative) for name, relative in LOGINS[kind]["env"].items()}}
    if journey_config:
        environment["HIDE_E2E_JOURNEY"] = str(journey_config)
    environment.update(fake_env or {})
    key_env = LOGINS[kind]["key_env"]
    keyed = key_env and not journey_config and fake_env is None
    return launcher(runtime.bin / kind, [program], environment=environment, unset=["CLAUDE_CONFIG_DIR"],
                    file_variables=[(key_env, key_file(runtime, kind))] if keyed else [])


def login(runtime, kind, launch, root):
    """Signs the agent in with the lane's key; returns the refusal, or None."""
    secret = os.environ[LOGINS[kind]["secret"]]
    keys = runtime.probe / "keys"
    # Registered before anything is written, so even an unconfirmed cleanup removes them.
    runtime.credential_roots.add(keys)
    if kind == "codex":
        runtime.credential_roots.add(runtime.home / LOGINS[kind]["env"]["CODEX_HOME"])
    private_directory(keys)
    write_private(key_file(runtime, kind), secret.encode())
    if kind == "claude":
        claude_settings(runtime.home, root, secret)
        return None
    if kind == "agy":
        settings = runtime.home / ".gemini/antigravity-cli/settings.json"
        private_directory(settings.parent)
        write_private(settings, json.dumps({"modelProvider": "gemini"}).encode())
        return None
    if kind == "omp":
        folder = runtime.home / ".omp/agent"
        private_directory(folder)
        write_private(folder / "config.yml", omp_state.SET_UP.encode())
        return None
    if kind != "codex":
        # The key in the agent's environment is the whole login.
        return None
    environment = {**clean_env(), **home_env(runtime.home), "PATH": runtime.env["PATH"],
                   **temp_env(runtime.run / "agent-tmp")}
    with open(key_file(runtime, kind), "rb") as key:
        code, _, error = runtime.owner.run([str(launch), "login", "--with-api-key"], env=environment,
                                           check=False, stdin=key)
    if code:
        return "codex login --with-api-key refused: " + error.strip()[:200]
    code, output, error = runtime.owner.run([str(launch), "login", "status"], env=environment, check=False)
    return None if not code else "codex login status after the key login: " + (output + error).strip()[:200]


def claude_settings(home, root, secret):
    """The disposable ~/.claude.json a signed-in Claude would hold for the measured repository."""
    write_private(home / ".claude.json", json.dumps({
        "hasCompletedOnboarding": True,
        "customApiKeyResponses": {"approved": [secret[-20:]], "rejected": []},
        # Claude Code names a Windows folder with `/` between names (docs/agent-hooks.md).
        "projects": {root.as_posix() if WINDOWS else str(root): {"hasTrustDialogAccepted": True}},
    }).encode())


def fixture_config(runtime, kind):
    """The stand-in's configuration: the hook commands the kit installed for
    `kind`, in the order its file lists them (web/e2e/shims/live-check.h)."""
    files = {"claude": runtime.home / ".claude/settings.json", "codex": runtime.home / ".codex/hooks.json"}
    document = json.loads(files[kind].read_text())
    # Each hook's stderr and exit status, kept in the run folder as evidence.
    lines = ["shell\t/bin/sh", f"index\t{runtime.probe / 'fixture-sessions'}", f"log\t{runtime.run / f'fixture-{kind}-hooks.log'}",
             f"app_server\t{runtime.checkout / 'hide-agent-hooks/tests/fixtures/fake-codex.py'}"]
    for event, groups in document.get("hooks", {}).items():
        for group in groups:
            for hook in group.get("hooks", []):
                command = hook.get("command")
                if hook.get("type") == "command" and isinstance(command, str) and "\n" not in command:
                    lines.append(f"hook\t{event}\t{command}")
    target = runtime.probe / f"fixture-{kind}.config"
    write_private(target, ("\n".join(lines) + "\n").encode())
    return target
