"""Test oracle: runs Scripts/sillclient.py's own message builders without running the script.

Usage: python3 -I sillclient_oracle.py PATH/TO/sillclient.py
Parses the script with `ast` (never imports or executes its top level: it connects to hosts),
extracts the named functions and constants and evaluates only those, with a fixed clock.
Prints one JSON object: {"built": {name: hex}, "parsed": {name: {...}}}.
Never opens a socket.
"""
import ast, json, struct, sys

path = sys.argv[1]
src = open(path, encoding="utf-8").read()
tree = ast.parse(src)
WANT_FUNCS = {"msg", "source", "number", "gesture", "pointer_input", "key_input", "viewport"}
WANT_ASSIGNS = {"GESTURES"}
chunks = []
for node in tree.body:
    if isinstance(node, ast.FunctionDef) and node.name in WANT_FUNCS:
        chunks.append(ast.get_source_segment(src, node))
    elif isinstance(node, ast.Assign) and any(isinstance(t, ast.Name) and t.id in WANT_ASSIGNS for t in node.targets):
        chunks.append(ast.get_source_segment(src, node))

class FakeTime:
    @staticmethod
    def time():
        return 1700000000.5

ns = {"json": json, "struct": struct, "time": FakeTime}
exec("\n".join(chunks), ns)
missing = (WANT_FUNCS | WANT_ASSIGNS) - set(ns)
if missing:
    sys.exit(f"sillclient.py no longer defines {sorted(missing)}")

msg = ns["msg"]
built = {
    "ping8": msg(10, struct.pack(">d", 12.25)),
    "empty_unknown_201": msg(201),
    "unknown_200_hello": msg(200, b"hello"),
    "keyframe_flag": msg(1, b"\x00\x01\x02", key=True),
    "pick_desktop": msg(6, json.dumps(ns["source"]("desktop")).encode()),
    "pick_none": msg(6, json.dumps(ns["source"]("none")).encode()),
    "pick_window": msg(6, json.dumps(ns["source"]("window:123")).encode()),
    "pointer_move": ns["pointer_input"]("move", 0.5, 0.25),
    "pointer_left_down": ns["pointer_input"]("leftDown", 0.5, 0.25),
    "key_down": ns["key_input"](44, True),
    "key_up": ns["key_input"](44, False),
    "viewport_60": ns["viewport"](60),
    "gesture_swipe_up": msg(28, json.dumps(ns["gesture"]("swipeUp")).encode()),
    "gesture_pinch_4": msg(28, json.dumps(ns["gesture"]("pinch,4")).encode()),
    "hello": msg(23, json.dumps({"appVersion": "1.0", "build": "1", "protocol": 1, "device": "Windows PC", "audio": ["aac-eld"]}).encode()),
    "stats": msg(12, json.dumps({"fps": 0, "frameAgeMs": 0, "rttMs": 0, "device": "Windows PC"}).encode()),
}

def parse(raw):
    kind, ts, key, ln = struct.unpack(">BdBI", raw[:14])
    payload = raw[14:]
    assert len(payload) == ln
    out = {"kind": kind, "timestamp": ts, "key": key, "len": ln}
    if payload[:1] in (b"{", b"["):
        out["json"] = json.loads(payload)
    return out

print(json.dumps({
    "built": {k: v.hex() for k, v in built.items()},
    "parsed": {k: parse(v) for k, v in built.items()},
}))
