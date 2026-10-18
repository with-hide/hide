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
from agent_live_check.runtime import clean_env
from agent_live_check.system import WINDOWS, home_env, launcher, temp_env

from . import cells
from .local import QUIET

LOGINS = {
    # Claude Code reads the key from ANTHROPIC_API_KEY; a fresh HOME first
    # asks to finish onboarding, to accept that key and to trust the folder,
    # which the disposable ~/.claude.json answers for the measured repository
    # only, so the never-trusted cell still meets Claude's folder question.
    "claude": {"secret": "HIDE_CLAUDE_API_KEY", "key_env": "ANTHROPIC_API_KEY"},
    # Codex stores an API key login in its home with `codex login --with-api-key`.
    "codex": {"secret": "HIDE_CODEX_API_KEY", "key_env": None},
}

# Each CLI's own folder in its HOME, which says to the kit that it is set up.
HOME_FOLDERS = {"claude": ".claude", "codex": ".codex"}

# A stand-in asks no folder question, so its start anywhere says nothing of one.
FIXTURE_UNTRUSTED = "not measured: a stand-in has no folder question; the login lane measures it"
# Cells the C stand-in does not imitate: it answers no question from its
# start's context, forks nothing, writes no title and no token usage. The
# keyless lane measures them once it runs the pinned CLIs on a fake model.
FIXTURE_UNIMITATED = {cell: "the C stand-in does not imitate it; the signed-in lane measures it"
                      for cell in ("guidance_memory", "fork", "title", "usage")}
# What the stand-ins imitate, so the fixture lane passes only when each passed.
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


def not_configured(kind, fixture_bin):
    """The secret this agent's login needs, when the lane has none."""
    if fixture_bin:
        return None
    secret = LOGINS[kind]["secret"]
    return None if os.environ.get(secret) else secret


def route_names(kind, fixture_bin):
    """The environment CI mode adds to an agent, by name only (its values are paths or a key)."""
    names = [*home_env(Path()), "HIDE_STATE_DIR", "PATH", *QUIET]
    if kind == "codex":
        names.append("CODEX_HOME")
    if fixture_bin:
        names.append("HIDE_E2E_JOURNEY")
    elif LOGINS[kind]["key_env"]:
        names.append(LOGINS[kind]["key_env"])
    return names


def key_file(runtime, kind):
    """Where the run keeps the key for the agent's wrapper to read; under the
    probe, which the cleanup removes, so no report or artifact carries it."""
    return runtime.probe / "keys" / kind


def wrapper(runtime, program, kind, path, journey_config=None):
    """The `claude` or `codex` a pane runs: the agent in the disposable HOME,
    with the runner's PATH behind the candidate's `hide` and, for Claude, its key."""
    private_directory(runtime.bin)
    environment = {**home_env(runtime.home), "HIDE_STATE_DIR": str(runtime.state),
                   "PATH": os.pathsep.join((str(runtime.bin), path)), **QUIET}
    if kind == "codex":
        environment["CODEX_HOME"] = str(runtime.home / ".codex")
    if journey_config:
        environment["HIDE_E2E_JOURNEY"] = str(journey_config)
    key_env = LOGINS[kind]["key_env"]
    return launcher(runtime.bin / kind, [program], environment=environment, unset=["CLAUDE_CONFIG_DIR"],
                    file_variables=[(key_env, key_file(runtime, kind))] if key_env and not journey_config else [])


def login(runtime, kind, launch, root):
    """Signs the agent in with the lane's key; returns the refusal, or None."""
    secret = os.environ[LOGINS[kind]["secret"]]
    keys = runtime.probe / "keys"
    # Registered before anything is written, so even an unconfirmed cleanup removes them.
    runtime.credential_roots.add(keys)
    if kind == "codex":
        runtime.credential_roots.add(runtime.home / HOME_FOLDERS[kind])
    private_directory(keys)
    write_private(key_file(runtime, kind), secret.encode())
    if kind == "claude":
        claude_settings(runtime.home, root, secret)
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
