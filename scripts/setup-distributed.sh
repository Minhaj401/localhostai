#!/bin/bash
# Builds llama.cpp with RPC support on the laptop (one-time, ~5-15 min).
# RPC is what lets model layers be split across laptop + phone memory.
set -euo pipefail
cd "$HOME/tools"
[ -d llama.cpp ] || git clone --depth 1 https://github.com/ggml-org/llama.cpp
cd llama.cpp
command -v cmake >/dev/null || { python3 -m venv "$HOME/tools/cmake-venv" 2>/dev/null || true; "$HOME/tools/cmake-venv/bin/pip" install -q cmake; export PATH="$HOME/tools/cmake-venv/bin:$PATH"; }
cmake -B build -DGGML_RPC=ON -DGGML_NATIVE=ON -DLLAMA_CURL=OFF -DCMAKE_BUILD_TYPE=Release
cmake --build build --target llama-server ggml-rpc-server -j2
echo "── done ──"
echo "llama-server: $PWD/build/bin/llama-server"
echo "rpc-server:   $PWD/build/bin/ggml-rpc-server"
