#!/usr/bin/env python3
"""Makes tests/fixtures/synthetic-hevc.sill: 30 frames of ffmpeg's testsrc2, 320x240, HEVC (libx265),
no B-frames, keyframes at 0 and 15, BT.709 limited, in Sill's framing as the Mac sends it:
kind 0 (nalLen=4, count, sets as len u16 BE + bytes), then kind 1 frames of 4-byte
length-prefixed NALs without parameter sets. Header: kind u8, f64 BE ts, keyframe u8, u32 BE len.
Needs the ffmpeg CLI with libx265. Usage: make_synthetic.py OUT.sill"""
import struct, subprocess, sys

out = sys.argv[1]
raw = subprocess.run(
    ["ffmpeg", "-v", "error", "-f", "lavfi", "-i", "testsrc2=size=320x240:rate=30", "-frames:v", "30",
     "-pix_fmt", "yuv420p", "-c:v", "libx265", "-preset", "fast",
     "-x265-params", "bframes=0:keyint=15:min-keyint=15:scenecut=0:repeat-headers=1:log-level=error:"
     "colorprim=bt709:transfer=bt709:colormatrix=bt709:range=limited",
     "-f", "hevc", "-"], check=True, capture_output=True).stdout

# Split Annex B into NALs.
nals, i = [], 0
starts = []
while True:
    j = raw.find(b"\x00\x00\x01", i)
    if j < 0:
        break
    starts.append(j)
    i = j + 3
for k, s in enumerate(starts):
    e = starts[k + 1] if k + 1 < len(starts) else len(raw)
    nal = raw[s + 3:e]
    while nal.endswith(b"\x00") and k + 1 < len(starts):
        nal = nal[:-1]  # trailing zero of the next 4-byte start code
    nals.append(nal)

sets, frames, cur = [], [], []
for n in nals:
    t = (n[0] >> 1) & 0x3F
    if t in (32, 33, 34):
        if len(sets) < 3:
            sets.append(n)
        continue
    cur.append(n)
    if t < 32:  # a VCL NAL ends this access unit (one slice per picture here)
        frames.append((cur, 16 <= t <= 21))
        cur = []
assert len(frames) == 30 and len(sets) == 3, (len(frames), len(sets))

def msg(kind, ts, key, payload):
    return struct.pack(">BdBI", kind, ts, key, len(payload)) + payload

blob = bytes([4, 3]) + b"".join(struct.pack(">H", len(s)) + s for s in sets)
data = msg(0, 0.0, 0, blob)
for n, (units, key) in enumerate(frames):
    payload = b"".join(struct.pack(">I", len(u)) + u for u in units)
    data += msg(1, n / 30, 1 if key else 0, payload)
open(out, "wb").write(data)
print(out, len(data), "bytes;", sum(1 for _, k in frames if k), "keyframes")
