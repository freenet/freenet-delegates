# Freenet Delegates

A collection of standard, useful delegates for the Freenet ecosystem.

## Available Delegates

### [upgrade-assistant](./upgrade-assistant/)

A general-purpose delegate that helps ANY delegate upgrade gracefully by tracking delegate key mappings.

**Problem:** When a delegate's WASM code changes, its delegate key changes (derived from code hash). This creates a new, empty delegate storage, making data from the previous version inaccessible.

**Solution:** The Upgrade Assistant stores delegate key mappings, partitioned by origin, so delegates can find and migrate from their previous versions.

## Building

The committed WASM artifacts are the delegates' on-network addresses, so they
must be built the one canonical way:

```bash
scripts/build-wasm.sh
```

This pins the toolchain (`rust-toolchain.toml`), builds `--locked` against the
committed `Cargo.lock`, strips machine-specific absolute paths out of the binary,
and refreshes `upgrade-assistant/wasm/upgrade_assistant.wasm`.

Do not use a bare `cargo build` to produce an artifact you intend to commit: it
skips the path remapping and produces a WASM only your machine can reproduce.
CI (`.github/workflows/check-wasm.yml`) rebuilds and diffs on every pull request,
so an artifact built the wrong way turns the build red.

## Testing

```bash
# Run all tests
cargo test

# Run tests for specific delegate
cargo test -p upgrade-assistant
```

## License

LGPL-3.0-only
