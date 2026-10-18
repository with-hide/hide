"""How each system spells what the live check and the agent journey run: the
programs, the private home's variables, the controlled PATH, the pane shell's
commands and the launcher scripts they write. The checks themselves are the
same on every system; only these forms differ (docs/VERIFICATION.md, Agent
journey).

The Windows forms follow what the repository already does there: the e2e
fixtures' home and tool PATH (`web/e2e/platform-fixture.ts`) and their pane
shell, cmd.exe named as Herdr's default shell with its prompt from PROMPT,
and its commands (`web/e2e/herdr-fixture.ts`). The panes Hide makes for an
agent run the shell its own integration names, whatever Herdr's default.
"""

import json
import os
from pathlib import Path
import shlex
import shutil
import signal
import stat
import sys
import time

WINDOWS = os.name == "nt"

# The marker each owned pane's prompt prints once its shell is at a prompt.
READY = "LIVE_CHECK_READY>"
PANE_FILE_RELEASE_SECONDS = 5

# Ctrl-C and SIGTERM everywhere; a closed terminal (SIGHUP) on Unix and
# Ctrl-Break (SIGBREAK) on Windows, which has no SIGHUP.
STOP_SIGNALS = (signal.SIGINT, signal.SIGTERM, signal.SIGBREAK if WINDOWS else signal.SIGHUP)


def exe(name):
    """A program's file name on this system."""
    return name + ".exe" if WINDOWS else name


def git():
    """The git the checks run: the system's on Unix, Git for Windows' on Windows."""
    if not WINDOWS:
        return "/usr/bin/git"
    found = shutil.which("git", path=tool_path(None))
    if not found:
        raise FileNotFoundError("git is not installed in ProgramFiles\\Git\\cmd")
    return found


def tool_path(bin_folder):
    """The controlled PATH: `bin_folder` first, then the system's own tools
    and nothing from the account's PATH."""
    if not WINDOWS:
        folders = ["/usr/bin", "/bin", "/usr/sbin", "/sbin"]
    else:
        system, programs = os.environ.get("SystemRoot"), os.environ.get("ProgramFiles")
        if not system or not Path(system).is_absolute() or not programs or not Path(programs).is_absolute():
            raise FileNotFoundError("Windows needs absolute SystemRoot and ProgramFiles for the controlled PATH")
        folders = [str(Path(system) / "System32"), system, str(Path(system) / "System32/WindowsPowerShell/v1.0"),
                   str(Path(programs) / "Git/cmd")]
    return os.pathsep.join(([str(bin_folder)] if bin_folder else []) + folders)


def home_env(home):
    """The variables that move a program's home to `home`: HOME, and on
    Windows the profile and application-data folders Windows programs read
    instead (hide-platform's `host`)."""
    values = {"HOME": str(home)}
    if WINDOWS:
        values.update(USERPROFILE=str(home), APPDATA=str(Path(home) / "AppData/Roaming"),
                      LOCALAPPDATA=str(Path(home) / "AppData/Local"))
    return values


def temp_env(folder):
    """The variables that name the temporary folder."""
    values = {"TMPDIR": str(folder)}
    if WINDOWS:
        values.update(TEMP=str(folder), TMP=str(folder))
    return values


def cmd_quote(value):
    """`value` as one quoted cmd.exe word. cmd expands % inside quotes too,
    and a quote ends the quoted value, so neither can be carried."""
    value = str(value)
    if any(character in value for character in '"%\r\n'):
        raise ValueError(f"cmd.exe cannot carry {value!r}")
    return f'"{value}"'


