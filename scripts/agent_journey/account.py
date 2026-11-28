"""The journey account: a macOS account that exists only to run the real-login
journey every day (scripts/agent_journey/daily.py, docs/VERIFICATION.md,
The daily real-login journey).

Its agents' logins belong to the journey and to nothing else, so two of local
mode's protections of an operator's own logins change meaning there: a login
copy that an agent refreshes inside the run is written back to the account
after the run, instead of the run refusing a login near its refresh, and the
account's own agy, whose keychain sign-in needs the account's HOME, is
measured.

An account is the journey's only while it holds the marker `setup` writes and
is not an administrator: every operator account on a Mac is one, so an
operator's Mac never reads as the journey account, whatever file it holds.
"""

import grp
import hashlib
import os
import pwd
from pathlib import Path

from agent_live_check.protection import stamp, write_private

MARKER = ".hide-journey-account"


def administrator(name):
    """Whether the account `name` is in macOS's `admin` group."""
    try:
        members = grp.getgrnam("admin").gr_mem
    except KeyError:
        return False
    return name in members or pwd.getpwnam(name).pw_gid == grp.getgrnam("admin").gr_gid


def dedicated(home=None):
    """Whether this process runs as the journey account."""
    home = Path(home or Path.home())
    name = pwd.getpwuid(os.getuid()).pw_name
    return (home / MARKER).is_file() and not administrator(name)


# The members SQLite keeps beside a database file.
DATABASE_SIDES = ("-wal", "-shm")


def members(source):
    """The account's files a copy of `source` stands for."""
    return [source, *(source.with_name(source.name + side) for side in DATABASE_SIDES)] \
        if source.suffix == ".db" else [source]


class WriteBack:
    """A login copy that goes back to the account after the run: the
    account's files as they were when the copy was made, to refuse writing
    over a login someone changed meanwhile."""

    def __init__(self, source, private):
        self.source, self.private = source, private
        self.before = [stamp(path) for path in members(source)]

    def apply(self, content):
        """What happened to this login: `unchanged`, `written back`, or why not."""
        if content is None:
            return "not written back: the agent removed its login copy"
        if [stamp(path) for path in members(self.source)] != self.before:
            return "not written back: the account's login changed during the run"
        if self.before[0] is not None and self.before[0].digest == hashlib.sha256(content).hexdigest() \
                and len(self.before) == 1:
            return "unchanged"
        replacement = self.source.with_name(self.source.name + ".journey")
        replacement.unlink(missing_ok=True)
        write_private(replacement, content)
        os.replace(replacement, self.source)
        # The database's main file now holds every change (runtime.kept_login).
        for side in members(self.source)[1:]:
            side.unlink(missing_ok=True)
        return "written back"


def write_back(entries, kept):
    """Each login copy's outcome by the account file's name, after the run's
    cleanup ended every process and its configuration check read the
    account's files."""
    return {entry.source.name: entry.apply(kept.get(entry.private)) for entry in entries}
