# Sill wire protocol: what a non-Apple device client (Rust, rustls, mdns-sd, FFmpeg) needs

Source of truth: the repo at /home/user/dev/sillfromwindows (read-only, main-era 0.6.x, SillProtocol 1). Paths are relative to it.
Everything below is read from code unless marked UNCLEAR. Scripts/sillclient.py (a Python device client that speaks to the real Swift host over TLS 1.3 with an OpenSSL certificate) is the best executable reference; where it confirms a byte I say "confirmed by sillclient".

Conventions: BE = big endian. "JSON" = Swift JSONEncoder/Decoder defaults (no key sorting needed, nil Optionals are OMITTED on encode and accepted as absent or null on decode, Data = standard base64 with padding, Double fields accept 1 or 1.0, Int fields: emit integers without a decimal point, since 60.0 for an Int field is UNCLEAR in JSONDecoder; Sources/StreamProtocol/Switcher.swift:179-187 Wire.encode/decode are plain JSONEncoder/JSONDecoder).

---------------------------------------------------------------------------
## 1. Framing

### 1.1 Header (14 bytes, BE) Sources/StreamProtocol/StreamMessage.swift:3-4,131-139,141-150
| off | len | field |
|---|---|---|
| 0 | 1 | kind (UInt8) |
| 1 | 8 | timestamp: IEEE-754 Double bit pattern, BE (host wall clock, seconds since 1970; device sends its own `Date()` since 1970 too; echoed unchanged in pong) |
| 9 | 1 | isKeyframe: 1 = true, anything else false (StreamMessage.swift:147: `== 1`) |
| 10 | 4 | payloadLength UInt32 BE |
Then payload. Python: `struct.pack(">BdBI", kind, ts, key, len)` (sillclient.py:188).
An unknown kind is skipped by length (StreamMessage.swift:143-145); keep reading. Send/accept the same on every door.

### 1.2 Size caps (StreamMessage.swift:107-117)
- device -> host any message: <= 1 MiB (1<<20); the host CLOSES the connection if a header announces more (StreamServer.swift:1459-1463).
- pairing connection kind 19: <= 4096 bytes (`maxPairingPayload`), Door.swift:392.
- host -> device: kind 1 (frame) <= 32 MiB; every other kind <= 4 MiB (MessageReader.swift:86 in the device; mirror this).
- audio (kind 43) reader caps, part of the compat floor: type-1 payload <= 4096, type-2 payload <= 16384 (Audio.swift:142-143).

### 1.3 Kinds (StreamMessage.swift:5-95)
| # | name | dir | payload |
|---|---|---|---|
| 0 | parameterSets | H->D | binary, 1.4 |
| 1 | frame | H->D | one HEVC access unit, length-prefixed NALs, header.isKeyframe says IDR |
| 2 | windowList | H->D | JSON WindowList (1.5) |
| 3 | thumbnail | H->D | windowID UInt32 BE + JPEG (Switcher.swift:190-201) |
| 4 | appIcon | H->D | bundleIDLength UInt16 BE + bundleID UTF-8 + PNG (Switcher.swift:203-218) |
| 5 | appList | H->D | JSON `[{"name":..,"bundleID":..}]` |
| 6 | selectSource | D->H | JSON StreamSource (1.6) |
| 7 | launchApp | D->H | JSON `{"bundleID":"com.apple.TextEdit"}` (Switcher.swift:166) |
| 8 | input | D->H | JSON InputEvent (section 6) |
| 9 | viewport | D->H | JSON Viewport (1.7) |
| 10 | ping | D->H | 8 bytes (any 8 bytes; iOS sends monotonic clock Double BE) ; 16 random bytes is used as a clipboard "receipt" |
| 11 | pong | H->D | the ping payload and header timestamp, unchanged (StreamServer.swift:1493) |
| 12 | clientStats | D->H | JSON ClientStats (1.8), 1/s |
| 13 | tick | H->D | empty; ~every 30 ms while live; ignore |
| 14 | cursorShape | H->D | 4 x Float32 BE (hotX, hotY, width, height in Mac points) + PNG (Viewport.swift:141-158) |
| 15 | windowCommand | D->H | JSON `{"id":<UInt32>,"action":"close"|"minimize"|"fullScreen"}` (Switcher.swift:172-177) |
| 16 | hostSettings | H->D | JSON HostSettingsState (7.1) |
| 17 | changeSettings | D->H | JSON HostSettingsChange (7.1) |
| 18 | macInfo | H->D | JSON SignedMacInfo (4.6) |
| 19 | pairRequest | D->H | JSON PairRequest, pairing connection only (4.4) |
| 20 | pairResult | H->D | JSON PairResult (4.4) |
| 21 | pairingWanted | D->H | empty. "Show your pairing code". Home door only; remote door ignores (StreamServer.swift:1505-1507) |
| 22 | goodbye | H->D | JSON Goodbye (5.5) |
| 23 | hello | D->H | JSON Hello (5.1) |
| 24 | macMenu | H->D | JSON MacMenu (7.4) |
| 25 | pressMenuItem | D->H | JSON PressMenuItem (7.4) |
| 26 | macPointer | H->D | JSON MacPointer (7.5) |
| 27 | fetchMenu | D->H | JSON FetchMenu (7.4) |
| 28 | gesture | D->H | JSON `{"gesture":"swipeUp","fingers":3}` (7.6) |
| 32 | clipboard | both | JSON Clipboard (7.3) |
| 43 | audio | H->D | binary, type byte first (7.7) |
| 44 | keyframeRequest | D->H | empty (5.4) |
29-31, 33-42 are unused by this host (reserved to another client). Never send them.

