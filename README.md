# PrivateLM

**Your laptop and your phone, one private LLM.** No cloud, no Python, no
config files to hand-edit. Two things to install: this binary and the app.

```
  ┌──────────────────────────────┐            ┌──────────────────────────┐
  │  privateml  (this binary)    │   LAN HTTP │  PrivateLM APK (phone)   │
  │                              │ ◄───────── │  chat UI                 │
  │  chat UI in your terminal    │            │  + compute worker        │
  │  + embedded node:            │            │    (libggmlrpc.so)       │
  │      HTTP API, discovery,    │            │                          │
  │      engine management       │ ─────────► │  layer split over RPC    │
  └───────────┬──────────────────┘            └──────────────────────────┘
              │
        Ollama / llama.cpp already on your machine
```

The laptop node is **not a service you run**. It is a task inside the
`privateml` process, so `privateml` gives you a chat UI *and* makes this
machine available to your phone at the same time.

## Quick start

```bash
cargo build --release
./target/release/privateml
```

That's it. It:

1. creates `~/.privateml/` (config + pair token) on first run
2. detects Ollama or llama.cpp already running on this machine
3. starts the LAN node on the first free port at or above 8765
4. opens the chat UI

Chat with your laptop alone, or open the phone app and it appears in the
node list automatically.

## Commands

| Command | What it does |
|---|---|
| `privateml` | node + interactive chat UI |
| `privateml --status` | print address, backend, models, pairing — then exit |
| `privateml --headless` | node only, no UI (SSH, systemd, servers) |
| `privateml --once "hello"` | one streamed reply, then exit |
| `privateml --port 9000` | force the listen port |
| `privateml --name "studio"` | name shown on the phone |
| `privateml --require-pairing` | make the pair token mandatory |
| `privateml --no-lan` | loopback only, phone cannot connect |
| `privateml --no-discovery` | don't answer UDP broadcasts |

In the UI: type → **Enter** to send · **↑** model picker · **Esc** to quit.

## Files

```
~/.privateml/config.toml   settings (node_name, port, engine, ...)
~/.privateml/pair_token    shared secret the phone must present
~/.privateml/models/       downloaded GGUF models
~/.privateml/logs/
```

## Node API

OpenAI-compatible, so any client that speaks `chat/completions` works:

```
GET  /health              status, backend, models  (public)
GET  /v1/models           model list
POST /v1/chat/completions streaming (SSE) or not
GET  /pair                token for the QR code
POST /pair/verify         confirm a token
```

With `require_pairing = true` every `/v1` request needs
`Authorization: Bearer <pair_token>`.

```bash
curl -X POST localhost:8765/v1/chat/completions \
  -H 'Content-Type: application/json' \
  -d '{"model":"qwen2.5:0.5b","stream":true,
       "messages":[{"role":"user","content":"hi"}]}'
```

## Engines

The node uses whatever is already on the machine, in this order:

1. **managed** — a llama-server PrivateLM started itself
2. **ollama** — auto-detected on `127.0.0.1:11434`
3. **llamacpp** — auto-detected on `127.0.0.1:8080`

Override in `config.toml` with `engine` and `ollama_url` / `llamacpp_url`.

## Distributed mode

Model layers and KV cache split across laptop and phone RAM, so a model
bigger than either device can hold still runs. The APK ships an ARM64
`rpc-server` as `libggmlrpc.so`; switching on **"Share this phone's
compute"** makes the phone a peer. Latency is bounded by Wi-Fi; the win is
memory.

## Repository layout

```
core/          privateml-core — the node, as a library
  config.rs    config.toml, dirs, pair token, port selection
  discovery.rs UDP responder + probe (phone finds this machine)
  server.rs    HTTP node: health, models, SSE chat, pairing
  api.rs       client used by the UI and tests
tui/           privateml — the binary: UI + node wiring
app/           Flutter APK (chat UI + compute worker)
laptop-node/   legacy Python node — superseded by core/, kept for reference
phone-rpc/     legacy Termux script — superseded by the in-APK worker
scripts/       llama.cpp build helpers
```

## Privacy

- History and settings live only on your machines.
- Laptop mode: prompts never leave your LAN.
- Pairing tokens are generated from `/dev/urandom`, stored `0600`.
- No telemetry, no accounts, no cloud unless you configure a cloud backend.
