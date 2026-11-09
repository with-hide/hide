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
# Cells the C stand-in does not imitate: it draws only fixed empty composers,
# with no held turn, picker, composer editing, fork, task title or token usage. The keyless lane measures them
# once it runs the pinned CLIs on a fake model. It does answer from its
# start's Memory index, as the fake model does, so `guidance_memory` is its.
FIXTURE_UNIMITATED = {cell: "the C stand-in does not imitate it; the fake model lane measures it"
                      for cell in ("letter_busy", "letter_menu", "composer", "fork", "title", "usage",
                                   "launch_brief", "profile")}
FIXTURE_CELLS = tuple(name for name in cells.CELLS
                      if name != "start_untrusted" and name not in FIXTURE_UNIMITATED)

# What a fixture stand-in cannot show, said in its report (PRD B27).
FIXTURE_LIMITS = [
    "A stand-in answers with the token its prompt or its hook context carries, or else the one in the Memory "
    "index its start hook printed, as the fake model does; no model reads the prompt.",
    "The engine's integration runs from the hook commands the kit installed, fed the payload the stand-in writes, "
    "so a change in the real CLI's hook payload is not seen.",
    "The working and idle screens and titles and the session file format are the stand-in's copies of the CLI's.",
    "Codex's app-server, through which Hide records folder and hook trust, is the trust tests' stand-in "
    "(hide-agent-hooks/tests/fixtures/fake-codex.py), which keeps trust beside config.toml, not in it.",
    "A stand-in waits 5 s before its start hooks and its first turn, as a CLI's start takes seconds, so a start "
    "hook that runs before Hide has connected the pane, or a first turn that ends sooner than Hide learns the "
    "session, is not exercised.",
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


def agent_environment(runtime, kind, path, journey_config=None, fake_env=None):
    """What `kind` runs with: the disposable HOME, the runner's PATH behind
    the candidate's `hide`, and the stand-in's configuration or the fake
    model's environment (fake_lane.py)."""
    environment = {**home_env(runtime.home), "HIDE_STATE_DIR": str(runtime.state),
                   "PATH": os.pathsep.join((str(runtime.bin), path)), **temp_env(runtime.run / "agent-tmp"), **QUIET,
                   **{name: str(runtime.home / relative) for name, relative in AGENT_ENV[kind].items()}}
    if journey_config:
        environment["HIDE_E2E_JOURNEY"] = str(journey_config)
    environment.update(fake_env or {})
    return environment


def wrapper(runtime, program, kind, path, journey_config=None, fake_env=None, sandbox=None, folder=None,
            account_vars=()):
    """The program that runs `kind` in its environment (agent_environment),
    in `folder`, else in the candidate's bin folder, where a pane finds it as
    `kind`. Its PATH and temporary folder are set here as well as on the
    daemon, since a Windows pane takes what the daemon lacks from the
    account's registry. The contract's `account_vars`, which a start with a
    profile sets to an account under the run's HOME, keep the start's value
    (the `profile` cell)."""
    folder = folder or runtime.bin
    private_directory(folder)
    environment = agent_environment(runtime, kind, path, journey_config, fake_env)
    command = sandbox.command([str(program)]) if sandbox else [program]
    return launcher(folder / kind, command, environment=environment, unset=["CLAUDE_CONFIG_DIR"],
                    keep=account_vars, within=runtime.home)


def runs_as_installed(row, fixture_bin):
    """Whether a Windows pane runs the agent's CLI as the runner installed it
    (npm's launcher, or its executable) rather than as the journey's
    wrapper: an agent whose first input rides its start. Hide reads npm's
    launcher as the program it runs (hide-platform `programs::Start`), and
    the wrapper, a batch file Hide cannot read, would make the start typed.
    Its environment goes to the daemon instead (share_with_daemon), whose
    panes inherit it; the agents typed by declaration keep the wrapper,
    whose variables (OpenCode's XDG folders) differ from the daemon's."""
    return WINDOWS and not fixture_bin and (row.get("input") or [None])[0] == "launch_prompt"


def share_with_daemon(runtime, environment):
    """Puts an agent's environment on the daemon, whose panes inherit it.
    The daemon's own controlled PATH gives way to the agents' one PATH; any
    other name the daemon or an earlier agent holds with another value is
    refused rather than overwritten."""
    for name, value in environment.items():
        current = runtime.daemon_env.get(name)
        replaceable = name == "PATH" and current == runtime.env.get("PATH")
        if current is not None and current != value and not replaceable:
            raise ProtectionError(f"agent_environment_conflict_{name}")
        runtime.daemon_env[name] = value
    runtime.daemon_env.pop("CLAUDE_CONFIG_DIR", None)


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