### 1.4 Video (kinds 0 and 1)
Parameter sets, kind 0 (StreamMessage.swift:154-191): `nalLen UInt8 (4)`, `count UInt8`, then count x (`len UInt16 BE`, bytes). The sets are raw HEVC NAL units WITHOUT start codes, in VideoToolbox order (expected VPS(32), SPS(33), PPS(34); HEVCEncoder.swift:525-541; the NAL type of each is in its first 2 header bytes, do not rely on order, classify by `(b0>>1)&0x3f`).
Frame, kind 1: the CMSampleBuffer bytes: a sequence of `[nalLen=4 bytes BE length][NAL]` (HVCC/AVCC style, HEVCEncoder.swift:511-523 copies the block buffer unchanged). header.isKeyframe=1 for IDR. Profile HEVC Main auto level, no B-frames, real-time, GOP = fps*4 (HEVCEncoder.swift:162-169). timestamp = host wall clock at send (use only for latency stats; there is no PTS: present frames in arrival order).
Order guaranteed per client: kind 0 immediately before the first keyframe it receives (StreamServer.swift:1584-1590 for home; paceRemote path 1722 likewise), and the host repeats kind 0 before every keyframe (StreamCoordinator.swift:2185-2188). A new/different SPS = new frame size, reinitialise the decoder. A client only gets deltas after its first keyframe, and after a dropped delta waits for the next keyframe (StreamServer.swift:1582-1603): do not decode deltas before a keyframe.
FFmpeg: either (a) build an hvcC extradata from VPS/SPS/PPS with length size 4 and feed packets as-is, or (b) convert each length-prefixed NAL to Annex-B (00 00 00 01) and prepend VPS+SPS+PPS Annex-B to every keyframe packet. (b) is simplest. Never reorder; no B-frames.

