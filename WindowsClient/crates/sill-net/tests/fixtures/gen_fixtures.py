#!/usr/bin/env python3 -I
"""Writes tests/fixtures/python_vectors.json: what Scripts/sillclient.py's own crypto functions
(macid, damm, b64u, spki, parse_link, the proofs of pair(), verify_macinfo's openssl check) and
/usr/bin/openssl compute for fixed inputs, so the Rust ports can be compared byte for byte.

usage: python3 -I gen_fixtures.py /path/to/Scripts/sillclient.py OUT.json
Never connects anywhere: it execs only the lines of sillclient.py between "the remote door's crypto"
and "def tls_connect", and runs openssl locally."""
import base64, hashlib, hmac, json, os, re, struct, subprocess, sys, tempfile, urllib.parse

src = open(sys.argv[1]).read()
a = src.index("# MARK: the remote door's crypto")
b = src.index("def tls_connect")
ns = {"re": re, "json": json, "struct": struct, "__name__": "sillclient_crypto"}
exec(compile(src[a:b], "sillclient.py(crypto)", "exec"), ns)
macid, damm, b64u, b64u_decode, spki, parse_link, SPKI_PREFIX = (
    ns[k] for k in ["macid", "damm", "b64u", "b64u_decode", "spki", "parse_link", "SPKI_PREFIX"])

OPENSSL = "/usr/bin/openssl"
out = {}
t = tempfile.mkdtemp()

# Damm
out["damm"] = [{"digits": d, "check": damm(d)} for d in
               ["48291355720", "0", "1", "57", "13", "00000000000", "99999999999", "12345678901", "31415926535"]]

# Mac ID
fps = [hashlib.sha256(x).digest() for x in [b"example", b"mac", b"device", b"relay", b""]]
fps += [bytes(32), bytes([255]) * 32]
out["macid"] = [{"fp": fp.hex(), "macid": macid(fp)} for fp in fps]

# Identities made by openssl, as sillclient's ensure_identity does (ecparam + req -x509)
ids = []
for n in range(3):
    d = os.path.join(t, f"id{n}"); os.makedirs(d)
    key, cert = os.path.join(d, "key.pem"), os.path.join(d, "cert.pem")
    subprocess.run([OPENSSL, "ecparam", "-name", "prime256v1", "-genkey", "-noout", "-param_enc", "named_curve", "-out", key], check=True, capture_output=True)
    subprocess.run([OPENSSL, "req", "-x509", "-new", "-key", key, "-subj", "/CN=sillclient", "-days", "3650", "-out", cert], check=True, capture_output=True)
    cert_der = base64.b64decode("".join(l for l in open(cert).read().splitlines() if "-----" not in l))
    pk8 = subprocess.run([OPENSSL, "pkcs8", "-topk8", "-nocrypt", "-in", key, "-outform", "der"], check=True, capture_output=True).stdout
    pub = subprocess.run([OPENSSL, "ec", "-in", key, "-pubout", "-outform", "der"], check=True, capture_output=True).stdout
    assert pub[:26] == SPKI_PREFIX and len(pub) == 91
    point = pub[26:]
    fp = hashlib.sha256(spki(cert_der)).digest()
    assert fp == hashlib.sha256(pub).digest()
    ids.append({"pkcs8": pk8.hex(), "cert_der": cert_der.hex(), "point": point.hex(), "fp": fp.hex(), "macid": macid(fp), "key_pem": key})
out["identities"] = [{k: v for k, v in i.items() if k != "key_pem"} for i in ids]

# Pairing proofs, exactly as pair() computes them
fp_mac = hashlib.sha256(b"mac").digest(); fp_dev = hashlib.sha256(b"device").digest()
proofs = []
def add(name, k, fm, fd):
    pd = hmac.new(k, b"sill-pair-v1 device\x00" + fd + fm, hashlib.sha256).digest()
    pm = hmac.new(k, b"sill-pair-v1 mac\x00" + fm + fd, hashlib.sha256).digest()
    proofs.append({"name": name, "key": k.hex(), "fp_mac": fm.hex(), "fp_dev": fd.hex(), "proof_d": b64u(pd), "proof_m": b64u(pm)})
for code in ["482913557208", "482913557217", "000000000000"]:
    for fm in [fp_mac, ids[0]["fp"] and bytes.fromhex(ids[0]["fp"])]:
        k = hashlib.pbkdf2_hmac("sha256", code.encode(), b"sill-pair-v1" + fm, 600_000, 32)
        add("code " + code, k, fm, fp_dev)
        proofs[-1]["code"] = code
