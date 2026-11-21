"""Reads hex-encoded messages (one per line) on stdin, parses each with the struct/json calls
sillclient.py's read loop uses (`struct.unpack(">BdBI", ...)`, `json.loads`) and prints a JSON list.
Never opens a socket."""
import json, struct, sys

out = []
for line in sys.stdin:
    raw = bytes.fromhex(line.strip())
    kind, ts, key, ln = struct.unpack(">BdBI", raw[:14])
    payload = raw[14:]
    assert len(payload) == ln, "length field does not match payload"
    item = {"kind": kind, "timestamp": ts, "key": key, "len": ln}
    if payload[:1] in (b"{", b"["):
        item["json"] = json.loads(payload)
    out.append(item)
print(json.dumps(out))
