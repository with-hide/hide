"""A stand-in model provider for the keyless lane: the real CLIs, with Hide's
real hooks, plugins and extensions, talk to it instead of a provider.

It speaks the four APIs the supported CLIs call (OpenAI Chat Completions and
Responses, Anthropic Messages, Gemini generateContent), streamed or whole, and
answers every request with the last journey token of the turn it answers,
everything the request sends after the model's own last turn: the prompt's
token, or the token of a letter a hook handed over with it. A token inside
Hide's Project Memory blocks never counts there, so a turn that carries none
is answered with the token of the Project Memory index the session started
with, and a request with neither is answered `OK`; any token from the whole
conversation would let an earlier turn's token pass a later turn's cell.
A request that asks for JSON of a schema, as a CLI's title request does, is
answered with an object of that schema whose strings name the token.
Each answer carries usage in the provider's own shape, so the usage and
title readers have something to read. It never calls a tool and never
reaches the network; it proves nothing about a real provider.

It answers at most `limit` model requests, the agent's request budget, and
at most twice that of every request; past either it answers 429, counts the
refusal in `refused` and logs only the first, so a CLI that overruns fails
its run's budget at once instead of filling the log until the run's cap. A
body it cannot read whole, chunked or past MAX_BODY, is refused and its
connection closed, so no unread byte becomes the next request.

Run as a program it serves on 127.0.0.1, writes its port to `--port-file` and
logs each request (path, model, whether it was streamed, the token it
answered) to `--log` as JSON lines.
"""

import argparse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
from pathlib import Path
import re
import sys
import threading
import time
import uuid

TOKEN = re.compile(r"JOURNEY-[0-9A-F]{8}")
# Hide's Project Memory blocks (hide-agent-hooks guidance): the index a
# session starts with and the cards a prompt hook adds to a turn.
MEMORY_BLOCK = re.compile(r"<hide-memory-(index|context)\b.*?</hide-memory-\1>", re.S)
MEMORY_INDEX = re.compile(r"<hide-memory-index\b.*?</hide-memory-index>", re.S)
MODEL = "journey-model"
# A request body past this is refused: a journey turn sends tens of KiB.
MAX_BODY = 8 * 1024 * 1024
# Roles whose text is the model's own; a token there was answered already.
MODEL_ROLES = ("assistant", "model")


def strings(value):
    """Every string in `value`, in document order, leaving out the model's own turns."""
    if isinstance(value, str):
        yield value
    elif isinstance(value, list):
        for item in value:
            yield from strings(item)
    elif isinstance(value, dict):
        if value.get("role") in MODEL_ROLES or value.get("type") in ("reasoning",):
            return
        for item in value.values():
            yield from strings(item)


def conversation(request):
    """The request's turns, in the field each API keeps them in."""
    for key in ("messages", "input", "contents"):
        items = request.get(key)
        if isinstance(items, list):
            return items
        if isinstance(items, str):
            return [items]
    return [request]


def latest(request):
    """What the request sends after the model's own last turn, with the
    instructions an API keeps apart when the model has not spoken yet."""
    items = conversation(request)
    spoken = [index for index, item in enumerate(items)
              if isinstance(item, dict) and (item.get("role") in MODEL_ROLES or item.get("type") == "reasoning")]
    if spoken:
        return items[spoken[-1] + 1:]
    return [request]


def answer(request):
    """The token the model answers `request` with, or `OK`."""
    turn = "\n".join(strings(latest(request)))
    found = TOKEN.findall(MEMORY_BLOCK.sub("", turn))
    if found:
        return found[-1]
    indexed = [token for text in strings(request) for block in MEMORY_INDEX.findall(text)
               for token in TOKEN.findall(block)]
    return indexed[-1] if indexed else "OK"


def schema(request):
    """The JSON schema the request asks its answer to follow, or None."""
    formats = [(request.get("output_config") or {}).get("format"), request.get("output_format"),
               (request.get("text") or {}).get("format"), request.get("response_format")]
    for form in formats:
        if isinstance(form, dict) and form.get("type") == "json_schema":
            return form.get("schema") or (form.get("json_schema") or {}).get("schema") or {}
    generation = request.get("generationConfig") or {}
    return generation.get("responseJsonSchema") or generation.get("responseSchema")


