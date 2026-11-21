# Upstream sync

Sill for Windows is a fork of [Saffsanity/sill](https://github.com/Saffsanity/sill). `main` is
upstream `main` plus one commit per sync that carries the fork's only additions:

- `WindowsClient/` — the Windows client.
- `docs/windows-client-plan.md`, `docs/windows-client/` — its plan, coverage matrix and wire spec.
- The credit at the top of `README.md` and `NOTICE`.
- `AGENTS.md`, `AGENT.md`, `HANDOFF.md`, `WindowsClient/AGENTS.md` and the mock screenshots in
  `docs/windows-client/screenshots/`.
- A four-line Windows pointer at the top of `CLAUDE.md`.

Upstream code is not edited. Two upstream files carry fork lines — the README credit and the
CLAUDE.md pointer — so keep both at the top when merging; everything else is a normal merge.

## This sync

- **Base:** upstream `main` `c33d105` (2026-11-20, "Privacy: lock this Mac when the last device
  leaves"). The previous fork was a squashed snapshot of `41e8cb8` (2026-11-09), 376 commits
  behind.
- **Carried over:** `WindowsClient/` (including this PC's stream colour fix) and the Windows docs;
  the agent handoff pack and mock screenshots were added after the sync.
- **Upstream moved from 0.6.0 to 0.6.3**, and added Privacy mode (lock the Mac when the last device
  leaves), the Desktop's displays (kind 47 `nextDisplay`), the off-console view, one-click update,
  Picture in Picture, full screen, away-through-relay, VPN address keeping, and the lean `CLAUDE.md`.

## What the Windows client does with the new wire

The framing reader maps an unknown kind to `Unknown(n)` and skips it by length, and serde ignores
unknown JSON fields, so the additive changes do not break an older Windows build:

- `nextDisplay` (kind 47): the Windows Desktop button now sends it while the Desktop streams on a
  Mac with two stops or more; the button reads "Desktop 2" and shows a dot per stop.
- `Hello.offConsoleView`: this client does not send it, so the host behaves as it does for any older
  device (the locked list, no windows).
- `Goodbye.restarting`: carries the Mac's `message` and `reconnect: true`; the client's unknown-reason
  path already shows the Mac's words and reconnects (`controller/words.rs`).

These are **wire compatibility**, not feature parity. The Desktop's displays are now ported; Privacy
mode and one-click update are host-side and need no client work. The off-console view remains
Windows work for later; the coverage matrix tracks it.

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
