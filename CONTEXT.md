# PrivateLM-LAN — Project Context (handoff)

Consolidated from pi session `01a1b628-6e8b-7302-8129-2cecf6013e96`
(Nov 7 2026, cwd `~`, model `free-pi/glm-5.3-flash`).

## Origin of the project

User asked (session Nov 3/7): "what does cross-platform llm client do" →
"how does it do the splitting?" → "make clone of it in which it uses the
resources of laptop also" → "do it make the apk and tui version of it in
laptop use rust for it".

Inspiration: `~/projects/cross-platform-llm-client/` (a.k.a. **PrivateLM**) —
a Flutter on-device LLM chat client.

### How PrivateLM splits work (the thing being cloned)
- **GPU/CPU layer offload** in llama.cpp: `n_gpu_layers` — how many
  transformer layers run on GPU vs CPU, via `LlamaFlutterAndroidPlugin.kt`.
- Coarse GPU eligibility gate `isGpuEligibleSoc()`: Android 12+, excludes
  Unisoc/Spreadtrum; escape hatch file `force_gguf_gpu` in app files dir.
- Driver blocklist `isBlockedGpu`: PowerVR/IMG, Mali-T (pre-2016 Midgard),
  Adreno < 640 (regex-parsed from GPU name).
- Layer split is computed from free VRAM/RAM heuristics.

## What was built (all verified working at end of session)

Repo: `~/projects/privatelm-lan/` (NOT a git repo yet).

```
laptop-node/    Python FastAPI node (runs on laptop)
  server.py      OpenAI-compatible /v1/chat/completions (SSE streaming),
                 /health, /v1/models; proxies to Ollama or llama-server
                 (auto-detected); UDP discovery responder on 50897;
                 CORSMiddleware added; serves Flutter web app at /app/;
                 /cluster/* endpoints to start/stop a llama.cpp RPC cluster
                 (accepts extra_args passthrough for model flags)
  run.sh         venv bootstrap + launch
app/            Flutter client (GetX: ui → controllers → services)
  lib/services/llm_backend.dart          LlmBackend interface
                 laptop_node_service.dart  backend "laptop" (the new one)
                 cloud_service.dart        OpenAI/Anthropic/Gemini/Kimi normalized
                 local_inference_stub.dart documented stub
                 node_discovery.dart       UDP broadcast + subnet-scan fallback
                 phone_worker_service.dart starts bundled rpc-server on :50052
                 storage_service.dart
  lib/ui/        chat_screen, nodes_screen (node picker + "Share this
                 phone's compute" toggle), settings_screen
  lib/services/inference_android.dart
  android manifest: INTERNET + ACCESS_NETWORK_STATE (release manifest was
                 missing INTERNET — that was a real bug, fixed)
  Kotlin MainActivity: method channel exposing nativeLibDir + phone LAN IP
tui/            Rust TUI (ratatui + crossterm + tokio + reqwest)
  main.rs/api.rs/app.rs/ui.rs
  `--once "prompt"` single-shot mode; ↑ model picker; Esc quits
  PRIVATELM_NODE env var or URL arg to point at another node
phone-rpc/setup-termux.sh   (legacy path, superseded by in-APK worker)
scripts/setup-distributed.sh
PrivateLM-LAN.apk   21.9MB, see below
```

## Distributed inference (llama.cpp RPC) — verified end-to-end

- llama.cpp built with RPC support from source: `ggml-rpc-server` +
  `llama-server`.
- Node `/cluster/start` launches llama-server with `--rpc <host:port>`
  peers, splitting model layers + KV cache across machines (real memory
  pooling across laptop/phone).
- **Android ARM64 rpc-server cross-compiled with NDK r27** and packaged
  into the APK as `libggmlrpc.so` in `lib/arm64-v8a/`, with
  `useLegacyPackaging = true` so it is extracted + executable.
  Android 10+ requires running it from the native lib dir (hence the
  Kotlin channel exposing `nativeLibDir`).
- Device-tier thread logic copied from PrivateLM: 6/4/3/2 threads by RAM.
- **No Termux needed** — the worker lives in the APK.

## Bugs found & fixed during the session (don't regress these)

1. Default backend URL was `127.0.0.1:8765` — wrong on a phone (that's the
   phone itself). Removed; startup now auto-discovers on LAN.
2. Missing CORS headers on the node → Chrome blocked requests. Fixed with
   `CORSMiddleware`, verified `OPTIONS` preflight → 200.
3. Release AndroidManifest had **no INTERNET permission** (flutter create
   only adds it to the debug manifest) → no socket at all. Fixed.
4. UDP broadcast dropped by many APs → added subnet-scan fallback.
5. Web build: Flutter `index.html` absolute asset paths 404 under `/app/`
   → rebuild with correct `base href`.
6. Discovery responder returned `"models": []` (asyncio.run in responder
   thread) → model list is now cached.
7. `pkill` patterns kept matching the invoking shell → kill by port
   (`fuser -k 8765/tcp`).
8. Firewall/UFW: `/etc/ufw/user.rules` initially unmodified (sudo commands
   had not applied). Later rules were added successfully.
9. YouTube-style distraction: web UI was ultimately **disabled** per user —
   backend only, since phone discovery (native sockets) worked and the
   browser build could never do UDP.

## Decisions locked in by the user

- **Network only.** No USB, no `adb reverse`, no Termux, no raw-socket-free
  browser tricks. Phone ↔ laptop over LAN.
