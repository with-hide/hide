#!/usr/bin/env python3
"""Every agent's journey through hide agent spawn on an isolated candidate. See docs/VERIFICATION.md, Agent journey."""

import sys

from agent_journey.cli import main
from agent_live_check.protection import ProtectionError

if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except ProtectionError as error:
        print(str(error), file=sys.stderr)
        raise SystemExit(2)
