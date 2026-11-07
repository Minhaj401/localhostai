#!/usr/bin/env bash
# Starts the PrivateLM-LAN node. Requires a local inference engine:
#   - Ollama (ollama serve)  — auto-detected
#   - llama-server (llama.cpp) on :8080 — auto-detected
set -euo pipefail
cd "$(dirname "$0")"

if [ ! -d .venv ]; then
  python3 -m venv .venv
  ./.venv/bin/pip install -q -r requirements.txt
fi
exec ./.venv/bin/python server.py
