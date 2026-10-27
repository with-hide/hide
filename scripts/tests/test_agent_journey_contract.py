"""A new adapter needs every journey route and a valid generated declaration.

These checks read declarations and route data only; no CLI, login or model runs.
"""

import copy
import json
from pathlib import Path
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
from agent_journey import ci, clis, fake_lane, local  # noqa: E402
from agent_journey.contract import active_rows, load_contract  # noqa: E402
from agent_live_check.protection import ProtectionError  # noqa: E402


class Registration(unittest.TestCase):
    def test_every_registered_agent_has_its_local_login_and_fake_model_routes(self):
        for row in active_rows(load_contract(ROOT)):
            for name, routes in (("local isolation", local.ROUTES), ("CI login", ci.LOGINS),
                                 ("fake model", fake_lane.PROVIDERS)):
                with self.subTest(agent=row["id"], route=name):
                    self.assertIn(row["herdr_kind"], routes, f"{row['id']} has no {name} route")

    def test_the_installed_cli_set_matches_the_registered_agents(self):
        self.assertEqual(set(clis.table()), {row["herdr_kind"] for row in active_rows(load_contract(ROOT))})

    def test_every_pinned_cli_is_at_least_its_agents_minimum_version(self):
        """CI must measure a CLI the kit integrates, never one it refuses as too old."""
        table = clis.table()
        for row in active_rows(load_contract(ROOT)):
            with self.subTest(agent=row["id"]):
                minimum = tuple(int(part) for part in row["minimum_version"].split("."))
                pinned = tuple(int(part) for part in table[row["herdr_kind"]]["version"].split("."))
                self.assertGreaterEqual(pinned, minimum)


class GeneratedContract(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        (self.root / "contracts").mkdir()
        self.path = self.root / "contracts/agent-journey.json"
        self.rows = load_contract(ROOT)

    def write(self, rows):
        self.path.write_text(json.dumps(rows))

    def test_a_new_registration_is_read_without_updating_a_support_list(self):
        added = {**self.rows[0], "id": "newly-registered-agent"}
        self.write([*self.rows, added])
        self.assertIn(added, active_rows(load_contract(self.root)))

    def test_a_missing_empty_or_malformed_contract_is_refused(self):
        with self.assertRaises(ProtectionError):
            load_contract(self.root)
        for content in ("{", "[]", "{}", "[null]"):
            with self.subTest(content=content):
                self.path.write_text(content)
                with self.assertRaises(ProtectionError):
                    load_contract(self.root)

    def test_duplicate_agents_and_missing_unknown_or_non_boolean_cells_are_refused(self):
        defects = []
        defects.append([*self.rows, self.rows[0]])
        for defect in ("missing", "unknown", "string", "integer"):
            rows = copy.deepcopy(self.rows)
            declared = rows[0]["cells"]
            if defect == "missing":
                del declared["title"]
            elif defect == "unknown":
                declared["invented"] = True
            else:
                declared["title"] = "true" if defect == "string" else 1
            defects.append(rows)
        for rows in defects:
            with self.subTest(rows=rows[:1]):
                self.write(rows)
                with self.assertRaises(ProtectionError):
                    load_contract(self.root)


if __name__ == "__main__":
    unittest.main()
