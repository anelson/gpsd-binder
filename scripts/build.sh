#!/usr/bin/env bash
# Build gpsd-binder for Android with the Android NDK.
#
#   ANDROID_NDK_HOME=/path/to/android-ndk-r27c scripts/build.sh [x86_64|arm64] [api-level]
#
# Tested on 2026-10-25: macOS arm64 host, NDK r27c, Rust 1.89.0, x86_64, API 30.
# arm64 is untested. Needs: rustup target add x86_64-linux-android (or aarch64-linux-android).
set -euo pipefail
arch=${1:-x86_64}
api=${2:-30}
: "${ANDROID_NDK_HOME:?set ANDROID_NDK_HOME to the NDK root directory}"
case "$arch" in
    x86_64) triple=x86_64-linux-android ;;
    arm64)  triple=aarch64-linux-android ;;
    *) echo "unknown architecture: $arch (use x86_64 or arm64)" >&2; exit 2 ;;
esac
case "$(uname -s)" in
    Darwin) host=darwin-x86_64 ;;   # NDK ships universal binaries under this name
    Linux)  host=linux-x86_64 ;;
    *) echo "unsupported build host: $(uname -s)" >&2; exit 2 ;;
esac
linker="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/$host/bin/${triple}${api}-clang"
[ -x "$linker" ] || { echo "linker not found: $linker" >&2; exit 1; }
var="CARGO_TARGET_$(echo "$triple" | tr 'a-z-' 'A-Z_')_LINKER"
export "$var=$linker"
cd "$(dirname "$0")/.."
cargo build --release --target "$triple"
echo "built: $(pwd)/target/$triple/release/gpsd-binder"
echo "install as: /vendor/bin/hw/android.hardware.gnss-service.gpsd-binder"
