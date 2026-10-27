"""Request counts from synthetic session records; no agent runs."""

import json
from pathlib import Path
import sqlite3
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from agent_journey import opencode_service
from agent_journey.requests import model_requests


FOLDER = tempfile.TemporaryDirectory()


def tearDownModule():
    FOLDER.cleanup()


def session(*records):
    path = Path(FOLDER.name) / f"{len(list(Path(FOLDER.name).iterdir()))}.jsonl"
    path.write_text("".join((record if isinstance(record, str) else json.dumps(record)) + "\n" for record in records))
    return path


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

    def test_grok_pi_omp_and_agy_count_their_own_response_records(self):
        grok = session({"params": {"update": {"sessionUpdate": "turn_completed", "prompt_id": "p1", "usage": {"modelCalls": 2}}}},
                       {"params": {"update": {"sessionUpdate": "turn_completed", "prompt_id": "p1", "usage": {"modelCalls": 2}}}},
                       {"params": {"update": {"sessionUpdate": "turn_completed", "prompt_id": "p2", "usage": {"modelCalls": 1}}}},
                       {"params": {"update": {"sessionUpdate": "agent_message_chunk"}}})
        self.assertEqual(model_requests("grok", [grok]), 3)
        pi = session({"type": "message", "message": {"role": "assistant"}}, {"type": "message", "message": {"role": "user"}},
                     {"type": "message", "message": {"role": "assistant"}}, {"type": "session"})
        self.assertEqual(model_requests("pi", [pi]), 2)
        self.assertEqual(model_requests("omp", [pi]), 2)
        agy = session({"type": "USER_INPUT"}, {"type": "PLANNER_RESPONSE"}, {"type": "SYSTEM_MESSAGE"}, {"type": "PLANNER_RESPONSE"})
        self.assertEqual(model_requests("agy", [agy]), 2)

    def test_opencode_counts_the_assistant_messages_of_its_own_session(self):
        with tempfile.TemporaryDirectory() as home:
            database = Path(home) / opencode_service.DATA / "opencode.db"
            database.parent.mkdir(parents=True)
            with sqlite3.connect(database) as connection:
                connection.execute("CREATE TABLE session_message (id TEXT, session_id TEXT, type TEXT)")
                connection.executemany("INSERT INTO session_message VALUES (?, ?, ?)", [
                    ("m1", "ses_a", "user"), ("m2", "ses_a", "assistant"), ("m3", "ses_a", "assistant"),
                    ("m4", "ses_a", "idle"), ("m5", "ses_b", "assistant")])
            connection.close()
            self.assertEqual(model_requests("opencode", ["opencode/ses_a"], home), 2)
            self.assertEqual(model_requests("opencode", ["opencode/ses_a", "opencode/ses_b"], home), 3)

    def test_codex_counts_token_count_events(self):
        path = session(*([{"type": "event_msg", "payload": {"type": "token_count"}}] * 3),
                       {"type": "event_msg", "payload": {"type": "agent_message"}})
        self.assertEqual(model_requests("codex", [path, path]), 6)


if __name__ == "__main__":
    unittest.main()
