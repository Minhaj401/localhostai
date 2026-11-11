# localhost.ai — operations guide

[Back to the project overview](../README.md)

**Run AI Locally.** The localhost.ai Flutter app connects to the PrivateLM Rust
coordinator: your laptop and your phone, one private LLM. Install the terminal
binary and the APK, then pair them over your LAN.

```
  ┌──────────────────────────────┐            ┌──────────────────────────┐
  │  privateml  (this binary)    │   LAN HTTP │  localhost.ai (phone)    │
  │                              │ ◄───────── │  chat UI                 │
  │  chat UI in your terminal    │            │  + compute worker        │
  │  + embedded node:            │            │    (libggmlrpc.so)       │
  │      HTTP API, discovery,    │            │                          │
  │      engine management       │ ─────────► │  layer split over RPC    │
  └───────────┬──────────────────┘            └──────────────────────────┘
              │
        managed llama.cpp + GGUF (installed from the TUI)
```

The laptop node is **not a service you run**. It is a task inside the
`privateml` process, so `privateml` gives you a chat UI *and* makes this
machine available to your phone at the same time.

For architecture, data flow, APIs, security, resource planning, development,
and troubleshooting details, see [`PROJECT_GUIDE.md`](../PROJECT_GUIDE.md).
The Flutter visual contract is [`app/DESIGN_FLUTTER.md`](../app/DESIGN_FLUTTER.md).

## Quick start

```bash
cargo build --release
./target/release/privateml
```

On first run the model picker opens automatically. Choose a model and press
**Enter**. PrivateLM then:

1. creates `~/.privateml/` (config + pair token) on first run
2. downloads a pinned, checksummed llama.cpp runtime when needed
3. downloads and verifies the selected GGUF with resumable progress
4. starts llama-server itself and opens chat
5. starts the LAN node on the first free port at or above 8765

No Ollama process or separate model server is required. Chat with the laptop
alone, or pair the phone from **Ctrl+P**.

Starter models:

| Model | Download | License | Intended use |
|---|---:|---|---|
| Qwen 2.5 0.5B Instruct Q4_K_M | 469 MiB | Apache-2.0 | quickest first test |
| Qwen 2.5 1.5B Instruct Q4_K_M | 1.04 GiB | Apache-2.0 | recommended default |
| Qwen 2.5 3B Instruct Q4_K_M | 1.96 GiB | Qwen Research | stronger laptop/phone pool |
| Qwen 2.5 7B Instruct Q3_K_M | 3.55 GiB | Apache-2.0 | stronger general model with lower quantization |
| Qwen 2.5 Coder 7B Instruct Q4_K_M | 4.36 GiB | Apache-2.0 | coding model for larger memory pools |

The 7B options require substantially more free RAM than their download size
once context and native runtime buffers are included. PrivateLM will refuse to
start them unless the current laptop/phone plan can place every layer safely.

### Download your own models

The starter catalog is optional. Download other text-model GGUFs supported by
the bundled llama.cpp runtime directly from the TUI or Android app:

- **TUI:** press **↑**, then **B** to browse. Enter a model search, a Hugging Face
  `owner/repository`, or a direct HTTP(S) `.gguf` link. An empty search shows
  popular repositories. Use **↑/↓** and **Enter** to open a repository, choose a
  quantization, and download it. **Tab** returns to the search field.
- **Android:** open **Models → Find and download models**. Search, choose a
  repository and GGUF, then tap **Download GGUF**. Downloads are stored on the
  paired laptop and appear in both model libraries.

Hugging Face selections are pinned to the selected commit, use published SHA-256
checksums when available, and automatically download every shard of a split
GGUF. The planner counts the combined shard size. Direct links must report a
file size; their downloads resume when a checksum or HTTP validator is available,
and otherwise restart safely. Hugging Face `blob` links are accepted too.

Custom downloads and source records live in `~/.privateml/models/custom/`.
Incomplete downloads remain in the library for retry (**I** in the TUI).
Completed files are checked as GGUFs, receive stored checksums, and are registered
automatically after a restart. Select the installed model to preview memory fit
and load it; downloading alone does not replace the running model.
Custom downloads are limited to 128 GiB and 1,024 files, and start only when the
laptop has enough free disk for the remaining bytes plus 512 MiB of headroom.
Non-Hugging-Face direct links are not allowed to redirect to another address.
Use direct links only from hosts you trust; the coordinator connects to the
address supplied in the link.

