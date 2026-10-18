"""The agent journey's workflows: a step that times out must leave the
report's upload, the job's last step, its time, so no job's cap is below the
sum of its steps' caps and no step runs uncapped; and the journey must end
itself before its step would, so the report and the cleanup still happen."""

from pathlib import Path
import re
import sys
import unittest

import yaml

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
from agent_journey import cells, ci, cli  # noqa: E402
from agent_live_check.processes import TEARDOWN_SECONDS  # noqa: E402
WORKFLOWS = (".github/workflows/agent-journey.yml", ".github/workflows/agent-clis.yml")


class StepCaps(unittest.TestCase):
    def test_every_step_is_capped_within_its_jobs_cap(self):
        for workflow in WORKFLOWS:
            for name, job in yaml.safe_load((ROOT / workflow).read_text())["jobs"].items():
                with self.subTest(workflow=workflow, job=name):
                    steps = job["steps"]
                    self.assertEqual([step.get("name") or step.get("uses") for step in steps
                                      if "timeout-minutes" not in step], [])
                    self.assertGreaterEqual(job["timeout-minutes"], sum(step["timeout-minutes"] for step in steps))


class JourneyCap(unittest.TestCase):
    def test_the_journey_ends_itself_inside_its_step(self):
        jobs = yaml.safe_load((ROOT / WORKFLOWS[0]).read_text())["jobs"]
        steps = [step for job in jobs.values() for step in job["steps"]
                 if "agent-journey-check.py" in step.get("run", "")]
        self.assertEqual(len(steps), 3)
        for step in steps:
            with self.subTest(step=step["name"]):
                named = re.search(r"--agents (\S+)", step["run"])
                fixture = "--fixture-bin" in step["run"]
                # With no --agents, a run measures every built agent, the stand-ins only theirs.
                agents = (len(named.group(1).split(",")) if named else
                          len(ci.FIXTURE_AGENTS) if fixture else len(cells.BUILT_AGENTS))
                keyless = fixture or "--fake-model" in step["run"]
                # The OpenCode stop's teardown window, then the runtime close's.
                needed = cli.run_seconds(keyless, agents) + 2 * TEARDOWN_SECONDS
                self.assertLessEqual(needed, step["timeout-minutes"] * 60)


if __name__ == "__main__":
    unittest.main()