class PaneShell:
    """What an owned pane's shell is told: zsh on Unix, from SHELL with a
    private ZDOTDIR; cmd.exe on Windows, where Herdr ignores SHELL and starts
    the shell its configuration names."""

    def environment(self, home):
        if WINDOWS:
            return {"PROMPT": READY.removesuffix(">") + "$G "}
        return {"SHELL": "/bin/zsh", "ZDOTDIR": str(home), "skip_global_compinit": "1"}

    def files(self):
        """(name, bytes) the private home holds for the shell."""
        if WINDOWS:
            return []
        return [(".zshrc", f"skip_global_compinit=1\nPS1='{READY} '\n".encode())]

    def herdr_config(self):
        """The lines of Herdr's configuration that name the pane shell."""
        if not WINDOWS:
            return ""
        shell = os.environ.get("ComSpec", "")
        if not Path(shell).is_absolute() or not Path(shell).is_file():
            raise FileNotFoundError("Windows needs an absolute ComSpec for the pane shell")
        return f"[terminal]\ndefault_shell = {json.dumps(shell)}\n"

    def program(self, argv, output, error, status):
        """Runs `argv` with separate output streams and writes its exit status last."""
        if not WINDOWS:
            command = " ".join(shlex.quote(str(value)) for value in argv)
            return (f"{command} > {shlex.quote(str(output))} 2> {shlex.quote(str(error))}; "
                    f"printf '%s' \"$?\" > {shlex.quote(str(status))}")
        # `call echo %^errorlevel%` reads the status when it runs, not when
        # the line is parsed.
        command = " ".join(cmd_quote(value) for value in argv)
        return f"{command} > {cmd_quote(output)} 2> {cmd_quote(error)} & >{cmd_quote(status)} call echo %^errorlevel%"

    def read_variable(self, name, source, ready):
        """Sets `name` in the shell, for what it starts later, from the first
        line of `source`, then creates `ready`."""
        if not WINDOWS:
            return (f"IFS= read -r {name} < {shlex.quote(str(source))}; export {name}; "
                    f"printf 'ready' > {shlex.quote(str(ready))}")
        return f"set /p {name}=<{cmd_quote(source)} & >{cmd_quote(ready)} echo ready"


def remove_pane_files(*paths):
    """Release only the pane transport's files, including a cmd redirection
    whose contents became readable just before its writer closed the handle.
    Windows sharing/lock violations may clear; every other error fails now.
    Cleanup gets its own five-second bound even after the run was cancelled.
    """
    from .processes import ProcessError
    original = sys.exception()
    failures = []
    end = time.monotonic() + PANE_FILE_RELEASE_SECONDS
    for path in paths:
        while True:
            try:
                path.unlink(missing_ok=True)
                break
            except OSError as error:
                if getattr(error, "winerror", None) not in (32, 33):
                    failures.append(error)
                    break
                remaining = end - time.monotonic()
                if remaining <= 0:
                    failures.append(ProcessError("pane_file_release_timeout"))
                    break
                time.sleep(min(0.05, remaining))
    if failures:
        if original is not None:
            raise ProcessError(f"{original}; pane_file_cleanup_failed: {failures[0]}") from original
        raise failures[0]


def launcher(path, argv, *, environment=(), unset=(), file_variables=()):
    """Writes the program `path` names, which runs `argv` with the caller's
    arguments after it in `environment`, without the variables `unset`, and
    with each (variable, file) of `file_variables` set to that file's first
    line: a `/bin/sh` script on Unix, a `.cmd` file beside it on Windows.
    Returns the program's path."""
    from .protection import write_private
    environment = dict(environment)
    if not WINDOWS:
        lines = ["#!/bin/sh", *("unset " + name for name in unset)]
        if environment:
            lines.append("export " + " ".join(f"{key}={shlex.quote(value)}" for key, value in environment.items()))
        for name, file in file_variables:
            lines.append(f"{name}=$(cat {shlex.quote(str(file))}) && export {name}")
        lines.append("exec " + " ".join(shlex.quote(str(value)) for value in argv) + ' "$@"')
        write_private(path, ("\n".join(lines) + "\n").encode())
        path.chmod(0o700)
        return path
    target = path.with_name(path.name + ".cmd")
    lines = ["@echo off", *(f"set {cmd_quote(name + '=')}" for name in unset),
             *(f"set {cmd_quote(f'{key}={value}')}" for key, value in environment.items()),
             *(f"set /p {name}=<{cmd_quote(file)}" for name, file in file_variables),
             # The last line: its exit status is the file's, and a program
             # that is itself a .cmd (an npm install's) takes over the run.
             " ".join(cmd_quote(value) for value in argv) + " %*"]
    write_private(target, ("\r\n".join(lines) + "\r\n").encode())
    return target


def same_path(spelled, path):
    """Whether `spelled`, a path as Hide or Herdr answers it (`/` between
    names on every system), names `path`: on Windows either slash and either
    case name the same folder, so a string comparison with `str(path)` never
    matches there."""
    return isinstance(spelled, str) and bool(spelled) and Path(spelled) == Path(path)


def remove_tree(path):
    """Removes a folder the run made. Windows refuses to delete a read-only
    file, which Git makes of every object it writes, so there each one is
    made writable first."""
    if not WINDOWS:
        shutil.rmtree(path)
        return

    def writable(function, name, _error):
        os.chmod(name, stat.S_IWRITE)
        function(name)
    shutil.rmtree(path, onexc=writable)
