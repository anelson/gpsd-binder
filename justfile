# The tools other than cargo and rustfmt run through cgx, which fetches the versions named in cgx.toml
# on first use.

# Format Rust and TOML. The nightly rustfmt pass applies rustfmt-nightly.toml (import grouping, comment
# wrapping, and errors on lines it cannot format); the stable pass then applies the standard formatting,
# which does not undo the nightly changes.
fmt:
    cargo +nightly fmt -- --config-path rustfmt-nightly.toml
    cargo fmt
    cgx taplo fmt

# Fail if anything is not formatted. For CI and precommit checks.
fmtcheck:
    cargo fmt --check
    cgx taplo fmt --check

# Structural lints that clippy cannot express. Rules live in .ast-grep/rules, their self-tests in
# .ast-grep/rule-tests.
ast-grep:
    cgx ast-grep test --skip-snapshot-tests
    cgx ast-grep scan

# Spell-check source, comments, docs and text files. Overrides go in typos.toml.
typos:
    cgx typos

# Parse the host installers and Android shell scripts without running them.
shell-syntax:
    for script in android/*.sh android/sepolicy/*.sh waydroid/*.sh; do sh -n "$script"; done

# Compile, clippy and doc checks, for the host (where the unit tests run) and for the Android targets.
# This must be completely clean before a change is presented for review.
vibecheck: ast-grep typos shell-syntax
    cargo check --all-targets
    cargo clippy --all-targets -- -D warnings
    cargo check --target x86_64-linux-android
    cargo clippy --target x86_64-linux-android -- -D warnings
    cargo clippy --target aarch64-linux-android -- -D warnings
    RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --document-private-items

# Security advisories, licences and duplicate versions (cargo-deny), unused dependencies (cargo-machete).
# cargo-machete is run by crate name: as `cgx cargo machete` it takes the subcommand word for a path,
# because cgx does not set the CARGO variable that cargo-machete uses to recognize that it was started
# by cargo.
depcheck:
    cgx cargo deny check
    cgx cargo-machete --with-metadata

# The unit tests run on the host. The gpsd module's container test needs a Docker daemon.
test:
    cargo test

# Release build for Android: `just build x86_64` or `just build arm64`, linked for Android API level
# `api`. Needs ANDROID_NDK_HOME; see BUILDING.md. The NDK's clang for the target does the linking.
# The recipe fails if either program needs any library other than libc.so, libdl.so and liblog.so.
[unix]
build arch api="30":
    #!/usr/bin/env bash
    set -euo pipefail
    case "{{arch}}" in
        x86_64) triple=x86_64-linux-android ;;
        arm64)  triple=aarch64-linux-android ;;
        *) echo "unknown architecture: {{arch}} (use x86_64 or arm64)" >&2; exit 2 ;;
    esac
    case "$(uname -s)" in
        Darwin) host=darwin-x86_64 ;;   # the NDK ships universal binaries under this name
        Linux)  host=linux-x86_64 ;;
        *) echo "unsupported build host: $(uname -s)" >&2; exit 2 ;;
    esac
    ndk="${ANDROID_NDK_HOME:?set ANDROID_NDK_HOME to the NDK root directory}"
    bin="$ndk/toolchains/llvm/prebuilt/$host/bin"
    linker="$bin/${triple}{{api}}-clang"
    [ -x "$linker" ] || { echo "linker not found: $linker" >&2; exit 1; }
    if ! rustup target list --installed | grep -qx "$triple"; then
        echo "Rust target $triple is not installed; run: rustup target add $triple" >&2
        exit 1
    fi
    cd "{{justfile_directory()}}"
    export "CARGO_TARGET_$(echo "$triple" | tr 'a-z-' 'A-Z_')_LINKER=$linker"
    cargo build --release --target "$triple"
    for program in gpsd-binder gpsd-binder-relay; do
        binary="target/$triple/release/$program"
        needed="$("$bin/llvm-readelf" -d "$binary" | sed -n 's/.*(NEEDED).*\[\(.*\)\]$/\1/p' | sort | tr '\n' ' ')"
        if [ "$needed" != "libc.so libdl.so liblog.so " ]; then
            echo "$binary needs libraries other than libc.so, libdl.so and liblog.so: $needed" >&2
            exit 1
        fi
    done
    echo "built: $(pwd)/target/$triple/release/gpsd-binder"
    echo "  install as /vendor/bin/hw/android.hardware.gnss-service.gpsd-binder"
    echo "built: $(pwd)/target/$triple/release/gpsd-binder-relay"
    echo "  install as /vendor/bin/gpsd-binder-relay"

precommit: fmt vibecheck depcheck test
