# Tezbar Linear extension

Linear runs as a standalone Rust process using Tezbar's external extension
protocol. It uses Linear's GraphQL endpoint and a workspace personal API key.
Create a key in Linear under **Settings → Security & access**, choose the teams
and permissions it needs, and enter it in the extension's **Linear Personal API
Key** password preference. Adding an API key grants the corresponding workspace
access to this extension. The active system tray inbox uses the key in memory
until the menu is stopped or Tezbar exits. Re-run **Unread Notifications** after
editing that command's preferences.

The original source is preserved in upstream/, with exact hashes in SOURCE.json.
The licensed assets and original metadata are packaged for both supported targets.
See PORT-REPORT.md for fixture coverage and differences in the Rust port.

## Local build and package

Run on the target platform with Rust 1.88 or later, Cargo, and Python 3 installed. Apple Silicon
packaging requires local macOS signing continuity per the Tezbar project's
release guidelines before signing or publication. This package script does no
signing or publication.

```powershell
$env:CARGO_BUILD_JOBS = "1"
cargo test --locked
cargo build --locked --features fixture
python scripts/test_protocol.py target/debug/tezbar-linear.exe
cargo build --locked --release
python scripts/test_protocol.py target/release/tezbar-linear.exe --production-smoke
python scripts/package.py --target x86_64-pc-windows-msvc --binary target/release/tezbar-linear.exe
```

On Apple Silicon use target/debug/tezbar-linear, target/release/tezbar-linear,
and --target aarch64-apple-darwin. The fixture feature is for loopback-only
integration tests. The packager rejects fixture-enabled binaries. Release
archives hash every ZIP member except the manifest in its own inventory.


See [parent verification and minimum host](PARENT-VERIFICATION.md) for the repaired preview scope and current build evidence.
