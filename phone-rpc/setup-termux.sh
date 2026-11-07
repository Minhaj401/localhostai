#!/data/data/com.termux/files/usr/bin/bash
# PrivateLM-LAN: turn this Android phone into a llama.cpp RPC worker.
# The laptop then splits model layers across phone + laptop memory/CPU.
#
# One-time setup (in Termux):
#   pkg install git cmake clang make -y
#   bash setup-termux.sh build     # ~10-20 min on a phone
# Every session:
#   bash setup-termux.sh run       # serves RPC on :50052
set -e
cd ~
LLAMA_DIR=~/llama.cpp
PORT=50052

# Device tier — same idea as PrivateLM's DeviceInfoService: scale threads
# to the SoC instead of hardcoding.
TOTAL_MB=$(grep MemTotal /proc/meminfo | awk '{print int($2/1024)}')
AVAIL_MB=$(grep MemAvailable /proc/meminfo | awk '{print int($2/1024)}')
CORES=$(nproc)
if   [ "$TOTAL_MB" -ge 12000 ]; then TIER=ultra; THREADS=6
elif [ "$TOTAL_MB" -ge 8000 ];  then TIER=high;  THREADS=4
elif [ "$TOTAL_MB" -ge 4000 ];  then TIER=mid;   THREADS=3
else                                 TIER=low;   THREADS=2
fi
echo "[phone-rpc] tier=$TIER ram=${TOTAL_MB}MB (free ${AVAIL_MB}MB) cores=$CORES threads=$THREADS"

case "${1:-run}" in
  build)
    pkg install -y git cmake clang make
    [ -d $LLAMA_DIR ] || git clone --depth 1 https://github.com/ggml-org/llama.cpp
    cd $LLAMA_DIR
    cmake -B build-termux -DGGML_RPC=ON -DGGML_NATIVE=ON \
          -DLLAMA_CURL=OFF -DCMAKE_BUILD_TYPE=Release
    cmake --build build-termux --target rpc-server -j"$THREADS"
    echo "[phone-rpc] build complete: $LLAMA_DIR/build-termux/bin/rpc-server"
    ;;
  run)
    RPC=$LLAMA_DIR/build-termux/bin/ggml-rpc-server
    [ -x "$RPC" ] || { echo "run 'bash setup-termux.sh build' first"; exit 1; }
    IP=$(ip route get 1.1.1.1 2>/dev/null | awk '{print $7; exit}')
    echo "[phone-rpc] RPC server on $IP:$PORT  (tell the laptop this address)"
    exec "$RPC" --host 0.0.0.0 --port "$PORT" -t "$THREADS"
    ;;
  *)
    echo "usage: bash setup-termux.sh [build|run]"
    ;;
esac
