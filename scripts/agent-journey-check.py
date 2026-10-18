#!/usr/bin/env python3
"""Every agent's journey through hide agent spawn on an isolated candidate. See docs/VERIFICATION.md, Agent journey."""

import locale
import sys

from agent_journey.cli import main
from agent_live_check.protection import ProtectionError

if __name__ == "__main__":
    # hide, Herdr and the agents write UTF-8, and the journey reads their files
    # in Python's default encoding, which on Windows is the ANSI code page
    # unless Python runs in UTF-8 mode.
    if locale.getpreferredencoding(False).replace("-", "").lower() != "utf8":
        print("the journey reads UTF-8: run it in Python's UTF-8 mode (python -X utf8, or PYTHONUTF8=1)",
              file=sys.stderr)
        raise SystemExit(2)
    try:
        raise SystemExit(main())
    except ProtectionError as error:
        print(str(error), file=sys.stderr)
        raise SystemExit(2)
