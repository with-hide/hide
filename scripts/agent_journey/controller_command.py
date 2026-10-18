"""One `hide` command of the journey's controller agent (controller.py): the
spawn provider runs this on each `!`, with the command file as its argument,
so the command is the agent's descendant on every system. Its exit status is
the command's, and its answer is in the response file."""

import json
import os
import subprocess
import sys

with open(sys.argv[1], encoding="utf-8") as stream:
    command = json.load(stream)
with open(command["response"], "wb") as response:
    sys.exit(subprocess.run(command["argv"], env={**os.environ, **command["env"]}, stdout=response).returncode)
