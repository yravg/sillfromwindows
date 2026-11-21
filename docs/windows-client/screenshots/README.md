# Mock screenshots

These are egui's own screenshots of the Windows client with `SILL_TEST_MOCK` sample data. They are
not a real Mac; no real screen, window title, key or account is in them.

| File | Case |
|---|---|
| `stream-bar.png` | `SILL_TEST_MOCK=stream-desktop` at 1600x1000 logical |
| `settings.png` | `SILL_TEST_MOCK=settings-default` with `SILL_TEST_STREAM_OPEN=settings` |
| `settings-away.png` | `SILL_TEST_MOCK=settings-away` with `SILL_TEST_STREAM_OPEN=settings` |

Regenerate them from `WindowsClient` (use `target-diag` if a Sill is running):

```powershell
$env:SILL_HOME='E:\orca\tmp\sill-home'
$env:SILL_TEST_WINDOW='1600x1000'
$env:SILL_TEST_MOCK='settings-default'
$env:SILL_TEST_STREAM_OPEN='settings'
$env:SILL_TEST_SHOT='E:\orca\tmp\settings.png@2'
$env:SILL_TEST_QUIT_AFTER='3.5'
$env:CARGO_TARGET_DIR='E:\dev\SillFromWindows\WindowsClient\target-diag'
cargo build --release -p sill-app
.\target-diag\release\Sill.exe
```

Screenshots are physical pixels: 1600×1000 logical is 2400×1500 at 150% Windows scaling. Compare
mock and live renders at the same scaling.
