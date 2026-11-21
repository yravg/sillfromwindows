# Working in WindowsClient

Read [`../AGENTS.md`](../AGENTS.md) (the rules) and [`../HANDOFF.md`](../HANDOFF.md) (the state)
first. This folder is the Rust workspace; run the checks from here:

```powershell
cargo test --workspace
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo build --release -p sill-app
```

The UI targets the iPad's look: use `src/theme.rs` and `src/ui/widgets.rs`, never raw colors or
sizes. The running `target\release\Sill.exe` locks the release build; close it or use
`$env:CARGO_TARGET_DIR='...\target-diag'`. Mock screenshots and all test hooks are in
`../AGENTS.md`. `../Sources/` and `../iOSClient/` are upstream's and the wire oracle; do not edit
them for a Windows task.
