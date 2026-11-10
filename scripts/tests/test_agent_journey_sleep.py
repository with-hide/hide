"""The sleep_wake step asks Hide to Sleep an agent only once the engine reads
it at rest, the state Hide's own Sleep requires (hide-core
`agent_sleep_engine::end_agent`). A Sleep it refuses is not asked again for an
hour, so a step that asked while the engine still read the turn working waited
out its whole bound for a row that could not sleep. Synthetic engine readings;
no agent runs."""

import json
from pathlib import Path
import sys
from types import SimpleNamespace
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from agent_journey import cells
from agent_journey.journey import Journey
from agent_live_check.processes import ProcessError

ROOT = Path(__file__).resolve().parents[2]


class Asked(Exception):
    pass


class Engine:
    """An engine that reads `statuses` one after another, the last one for good."""

    def __init__(self, statuses):
        self.statuses, self.events = list(statuses), []

    def agent(self, pane):
        self.events.append("read")
        return {"agent_status": self.statuses.pop(0) if len(self.statuses) > 1 else self.statuses[0]}

    def wait(self, predicate, seconds):
        for _ in range(8):
            if value := predicate():
                return value
        raise ProcessError("scene_timeout")


class Daemon:
    def __init__(self, engine):
        self.engine = engine

    def dispatch(self, kind, fields):
        self.engine.events.append(kind)
        if kind == "agent_sleep":
            raise Asked


class SleepWake(unittest.TestCase):
    def step(self, statuses):
        contract = json.loads((ROOT / "contracts/agent-journey.json").read_text())
        row = next(row for row in contract if row["id"] == "claude-code")
        engine = Engine(statuses)
        runtime = SimpleNamespace(run=Path("run"), probe=Path("run/probe"), home=Path("run/home"),
                                  agent=engine.agent, wait=engine.wait)
        table = cells.plan(contract, [row["id"]])
        cells.settle(table, row["id"], "start", cells.PASS, "started")
        journey = Journey(runtime, Daemon(engine), None, table, row,
                          Path("repository"), {}, None, branch="journey-claude-code", created=[],
                          sessions=Path("sessions"), route={}, composer_judge=None, failure_logs=None)
        journey.pane = "h1:p1"
        journey.engine_agent = lambda: runtime.agent(journey.pane)
        return journey, engine

    def test_sleep_is_asked_after_the_engine_stops_reading_the_turn_working(self):
        journey, engine = self.step(["working", "working", "idle"])
        with self.assertRaises(Asked):
            journey.sleep_wake()
        self.assertLess(engine.events.index("read"), engine.events.index("agent_sleep"))
        self.assertEqual(engine.statuses, ["idle"])

    def test_an_agent_the_engine_never_reads_at_rest_is_not_asked_to_sleep(self):
        journey, engine = self.step(["working"])
        with self.assertRaisesRegex(ProcessError, "agent_not_at_rest: engine reads working"):
            journey.sleep_wake()
        self.assertNotIn("agent_sleep", engine.events)


if __name__ == "__main__":
    unittest.main()