### 1.5 WindowList (kind 2) Sources/StreamProtocol/Switcher.swift:34-89
```
{"macName":String,"windows":[{"id":UInt32,"title":String,"appName":String,"bundleID":String,"width":Int,"height":Int}],
 "active": StreamSource, "launchID":String?, "hostVersion":String?, "protocol":Int?, "gestures":Int?, "clipboard":Int?,
 "keyRepeat":Int?, "locked":Bool?, "offConsole":Bool?}
```
Sent right on connect and on every change. `gestures`>=1: host takes kind 28. `clipboard`>=1: host takes kind 32 text (absent when the Mac's Share Clipboard is off). `keyRepeat`>=1: send hardware keyboard characters as `key` events (the host auto-repeats a held key). `locked`: Mac locked, no windows, host streams a black placeholder.

### 1.6 StreamSource (Switcher.swift:4-8) Swift enum with associated values, synthesized Codable. Confirmed by sillclient.py:191-195:
- none: `{"none":{}}`  desktop: `{"desktop":{}}`  window: `{"window":{"_0":123}}`
The same shape is used inside WindowList.active.
The host starts streaming ONLY after the device picks (kind 6). The iOS client sends `{"desktop":{}}` by itself when the first list says nothing streams (StreamClient.swift: autoDesktop). A Windows client must do the same.

### 1.7 Viewport (kind 9) Viewport.swift:11-27, bounds 45-96
`{"width":Double,"height":Double,"scale":Double?,"localCursor":Bool?,"fps":Int?}`; width/height 1..16384 points, scale 0.25..4 or omitted (omitted = leave Mac window alone), fps = wanted rate (host clamps to >=24 and its max; 60 or 120). Out-of-range or non-decoding payload is refused whole with a log line. localCursor is legacy; host never draws the Mac cursor in the video and streams its shape (kind 14). Send once after the first window list and again on resize. sillclient sends `{"width":1117,"height":642,"scale":null,"fps":60}` (sillclient.py:714).

### 1.8 ClientStats (kind 12) Viewport.swift:107-128
`{"fps":Int,"frameAgeMs":Int,"rttMs":Int,"device":String,"frameAgeMaxMs":Int?,"rttMaxMs":Int?,"audioBehindMs":Int?,"audioLate":Int?}` -1 = no sample. Optional for correctness (host only logs it) but needed for the Mac's menu card.

---------------------------------------------------------------------------
## 2. Discovery (Bonjour)

- Service type `_sill._tcp`, domain local (StreamServer.swift:450 default, :5). Instance name = the Mac's name (may get " (2)"). The SRV port is the HOME DOOR port (dynamic, take it from SRV; UNCLEAR if Sill.app pins one: StreamServer listener creation not traced). Test builds use `_silltest._tcp` (never in production).
- TXT keys (Pairing.swift:143-222):
  - `r` = recognition tag: 16 chars base64url (no padding) = 12 bytes `p(6 random) || HMAC-SHA256(key=recognitionKey(32 bytes), msg="sill-tag-v1" || p)[0..<6]`. A saved Mac stores `recognitionKey` (given in kind 20 ok, 4.4). Verify: decode base64url (reject non-canonical), recompute over bytes[0..6], compare 6 bytes (RecognitionTag.make/matches, Pairing.swift:143-182). Replayable, it is only a "which saved Mac is this" hint, never authentication. `p` (6 bytes) is fresh at every registration.
  - `p` = home door mode: absent = plain TCP door (dev builds/CLI without --pairing; do NOT dial plain, production Sill.app always has `p`); "0" = TLS, any device may connect (Require pairing off); anything else ("1") = TLS, paired devices only (HomeDoorTXT.door, Pairing.swift:185-222). Key is case-insensitive.
- The home door speaks TLS 1.3 mutual with ALPN sill/1 (session) and sill-pair/1 (pairing), see 3. Remote door (VPN/internet) is another TCP port, default 7455 (HostConfig.swift:39-41, 96), same TLS, but the pairing window must be open for sill-pair/1 and the key must be paired for sill/1 (DoorPolicy.swift:69-117). Addresses for the remote door arrive in kind 18 (4.6) or the pairing link (4.3).
- Interop note: Windows mdns-sd must browse `_sill._tcp.local.`, resolve SRV+TXT+A/AAAA. The host also registers on AWDL (irrelevant). Link-local IPv6 needs a scope id; prefer IPv4 where listed.

---------------------------------------------------------------------------
## 3. TLS

### 3.1 Parameters RemoteTLS.swift:24-81
- TLS 1.3 only (min=max=TLSv13). Mutual: the server REQUIRES a client certificate (`peer_authentication_required`). Tickets and resumption disabled on both sides: configure rustls with `Resumption::disabled()`.
- ALPN: the device offers EXACTLY ONE protocol: `sill/1` for a session, `sill-pair/1` for pairing. Server offers both (RemoteTLS.swift:35). Verify the negotiated ALPN equals what you offered (sillclient.py:339-341).
- SNI: always `sill` (RemoteTLS.swift:38); the host ignores it. In rustls use `ServerName::try_from("sill")` (valid DNS label) regardless of the IP dialled.
- TCP: no Nagle; host keepalive 5 s/2 s/3. Dial timeout 10 s per attempt (RemoteTLS.swift:40). Device tries candidate addresses "happy eyeballs" 1 s apart, whole dial 20 s (RemoteDialPolicy.swift:12-17).
- Home door cipher seen on loopback: TLS13_AES_256_GCM_SHA384 (0x1302) (RemoteTLS.swift:19-21). rustls supports it.

### 3.2 Pinning (both directions) RemoteIdentity.swift:197-226
Fingerprint = SHA-256 of the DER SubjectPublicKeyInfo of the peer's LEAF certificate, 32 bytes. SPKI DER for P-256 = `3059 3013 0607 2a8648ce3d0201 0608 2a8648ce3d030107 034200 04||X||Y` (prefix hex 3059301306072a8648ce3d020106082a8648ce3d030107034200 + 65-byte point; sillclient.py:316 SPKI_PREFIX). Only P-256 keys are accepted; any other key type gives nil and nothing matches. Names, dates, chains, extensions are never examined (RemoteTLS.swift:70-79).
In rustls: implement `ServerCertVerifier` (dangerous) that (a) computes SHA-256 over SPKI of the end-entity cert (parse with x509-parser/webpki), (b) compares with the saved pin (or accepts any P-256 key when pin = None, used only for typed-code pairing/ask), (c) STILL verifies the TLS1.3 CertificateVerify signature using the cert's key (`rustls::crypto::verify_tls13_signature` with the provider's `signature_verification_algorithms`), otherwise a MITM could replay the cert. Supported scheme: ECDSA_NISTP256_SHA256 only.

### 3.3 Mac ID (RemoteIdentity.swift:228-253; confirmed sillclient.py:280-282)
`macID` = Crockford base32 (alphabet `0123456789ABCDEFGHJKMNPQRSTVWXYZ`, uppercase) of the first 10 bytes of the fingerprint, MSB first, 16 characters. A device always checks `macID == MacID(fp)`. Keys saved Macs.

