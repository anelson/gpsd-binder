# Building gpsd-binder

If you just want to run `gpsd-binder` on an Android device, you don't need to
go through the steps below, just download a binary from the Releases page on
this Github repo, or see [INSTALLING.md](INSTALLING.md).

gpsd-binder has two Rust programs, the GNSS service and its TCP relay, cross-compiled for Android
with the Android NDK. `just` is the task runner: the recipes in `justfile` are the documented way to build, check and test, and they capture
the details that plain `cargo` commands would not. Raw `cargo` works for everything except linking
the Android binary, which needs the NDK linker that the `build` recipe sets up.

Run this to see the recipes:

```sh
just --list
```

## Build dependencies

Required to build the driver:

- `rustup`. `rust-toolchain.toml` pins the Rust version that we target, plus
  necessary components and both Android targets; rustup installs all
  of them the first time `cargo` runs in this directory.
- Android NDK r30, the version CI uses, from https://developer.android.com/ndk/downloads. On macOS the
  `.dmg` holds the NDK root at
  `AndroidNDK<build>.app/Contents/NDK`; mount the image and point `ANDROID_NDK_HOME` at that
  directory. On Linux, unpack the `.zip` and point `ANDROID_NDK_HOME` at the unpacked directory.
  The NDK ships universal macOS binaries under `toolchains/llvm/prebuilt/darwin-x86_64`, which
  the `build` recipe uses on both Intel and Apple silicon.
- `just`.

Required for the checks:

- [cgx](https://github.com/anelson/cgx). The justfile runs numerous other Rust
  tools through `cgx`, which fetches the versions named in `cgx.toml` the first
  time each is used (prebuilt binaries where they exist, a source build
  otherwise). Nothing else needs to be installed for them; `cgx --prefetch-all`
  fetches them ahead of time and `cgx --list-tools` shows what is configured.
- A nightly toolchain with `rustfmt`, for the first pass of `just fmt`.

### Installing `just` and `cgx`

On macOS with Homebrew:

```sh
brew install just
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/anelson/cgx/releases/latest/download/cgx-installer.sh | sh
rustup toolchain install nightly --profile minimal --component rustfmt
```

Alternatively, on any supported platform with a compatible Rust toolchain, you
can build `just` and `cgx` from source and install them locally with `cargo
install just cgx --locked`.

Required for running some of the unit tests:

- A Docker daemon, for the gpsd module's test against a real gpsd. Docker Desktop, OrbStack and
  Podman with a Docker-compatible socket all work; testcontainers finds the daemon through
  `DOCKER_HOST` or `/var/run/docker.sock`, so the `docker` command need not be on `PATH`. The first
  run builds a Debian image with gpsd and gpsfake (about a minute); later runs take about 5 s.
  Without a daemon that one test fails with a message saying so; the other tests still run.

Builds have been made on macOS on Apple silicon, and CI builds on Ubuntu 26.04 on x86_64. WSL has
not been tried. Native Windows builds are not supported.

## Building the driver

```sh
ANDROID_NDK_HOME=/path/to/android-ndk-r30 just build x86_64      # API level 30
ANDROID_NDK_HOME=/path/to/android-ndk-r30 just build arm64 30
```

The architecture is required; the API level defaults to 30. The recipe sets
`CARGO_TARGET_X86_64_LINUX_ANDROID_LINKER` (or the aarch64 equivalent) to the NDK's
`x86_64-linux-android30-clang` and runs `cargo build --release --target x86_64-linux-android`. API
level 30 lets the binary load on Android 11 and later; the AIDL interface it implements needs
Android 13 anyway. The release profile links with LTO and strips the binary, so a clean build takes
about 20 s and produces `gpsd-binder` and `gpsd-binder-relay` under
`target/x86_64-linux-android/release/`. Both are required; [INSTALLING.md](INSTALLING.md)
describes their install paths. The arm64 programs build the same way and have run in the
Android 13, 16 and 17 emulators, but not on a physical arm64 device.

The programs may link only against LLNDK libraries, so that one build serves several Android
releases. The recipe checks both programs with `llvm-readelf -d` and fails unless their `NEEDED`
entries are exactly `libc.so`, `libdl.so` and `liblog.so`.

While the tests can run on a Linux or macOS host, there is no point in trying to build the binary for these platforms as it's tightly coupled to Android.

## Checks

To run all checks before preparing a PR or committing changes, run `just precommit`. It internally runs the various other checks.

The following are also sometimes useful:

- `just fmt` - Applies automatic formatting to Rust and TOML files. NOTE: Requires both nightly and stable `rustfmt`
- `just fmtcheck` - Run all lints but don't actually compile a binary, so it's faster than a full build.
- `just test` - Run Rust unit tests on the host. NOTE: Some tests require Docker.

## CI and releases

CI (`.github/workflows/ci.yml`) runs on every push to master and on every pull request:
`just fmtcheck`, `just vibecheck`, `just depcheck`, `just test`, `cargo package`, and `just build`
for both architectures. It runs just and the other tools through cgx (`cgx just vibecheck`), so
it uses the versions in `cgx.toml`. Pull requests from forks run only after the maintainer has
read them and approved the run.

A release is made by pushing a tag `vX.Y.Z` on master. The tag runs
`.github/workflows/release.yml`, which builds both architectures, creates a GitHub release with
the archives that INSTALLING.md describes, and publishes the crate to crates.io.
`.agents/skills/release/SKILL.md` lists every step; an agent can follow it as the `release`
skill, and a person can follow it by hand. The release notes are the version's section of
`CHANGELOG.md`, so record each notable change under `## [Unreleased]` when you make it.

## Troubleshooting

- `linker not found: .../x86_64-linux-android30-clang`: `ANDROID_NDK_HOME` does not point at the
  NDK root. On macOS the root is inside the `.app` bundle on the mounted image.
- `cgx: command not found`: install cgx as above; all of the other tools are run using `cgx`.
- `building the gpsd image needs a running Docker daemon`: start the daemon, or set `DOCKER_HOST`
  if its socket is somewhere unusual.
