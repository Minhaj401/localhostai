#!/usr/bin/env bash
# Build the single executable shipped in the APK. Statically link ggml and the
# C++ runtime so Android does not need unbundled libggml*.so dependencies.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
LLAMA_REF=${LLAMA_CPP_REF:-b12110}
LLAMA_DIR=${LLAMA_CPP_DIR:-"$ROOT/target/llama.cpp-$LLAMA_REF"}
NDK=${ANDROID_NDK_ROOT:-"${ANDROID_HOME:-$HOME/tools/android-sdk}/ndk/27.2.12479018"}
BUILD_DIR=${PHONE_WORKER_BUILD_DIR:-"$ROOT/target/phone-worker-$LLAMA_REF-guarded"}
CMAKE=${CMAKE:-cmake}
if ! command -v "$CMAKE" >/dev/null 2>&1; then
    CMAKE="$HOME/tools/cmake-venv/bin/cmake"
fi
if [[ ! -f "$LLAMA_DIR/CMakeLists.txt" ]]; then
    git clone --depth 1 --branch "$LLAMA_REF" https://github.com/ggml-org/llama.cpp "$LLAMA_DIR"
fi
[[ -f "$NDK/build/cmake/android.toolchain.cmake" ]] || { printf 'Set ANDROID_NDK_ROOT to an installed Android NDK.\n' >&2; exit 1; }
"$CMAKE" -S "$ROOT/phone-rpc" -B "$BUILD_DIR" \
    -DLLAMA_CPP_DIR="$LLAMA_DIR" \
    -DCMAKE_TOOLCHAIN_FILE="$NDK/build/cmake/android.toolchain.cmake" \
    -DANDROID_ABI=arm64-v8a -DANDROID_PLATFORM=android-24 \
    -DANDROID_STL=c++_static -DCMAKE_BUILD_TYPE=Release \
    -DBUILD_SHARED_LIBS=OFF -DGGML_BACKEND_DL=OFF -DGGML_RPC=ON \
    -DGGML_NATIVE=OFF -DGGML_CPU_ARM_ARCH=armv8-a \
    -DGGML_OPENMP=OFF -DGGML_CPU_KLEIDIAI=OFF -DGGML_RPC_RDMA=OFF \
    -DLLAMA_CURL=OFF -DLLAMA_BUILD_TESTS=OFF -DLLAMA_BUILD_EXAMPLES=OFF \
    -DCMAKE_EXE_LINKER_FLAGS="-Wl,-z,max-page-size=16384"
"$CMAKE" --build "$BUILD_DIR" --target ggml-rpc-server -j"${JOBS:-2}"
"$CMAKE" -E copy_if_different "$BUILD_DIR/bin/ggml-rpc-server" \
    "$ROOT/app/android/app/src/main/jniLibs/arm64-v8a/libggmlrpc.so"
printf 'Bundled ARM64 RPC worker rebuilt from llama.cpp %s\n' "$LLAMA_REF"
