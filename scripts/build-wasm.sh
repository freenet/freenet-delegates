#!/usr/bin/env bash
# Reproducibly build upgrade_assistant.wasm and refresh the committed artifact.
#
# The delegate key is BLAKE3(BLAKE3(wasm) || params), so the compiled bytes ARE
# the delegate's on-network address. "Which bytes does this source produce" is
# therefore a correctness question, not build hygiene: any change to the bytes
# moves the address and strands whatever was stored under the old one.
#
# Three things have to be nailed down for the answer to be the same everywhere:
#
#   1. Cargo.lock is committed. Without it every checkout re-resolves the
#      dependency tree, and an upstream release nobody in this repo asked for
#      changes the bytes. This is not hypothetical here: at the time the
#      lockfile was added, a plain `cargo build` resolved freenet-macros 0.1.3
#      (a patch bump inside the `^0.1.0-rc1` requirement that freenet-stdlib
#      0.1.30 declares) and the workspace did not compile at all.
#
#   2. rust-toolchain.toml pins the compiler. Different rustc, different codegen,
#      different key.
#
#   3. --remap-path-prefix strips machine-specific absolute paths. `panic!` and
#      friends embed `file!()`, which for registry and sysroot crates is an
#      absolute path under $CARGO_HOME / $RUSTUP_HOME. Those strings are ordinary
#      rodata and `profile.release.strip = true` does NOT remove them, so without
#      remapping the same source built on two machines produces two different
#      delegate keys. `-Ztrim-paths` would be the tidy fix but is not stable, so
#      this remaps by hand. The remap targets are fixed placeholders, identical
#      everywhere, which is the entire point.
#
# Run from anywhere; paths resolve relative to this script.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
workspace="$(cd "$here/.." && pwd)"
cargo_home="${CARGO_HOME:-$HOME/.cargo}"
rustup_home="${RUSTUP_HOME:-$HOME/.rustup}"

# Order matters: rustc applies the first matching prefix, so remap the most
# specific (the checkout) before the broader home directories.
#
# Appended, not assigned, so a caller's RUSTFLAGS still apply. Anyone who sets
# their own gets different bytes and a different key -- unavoidable, and the
# reason the canonical build is this script with nothing else set.
export RUSTFLAGS="\
--remap-path-prefix=$workspace=/freenet-delegates \
--remap-path-prefix=$cargo_home=/cargo \
--remap-path-prefix=$rustup_home=/rustup \
${RUSTFLAGS:-}"

cd "$workspace"

# --locked: the artifact's hash IS the delegate's address, and the dependency
# requirements are caret requirements, so letting cargo re-resolve would silently
# change the address. Fail loudly on a stale lockfile instead.
cargo build --locked --release --target wasm32-unknown-unknown -p upgrade-assistant

built="$workspace/target/wasm32-unknown-unknown/release/upgrade_assistant.wasm"
dest="$workspace/upgrade-assistant/wasm/upgrade_assistant.wasm"

# Fail loudly rather than silently shipping a machine-specific binary.
#
# `grep -a` on the binary rather than `strings`: this check is the only thing
# standing between an unreproducible build and a publish, and gating it on
# binutils being installed would mean it silently does not run on the one
# machine that lacks it. grep is everywhere.
for leaked_dir in "$cargo_home" "$rustup_home" "$workspace"; do
    if grep -a -q -F "$leaked_dir" "$built"; then
        echo "ERROR: absolute path(s) from $leaked_dir are embedded in the WASM." >&2
        echo "The delegate key is therefore machine-specific: this build cannot be" >&2
        echo "reproduced elsewhere. Check that RUSTFLAGS was not overridden." >&2
        exit 1
    fi
done

cp "$built" "$dest"
echo "refreshed $dest"
# Printed so a CI log and a local build can be compared by eye, which is the
# only direct evidence that the reproducibility fix is still holding. This is
# the code hash only; the delegate key is BLAKE3(code_hash || params) and the
# parameters are supplied at publish time, not by this build.
if command -v b3sum >/dev/null 2>&1; then
    echo "delegate code_hash: $(b3sum --no-names "$dest")"
fi
