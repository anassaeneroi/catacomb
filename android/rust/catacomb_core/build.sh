#!/usr/bin/env bash
# Cross-compile the Catacomb Android JNI core to Android .so files.
#
# Portable: locates the NDK from (in order) $ANDROID_NDK_HOME, $ANDROID_NDK_ROOT,
# or the newest ndk/<ver> under $ANDROID_HOME / $ANDROID_SDK_ROOT / ~/Android/Sdk.
# Only the target linker is needed — every dependency (serde, serde_json, jni)
# is pure Rust, so there's no C compiler / archiver to configure.
#
# Usage:
#   ./build.sh                 # build arm64-v8a + x86_64 (release)
#   ./build.sh arm64-v8a       # single ABI
#   API=24 ./build.sh          # override the min-API linker (default 24)
#
# Output .so files land in target/<rust-triple>/release/libcatacomb_core.so.
set -euo pipefail
cd "$(dirname "$0")"

API="${API:-24}"

# ── Locate the NDK ──────────────────────────────────────────────────────────
find_ndk() {
    if [[ -n "${ANDROID_NDK_HOME:-}" && -d "$ANDROID_NDK_HOME" ]]; then
        echo "$ANDROID_NDK_HOME"; return
    fi
    if [[ -n "${ANDROID_NDK_ROOT:-}" && -d "$ANDROID_NDK_ROOT" ]]; then
        echo "$ANDROID_NDK_ROOT"; return
    fi
    for sdk in "${ANDROID_HOME:-}" "${ANDROID_SDK_ROOT:-}" "$HOME/Android/Sdk"; do
        [[ -n "$sdk" && -d "$sdk/ndk" ]] || continue
        # newest installed NDK version
        local ver
        ver="$(ls -1 "$sdk/ndk" | sort -V | tail -1)"
        [[ -n "$ver" ]] && { echo "$sdk/ndk/$ver"; return; }
    done
    echo "ERROR: could not locate an Android NDK. Set ANDROID_NDK_HOME." >&2
    exit 1
}

NDK="$(find_ndk)"
HOST_TAG="linux-x86_64"
[[ "$(uname -s)" == "Darwin" ]] && HOST_TAG="darwin-x86_64"
TOOLS="$NDK/toolchains/llvm/prebuilt/$HOST_TAG/bin"
echo "Using NDK: $NDK (API $API)"

# ── ABI → rust triple + clang wrapper ────────────────────────────────────────
declare -A TRIPLE=( [arm64-v8a]=aarch64-linux-android [x86_64]=x86_64-linux-android )

build_one() {
    local abi="$1" triple="${TRIPLE[$1]}"
    local linker="$TOOLS/${triple}${API}-clang"
    if [[ ! -x "$linker" ]]; then
        echo "ERROR: linker not found: $linker" >&2; exit 1
    fi
    echo "── Building $abi ($triple) ─────────────────────────────"
    # cargo derives the env var name from the uppercased triple.
    local var="CARGO_TARGET_$(echo "$triple" | tr 'a-z-' 'A-Z_')_LINKER"
    env "$var=$linker" cargo build --release --target "$triple"
    local out="target/$triple/release/libcatacomb_core.so"
    if [[ ! -f "$out" ]]; then
        echo "  ✗ expected $out was not produced" >&2; exit 1
    fi
    # Deposit into the app's ABI-specific jniLibs dir so the Kotlin
    # `System.loadLibrary("catacomb_core")` finds it at packaging time.
    local jnidir="../../app/src/main/jniLibs/$abi"
    mkdir -p "$jnidir"
    cp "$out" "$jnidir/libcatacomb_core.so"
    echo "  ✓ $out ($(du -h "$out" | cut -f1)) → $jnidir/"
}

if [[ $# -gt 0 ]]; then
    build_one "$1"
else
    build_one arm64-v8a
    build_one x86_64
fi

echo "Done."