### 3.4 The device's certificate (what the host requires)
Host side checks (Door.swift:172, DoorPolicy.swift:69-117): a client cert must be present, leaf key must be P-256, and its SPKI-SHA256 must be on the trust list for sill/1 (or Require pairing off at home; or any key for sill-pair/1 at home; remote sill-pair/1 only while a pairing window is open). Nothing else is checked (no names, dates, key usage, chain).
- iOS builds a minimal hand-made X.509 v3: version [0] v3, 16-byte random positive serial, signature ecdsa-with-SHA256 (1.2.840.10045.4.3.2) with NO parameters field, issuer = subject = `SEQUENCE{SET{SEQUENCE{OID 2.5.4.3, UTF8String <16 random hex chars>}}}`, validity UTCTime 260101000000Z to GeneralizedTime 99991231235959Z, subject SPKI as 3.2, NO extensions, signature = ECDSA X9.62 DER over TBS (RemoteIdentity.swift:258-288). 
- BUT sillclient.py proves a stock `openssl req -x509 -subj /CN=sillclient -days 3650` P-256 cert (v3 with SKI/AKI/basicConstraints, validity 10 y) is accepted by the real host (sillclient.py:316-325). So generate a normal self-signed P-256/ECDSA-SHA256 cert with rcgen; no need to copy the hand-built DER. Persist the P-256 private key; the pin the Mac stores is the SPKI hash of your key, so the key (not the cert) is your identity. Re-making the cert each start is fine.
- rustls client cert: `ResolvesClientCert` returning a `CertifiedKey` (cert DER + `any_ecdsa_type(PKCS8 key)`); ignore the CertificateRequest issuer list. Whether Apple's server sends a certificate_authorities list or which signature_algorithms it offers: UNCLEAR (not in repo; Python/OpenSSL works, which implies ecdsa_secp256r1_sha256 is offered).
- Server cert as seen by you (RemoteIdentity.swift:258-288): the hand-built cert described above: v3, no extensions, no SAN, empty-ish CN of 16 hex, notBefore 2026-01-01, notAfter 9999-12-31 (GeneralizedTime). webpki/rustls-webpki parsing of a no-extension v3 cert and of a year-9999 GeneralizedTime: UNCLEAR, untested here. If you parse with `x509-parser`/`der` instead of webpki for SPKI extraction you avoid the question, but `verify_tls13_signature` calls webpki's EndEntityCert parser internally: TEST THIS FIRST against a real Mac (Risk #1).
- Failure signals: pin mismatch on the device: handshake aborted before sending its cert. Host refusing your key (unpaired on sill/1): handshake completes on your side (client is ready), first read fails with a TLS alert (-9825 in Apple terms) or EOF. Not-TLS-1.3 to the host: -9836.

---------------------------------------------------------------------------
## 4. Pairing

### 4.1 Doors/flow
Pairing connection = TLS with ALPN `sill-pair/1`, exactly ONE kind 19 from the device (no hello), then ONE kind 20 from the host, then the host closes (Door.swift:378-420; sillclient.py:375-415). Deadlines: host expects kind 19 within 4 s of TLS ready (DoorPolicy `requestDeadline` = 4, DoorPolicy.swift:244), 10 s total admission; device waits 15 s for kind 20 (RemoteDialPolicy.pairingReplyDeadline). Compute the proof BEFORE sending but start counting at handshake completion: PBKDF2 (4.3) must finish well under 4 s.
Ways to start: (a) QR/link (scan or paste), (b) typed 12-digit code + Mac address, (c) "ask" at the home door (4.5), which makes the Mac open its pairing window / show a code.