For private or gated Hugging Face repositories, set `HF_TOKEN` in the laptop
coordinator's environment after obtaining repository access. The token is not
stored in the model library or sent to arbitrary download hosts.
Model browsing and every download/resume action require the pair token, even if
the node is configured to allow unauthenticated chat.

## Commands

| Command | What it does |
|---|---|
| `privateml` | node + interactive chat UI |
| `privateml --status` | print address, backend, models, pairing — then exit |
| `privateml --headless` | node only, no UI (SSH, systemd, servers) |
| `privateml --once "hello"` | one streamed reply from an already active backend |
| `privateml --port 9000` | prefer this port, trying the next free one if occupied |
| `privateml --name "studio"` | name shown on the phone |
| `privateml --require-pairing` | make the pair token mandatory |
| `privateml --no-lan` | loopback only, phone cannot connect |
| `privateml --no-discovery` | don't answer UDP broadcasts |
| `privateml --connect http://HOST:PORT` | use an existing node; also supports `--once` and `--status` |

In the UI: type → **Enter** to send · **↑** model picker · **F1** help ·
**Ctrl+A** activity · **Ctrl+X** stop generation/model · **Ctrl+D** devices ·
**Ctrl+P** pairing QR · **Esc** to quit. Set `PRIVATELM_TOKEN` when connecting
to a remote node that requires pairing.

The model picker previews fit before downloading, including estimated per-device
layers and memory. Use **←/→** for 1K–16K context sizes, **Enter** to load,
**D** to reload through the managed path, or **I** to install/verify without
loading. Every actual load re-plans from the verified GGUF and current free RAM.

Inside **Ctrl+D**, use **↑/↓** to select a device and **Space** to include or
exclude it from future model loads. The panel reports free/total RAM, shared
capacity, CPU threads/cores, worker reachability, heartbeat age, RPC address,
and the active layer/memory allocation. Excluding a device never interrupts the
current response; it takes effect the next time a model is loaded.

## Files

```
~/.privateml/config.toml   settings (node_name, port, engine, ...)
~/.privateml/pair_token    shared secret the phone must present
~/.privateml/models/       downloaded GGUF models
~/.privateml/bin/          managed llama.cpp runtime
~/.privateml/logs/
```

## Node API

OpenAI-compatible, so any client that speaks `chat/completions` works:

```
GET  /health              status, backend, models  (public)
GET  /v1/models           model list
POST /v1/chat/completions streaming (SSE) or not
GET  /pair                QR payload (local or already authenticated callers)
POST /pair/verify         confirm a token
GET/POST /peers           compute pool / worker heartbeat
POST /peers/toggle        include/exclude a device from future split plans
GET/POST /models          registered GGUFs / register a file
GET  /managed             built-in catalog, runtime state, download progress
POST /models/browse       search Hugging Face, list a repo, or inspect a GGUF link
POST /managed/download    download and register a user-selected GGUF source
POST /managed/install     download, resume, or verify a starter/custom model
GET  /cluster             live cluster state
POST /cluster/plan        plan a model split
POST /cluster/start       load a planned model
POST /cluster/stop        stop the managed processes
```

With `require_pairing = true`, chat, model, peer, and cluster requests need
`Authorization: Bearer <pair_token>`. Health remains public for discovery.
The pairing token is never served to an unauthenticated LAN caller.

```bash
curl -X POST localhost:8765/v1/chat/completions \
  -H 'Content-Type: application/json' \
  -d '{"model":"qwen2.5-1.5b-instruct-q4_k_m","stream":true,
       "messages":[{"role":"user","content":"hi"}]}'
```

## Engines

The default engine is **managed**. PrivateLM installs llama.cpp and owns its
process lifecycle; exiting the TUI stops the managed processes. Ctrl+C, SIGTERM,
and terminal hangup also clean up the model process. On Linux, a kernel
parent-death guard releases it even if the node crashes or is forcibly killed.

Pinned runtime downloads currently support Linux x86_64/ARM64 and macOS
x86_64/ARM64. Set both `PRIVATELM_LLAMA_SERVER` and `PRIVATELM_RPC_SERVER`
to use compatible custom binaries instead.

Ollama and an external llama-server remain optional compatibility backends.
Set `engine = "auto"`, `"ollama"`, or `"llamacpp"` in `config.toml` only when
you explicitly want one of them.

## Distributed mode

Model layers and KV cache split across laptop and phone RAM, so a model
bigger than either device can hold still runs. The APK ships an ARM64
`rpc-server` as `libggmlrpc.so`; switching on **"Share this phone's
compute"** makes the phone a peer. Latency is bounded by Wi-Fi; the win is
memory. The worker prefers port 50052 and automatically falls back through
50062 if an older or orphan worker already owns a port.