- **One APK.** Phone compute worker ships inside the app.
- Web UI disabled — backend only.
- User asked for a **4B model** → `qwen3:4b` (Q4_K_M, 2.4GB) pulled via
  Ollama and loaded into the RPC cluster.

## State at end of that session

- Cluster **running** with `qwen3:4b` on `http://127.0.0.1:8081/v1`,
  node backend = `distributed`.
- Measured **~5.6 tok/s** through the RPC split (both "peers" were the
  laptop CPU, so RPC overhead with no extra RAM —
  expected; real speedup needs the phone peer enabled).
- Open issue: **qwen3 is a thinking model.** It emits
  `<think …>` reasoning before visible text, eating the token budget.
  Tried and failed: request-level `enable_thinking=false`,
  `--reasoning-budget 0`, `--jinja` + `chat_template_kwargs`. This
  llama.cpp build's qwen3 template ignores the flag.
- Unresolved question at hand-off: **swap to `qwen2.5:3b-instruct`**
  (non-thinking, ~3× faster feel) **or keep qwen3:4b?**

## Environment notes from that session

- Installed during session: Rust 1.99 (`~/.cargo`), Flutter 3.24.5,
  Android SDK 34, JDK 17, Android NDK r27.
- Flutter needed `~/.cargo/env` sourcing; Gradle builds can OOM when Ollama
  has a model loaded (only ~1GB RAM free at times) — stop the model or
  retry; failures were transient.
- Node default port **8765** (discovery UDP 50897); cluster llama-server on
  **8081**; phone rpc-server on **50052**.

## How to run

```bash
# 1. laptop node (must be first)
cd ~/projects/privatelm-lan/laptop-node && ./run.sh
curl http://localhost:8765/health

# 2. TUI
cd ~/projects/privatelm-lan/tui && source ~/.cargo/env
./target/release/privatelm-tui              # interactive
./target/release/privatelm-tui --once "hi"  # one shot
```

## Suggested next steps

1. Answer the open question: keep `qwen3:4b` or pull
   `qwen2.5:3b-instruct` and set it as default.
2. Enable "Share this phone's compute" in the APK so the layer split is
   real (laptop + phone RAM) and try a 7–8B model.
3. `git init` + first commit (repo currently untracked).
4. Consider a `max_tokens` / reasoning-budget guard in the node so
   thinking models don't burn the whole budget on short prompts.

---

# Productization — phase 1 (embedded node)

## The constraint that changed everything

User: **"everything should be in tui or apk, nothing else."**

So the Python node stopped being a thing anyone runs. The TUI binary now
**is** the node. Layout became a Cargo workspace:

```
core/   privateml-core (library — the node)
tui/    privateml      (binary — UI + starts the node)
```

No Python, no venv, no run.sh, no systemd unit, no second process, no
port/config drift between client and server.

## Decided: fully self-contained (option A)

The binary manages its own engine. Ollama is used if it happens to be
installed (detected), but a brand-new user should never need to install
anything else: managed llama.cpp + GGUF downloads into `~/.privateml/models`
is the target. This part is **not built yet** — currently the binary detects
an existing Ollama/llama-server.

## What exists and is verified

| Piece | State |
|---|---|
| `--status` | ✅ prints address, backend, models, pairing |
| `--headless` | ✅ node only, no UI (SSH/systemd) |
| `--once "prompt"` | ✅ streamed reply, verified live (Ollama) |
| interactive TUI | ✅ (needs a real TTY) |
| `/health` `/v1/models` `/v1/chat/completions` | ✅ SSE passthrough |
| `/pair` `/pair/verify` + `--require-pairing` | ✅ 401 without token, 200 with |
| config.toml + pair token in `~/.privateml` | ✅ 0600 token |
| port auto-selection | ✅ first free ≥ 8765 |
| UDP discovery | ✅ in-process; sandbox blocks bind but degrades |
| engine re-probe | ✅ every 15s |

Release binary: **5.4 MB**, `target/release/privateml`.

## Bugs found by running it (do not regress)

1. `fs::read("/dev/urandom")` — an endless device; hangs and eats all RAM.
   Must `File::open` + `read_exact`.
2. `/health` returns models as `["a"]`, `/v1/models` as `[{"id":"a"}]`.
   Two shapes, one parser → silently zero models. Both accepted + tested.
3. Async startup probe → `--status` and first chat saw `backend: none`.
   Probe is now synchronous before `start()` returns, then rechecks.
4. No TTY crashed with a raw `ENXIO`. Now a readable hint naming `--status`.
5. Unknown flags silently launched the UI. Now rejected with usage.
6. No-backend chat tried `:8080` and reported a bare 502. Now 503 with
   "start Ollama, PrivateLM picks it up within 15s".
7. JSON advert parse ignored `\"` escapes (caught by unit test).
   Parser now walks with escape handling.

## Still to do (next phases)

1. **Self-contained models** — download/manage llama.cpp + GGUF so no
   Ollama is required (option A, the remaining half).
2. **QR pairing** — `GET /pair` exists; the TUI needs to render a QR and
   the APK needs to scan it (kills manual IP typing for good).
3. **APK**: drop the Python-era assumptions, point at the embedded node,
   rebuild release, sign it.
4. **Delete legacy** `laptop-node/`, `phone-rpc/`, `scripts/setup-distributed.sh`
   once nothing references them.
5. **Distributed mode in Rust** — cluster/RPC orchestration currently still
   lives only in the Python file, so it must be ported to `core/`.
6. Release tooling: tag, checksums, install one-liner, licence, name check.
