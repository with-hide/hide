"""A new adapter needs every journey route, every place a script or a guide
names each agent, and a valid generated declaration.

These checks read declarations, route data and guides only; no CLI, login or
model runs. docs/agent-hooks.md, Adding an agent, is the checklist they hold.
"""

import copy
import json
from pathlib import Path
import re
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
from agent_journey import ci, clis, fake_lane, fake_private, local  # noqa: E402
from agent_journey.contract import active_rows, load_contract  # noqa: E402
from agent_live_check.protection import ProtectionError  # noqa: E402

CELL = re.compile(r"(?<!\\)\|")


def section(path, heading):
    """The text under `heading` in a guide, up to the next heading of the same or a higher level."""
    lines = (ROOT / path).read_text().splitlines()
    level = len(heading) - len(heading.lstrip("#"))
    start = lines.index(heading) + 1
    end = next((index for index in range(start, len(lines))
                if re.match(r"#{1,%d} " % level, lines[index])), len(lines))
    return lines[start:end]


def tables(lines):
    """Each Markdown table in `lines` as its rows of cells, the header row first and no separator row."""
    found, rows = [], []
    for line in [*lines, ""]:
        if line.startswith("|"):
            cells = [cell.strip() for cell in CELL.split(line.strip())[1:-1]]
            if not all(re.fullmatch(r":?-+:?", cell) for cell in cells):
                rows.append(cells)
        elif rows:
            found.append(rows)
            rows = []
    return found


def version(text):
    return tuple(int(part) for part in text.split("."))


class Guides:
    """The per-agent rows of docs/agent-hooks.md and docs/INSTALL.md."""

    def __init__(self):
        support, features = tables(section("docs/agent-hooks.md", "### Support table"))[:2]
        self.support = {row[0] for row in support[1:]}
        self.features = {label.strip() for cell in features[0][2:] for label in cell.split(",")}
        minimums = tables(section("docs/agent-hooks.md", "### The oldest version of each agent"))[0]
        self.minimums = {row[0]: row[1] for row in minimums[1:]}
        updates = tables(section("docs/agent-hooks.md", "### Updating an agent that is too old"))[0]
        self.updates = {row[0] for row in updates[1:]}
        systems = [row[0] for table in tables(section("docs/INSTALL.md", "### What each system supports"))
                   for row in table[1:]]
        listed = [cell for cell in systems if "each with its hook, plugin or extension and skill" in cell]
        self.agents_row = listed[0] if len(listed) == 1 else ""
        self.letters = {cell.rsplit("one row per agent: ", 1)[-1] for cell in systems
                        if cell.startswith("Letters to an agent while it works")} | set(systems)


def integration_targets():
    """The agents the pinned Herdr ships an integration for (`herdr integration install <target>`)."""
    schema = json.loads((ROOT / "contracts/herdr-api.schema.json").read_text())
    return set(schema["schemas"]["request"]["$defs"]["IntegrationTarget"]["enum"])


def watched(home):
    """Whether the local keyless journey's operator watch sees the agent's folder in the operator's HOME."""
    folder = "/".join(home)
    return (any(folder == name or folder.startswith(name + "/") for name in fake_private.WATCH)
            or any(name.startswith(folder + "/") for name in fake_private.CONFIGURATION))


class Registration(unittest.TestCase):
    def test_every_registered_agent_has_every_piece_outside_its_declaration(self):
        """One verdict per agent that names every piece it still lacks, each
        with the line of docs/agent-hooks.md, Adding an agent, that adds it."""
        labels = {row["id"]: row["label"] for row in json.loads((ROOT / "contracts/agent-adapters.json").read_text())}
        guides, pins, targets = Guides(), clis.table(), integration_targets()
        missing = {}
        for row in active_rows(load_contract(ROOT)):
            kind, label, minimum = row["herdr_kind"], labels[row["id"]], row["minimum_version"]
            pin = pins.get(kind)
            checks = [
                (kind in local.ROUTES, "a local isolation route in scripts/agent_journey/local.py (ROUTES)"),
                (kind in ci.AGENT_ENV, "a disposable CI home in scripts/agent_journey/ci.py (AGENT_ENV)"),
                (kind in fake_lane.PROVIDERS, "a fake model route in scripts/agent_journey/fake_lane.py (PROVIDERS)"),
                (pin is not None, "a CLI pin in contracts/agent-clis.json"),
                (pin is None or version(pin["version"]) >= version(minimum),
                 f"a CLI pin at or above its minimum {minimum} in contracts/agent-clis.json"),
                (row["herdr_source"].removeprefix("herdr:") in targets,
                 "a Herdr integration target the pinned Herdr lists (contracts/herdr-api.schema.json)"),
                (watched(row["home_folder"]),
                 f"its folder {'/'.join(row['home_folder'])} in the operator watch of "
                 "scripts/agent_journey/fake_private.py (WATCH or CONFIGURATION)"),
                (label in guides.support, f"a row for {label} in docs/agent-hooks.md, Support table"),
                (label in guides.features, f"a column for {label} in docs/agent-hooks.md, Support table's features"),
                (guides.minimums.get(label) == minimum,
                 f"a row for {label} at {minimum} in docs/agent-hooks.md, The oldest version of each agent"),
                (bool(row.get("update_commands")),
                 "an update route for each documented way of installing it, with its source, in "
                 "hide-agent-adapter/src/declarations.rs (updates)"),
                (label in guides.updates,
                 f"rows for {label} in docs/agent-hooks.md, Updating an agent that is too old"),
                (re.search(r"(?<!\w)" + re.escape(label) + r"(?!\w)", guides.agents_row) is not None,
                 f"{label} in the agents row of docs/INSTALL.md, What each system supports"),
                (label in guides.letters,
                 f"a row for {label} under Letters to an agent while it works in docs/INSTALL.md, "
                 "What each system supports"),
            ]
            lacking = [piece for present, piece in checks if not present]
            if lacking:
                missing[row["id"]] = lacking
        self.assertFalse(missing, "\n" + "\n".join(f"{agent} lacks:\n  - " + "\n  - ".join(pieces)
                                                       for agent, pieces in missing.items()))

    def test_the_installed_cli_set_matches_the_registered_agents(self):
        self.assertEqual(set(clis.table()), {row["herdr_kind"] for row in active_rows(load_contract(ROOT))})


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