Select a built-in model in the terminal and press **Enter**: installation,
planning, and loading all happen there. **D** forces the same distributed
path. The phone appears in the plan after **Share this phone** is enabled.
The phone switch is the device-side consent gate; the **Ctrl+D** panel is the
coordinator-side control, so either side can refuse resource sharing.
The planner fills the laptop's safe memory budget first and sends only the
whole-layer remainder to phones, minimizing Wi-Fi traffic and phone pressure.
It also leaves 256 MiB of each remote budget for native RPC overhead. The phone
worker caches eligible weight tensors in Android's private cache so retries and
reloads can avoid transferring the same data again.

You can still drop another GGUF into `~/.privateml/models/` and restart, or
register it with `POST /models`. PrivateLM reads the real transformer block
count and attention metadata from the file before planning. Memory placement
remains an estimate; supplied layer counts must match the GGUF.

Idle workers are checked using the pinned RPC protocol, including version and
device count. Workers already used by a cluster are not repeatedly probed through
their single-client socket. If a worker disappears, the failed run is removed
from health/model listings and its error identifies the RPC endpoints.

Each model load writes a separate `~/.privateml/logs/cluster-<pid>-<run>.log`;
the exact path is shown in the error and `GET /cluster`. Model reloads account
for the previous run's releasable memory instead of requiring space for two runs.

The bundled phone worker and managed laptop runtime are both pinned to
llama.cpp `b12110`. To rebuild the worker, run
`bash scripts/build-phone-worker.sh`; `LLAMA_CPP_REF`, `LLAMA_CPP_DIR`,
`ANDROID_NDK_ROOT`, and `CMAKE` can override the inputs. The worker statically
links ggml and the C++ runtime.

### RPC reliability (0.2.2)

Phone sharing uses an Android foreground notification with CPU/Wi-Fi locks.
The worker exits when its owning app process exits, and the notification has a
stop action. Worker-session IDs and report sequence numbers prevent old
heartbeats or withdrawals from overwriting a restarted worker.

Pool probes are serialized per endpoint and excluded from model startup;
active RPC sockets remain owned by llama-server. A background monitor detects
fatal RPC output even before the child finishes crash handling. An address or
session change invalidates the old loaded buffers rather than treating the new
worker as a continuation of that run.

The reported `cluster-502128-1.log` failure occurred after a successful model
load and three replies. Its socket error proves a mid-inference transport
failure, but cannot establish whether Android killed the worker, it crashed,
or the network dropped. The updated APK preserves phone exit codes and native
output so subsequent incidents have that evidence. Update both the APK and
the laptop binary, restart sharing, and reload the model.

## Repository layout

```
core/          privateml-core — the node, as a library
  config.rs    config.toml, dirs, pair token, port selection
  discovery.rs UDP responder + probe (phone finds this machine)
  managed.rs   checksummed runtime/model catalog + resumable downloads
  model_hub.rs Hugging Face search, GGUF choices, and direct-link metadata
  server.rs    HTTP node: health, models, SSE chat, pairing
  api.rs       client used by the UI and tests
tui/           privateml — the binary: UI + node wiring
app/           Flutter APK (chat UI + compute worker)
laptop-node/   legacy Python node — superseded by core/, kept for reference
phone-rpc/     native worker lifecycle wrapper + legacy Termux reference
scripts/       llama.cpp build helpers
```

## Privacy

- History and settings live only on your machines.
- Laptop mode: prompts never leave your LAN.
- Pairing tokens are generated from `/dev/urandom`, stored `0600`.
- No telemetry, no accounts, no cloud unless you configure a cloud backend.
- Runtime/model installation contacts GitHub Releases and Hugging Face; prompts
  remain local after installation.

## Development checks

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo build --workspace
python3 scripts/check-runtime.py
```

For real inference, RPC placement, repeated pool polling, reloads, worker-loss
recovery, and shutdown cleanup with an existing GGUF:

```bash
python3 scripts/check-inference.py \
  --model /path/to/model.gguf \
  --llama-server ~/.privateml/bin/llama-b12110/llama-server \
  --rpc-server ~/.privateml/bin/llama-b12110/ggml-rpc-server
```

The RPC workers in this test run on loopback as phone stand-ins.

Inside `app/`, run `flutter analyze`, `flutter test`, and
`flutter build apk --release --target-platform android-arm64` (JDK 17).
The retained Python node has isolated checks in `laptop-node/`:
`python3 -m unittest -v test_server`.
