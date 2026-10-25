# Format Rust and TOML. The nightly rustfmt pass applies rustfmt-nightly.toml (import grouping, comment
# wrapping, and errors on lines it cannot format); the stable pass then applies the standard formatting,
# which does not undo the nightly changes.
fmt:
    cargo +nightly fmt -- --config-path rustfmt-nightly.toml
    cargo fmt
    taplo fmt

# Fail if anything is not formatted. For CI and precommit checks.
fmtcheck:
    cargo fmt --check
    taplo fmt --check

# Structural lints that clippy cannot express. Rules live in .ast-grep/rules, their self-tests in
# .ast-grep/rule-tests.
ast-grep:
    ast-grep test --skip-snapshot-tests
    ast-grep scan

# Spell-check source, comments, docs and text files. Overrides go in typos.toml.
typos:
    typos

# Compile, clippy and doc checks, for the host (where the unit tests run) and for the Android target.
# This must be completely clean before a change is presented for review.
vibecheck: ast-grep typos
    cargo check --all-targets
    cargo clippy --all-targets -- -D warnings
    cargo check --target x86_64-linux-android
    cargo clippy --target x86_64-linux-android -- -D warnings
    RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --document-private-items

# Security advisories, licences and duplicate versions (cargo-deny), unused dependencies (cargo-machete).
depcheck:
    cargo deny check
    cargo machete --with-metadata

# The unit tests run on the host. The gpsd module's container test needs a Docker daemon.
test:
    cargo test

# Release build for Android. Needs ANDROID_NDK_HOME; see README.md.
build arch="x86_64" api="30":
    scripts/build.sh {{arch}} {{api}}

precommit: fmt vibecheck depcheck test
