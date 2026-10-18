"""Authentication menus are terminal outcomes, never bell scenes."""

import re

from .processes import ProcessError


class AuthenticationRequired(ProcessError):
    pass


_WALL = re.compile(r"\b(?:sign[ -]?in|log[ -]?in|login required|authenticate|authentication required|not logged in)\b",
                   re.IGNORECASE)
# A provider row an agent marks as signed in (omp: "● logged in (login)").
_SIGNED_IN_ROW = re.compile(r"●\s*logged in", re.IGNORECASE)
# A path or a command on the screen is never a login wall, whatever it is
# called: a prompt's folder (the signed-in lane's run folder is
# agent-journey-login-<system>), a login page's address, or a hint that names
# the login command without asking for it (Pi: "Use /login to add providers.").
_PATH = re.compile(r"\S*[/\\]\S*")


def require_no_login(screen: str) -> None:
    # A provider chooser that lists an account as signed in is a setup menu the
    # scene deadline reports as it is, not a login wall: calling it
    # unauthenticated would skip every scene of an agent that is logged in.
    if _SIGNED_IN_ROW.search(screen):
        return
    text = _PATH.sub(" ", screen)
    if _WALL.search(text):
        raise AuthenticationRequired("not_authenticated_no_login_attempted")
