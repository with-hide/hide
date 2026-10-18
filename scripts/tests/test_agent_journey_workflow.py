"""The agent journey's workflows: a step that times out must leave the
report's upload, the job's last step, its time, so no job's cap is below the
sum of its steps' caps and no step runs uncapped."""

from pathlib import Path
import unittest

import yaml

ROOT = Path(__file__).resolve().parents[2]
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


if __name__ == "__main__":
    unittest.main()
