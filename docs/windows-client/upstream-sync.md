# Upstream sync

Sill for Windows is a fork of [Saffsanity/sill](https://github.com/Saffsanity/sill). `main` is
upstream `main` plus one commit per sync that carries the fork's only additions:

- `WindowsClient/` — the Windows client.
- `docs/windows-client-plan.md`, `docs/windows-client/` — its plan, coverage matrix and wire spec.
- The credit at the top of `README.md` and `NOTICE`.

Upstream files are not edited. That keeps every future sync a normal merge.

## This sync

- **Base:** upstream `main` `c33d105` (2026-11-20, "Privacy: lock this Mac when the last device
  leaves"). The previous fork was a squashed snapshot of `41e8cb8` (2026-11-09), 376 commits
  behind.
- **Carried over unchanged:** `WindowsClient/` (including this PC's stream colour fix) and the
  three Windows docs.
- **Upstream moved from 0.6.0 to 0.6.3**, and added Privacy mode (lock the Mac when the last device
  leaves), the Desktop's displays (kind 47 `nextDisplay`), the off-console view, one-click update,
  Picture in Picture, full screen, away-through-relay, VPN address keeping, and the lean `CLAUDE.md`.

## What the Windows client does with the new wire

The framing reader maps an unknown kind to `Unknown(n)` and skips it by length, and serde ignores
unknown JSON fields, so the additive changes do not break an older Windows build:

- `nextDisplay` (kind 47): this client never sends it, so a current host never sends it one.
- `Hello.offConsoleView`: this client does not send it, so the host behaves as it does for any older
  device (the locked list, no windows).
- `Goodbye.restarting`: carries the Mac's `message` and `reconnect: true`; the client's unknown-reason
  path already shows the Mac's words and reconnects (`controller/words.rs`).

These are **wire compatibility**, not feature parity. New applies-to-Windows features (Privacy mode,
the Desktop's displays, one-click update) still need a Windows pass; the coverage matrix is where
that work is tracked.

## The next sync

```sh
git fetch upstream main
git switch main
git merge upstream/main
# WindowsClient/ is additive; resolve only if upstream touched it (it has not)
cd WindowsClient && cargo test --workspace && cargo fmt --check
cargo clippy -p sill-app -p sill-media --all-targets -- -D warnings
cargo build --release -p sill-app
git commit            # "Sync upstream main <sha>"
git push origin main
```

If GitHub resets the connection on this network (China), use HTTP/1.1 for the fetch:

```sh
git -c http.version=HTTP/1.1 -c http.lowspeedlimit=0 fetch upstream main
```

Keep the `upstream` remote pointed at `https://github.com/Saffsanity/sill.git`:

```sh
git remote add upstream https://github.com/Saffsanity/sill.git   # once
git remote set-url upstream https://github.com/Saffsanity/sill.git
```