add("qr 0x22*16", bytes([0x22]) * 16, fp_mac, fp_dev)
add("qr 0..15", bytes(range(16)), bytes.fromhex(ids[1]["fp"]), bytes.fromhex(ids[2]["fp"]))
out["proofs"] = proofs

# Recognition tag (the plan's vector, then more). Mirrors the plan 3.5 and Swift RecognitionTag.
def tag(key, p): return b64u(p + hmac.new(key, b"sill-tag-v1" + p, hashlib.sha256).digest()[:6])
tags = [{"key": (bytes([0x11]) * 32).hex(), "nonce": "a1a2a3a4a5a6", "tag": tag(bytes([0x11]) * 32, bytes.fromhex("a1a2a3a4a5a6"))}]
assert tags[0]["tag"] == "oaKjpKWmL4V47Ctm", tags[0]
for i in range(4):
    k = hashlib.sha256(b"rk%d" % i).digest(); p = hashlib.sha256(b"p%d" % i).digest()[:6]
    tags.append({"key": k.hex(), "nonce": p.hex(), "tag": tag(k, p)})
out["tags"] = tags

# Links: sillclient.parse_link on URLs built with Swift's escape rule (unreserved + ":")
def esc(s): return urllib.parse.quote(s, safe="-._~:")
def link(fp, secret, name, port, addrs, extra=""):
    items = [("v", "1"), ("m", macid(fp)), ("k", b64u(fp)), ("s", b64u(secret)), ("n", name), ("p", str(port))] + [("a", a_) for a_ in addrs]
    return "sill://pair?" + "&".join(f"{k}={esc(v)}" for k, v in items) + extra
fpl = bytes.fromhex(ids[0]["fp"]); sec = bytes(range(16))
urls = [
    link(fpl, sec, "Mac mini", 7455, ["100.101.102.103"]),
    link(fpl, sec, "Tom & Jerry’s Mac=mini?", 7455, ["mac-mini.tail1234.ts.net", "100.101.102.103", "[fd7a:115c:a1e0::1234]", "192.168.1.20", "home.example.net:17455"]),
    link(fpl, sec, "Näme \U0001F34E", 65535, []),
]
links = []
for u in urls:
    p = parse_link(u)
    links.append({"url": u, "fp": p["fp"].hex(), "secret": p["secret"].hex(), "port": p["port"], "name": p["name"], "addresses": p["addresses"], "macID": p["macID"]})
out["links"] = links

# kind 18 signed by openssl (dgst -sign) and verified by openssl (dgst -verify), as verify_macinfo does
k18 = []
for n, i in enumerate(ids):
    info = json.dumps({"v": 1, "macID": i["macid"], "name": "Mac mini", "issuedAt": 1790265600.5 + n, "remoteAccess": True,
                       "remotePort": 7455, "internet": False,
                       "addresses": [{"host": "100.101.102.103", "kind": "vpn", "via": "Tailscale"}]}, separators=(",", ":")).encode()
    ip = os.path.join(t, f"info{n}"); open(ip, "wb").write(info)
    sig = subprocess.run([OPENSSL, "dgst", "-sha256", "-sign", i["key_pem"], ip], check=True, capture_output=True).stdout
    point = bytes.fromhex(i["point"])
    spki_der = SPKI_PREFIX + point
    pub = os.path.join(t, f"pub{n}.pem"); open(pub, "w").write("-----BEGIN PUBLIC KEY-----\n" + base64.encodebytes(spki_der).decode() + "-----END PUBLIC KEY-----\n")
    sp = os.path.join(t, f"sig{n}"); open(sp, "wb").write(sig)
    r = subprocess.run([OPENSSL, "dgst", "-sha256", "-verify", pub, "-signature", sp, ip], capture_output=True, text=True)
    assert "Verified OK" in r.stdout
    k18.append({"info": base64.b64encode(info).decode(), "key": base64.b64encode(point).decode(), "sig": base64.b64encode(sig).decode(),
                "macid": i["macid"], "fp": i["fp"], "info_hex": info.hex()})
out["kind18"] = k18

json.dump(out, open(sys.argv[2], "w"), indent=1, sort_keys=True)
print("wrote", sys.argv[2], {k: len(v) for k, v in out.items()})
