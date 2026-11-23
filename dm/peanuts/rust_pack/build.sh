#!/bin/sh
# Builds libpn_pack.so (dm/peanuts/rust_pack) into an on-device packer stage: <stage>/lib/libpn_pack.so.
# Usage: build.sh <stage dir>
#   ANDROID_NDK_HOME must point to the Android NDK.
set -eu
S=${1:?usage: build.sh <stage dir>}
HERE=$(cd "$(dirname "$0")" && pwd)
[ -d "$S/lib" ] || { echo "[RED] $S has no lib/: run android/ondevice/build.sh first"; exit 1; }
[ -n "${ANDROID_NDK_HOME:-}" ] || { echo "[RED] set ANDROID_NDK_HOME"; exit 1; }
command -v cargo >/dev/null || { echo "[RED] cargo is required to build libpn_pack.so"; exit 1; }
BIN=$(ls -d "$ANDROID_NDK_HOME"/toolchains/llvm/prebuilt/*/bin | head -1)
PATH="$BIN:$PATH" \
CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$BIN/aarch64-linux-android33-clang" \
CC_aarch64_linux_android="$BIN/aarch64-linux-android33-clang" \
AR_aarch64_linux_android="$BIN/llvm-ar" \
cargo build --manifest-path "$HERE/Cargo.toml" --release --target aarch64-linux-android
cp "$HERE/target/aarch64-linux-android/release/libpn_pack.so" "$S/lib/libpn_pack.so"
"$BIN/llvm-strip" --strip-debug "$S/lib/libpn_pack.so"
ls -l "$S/lib/libpn_pack.so"
