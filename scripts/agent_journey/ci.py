"""CI mode: each agent in the run's disposable HOME, with no key and no
write sandbox (PRD agent-journey-check D-31).

The candidate daemon's HOME is the agents' HOME, so the kit's hooks and
Herdr's integration are where each agent reads its own configuration, as on
a new machine. This table is CI mode's only per-agent CLI knowledge, as the
route table is local mode's. With `--fixture-bin` the compiled stand-ins of
`web/e2e/shims` run in place of the CLIs, and with `--fake-model` the CLIs
are pointed at the journey's fake model (fake_lane.py). CI signs no agent in
(operator decision, 2026-10-27): a login is measured by local mode on the
operator's Mac.
"""

import json
import os
from pathlib import Path
import shutil

from agent_live_check.protection import ProtectionError, private_directory, write_private
from agent_live_check.runtime import XDG_FOLDERS
from agent_live_check.system import WINDOWS, home_env, launcher, temp_env

from . import cells, opencode_service
from .local import QUIET

# What a set-up omp 18.7.0 keeps in its config.yml: without it the first
# start runs omp's setup wizard over the prompt.
OMP_SET_UP = "setupVersion: 2\n"

# The folders each agent is named in the disposable HOME, relative to it.
AGENT_ENV = {
    "claude": {},
    "codex": {"CODEX_HOME": ".codex"},
    "agy": {},
    "grok": {},
    "pi": {},
    "omp": {},
    # OpenCode's XDG folders are the probe's, named for every command so a
    # `service stop` run outside the pane stays in them, and the mark names
    # the server it starts as the run's (opencode_service.py).
    "opencode": {**XDG_FOLDERS, "XDG_CONFIG_HOME": str(opencode_service.CONFIG.parent),
                 opencode_service.MARK: "."},
}

# A stand-in asks no folder question, so its start anywhere says nothing of one.
FIXTURE_UNTRUSTED = "not measured: a stand-in has no folder question; the fake model lane measures it"
# The agents the stand-ins imitate (web/e2e/shims/live-check.h) and the cells
# they pass, so the fixture lane passes only when each passed.
FIXTURE_AGENTS = ("claude-code", "codex")
# Why the stand-in lane leaves every other agent unstarted.
FIXTURE_ONLY = "a stand-in imitates Claude Code and Codex only; the fake model lane measures this agent"
# Cells the C stand-in does not imitate: it answers no question from its
# start's context, draws no composer, forks nothing, writes no title and no
# token usage. The keyless lane measures them once it runs the pinned CLIs on a fake model.
FIXTURE_UNIMITATED = {cell: "the C stand-in does not imitate it; the fake model lane measures it"
                      for cell in ("guidance_memory", "composer", "fork", "title", "usage")}
FIXTURE_CELLS = tuple(name for name in cells.CELLS
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


def executable(fixture_bin, kind):
    """The program a pane's `claude` or `codex` runs, or None when it is not installed."""
    if fixture_bin:
        program = fixture_bin / (kind + (".exe" if os.name == "nt" else ""))
        if not program.is_file() or not os.access(program, os.X_OK):
            raise ProtectionError(f"fixture_bin_missing_{kind}: build web/e2e/shims/claude-shim.c into {fixture_bin}")
        return program
    found = shutil.which(kind)
    return Path(found) if found else None


def route_names(kind, fixture_bin, fake_env=None):
    """The environment CI mode adds to an agent, by name only (its values are paths)."""
    names = [*home_env(Path()), "HIDE_STATE_DIR", "PATH", *temp_env(Path()), *QUIET, *AGENT_ENV[kind]]
    if fixture_bin:
        names.append("HIDE_E2E_JOURNEY")
    elif fake_env is not None:
        names.extend(fake_env)
    return names


def wrapper(runtime, program, kind, path, journey_config=None, fake_env=None, sandbox=None):
    """The program a pane runs as `kind`: the agent in the disposable HOME,
    with the runner's PATH behind the candidate's `hide`, and the stand-in's
    configuration or the fake model's environment (fake_lane.py). Its PATH and
    temporary folder are set here, since a Windows pane takes both from the
    account's registry rather than from Herdr."""
    private_directory(runtime.bin)
    environment = {**home_env(runtime.home), "HIDE_STATE_DIR": str(runtime.state),
                   "PATH": os.pathsep.join((str(runtime.bin), path)), **temp_env(runtime.run / "agent-tmp"), **QUIET,
                   **{name: str(runtime.home / relative) for name, relative in AGENT_ENV[kind].items()}}
    if journey_config:
        environment["HIDE_E2E_JOURNEY"] = str(journey_config)
    environment.update(fake_env or {})
    command = sandbox.command([str(program)]) if sandbox else [program]
    return launcher(runtime.bin / kind, command, environment=environment, unset=["CLAUDE_CONFIG_DIR"])


def agy_set_up(home):
    """What a set-up agy 1.4.2 keeps in its HOME: Gemini as its provider,
    and its first-run onboarding (a color scheme, then a security notice)
    finished, the status it saves in `cache/onboarding.json`. In a HOME
    without it the onboarding opens over the first prompt and can hold it,
    so the start answers nothing."""
    folder = home / ".gemini/antigravity-cli"
    private_directory(folder / "cache")
    write_private(folder / "settings.json", json.dumps({"modelProvider": "gemini"}).encode())
    write_private(folder / "cache/onboarding.json", json.dumps({
        "onboardingComplete": True, "consumerOnboardingComplete": True,
        "enterpriseOnboardingComplete": False}).encode())


def claude_project(folder):
    """The key Claude Code's configuration names `folder` by: it names a
    Windows folder with `/` between names (docs/agent-hooks.md)."""
    return folder.as_posix() if WINDOWS else str(folder)


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
