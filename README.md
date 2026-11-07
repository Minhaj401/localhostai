# PrivateLM-LAN

A clone of [PrivateLM](../../cross-platform-llm-client/) (cross-platform LLM chat client) with one
big addition: **the phone can offload inference to your laptop** over the LAN.

Where PrivateLM offers *on-device* (llama.cpp) and *cloud* backends, this clone adds a third:
**Laptop node** — your laptop's CPU/GPU does the heavy token generation, the phone just relays
the stream. Same privacy property as on-device mode: nothing leaves your network.

```
┌──────────────┐  UDP discover    ┌─────────────────────────────┐
│  Flutter app │ ───────────────► │ laptop-node (this repo)     │
│  (phone/PC)  │  HTTP stream     │  OpenAI-compatible proxy    │
│              │ ◄─────────────── │  → Ollama / llama-server    │
└──────────────┘                  │  (laptop CPU/GPU)           │
        │                         └─────────────────────────────┘
        └── cloud fallback: OpenAI / Anthropic / any OpenAI-compatible API
```

## Layout

```
laptop-node/     Python FastAPI node: /health, /v1/chat/completions (SSE),
                 UDP discovery responder on :50897
tui/             Rust terminal client (ratatui) — runs on the laptop itself,
                 talks to the node (or any OpenAI-compatible server)
app/             Flutter client (GetX) — Android APK + web, same layering as
                 PrivateLM: UI → ChatController → LlmBackend implementations
```

## Rust TUI (runs on the laptop)

```bash
cd tui
cargo build --release
./target/release/privatelm-tui                    # interactive TUI
./target/release/privatelm-tui --once "hi"        # single-shot, testable
PRIVATELM_NODE=http://192.168.1.20:8765 privatelm-tui   # remote node
```

Keys: type → Enter to send · ↑ opens model picker · Esc quits.

## APK

Prebuilt: `PrivateLM-LAN.apk` (release, ~21MB). Rebuild with:

```bash
cd app && flutter build apk --release
# → build/app/outputs/flutter-apk/app-release.apk
```

## Backends (pluggable via `LlmBackend`)

| id          | Where inference runs            | Status in this repo             |
|-------------|---------------------------------|---------------------------------|
| `laptop`    | Your laptop (Ollama/llama.cpp)  | ✅ Working                       |
| `cloud`     | OpenAI-compatible API           | ✅ Working (key in Settings)     |
| `anthropic` | Anthropic Messages API          | ✅ Working                       |
| `local`     | On-device llama.cpp             | 🚧 Stub — see integration seam   |

All backends implement the same `chat()` stream, so the chat UI is
backend-agnostic — exactly like PrivateLM's `InferenceService`/`CloudService` split.

## Distributed mode — laptop + phone memory pooled (llama.cpp RPC)

Model layers and KV cache are split across devices, so you can run models
bigger than either device's RAM alone. Latency per token is bounded by the
Wi-Fi link; memory pooling is the win.

```
phone (Termux):  rpc-server :50052   ← holds its share of layers in RAM
laptop:          llama-server --rpc <phone>:50052
                 └── node proxies it as backend "distributed"
```

**Laptop (one-time):**
```bash
bash scripts/setup-distributed.sh        # builds llama-server + rpc-server
```

**Phone (Termux alternative — the APK has this built in):**

The APK ships a bundled ARM64 `rpc-server` (`libggmlrpc.so`). In the app:
**laptop icon → "Share this phone's compute" → switch on**. The worker
listens on :50052 and the laptop auto-discovers it. Keep the app open
(Android pauses background processes). Only use the Termux script if you
want the worker without running the app:

```bash
pkg install git cmake clang make -y
bash phone-rpc/setup-termux.sh build     # ~10-20 min
bash phone-rpc/setup-termux.sh run       # prints phone IP:50052
```

**Start a distributed run (laptop):**
```bash
curl -X POST localhost:8765/cluster/start -H 'Content-Type: application/json' \
  -d '{"model": "/path/to/model.gguf", "ctx": 4096}'
# peers auto-discovered by scanning the /24 on :50052
```

`GET /cluster` lists detected RPC peers; `POST /cluster/stop` returns to the
local backend. While a cluster is running, the node's backend is
"distributed" — the app and TUI need no changes.

Layer split is automatic (llama.cpp's scheduler balances by free memory per
host). Threads per device follow PrivateLM's device-tier idea — see
`phone-rpc/setup-termux.sh`.

## Run the laptop node

```bash
cd laptop-node
./run.sh                    # first run creates .venv and installs deps
```

Requirements: Python 3.10+, and one of:
- **Ollama** — `ollama serve` (auto-detected at `127.0.0.1:11434`)
- **llama.cpp** — `llama-server --port 8080` (auto-detected)

Env overrides: `PRIVATELM_PORT`, `PRIVATELM_BACKEND` (`ollama|llamacpp|auto`),
`PRIVATELM_NODE_NAME`, `PRIVATELM_OLLAMA_URL`, `PRIVATELM_LLAMACPP_URL`,
`PRIVATELM_SERVE_WEB=1` (optionally serve the Flutter web build at `/app/`).

Verify: `curl http://localhost:8765/health`

## Run the app

```bash
cd app
flutter pub get
flutter run                 # Android, iOS, macOS, or web
```

No Flutter SDK? The app targets the same platforms as PrivateLM
(Android/iOS/web); the `local` backend is intentionally stubbed — wire
llama.cpp into `lib/services/local_inference_stub.dart` (the file documents
the conditional-import seam) if you want on-device too.

In the app:
1. Tap the **laptop icon** → nodes are discovered automatically (UDP broadcast).
2. If your Wi-Fi blocks broadcast, use **Add node manually** with `http://<laptop-ip>:8765`.
3. Pick a model, chat. Replies stream token-by-token from the laptop.

## Privacy model

- Chat history + settings: local Hive box, never synced.
- Laptop mode: traffic stays on your LAN; the laptop sees prompts, nobody else does.
- Cloud mode: keys stored locally, sent only to the provider endpoint.

## What's deliberately different from PrivateLM

- Laptop node instead of (not instead of — *in addition to*) on-device GGUF.
- Discovery: UDP broadcast responder/client, so no manual IP typing on the phone.
- No Firebase/background services yet; the task-management view isn't cloned.
