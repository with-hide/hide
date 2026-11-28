#!/usr/bin/env python3
"""The daily real-login journey on the journey account. See docs/VERIFICATION.md, The daily real-login journey."""

import sys
import threading


def report(kind):
    """An uncaught failure, in the runner or one of its threads, names only
    its kind: launchd keeps this stderr in the account's log, and a message
    or a traceback can carry a path, an account or what a command printed."""
    sys.stderr.write(f"agent-journey-daily: failed: {kind.__name__}\n")
    sys.stderr.flush()


# Before the runner is imported, so a failure while importing it is covered too.
sys.excepthook = lambda kind, value, trace: report(kind)
threading.excepthook = lambda failure: report(failure.exc_type)

if __name__ == "__main__":
    from agent_journey.daily import main

    sys.exit(main())