### 4.2 Pairing code (Pairing.swift:14-60; Damm table confirmed sillclient.py:283-289)
12 ASCII digits: 11 random + 1 Damm check digit. Damm: interim=0; for each digit d: interim = TABLE[interim][d]; a valid full code ends at 0. Table rows:
0:[0,3,1,7,5,9,8,6,4,2] 1:[7,0,9,2,1,5,4,8,6,3] 2:[4,2,0,6,8,7,1,3,5,9] 3:[1,7,5,0,9,8,3,4,2,6] 4:[6,1,2,3,0,4,5,9,7,8] 5:[3,6,7,4,2,0,9,5,8,1] 6:[5,8,6,9,7,2,0,1,3,4] 7:[8,9,4,5,3,6,2,0,1,7] 8:[9,4,3,8,6,1,7,2,0,5] 9:[2,5,8,1,4,3,6,7,9,0].
Typed input: strip spaces, hyphens, U+00A0; must be 12 ASCII digits with checksum 0 else reject locally (typo never costs one of the Mac's 5 tries). Shown as "4829 1355 7208".

### 4.3 Link (QR) `sill://pair?v=1&m=<macID>&k=<fp>&s=<secret>&n=<name>&p=<port>&a=<addr>&a=<addr>...` Pairing.swift:223-307
- v must be `1`. m = macID (16 Crockford chars), k = base64url(no pad) of the 32-byte Mac fingerprint, s = base64url of 16-byte one-time secret, n = Mac name (percent-encoded, SafeText, <=64 chars), p = remote port 1-65535 (digits, <=5), a (repeated, <=5, keep first 5) = `host` or `host:port`, IPv6 in brackets, no zone ids, strict dotted IPv4 / RFC1123 names (AddressParser.swift:6-170). Each of m,k,s,p,v exactly once. Require `m == MacID(k)`. Unknown params ignored. Values percent-encoded except `A-Za-z0-9-._~:`.
- QR path key K = the 16-byte secret `s` directly. Pin = k. Dial `a` addresses (port `p` unless the entry has its own) or, at home, the Bonjour row.
- Typed path key K = PBKDF2-HMAC-SHA256(password = the 12 ASCII digit bytes, salt = ASCII "sill-pair-v1" || fp_mac (32 raw bytes), iterations = 600000, dkLen = 32) (Pairing.swift:80-104; confirmed sillclient.py:389). fp_mac is the Mac's SPKI hash from THIS TLS session, never from a message. The typed path connects with pin = None (or the key the ask saw) because the Mac key is learned from the handshake; the proofs bind both keys.

### 4.4 Messages Remote.swift:111-206
Proofs (Pairing.swift:80-135): with fp_d = your cert SPKI SHA-256, fp_m = Mac's:
- proof_D = HMAC-SHA256(K, "sill-pair-v1 device" || 0x00 || fp_d || fp_m)   (device message string is `"sill-pair-v1 device"` then one zero byte then 32+32 bytes)
- proof_M = HMAC-SHA256(K, "sill-pair-v1 mac" || 0x00 || fp_m || fp_d)
Kind 19 (D->H): `{"v":1,"method":"qr"|"code"|"ask"|"cancel","proof":"<base64url(proof_D)>","name":"<device name>","model":"<model>"?,"cable":Bool?}`. proof "" for ask/cancel. `v` must be 1 else host answers `closed` with no try counted (DoorPolicy.swift:180). The Mac applies SafeText to name. Omit `cable`.
Kind 20 (H->D): `{"ok":Bool,"proof":String?,"macID":String?,"name":String?,"recognitionKey":String?,"reason":String?,"triesLeft":Int?,"retryAfter":Double?,"method":String?,"message":String?}`.
- ok:true after qr/code: proof = base64url(proof_M) (CHECK it with constant-time compare against HMAC with the same K before saving anything), macID (must equal MacID(fp_m)), name, recognitionKey = base64url of 32 bytes. Save {macID, name, fp_m (pin), recognitionKey, addresses/port}. (StreamClient+Home.swift:949-957.)
- ok:false reasons: "code" (wrong proof, triesLeft), "closed", "expired", "stopped", "busy" (retryAfter seconds; iOS retries once silently after max(0.2,min(retryAfter,10))), and for ask: "shown" (window showing a code now: scan/type it), "openOnMac", "locked"; any other reason carries `message` to show. The window allows 5 wrong tries, 5 minutes, single use (PairingWindow.swift not read: from comments in Remote.swift/CLAUDE.md).
- Each exchange is a NEW TLS connection. Do not reuse.

### 4.5 ask / cancel (home door only; Remote.swift:111-146, DoorPolicy.swift:160-185)
Send only to a Mac whose TXT has `p`. `{"v":1,"method":"ask","proof":"","name":..,"model":..}` over sill-pair/1 pinned to nothing (TOFU: remember the key that answered, "askedKey"). Answers: ok true + method "cable" (USB cable auto-pair, needs `cable:true` and the host's own cable check, irrelevant for Windows), or ok false reason "shown" (the Mac displays a code; user types it; then a second sill-pair/1 connection to the same endpoint, pinned to the askedKey, method "code") / "openOnMac" / "locked". "cancel" `{"method":"cancel","proof":""}` pinned to the asked key closes the window the ask opened; answer always "closed". The remote door answers ask and cancel "closed". "Pair This iPad" from a live session = kind 21 (empty) on the session, then the same flow (home door only, once per 30 s).

### 4.6 kind 18 SignedMacInfo Remote.swift:37-109
H->D after kind 16 on every session. `{"info":"<base64 of the exact MacInfo JSON bytes>","key":"<base64(std, padded) of the Mac's 65-byte X9.63 uncompressed P-256 point>","sig":"<base64 of a DER ECDSA P-256 / SHA-256 signature over the raw info bytes>"}`.
Verify: base64-decode info/key/sig; key must be 65 bytes 0x04||X||Y; verify ECDSA-P256-SHA256 (DER signature, message = info bytes, hashed with SHA-256) with that key; decode MacInfo; require `info.macID == MacID(SHA256(SPKI(key)))`; for a saved Mac also require SPKI hash == saved pin, and `issuedAt` newer than stored. Never re-serialize the JSON before verifying. MacInfo: `{"v":1,"macID":String,"name":String,"issuedAt":Double,"remoteAccess":Bool,"remotePort":Int,"internet":Bool,"addresses":[{"host":String,"port":Int?,"kind":"vpn"|"lan"|"internet","via":String}]}`. Use `addresses` (empty while remote access off) + `remotePort` as the remote dial list. Key and signature: confirmed by sillclient.py:361-372 (openssl dgst).

---------------------------------------------------------------------------
## 5. Session

### 5.1 Start order (sillclient.py + StreamClient.swift:2031; Door.swift:326-355)
1. TCP, TLS1.3, ALPN sill/1, present device cert; Mac pinned (saved record) or unpinned only for an unpaired first contact at an open door.
2. D->H FIRST MESSAGE: kind 23 hello, written before anything else (host reads it first; the iOS client sends it the moment the connection exists): `{"appVersion":"1.0","build":"1","protocol":1,"device":"Windows PC (name)","audio":["aac-eld"]}` all fields optional; `audio` (best first) is REQUIRED if you want sound (host sends kind 43 only to a device whose hello lists a codec it makes). Compatibility.swift:104-124. The device-version floor is "0" today (nobody refused); if raised the host answers kind 22 "update" and closes.
3. H->D in this order (StreamCoordinator.swift:3340-3379): kind 2 windowList, kind 16 hostSettings (silent state), kind 18 macInfo, N x kind 4 appIcon, kind 5 appList, kind 14 cursorShape. Then video: the host also requests a keyframe at connect (StreamCoordinator.swift:538), but frames only flow after a source is picked.
4. D->H kind 6 pick (typically `{"desktop":{}}`), kind 9 viewport (re-send on resize), optional kind 17/27 subscriptions.
5. Video arrives: kind 0, kind 1 key, kind 1 deltas; thumbnails (kind 3) refresh every 1-2 s per window; kind 13 ticks.
Unknown host->device kinds must be skipped.

### 5.2 Liveness
- Device pings every 0.25 s (kind 10, 8-byte payload; iOS = monotonic Double BE) and expects pong echo; RTT = now - echoed value. Send kind 12 stats every 1 s.
- Device declares the connection lost when NO byte has arrived for max(6 s, 4 x worst recent RTT) (StreamClient.swift:989,4451-4460). Host (ticks every 30 ms) keeps the radio awake.
- Host drops a REMOTE client that sent nothing for 8 s (after 8 s since admit), and a home client that stops draining for 4 s (8 s grace) (StreamServer.swift:1285-1294,1316). Keep pinging; keep reading continuously.
- The host closes a session whose device exceeds 1 MiB messages, or kind 17 faster than 4/s (refused), kind 28 faster than 4/s, kind 44 faster than 1/s (dropped).

### 5.3 Pick / launch
kind 6 JSON StreamSource (1.6); kind 7 launchApp `{"bundleID":"..."}`: opens the app and selects its first window. kind 15 window command `{"id":N,"action":"close"|"minimize"|"fullScreen"}`. A pick of a window resizes it only if a viewport with a `scale` was sent.

### 5.4 Keyframe request: kind 44, empty payload. Send when your decoder lost sync or errored; once per interruption, again after max(4 s, 4 x RTT) if no keyframe, >= 2 s apart; host honours at most 1/s per connection (StreamCoordinator.swift:1220-1237). A still window never sends a keyframe by itself (GOP 4 s of encoded frames).

### 5.5 Goodbye (kind 22, H->D) Remote.swift:208-236
`{"reason":String,"message":String?,"minimumVersion":String?,"reconnect":Bool?}`. reasons: "removed" (this device was un-paired: do not reconnect, tell user to re-pair), "remoteOff", "internetOff", "quit" (Mac closing Sill: reconnect later by Bonjour), "busy" (8-session limit), "pairingRequired" (Require pairing turned on; pair again), "update" (message says update, reconnect:false). Unknown reasons: show `message` (<=300 chars), reconnect only if reconnect==true. After sending, host FINs (<=1 s later); later bytes are ignored.

---------------------------------------------------------------------------
## 6. Input (kind 8) Sources/StreamProtocol/Input.swift (shape confirmed by sillclient.py:87,712,711)
Swift synthesized Codable of an enum with associated values: one top-level key = case name; unlabeled associated values are `_0`.
- pointer: `{"pointer":{"_0":"move"|"leftDown"|"leftUp"|"rightDown"|"rightUp","x":0.5,"y":0.5}}`
- scroll: `{"scroll":{"x":..,"y":..,"dx":..,"dy":..}}`
- text: `{"text":{"_0":"hello"}}` ("\n","\t","\u{8}"(backspace) are delivered as Return/Tab/Delete keys, everything else typed as Unicode)
- scrollGesture: `{"scrollGesture":{"_0":"began"|"ended"|"momentumBegan"|"momentumEnded","x":..,"y":..}}`
- key: `{"key":{"hidUsage":<UInt16>,"down":true|false,"modifiers":<UInt64>}}` (sillclient.py:711 key order is irrelevant)
Coordinate space: x,y are FRACTIONS 0..1 of the streamed video frame (0,0 top-left), not pixels. Out of range values are clamped to the source's last pixel row/column by the host (InputInjector.swift:121-127, InputLanding). If nothing streams, input is dropped (only key/button ups still go through).
Scroll: dx,dy are fractions of the frame; host converts `dy * source height` to Mac points; positive dy = content moves DOWN (natural scrolling: finger dragged down). Wrap trackpad scrolling as: scrollGesture began, scroll deltas..., scrollGesture ended, optionally momentumBegan, scroll deltas (client-generated decaying), momentumEnded. A bare `scroll` with no phase = plain wheel. Host closes an unfinished gesture after a timeout.
Key: HID usage (USB HID Usage Page 0x07, same as UIKeyboardHIDUsage). Mapped usages only (KeyStrokes.swift:184-240): letters 0x04-0x1D, digits 0x1E-0x27, Return 0x28, Esc 0x29, Delete(backspace) 0x2A, Tab 0x2B, Space 0x2C, - = [ ] \ ; ' ` , . / = 0x2D 0x2E 0x2F 0x30 0x31 0x33 0x34 0x35 0x36 0x37 0x38, CapsLock 0x39, F1-F12 0x3A-0x45, Home 0x4A, PgUp 0x4B, ForwardDelete 0x4C, End 0x4D, PgDn 0x4E, Right 0x4F, Left 0x50, Down 0x51, Up 0x52, modifier keys 0xE0-0xE7 (LCtrl, LShift, LAlt=Option, LCmd=0xE3, RCtrl, RShift, ROpt, RCmd). Any other usage is dropped and counted. Numpad, ISO 0x32/0x64, F13+ are NOT mapped. No Windows key mapping exists; map Win key -> 0xE3 (Command) is a product call.
Modifiers: UIKeyModifierFlags bits: bit16 (0x10000) CapsLock, bit17 (0x20000) Shift, bit18 (0x40000) Control, bit19 (0x80000) Option(Alt), bit20 (0x100000) Command; all other bits ignored (KeyStrokes.swift:166-176). Send shortcut keys the way iOS does: modifiers' own keys (0xE0-0xE7) down, the key down/up with the modifier bits set, modifiers up (or one down+up with `modifiers` set; the host handles both). A key's up should carry modifiers=0 or what is still held. When `keyRepeat`>=1 in the window list, send held letters as key down/up only (host auto-repeats at the Mac's rate, do NOT send repeats); otherwise use `text` for characters.
Pointer: left/right buttons only; there is no middle button or move-with-button-held event (drag = leftDown, moves, leftUp). Moves are coalesced by the iOS client; host takes unlimited rate (no cap found) but logs gaps.

---------------------------------------------------------------------------
## 7. Other kinds

### 7.1 Settings (16/17) HostSettings.swift:64-308
Kind 16 H->D (sent after the list, on change, and as reply with `answering`): `{"settings":{"maxFPS":Int,"bitrate":Int,"captureScale":Double,"prioritizeSpeed":Bool,"virtualDisplay":Bool,"directWireless":Bool?,"sendAudio":Bool?},"persistent":Bool,"virtualDisplayAvailable":Bool,"virtualDisplayNote":String?,"softwareEncoder":Bool,"stream":{"width":Int,"height":Int,"fps":Int,"mbps":Int,"onVirtualDisplay":Bool}?,"answering":Int?,"away":{homeBitrate,homeCaptureScale,awayBitrate,awayCaptureScale,thisConnectionAway,awayRunning}?,"link":{"state":"behind"|"stalled","withheldPerSecond":Int,"bitrate":Int,"carriedKbps":Int?,"suggestedBitrate":Int?,"suggestedCaptureScale":Double?}?,"audioNote":String?,"sessionPaired":Bool?}`.
Kind 17 D->H: `{"token":Int?,"maxFPS":Int?,"bitrate":Int?,"captureScale":Double?,"prioritizeSpeed":Bool?,"virtualDisplay":Bool?,"directWireless":Bool?,"sendAudio":Bool?}` only the changed fields, absolute values, token strictly increasing per process; reply 16 carries `answering`=token. Allowed values: maxFPS 60|120; captureScale 2|1; bitrate 4000000,8000000,15000000,25000000,40000000,80000000,150000000 (per 60 fps; Low..Extreme); a value outside is refused (reply shows old). directWireless only from a paired session at home (sessionPaired true). Max 4 changes/s/connection. Don't send before the first kind 16 arrived (older Macs never send one).

### 7.2 Kind 14 cursor shape: 16 bytes of 4 x Float32 BE (hotspot x,y, size w,h in Mac points) then PNG (usually 2x). Draw your own cursor from kind 26/pointer input; the Mac's cursor is not in the video.

### 7.3 Clipboard (32) Clipboard.swift, ClipboardPolicy.swift
JSON `{"text":String?,"image":base64?,"stop":Bool?}` (nil omitted). Only if the window list `clipboard`>=1. D->H: any kind 32 subscribes (send `{}` at connect while the user's Share Clipboard is on), `{"text":"..."}` sets the Mac's clipboard, `{"stop":true}` unsubscribes. H->D: `{"text":...}` copies. Text <= 262144 bytes UTF-8, whole JSON <= 1 MiB; host rate 4/s per device. Never send `image`. "Receipt": a ping with a 16-byte random payload; the pong with that payload means the Mac has processed everything sent before (ClipboardPolicy.swift:333).

### 7.4 Menus (24/25/27) MacMenu.swift
Subscribe: kind 27 `{"token":N}` (no id) once per connection; host then pushes kind 24 `{"version":Int,"app":String,"bundleID":String,"menus":[MacMenuItem]}` (top level) whenever it changes. Fetch a menu's items: kind 27 `{"version":V,"id":"2","title":"File","token":N}` -> kind 24 `{"answering":N,"menu":"File","items":[...],"more":Int?,"stale":Bool?,"note":String?}`. Choose: kind 25 `{"version":V,"id":"2.9","title":"Save","token":N}` -> kind 24 `{"answering":N,"pressed":Bool,"note":..}`. MacMenuItem = `{"id":String,"title":String,"separator":Bool?,"enabled":Bool?,"mark":String?,"key":String?(shortcut as text, e.g. "⇧⌘S"),"submenu":Bool?}`. id = path of child indexes "2.9" valid only within one `version`; host acts only if the item still has the shown title; only a subscriber is served; 20 fetches and 4 presses per second. All fields optional. Menu button logic and timeouts: iOSClient/MacMenuState.swift (not read in full; UNCLEAR on exact timeout beyond max(4 s, 4 x RTT) per CLAUDE.md).

### 7.5 Pointer (26) Pointer.swift: `{"x":0.4213,"y":0.1873,"inside":true,"seen":12}` or `{"inside":false,"seen":12}`. x,y 4 decimals, fractions of the frame. `seen` = number of kind-8 messages the host had read on this connection; drop a report whose `seen` is lower than the number of inputs you have sent since (it predates your own input).

### 7.6 Gestures (28) Gesture.swift: `{"gesture":"swipeUp"|"swipeDown"|"swipeLeft"|"swipeRight"|"pinch"|"spread","fingers":3|4}`; only if the list says `gestures`>=1; the Mac turns each into its own shortcut (Mission Control, App Exposé, Spaces, Apps, Show Desktop). 4/s limit.

### 7.7 Audio (43) Audio.swift:1-192, host AudioEncoder.swift
Only after your hello listed "aac-eld" and the Mac's Send Audio is on (kind 16 `settings.sendAudio`; a device can turn it on with kind 17). Payload byte 0 = type.
- type 1 (format): byte 0x01 + JSON `{"codec":"aac-eld","sampleRate":48000,"channels":2,"framesPerPacket":480|512,"primingFrames":240|256,"bitrate":128000,"epoch":0..65535,"cookie":"<base64>","source":"app"|"desktop"|"test","app":String?}`. Sent before the first packet of every epoch (a new encoder).
- type 2 (packets): `0x02, epoch UInt16 BE, seq UInt32 BE, flags UInt8 (bit0 = first packet is first of a segment: reset decoder and drop `primingFrames` of output), count UInt8 (1..255), then count x (length UInt16 BE, bytes)`. Packets of one message are consecutive (seq, seq+1, ...). Header timestamp = host wall clock (seconds since 1970) of the first decoded sample of the first packet; following packets are framesPerPacket/48000 s apart.
- Codec: AAC-ELD (MPEG-4 AOT 39), 48 kHz, stereo, 128 kbps, 480 or 512 frames/packet (10/10.67 ms), one raw access unit per packet (AudioConverter output, no ADTS: inferred, not asserted in a file). FFmpeg: the native `aac` decoder supports ELD when given AudioSpecificConfig extradata.
- UNCLEAR: whether `cookie` is the bare AudioSpecificConfig or an MPEG-4 ES descriptor (0x03 ES_Descr ... 0x05 DecoderSpecificInfo) as CoreAudio usually returns for kAudioConverterCompressionMagicCookie. The repo only says "its AudioSpecificConfig" (Audio.swift:59; AudioEncoder.swift:38,81-89; docs/audio-plan.md:442,471, whose sample `"+AFAIAQA…"` does not decode to an AOT-39 ASC, so treat it as illustrative). Handle both: if cookie[0]==0x03 walk descriptors to tag 0x05 and use that payload, else use it as ASC. Capture a real kind 43 type-1 from a live Mac to settle.
- A device joining mid-segment gets garbage for ~3 packets: drop the first, fade in. The device mutes after 10 s without a packet.

---------------------------------------------------------------------------
## 8. The 5 riskiest interop points (rustls / FFmpeg)

1. rustls vs the Mac's hand-made server certificate: v3, NO extensions, no SAN, GeneralizedTime notAfter 9999-12-31 (RemoteIdentity.swift:258-288). rustls-webpki (used by `verify_tls13_signature`) may reject or mis-parse it; you must still verify CertificateVerify with the cert key. Test against a real Mac before anything else; fallback = verify the signature yourself with `p256`/`ring` over the TLS 1.3 transcript (hard) or patch webpki.
2. Mutual TLS 1.3 client-auth negotiation with Apple's stack: server REQUIRES a client cert, only ECDSA P-256/SHA-256 keys, exactly one ALPN offered, SNI "sill", resumption off. A rustls client with a custom `ResolvesClientCert` must offer cert even if the CertificateRequest lists no matching CA. The failure mode when the Mac rejects your key is a post-handshake alert, not a handshake error (looks like success then EOF).
3. HEVC delivery format: length-prefixed (AVCC-style, 4-byte) NALs plus out-of-band VPS/SPS/PPS (kind 0) that change on resize; decoder reinit on new SPS, never feed deltas before a keyframe; timestamps are wall-clock, not PTS (set your own PTS). Converting to Annex B or building hvcC extradata is on you; DXVA/NVDEC accept the Main profile but check HW fallback on odd sizes (Retina sizes like 3024x1964, even-padded).
4. Pairing crypto exactness: PBKDF2-HMAC-SHA256 600000 rounds with salt "sill-pair-v1"||fp_mac(32 raw bytes) must finish well inside the host's 4 s window after TLS ready; proofs bind the fingerprints from THIS TLS session; the host enforces 5 wrong tries per window; base64url unpadded canonical for proofs/tags, standard padded base64 for kind 18; DER (not raw r||s) ECDSA signatures for kind 18. One wrong byte costs a try.
5. AAC-ELD audio (AOT 39, 480/512 frame length): the cookie format is UNCLEAR (ASC vs ES descriptor), FFmpeg native aac ELD needs correct ASC with frame_length_short for 480, priming-frame dropping per segment, epoch/seq discontinuity handling and A/V sync from header timestamps; consider shipping video-only first.
Also: Swift Codable enum JSON (`_0`) and strict Int decoding (emit integers); a missing hello `audio` array = no sound; input is in frame fractions, keys are USB HID usages with Apple modifier bits (no Windows VK table exists in the repo).

## 9. UNCLEAR list (consolidated)
- Home door port policy (dynamic vs fixed): not traced (StreamServer listener creation, ~line 450-930).
- Apple server's CertificateRequest contents (certificate_authorities, signature_algorithms).
- rustls-webpki acceptance of the host's no-extension v3 cert (needs a live test).
- Audio cookie format (ASC vs ESDS); packets assumed raw AUs.
- Swift JSONDecoder behaviour for `60.0` into an Int field.
- PairingWindow details (5 tries, 5 min TTL, window opening rules): from comments only; PairingWindow.swift not opened.
- MacMenuState timeouts and tree-version semantics beyond the wire (iOSClient/MacMenuState.swift not read).
- ClientStatsReporter device-name format (free text, SafeText-cleaned on the host, <=64 chars).