def instance(shape, text):
    """A value of the JSON schema `shape`, each string `text` cut to fit."""
    kind = shape.get("type", "object")
    if isinstance(kind, list):
        kind = next((name for name in kind if name != "null"), "string")
    # Gemini spells the types in capitals.
    kind = str(kind).lower()
    if kind == "object":
        properties = shape.get("properties") or {}
        required = shape.get("required") or list(properties)
        return {name: instance(properties[name], text) for name in required if name in properties}
    if kind == "array":
        return [instance(shape["items"], text)] if shape.get("minItems") and shape.get("items") else []
    if kind == "string":
        if shape.get("enum"):
            return shape["enum"][0]
        return text[:shape.get("maxLength") or len(text)]
    if kind == "boolean":
        return False
    return shape.get("minimum", 0)


def reply(request):
    """(the text the model answers with, the token it names)."""
    token = answer(request)
    shape = schema(request)
    if shape is None:
        return token, token
    return json.dumps(instance(shape, "Journey " + token if token != "OK" else "Journey check")), token


def usage(request, text):
    """Rough token counts: a token is about four bytes."""
    prompt = max(1, len(json.dumps(request)) // 4)
    return prompt, max(1, len(text) // 4)


class Refused(Exception):
    """A request body the server will not read, with the status it answers."""

    def __init__(self, status, reason):
        super().__init__(reason)
        self.status, self.reason = status, reason


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    server_version = "journey-fake-model"

    def log_message(self, *_):
        pass

    def record(self, **fields):
        log = self.server.log
        if log:
            with self.server.lock, open(log, "a", encoding="utf-8") as stream:
                stream.write(json.dumps({"at": int(time.time() * 1000), "method": self.command,
                                         "path": self.path, **fields}) + "\n")

    def body(self):
        """The request's JSON; a body it cannot read whole raises with the
        status to answer, and the connection is closed after that answer."""
        if self.headers.get("Transfer-Encoding"):
            self.close_connection = True
            raise Refused(411, "request_body_chunked")
        try:
            size = int(self.headers.get("Content-Length") or 0)
        except ValueError:
            self.close_connection = True
            raise Refused(400, "request_length_unreadable") from None
        if size < 0 or size > MAX_BODY:
            self.close_connection = True
            raise Refused(413, "request_too_large")
        data = self.rfile.read(size) if size else b""
        try:
            return json.loads(data) if data else {}
        except ValueError:
            raise Refused(400, "request_body_not_json") from None

    def admitted(self, generation):
        """Whether the request is within the server's caps; a refused one is
        answered 429 here, and only the first refusal is logged."""
        server = self.server
        with server.counts:
            server.requests += 1
            server.generations += generation
            refused = server.limit is not None and (server.generations > server.limit
                                                    or server.requests > 2 * server.limit)
            server.refused += refused
            first = refused and server.refused == 1
        if first:
            self.record(kind="limited", generation=generation, limit=server.limit)
        if refused:
            self.send(429, {"error": {"message": "past the agent's request budget on the journey's fake model",
                                      "type": "rate_limit_error"}})
        return not refused

    def send(self, status, payload, content_type="application/json"):
        data = payload if isinstance(payload, bytes) else json.dumps(payload).encode()
        self.send_response(status)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def stream(self, events):
        """Server-sent events: each item is (event name or None, JSON payload) or a raw line."""
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.send_header("Cache-Control", "no-cache")
        self.send_header("Connection", "close")
        self.end_headers()
        for item in events:
            if isinstance(item, str):
                chunk = item + "\n\n"
            else:
                name, payload = item
                chunk = (f"event: {name}\n" if name else "") + "data: " + json.dumps(payload) + "\n\n"
            self.wfile.write(chunk.encode())
            self.wfile.flush()
        self.close_connection = True

    def do_GET(self):
        if not self.admitted(False):
            return
        path = self.path.split("?", 1)[0].rstrip("/")
        if path.endswith("/models"):
            self.record(kind="models")
            if "/v1beta" in path:
                self.send(200, {"models": [{"name": "models/" + MODEL, "displayName": MODEL,
                                            "supportedGenerationMethods": ["generateContent", "streamGenerateContent"]}]})
            else:
                self.send(200, {"object": "list", "data": [{"id": MODEL, "object": "model", "created": 0,
                                                            "owned_by": "journey"}]})
            return
        self.record(kind="unknown")
        self.send(404, {"error": {"message": "not served by the journey's fake model", "type": "not_found"}})

    def do_POST(self):
        try:
            request = self.body()
        except Refused as error:
            self.record(kind="refused", error=error.reason)
            self.send(error.status, {"error": {"message": error.reason, "type": "invalid_request_error"}})
            return
        path = self.path.split("?", 1)[0]
        generation = path.endswith(("/chat/completions", "/responses", "/messages")) or ":generateContent" in path \
            or ":streamGenerateContent" in path
        if generation and self.server.api_key is not None:
            key = self.server.api_key
            authenticated = (self.headers.get("Authorization") == "Bearer " + key
                             or self.headers.get("x-api-key") == key
                             or self.headers.get("x-goog-api-key") == key)
            if not authenticated:
                with self.server.counts:
                    self.server.auth_failures += 1
                self.record(kind="auth_refused", authenticated=False)
                self.send(401, {"error": {"message": "fake_api_key_required"}})
                return
            with self.server.counts:
                self.server.authenticated += 1
        if not self.admitted(generation):
            return
        text, token = reply(request)
        streamed = bool(request.get("stream")) or "streamGenerateContent" in path or "alt=sse" in self.path
        self.record(kind=path.rsplit("/", 1)[-1], model=request.get("model"), stream=streamed, generation=generation,
                    answered=token if token != "OK" else None, schema=text != token)
        if path.endswith("/chat/completions"):
            self.chat(request, text, streamed)
        elif path.endswith("/responses"):
            self.openai_responses(request, text, streamed)
        elif path.endswith("/messages/count_tokens"):
            self.send(200, {"input_tokens": usage(request, text)[0]})
        elif path.endswith("/messages"):
            self.messages(request, text, streamed)
        elif ":generateContent" in path or ":streamGenerateContent" in path:
            self.gemini(request, text, streamed)
        else:
            self.send(404, {"error": {"message": "not served by the journey's fake model", "type": "not_found"}})

    def chat(self, request, text, streamed):
        prompt, completion = usage(request, text)
        counts = {"prompt_tokens": prompt, "completion_tokens": completion, "total_tokens": prompt + completion}
        base = {"id": "chatcmpl-" + uuid.uuid4().hex, "created": int(time.time()), "model": request.get("model") or MODEL}
        if not streamed:
            self.send(200, {**base, "object": "chat.completion", "usage": counts, "choices": [
                {"index": 0, "finish_reason": "stop", "message": {"role": "assistant", "content": text}}]})
            return
        chunk = {**base, "object": "chat.completion.chunk"}
        self.stream([(None, {**chunk, "choices": [{"index": 0, "delta": {"role": "assistant", "content": text},
                                                   "finish_reason": None}]}),
                     (None, {**chunk, "choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}]}),
                     (None, {**chunk, "choices": [], "usage": counts}),
                     "data: [DONE]"])

    def openai_responses(self, request, text, streamed):
        prompt, completion = usage(request, text)
        message_id, response_id = "msg_" + uuid.uuid4().hex, "resp_" + uuid.uuid4().hex
        part = {"type": "output_text", "text": text, "annotations": []}
        item = {"type": "message", "id": message_id, "status": "completed", "role": "assistant", "content": [part]}
        response = {"id": response_id, "object": "response", "created_at": int(time.time()), "status": "completed",
                    "model": request.get("model") or MODEL, "output": [item],
                    "usage": {"input_tokens": prompt, "input_tokens_details": {"cached_tokens": 0},
                              "output_tokens": completion, "output_tokens_details": {"reasoning_tokens": 0},
                              "total_tokens": prompt + completion}}
        if not streamed:
            self.send(200, response)
            return
        pending = {**response, "status": "in_progress", "output": [], "usage": None}
        self.stream([
            ("response.created", {"type": "response.created", "sequence_number": 0, "response": pending}),
            ("response.output_item.added", {"type": "response.output_item.added", "sequence_number": 1, "output_index": 0,
                                            "item": {**item, "status": "in_progress", "content": []}}),
            ("response.content_part.added", {"type": "response.content_part.added", "sequence_number": 2,
                                             "item_id": message_id, "output_index": 0, "content_index": 0,
                                             "part": {**part, "text": ""}}),
            ("response.output_text.delta", {"type": "response.output_text.delta", "sequence_number": 3,
                                            "item_id": message_id, "output_index": 0, "content_index": 0, "delta": text}),
            ("response.output_text.done", {"type": "response.output_text.done", "sequence_number": 4,
                                           "item_id": message_id, "output_index": 0, "content_index": 0, "text": text}),
            ("response.content_part.done", {"type": "response.content_part.done", "sequence_number": 5,
                                            "item_id": message_id, "output_index": 0, "content_index": 0, "part": part}),
            ("response.output_item.done", {"type": "response.output_item.done", "sequence_number": 6, "output_index": 0,
                                           "item": item}),
            ("response.completed", {"type": "response.completed", "sequence_number": 7, "response": response}),
        ])

    def messages(self, request, text, streamed):
        prompt, completion = usage(request, text)
        message = {"id": "msg_" + uuid.uuid4().hex, "type": "message", "role": "assistant",
                   "model": request.get("model") or MODEL, "stop_reason": "end_turn", "stop_sequence": None,
                   "content": [{"type": "text", "text": text}],
                   "usage": {"input_tokens": prompt, "output_tokens": completion,
                             "cache_creation_input_tokens": 0, "cache_read_input_tokens": 0}}
        if not streamed:
            self.send(200, message)
            return
        self.stream([
            ("message_start", {"type": "message_start", "message": {
                **message, "content": [], "stop_reason": None, "usage": {**message["usage"], "output_tokens": 1}}}),
            ("content_block_start", {"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}}),
            ("content_block_delta", {"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": text}}),
            ("content_block_stop", {"type": "content_block_stop", "index": 0}),
            ("message_delta", {"type": "message_delta", "delta": {"stop_reason": "end_turn", "stop_sequence": None},
                               "usage": {"output_tokens": completion}}),
            ("message_stop", {"type": "message_stop"}),
        ])

    def gemini(self, request, text, streamed):
        prompt, completion = usage(request, text)
        reply = {"candidates": [{"index": 0, "finishReason": "STOP",
                                 "content": {"role": "model", "parts": [{"text": text}]}}],
                 "usageMetadata": {"promptTokenCount": prompt, "candidatesTokenCount": completion,
                                   "totalTokenCount": prompt + completion},
                 "modelVersion": MODEL, "responseId": uuid.uuid4().hex}
        if streamed:
            self.stream([(None, reply)])
        else:
            self.send(200, reply)


def serve(log=None, limit=None, api_key=None):
    """A started server on 127.0.0.1 and its thread; `server.server_address[1]`
    is the port, and `limit` the most model requests it answers."""
    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    server.daemon_threads = True
    server.log, server.lock = log, threading.Lock()
    server.limit, server.counts = limit, threading.Lock()
    server.requests = server.generations = server.refused = 0
    server.api_key, server.authenticated, server.auth_failures = api_key, 0, 0
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    return server, thread


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--port-file", type=Path, required=True)
    parser.add_argument("--log", type=Path)
    args = parser.parse_args(argv)
    server, thread = serve(args.log)
    args.port_file.write_text(str(server.server_address[1]))
    try:
        thread.join()
    except KeyboardInterrupt:
        server.shutdown()
    return 0


if __name__ == "__main__":
    sys.exit(main())
