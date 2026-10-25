# Building gpsd-binder

If you just want to run `gpsd-binder` on an Android device, you don't need to
go through the steps below, just download a binary from the Releases page on
this Github repo, or see [INSTALLING.md](INSTALLING.md).

gpsd-binder is a Rust program cross-compiled for Android with the Android NDK. `just` is the task
runner: the recipes in `justfile` are the documented way to build, check and test, and they capture
the details that plain `cargo` commands would not. Raw `cargo` works for everything except linking
the Android binary, which needs the NDK linker that the `build` recipe sets up.

Run this to see the recipes:

```sh
just --list
```

## Build dependencies

Required to build the driver:

- `rustup`. `rust-toolchain.toml` pins the Rust version that we target, plus
  necessary components and the `x86_64-linux-android` target; rustup installs all
  of them the first time `cargo` runs in this directory.
- Android NDK r27c, from https://developer.android.com/ndk/downloads. On macOS the
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

The build host that has been used is macOS on Apple silicon. The `build` recipe is a POSIX shell
script that also knows the Linux NDK layout, so it should work on Linux, presumably including WSL,
but neither has been tried. Native Windows builds are not supported.

## Building the driver

```sh
ANDROID_NDK_HOME=/path/to/android-ndk-r27c just build x86_64      # API level 30
ANDROID_NDK_HOME=/path/to/android-ndk-r27c just build arm64 30    # untested
```

The architecture is required; the API level defaults to 30. The recipe sets
`CARGO_TARGET_X86_64_LINUX_ANDROID_LINKER` (or the aarch64 equivalent) to the NDK's
`x86_64-linux-android30-clang` and runs `cargo build --release --target x86_64-linux-android`. API
level 30 lets the binary load on Android 11 and later; the AIDL interface it implements needs
Android 13 anyway. The release profile links with LTO and strips the binary, so a clean build takes
about 20 s and gives a position-independent executable of about 1 MB at
`target/x86_64-linux-android/release/gpsd-binder`. INSTALLING.md describes how to install this.
`rust-toolchain.toml` installs the x86_64 target; for arm64, run
`rustup target add aarch64-linux-android` first.

The binary may link only against LLNDK libraries, so that one build serves several Android
releases. After every build, check that nothing else crept in:

```sh
$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/darwin-x86_64/bin/llvm-readelf -d \
    target/x86_64-linux-android/release/gpsd-binder | grep NEEDED
```

The output must list only `libc.so` and `libdl.so`.

While the tests can run on a Linux or macOS host, there is no point in trying to build the binary for these platforms as it's tightly coupled to Android.

## Checks

To run all checks before preparing a PR or committing changes, run `just precommit`. It internally runs the various other checks.

The following are also sometimes useful:

- `just fmt` - Applies automatic formatting to Rust and TOML files. NOTE: Requires both nightly and stable `rustfmt`
- `just fmtcheck` - Run all lints but don't actually compile a binary, so it's faster than a full build.
- `just test` - Run Rust unit tests on the host. NOTE: Some tests require Docker.

## Troubleshooting

- `linker not found: .../x86_64-linux-android30-clang`: `ANDROID_NDK_HOME` does not point at the
  NDK root. On macOS the root is inside the `.app` bundle on the mounted image.
- `cgx: command not found`: install cgx as above; all of the other tools are run using `cgx`.
- `building the gpsd image needs a running Docker daemon`: start the daemon, or set `DOCKER_HOST`
  if its socket is somewhere unusual.
