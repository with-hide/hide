#!/usr/bin/env python3
"""The daily real-login journey on the journey account. See docs/VERIFICATION.md, The daily real-login journey."""

import sys

from agent_journey.daily import main, quiet_failures

if __name__ == "__main__":
    quiet_failures()
    sys.exit(main())
