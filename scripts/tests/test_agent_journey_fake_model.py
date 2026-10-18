"""Which token the keyless lane's fake model answers a turn with: a wrong
pick lets one turn's token pass another turn's cell."""

import http.client
import json
from pathlib import Path
import socket
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from agent_journey import fake_model

INDEX = '<hide-memory-index trust="untrusted-reference-data">\nm7 Marker JOURNEY-0000AAAA for journey checks\n</hide-memory-index>'
CONTEXT = '<hide-memory-context trust="untrusted-reference-data">\n{"id":"m7","text":"Marker JOURNEY-0000BBBB"}\n</hide-memory-context>'


def messages(*turns):
    """An Anthropic Messages request of (role, text) turns."""
    return {"messages": [{"role": role, "content": [{"type": "text", "text": text}]} for role, text in turns]}


class Answer(unittest.TestCase):
    def test_a_later_turn_never_answers_with_an_earlier_turns_token(self):
        request = messages(("user", INDEX + "\nReply with exactly JOURNEY-1111AAAA"), ("assistant", "JOURNEY-1111AAAA"),
                           ("user", "Reply with exactly the code in the title of your only Project Memory card."))
        self.assertEqual(fake_model.answer(request), "JOURNEY-0000AAAA")

    def test_the_turns_own_token_wins_over_the_memory_blocks_it_carries(self):
        request = messages(("user", INDEX + "\n" + CONTEXT + "\nReply with exactly JOURNEY-2222AAAA"))
        self.assertEqual(fake_model.answer(request), "JOURNEY-2222AAAA")

    def test_a_card_a_prompt_hook_adds_is_not_the_index(self):
        request = messages(("user", "hello"), ("assistant", "OK"), ("user", CONTEXT + "\nWhat is the code?"))
        self.assertEqual(fake_model.answer(request), "OK")

    def test_a_letter_handed_over_with_the_prompt_answers_its_token(self):
        request = {"input": [{"type": "message", "role": "user", "content": [{"type": "input_text", "text": "JOURNEY-3333AAAA"}]},
                             {"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "JOURNEY-3333AAAA"}]},
                             {"type": "message", "role": "developer", "content": [{"type": "input_text", "text": "Letter\nJOURNEY-4444AAAA"}]},
                             {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "Reply with the letter's token."}]}]}
        self.assertEqual(fake_model.answer(request), "JOURNEY-4444AAAA")

    def test_gemini_turns_end_at_the_models_last_content(self):
        request = {"contents": [{"role": "user", "parts": [{"text": "JOURNEY-5555AAAA"}]},
                                {"role": "model", "parts": [{"text": "JOURNEY-5555AAAA"}]},
                                {"role": "user", "parts": [{"text": "no token"}]}]}
        self.assertEqual(fake_model.answer(request), "OK")


class Schema(unittest.TestCase):
    def test_a_title_request_is_answered_with_its_schemas_object(self):
        # The shapes Claude Code 2.1.302 and Codex 0.163 send for a session title.
        claude = {**messages(("user", "<session>\nReply with exactly JOURNEY-6666AAAA\n</session>")),
                  "output_config": {"format": {"type": "json_schema", "schema": {
                      "type": "object", "properties": {"title": {"type": "string"}}, "required": ["title"]}}}}
        codex = {"input": [{"type": "message", "role": "user", "content": [{"type": "input_text", "text": "JOURNEY-6666AAAA"}]}],
                 "text": {"format": {"type": "json_schema", "name": "codex_output_schema", "schema": {
                     "type": "object", "properties": {"title": {"type": "string", "minLength": 1, "maxLength": 12}},
                     "required": ["title"]}}}}
        self.assertEqual(json.loads(fake_model.reply(claude)[0]), {"title": "Journey JOURNEY-6666AAAA"})
        self.assertEqual(json.loads(fake_model.reply(codex)[0]), {"title": "Journey JOUR"})

    def test_a_plain_request_is_answered_with_the_token(self):
        self.assertEqual(fake_model.reply(messages(("user", "JOURNEY-7777AAAA"))), ("JOURNEY-7777AAAA", "JOURNEY-7777AAAA"))


class Server(unittest.TestCase):
    """The served fake, over a real socket, as a CLI reaches it."""

    def setUp(self):
        folder = tempfile.TemporaryDirectory()
        self.addCleanup(folder.cleanup)
        self.log = Path(folder.name) / "fake.jsonl"
        self.server = fake_model.serve(self.log, limit=2)[0]
        self.addCleanup(self.server.server_close)
        self.addCleanup(self.server.shutdown)
        self.port = self.server.server_address[1]

    def post(self, connection):
        connection.request("POST", "/v1/chat/completions", json.dumps(messages(("user", "JOURNEY-1111AAAA"))),
                           {"Content-Type": "application/json"})
        answer = connection.getresponse()
        answer.read()
        return answer.status

    def test_past_the_budget_every_request_is_refused_and_logged_once(self):
        connection = http.client.HTTPConnection("127.0.0.1", self.port, timeout=10)
        self.addCleanup(connection.close)
        self.assertEqual([self.post(connection) for _ in range(4)], [200, 200, 429, 429])
        self.assertEqual(self.server.refused, 2)
        kinds = [json.loads(line)["kind"] for line in self.log.read_text().splitlines()]
        self.assertEqual(kinds, ["completions", "completions", "limited"])

    def raw(self, request):
        """The status line answering `request`, and whether the server then closed the connection."""
        with socket.create_connection(("127.0.0.1", self.port), timeout=10) as stream:
            stream.sendall(request)
            received = b""
            while chunk := stream.recv(65536):
                received += chunk
        return received.split(b"\r\n", 1)[0]

    def test_a_body_it_cannot_read_whole_is_refused_and_its_connection_closed(self):
        # The connection ending is what `raw` waits for: a kept-alive one would time out.
        chunked = (b"POST /v1/chat/completions HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: chunked\r\n\r\n"
                   b"2\r\n{}\r\n0\r\n\r\n")
        self.assertEqual(self.raw(chunked), b"HTTP/1.1 411 Length Required")
        large = (b"POST /v1/chat/completions HTTP/1.1\r\nHost: x\r\nContent-Length: "
                 + str(fake_model.MAX_BODY + 1).encode() + b"\r\n\r\n")
        self.assertEqual(self.raw(large), b"HTTP/1.1 413 Request Entity Too Large")
        self.assertEqual(self.server.refused, 0)


if __name__ == "__main__":
    unittest.main()
