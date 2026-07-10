# Freenet Delegates

Standard, reusable delegates and upgrade-migration tooling for the Freenet ecosystem.

## Available Delegates

### [upgrade-assistant](./upgrade-assistant/)

A general-purpose delegate that helps ANY delegate upgrade gracefully by tracking delegate key mappings.

**Problem:** When a delegate's WASM code changes, its delegate key changes (derived from code hash). This creates a new, empty delegate storage, making data from the previous version inaccessible.

**Solution:** The Upgrade Assistant stores delegate key mappings, partitioned by origin, so delegates can find and migrate from their previous versions.

## Migration Crates

### [freenet-migrate](./freenet-migrate/)

Reusable contract/delegate **upgrade-migration machinery** for Freenet dApps.
Content-addressed identity (`blake3(wasm ‖ params)`) means any rebuild can
re-key a contract or delegate and strand user state under the old key; this
crate packages the carry-forward patterns River and Delta already ship — with
the safety preconditions (mergeable state, a fail-closed self-authorizing
`verify()` gate, a key-only release-signing identity) made mechanical. Horizon-A
step A3 of the graceful-upgrades design ([freenet-core#2776](https://github.com/freenet/freenet-core/issues/2776)).

### [freenet-migrate-build](./freenet-migrate-build/)

Build-dependency companion: parses a unified `legacy.toml` predecessor registry,
codegens the `CONTRACT_LINEAGE` / `DELEGATE_LINEAGE` consts, and provides the CI
hash-guard ("WASM hash changed ⇒ old hash must be registered").

## Building

```bash
# Build all delegates
cargo build --release --target wasm32-unknown-unknown

# Build specific delegate
cargo build --release --target wasm32-unknown-unknown -p upgrade-assistant
```

## Testing

```bash
# Run all tests
cargo test

# Run tests for specific delegate
cargo test -p upgrade-assistant
```

## License

LGPL-3.0-only
