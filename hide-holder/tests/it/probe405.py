"""TEMPORARY (with-hide/hide-private#405 measurement, removed before merge).

Runs in a holder's pane on Windows. Asks OSC 11 ROUNDS times, half of them
right after 64 KiB of output, and reads the console's input records the way
a Go TUI's reader does (ultraviolet: wait, read every record there is, again):
for each answer it records each read's time since the question and the
characters its key-down records carry, so a reader that sees the ESC apart
from the rest, and how far apart, shows. Writes JSON to argv[1].
"""
import ctypes
import json
import sys
import time
from ctypes import wintypes

ROUNDS = 120
STD_INPUT, STD_OUTPUT = -10, -11
ENABLE_VIRTUAL_TERMINAL_INPUT = 0x200
ENABLE_PROCESSED_OUTPUT, ENABLE_VIRTUAL_TERMINAL_PROCESSING = 0x1, 0x4
KEY_EVENT = 0x1


class KEY_EVENT_RECORD(ctypes.Structure):
    _fields_ = [("bKeyDown", wintypes.BOOL), ("wRepeatCount", wintypes.WORD),
                ("wVirtualKeyCode", wintypes.WORD), ("wVirtualScanCode", wintypes.WORD),
                ("uChar", wintypes.WCHAR), ("dwControlKeyState", wintypes.DWORD)]


class EVENT(ctypes.Union):
    _fields_ = [("KeyEvent", KEY_EVENT_RECORD), ("pad", ctypes.c_byte * 16)]


class INPUT_RECORD(ctypes.Structure):
    _fields_ = [("EventType", wintypes.WORD), ("Event", EVENT)]


k32 = ctypes.WinDLL("kernel32", use_last_error=True)
hin, hout = k32.GetStdHandle(STD_INPUT), k32.GetStdHandle(STD_OUTPUT)
k32.SetConsoleMode(hin, ENABLE_VIRTUAL_TERMINAL_INPUT)
k32.SetConsoleMode(hout, ENABLE_PROCESSED_OUTPUT | ENABLE_VIRTUAL_TERMINAL_PROCESSING)


def write(text):
    data = text.encode()
    written = wintypes.DWORD()
    k32.WriteFile(hout, data, len(data), ctypes.byref(written), None)


def read_answer(asked):
    """Reads until the answer's terminator or 2 s; each read: (ms, chars, vks)."""
    reads, text = [], ""
    buffer = (INPUT_RECORD * 256)()
    while True:
        left = 2.0 - (time.perf_counter() - asked)
        if left <= 0:
            return reads, "timeout"
        if k32.WaitForSingleObject(hin, int(left * 1000)) != 0:
            continue
        count = wintypes.DWORD()
        k32.ReadConsoleInputW(hin, buffer, 256, ctypes.byref(count))
        at = (time.perf_counter() - asked) * 1000
        chars, vks = "", []
        for record in buffer[:count.value]:
            if record.EventType == KEY_EVENT and record.Event.KeyEvent.bKeyDown:
                chars += record.Event.KeyEvent.uChar
                vks.append(record.Event.KeyEvent.wVirtualKeyCode)
        reads.append({"ms": round(at, 2), "records": count.value, "chars": chars, "vks": vks})
        text += chars
        if text.endswith("\x07") or text.endswith("\x1b\\"):
            return reads, "whole"


rounds = []
for index in range(ROUNDS):
    if index % 2:
        write(("x" * 127 + "\r\n") * 512)
    write("\x1b[H\x1b[2J")
    asked = time.perf_counter()
    write("\x1b]11;?\x07")
    reads, end = read_answer(asked)
    first = reads[0]["chars"] if reads else ""
    rounds.append({"round": index, "after_output": bool(index % 2), "end": end, "reads": reads,
                   "split": len(reads) > 1, "esc_alone_first": first == "\x1b",
                   "gap_ms": round(reads[-1]["ms"] - reads[0]["ms"], 2) if reads else None})
summary = {
    "rounds": ROUNDS,
    "whole_in_one_read": sum(1 for r in rounds if r["end"] == "whole" and not r["split"]),
    "split": sum(1 for r in rounds if r["split"]),
    "esc_alone_first": sum(1 for r in rounds if r["esc_alone_first"]),
    "split_over_50ms": sum(1 for r in rounds if r["split"] and r["gap_ms"] > 50),
    "timeout": sum(1 for r in rounds if r["end"] == "timeout"),
    "first_read_ms_max": max((r["reads"][0]["ms"] for r in rounds if r["reads"]), default=None),
}
with open(sys.argv[1], "w", encoding="utf-8") as out:
    json.dump({"summary": summary, "rounds": rounds}, out)
write("probe-405-done\r\n")
time.sleep(600)
