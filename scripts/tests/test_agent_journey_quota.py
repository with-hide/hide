"""The provider quota contract and request counts from synthetic session records; no agent runs."""

import json
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from agent_journey import quota
from agent_journey.quota import model_requests, spent_quota


FOLDER = tempfile.TemporaryDirectory()


def tearDownModule():
    FOLDER.cleanup()


def session(*records):
    path = Path(FOLDER.name) / f"{len(list(Path(FOLDER.name).iterdir()))}.jsonl"
    path.write_text("".join((record if isinstance(record, str) else json.dumps(record)) + "\n" for record in records))
    return path


ROW = {"agents": ["antigravity"], "error_code": 429, "status": "RESOURCE_EXHAUSTED",
       "spent_quota_ids": ["GenerateRequestsPerDayPerProjectPerModel-FreeTier"]}


def contract_file(value):
    path = Path(FOLDER.name) / f"contract-{len(list(Path(FOLDER.name).iterdir()))}.json"
    path.write_text(value if isinstance(value, str) else json.dumps(value))
    return path


class Quota(unittest.TestCase):
    def test_a_missing_or_misshapen_contract_stops_the_run(self):
        with self.assertRaisesRegex(ValueError, "unreadable"):
            quota.contract(Path(FOLDER.name) / "absent.json")
        with self.assertRaisesRegex(ValueError, "unreadable"):
            quota.contract(contract_file("{"))
        for broken in ({}, {"gemini": {**ROW, "spent_quota_ids": []}}, {"gemini": {**ROW, "error_code": "429"}},
                       {"gemini": {key: value for key, value in ROW.items() if key != "agents"}}):
            with self.assertRaisesRegex(ValueError, "malformed"):
                quota.contract(contract_file(broken))

    def test_every_refusal_of_an_agent_without_a_row_is_a_failure(self):
        rows = quota.contract(contract_file({"gemini": ROW}))
        # A per-minute limit and a spent plan in Claude's and Codex's own records.
        limited = session({"type": "assistant", "isApiErrorMessage": True, "error": "rate_limit"},
                          {"type": "event_msg", "payload": {"type": "error", "codex_error_info": "usage_limit_exceeded"}})
        self.assertIsNone(spent_quota("claude-code", [limited], rows))
        self.assertIsNone(spent_quota("codex", [limited], rows))

    def test_a_row_the_journey_cannot_read_is_an_error_not_a_pass(self):
        rows = quota.contract(contract_file({"anthropic": {**ROW, "agents": ["claude-code"]}}))
        with self.assertRaisesRegex(ValueError, "reader_missing"):
            spent_quota("claude-code", [session({"type": "x"})], rows)

    def test_the_repository_contract_has_the_shape_the_journey_reads(self):
        self.assertTrue(quota.contract())


class Requests(unittest.TestCase):
    def test_claude_counts_responses_by_message_id(self):
        path = session({"type": "assistant", "message": {"id": "a"}}, {"type": "assistant", "message": {"id": "a"}},
                       {"type": "assistant", "message": {"id": "b"}}, {"type": "user", "message": {"id": "c"}},
                       {"type": "assistant", "isApiErrorMessage": True, "message": {"id": "d"}})
        self.assertEqual(model_requests("claude", [path]), 2)

    def test_a_claude_fork_counts_the_responses_it_copied_once(self):
        source = session({"type": "assistant", "message": {"id": "a"}}, {"type": "assistant", "message": {"id": "b"}})
        fork = session({"type": "assistant", "message": {"id": "a"}}, {"type": "assistant", "message": {"id": "b"}},
                       {"type": "assistant", "message": {"id": "c"}})
        self.assertEqual(model_requests("claude", [source, fork]), 3)

    def test_codex_counts_token_count_events(self):
        path = session(*([{"type": "event_msg", "payload": {"type": "token_count"}}] * 3),
                       {"type": "event_msg", "payload": {"type": "agent_message"}})
        self.assertEqual(model_requests("codex", [path, path]), 6)


if __name__ == "__main__":
    unittest.main()
