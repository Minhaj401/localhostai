# localhost.ai-LAN — Project Context (handoff)

Consolidated from pi session `01a1b628-6e8b-7302-8129-2cecf6013e96`
(Nov 7 2026, cwd `~`, model `free-pi/glm-5.3-flash`).

## Origin of the project

User asked (session Nov 3/7): "what does cross-platform llm client do" →
"how does it do the splitting?" → "make clone of it in which it uses the
resources of laptop also" → "do it make the apk and tui version of it in
laptop use rust for it".

Inspiration: `~/projects/cross-platform-llm-client/` (a.k.a. **localhost.ai**) —
a Flutter on-device LLM chat client.

### How localhost.ai splits work (the thing being cloned)
- **GPU/CPU layer offload** in llama.cpp: `n_gpu_layers` — how many
  transformer layers run on GPU vs CPU, via `LlamaFlutterAndroidPlugin.kt`.
- Coarse GPU eligibility gate `isGpuEligibleSoc()`: Android 12+, excludes
  Unisoc/Spreadtrum; escape hatch file `force_gguf_gpu` in app files dir.
- Driver blocklist `isBlockedGpu`: PowerVR/IMG, Mali-T (pre-2016 Midgard),
  Adreno < 640 (regex-parsed from GPU name).
- Layer split is computed from free VRAM/RAM heuristics.

## What was built (all verified working at end of session)

Repo: `~/projects/localhostai/` (NOT a git repo yet).

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
  LOCALHOSTAI_NODE env var or URL arg to point at another node
phone-rpc/setup-termux.sh   (legacy path, superseded by in-APK worker)
scripts/setup-distributed.sh
localhost.ai-LAN.apk   21.9MB, see below
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
- Device-tier thread logic copied from localhost.ai: 6/4/3/2 threads by RAM.
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
cd ~/projects/localhostai/laptop-node && ./run.sh
curl http://localhost:8765/health

# 2. TUI
cd ~/projects/localhostai/tui && source ~/.cargo/env
./target/release/localhostai-tui              # interactive
./target/release/localhostai-tui --once "hi"  # one shot
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
core/   localhostai-core (library — the node)
tui/    localhostai      (binary — UI + starts the node)
```

No Python, no venv, no run.sh, no systemd unit, no second process, no
port/config drift between client and server.

## Decided: fully self-contained (option A)

The binary manages its own engine. Ollama is used if it happens to be
installed (detected), but a brand-new user should never need to install
anything else: managed llama.cpp + GGUF downloads into `~/.localhostai/models`
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
| config.toml + pair token in `~/.localhostai` | ✅ 0600 token |
| port auto-selection | ✅ first free ≥ 8765 |
| UDP discovery | ✅ in-process; sandbox blocks bind but degrades |
| engine re-probe | ✅ every 15s |

Release binary: **5.4 MB**, `target/release/localhostai`.

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
   "start Ollama, localhost.ai picks it up within 15s".
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

---

# Productization — phase 2 (split planning, devices, QR)

## Built and verified

| Feature | Where | Verified how |
|---|---|---|
| RAM/cores/threads probe | `core/hardware.rs` | real: 7720 MB → 6320 MB budget, 3 threads |
| Layer split planner | `core/plan.rs` | unit tests + live `19/36L + 17/36L` plan |
| Cluster manager | `core/cluster.rs` | spawned real rpc-server + llama-server, chat returned `SPLIT WORKS` |
| Device pool API | `GET/POST /peers` | real worker usable, fake peer excluded |
| Model registry | `GET/POST /models` | registered a real 2.4 GB GGUF |
| `POST /cluster/plan` `/start` `/stop` | `core/server.rs` | live start → running → inference |
| QR pairing | `core/qr.rs`, TUI `p` | OpenCV decoded the live URI exactly |
| Model picker + pool | TUI `Up` | pool size shown, `D` runs distributed |
| Device panel | TUI `d` | shows budget/threads and "NOT counted" reasons |
| `--connect <URL>` | TUI flag | attaches to a running node |

## Critical honesty rules (learned the hard way)

1. **Never count memory you cannot reach.** A peer is only usable if its RPC
   worker answers. Earlier a fabricated peer was counted and inflated the
   pool; that is now impossible — `usable` is always a live probe result.
2. **Stale ≠ dead.** Heartbeat age and reachability are separate facts and
   are reported separately. A live worker with an old heartbeat keeps its
   memory; a silent one does not.
3. **Never hand-write a QR encoder.** The first one looked correct and failed
   to scan. Use a maintained library and verify with a real decoder.

## What is NOT done yet

1. **The APK does not speak this protocol yet.** It must: scan the QR, store
   the token, `POST /peers` with its RAM/cores on join, start its bundled
   rpc-server, and heartbeat every ~10s so it doesn't go stale. Until then the
   pool is laptop-only in practice.
2. **Ollama-only models cannot be split.** Distributed mode needs a GGUF
   path; `ollama` blobs are not registered with `/models` automatically.
   Needs either a GGUF export step or a model downloader (the "self-contained"
   half of option A).
3. Model layer counts are estimated from parameter count unless the caller
   supplies them; reading GGUF metadata (n_layer) would make plans exact.
4. Still to delete: `laptop-node/`, `phone-rpc/`, `scripts/setup-distributed.sh`.
5. Release hygiene: tag, checksums, licence, install one-liner.

---

# Productization — phase 3 (self-contained TUI)

The normal product path no longer requires Ollama or a manually started
llama-server. The TUI exposes a pinned model catalog; selecting an uninstalled
model downloads a checksummed llama.cpp `b12110` runtime and a checksummed,
resumable GGUF, registers it, plans the split, and starts inference. Managed
runtime/model state is exposed through `GET /managed` and
`POST /managed/install`.

Built-in choices include Qwen 2.5 Instruct at 0.5B, 1.5B, 3B, and 7B, plus
Qwen 2.5 Coder 7B. The larger general model uses the official single-file
Q3_K_M build; Coder uses Q4_K_M. The 1.5B model remains the default
recommendation. The ARM64 APK worker is built from the same llama.cpp revision
to keep the RPC protocol aligned.

Resource sharing is explicit on both sides. The Android **Share this phone**
switch starts/stops the worker. The TUI **Ctrl+D** panel can independently
include or exclude each device with Space; that choice survives heartbeats for
the node session. It reports free/total RAM, share cap, cores/threads,
reachability, heartbeat age, RPC address, and active layer/memory allocation.

---

# Reliability pass — 0.2.1

Historical release versions for this section: Rust `0.2.2`, localhost.ai Android `0.2.2+7`. See
`PROJECT_GUIDE.md` for the current architecture; earlier phase roadmaps above
describe historical states.

- GGUF v2/v3 metadata now supplies real transformer counts and attention-based
  KV estimates. Memory placement remains approximate. Replacement plans account
  for the old run's releasable allocation, and extra prompt-cache RAM is disabled.
- Idle workers must complete a matching RPC 8.0.x HELLO/device-count preflight.
  Do not repeatedly connect to an active worker's single-client socket: use its
  owning cluster state and the API's `in_use` flag. Failed runs clear readiness.
- Managed downloads validate resume ranges and use an installation lock.
  Installed catalog models are SHA-256 checked before use; successful checks are
  cached by size/mtime. Phone heartbeats cannot re-register a stopped worker.
- A stream must finish explicitly; bare EOF is an interrupted reply. Interrupted
  single-shot CLI commands also return failure exit codes.
- SIGTERM/SIGHUP clean up the owned engine. Linux also uses a parent-death guard
  so SIGKILL or a crashed node cannot leave native model RAM/ports allocated.
- Each load has a separate `cluster-<pid>-<run>.log`, referenced by `/cluster`
  and failure messages.
- Split planning is coordinator-first: fill the laptop's safe budget, then spill
  only the whole-layer remainder to remote workers in descending budget order.
  A fitting local model leaves phones idle, reducing Wi-Fi traffic and avoiding
  unnecessary Android RPC memory pressure.
- Remote assignments preserve 256 MiB for native RPC overhead. The Android
  worker reports memory again after startup, uses its private weight cache, and
  retains its last valid LAN address across transient lookup failures. Explicit
  withdrawal or fatal RPC load output now fails the cluster promptly.

`scripts/check-runtime.py` exercises the compiled CLI and terminal. With an
existing GGUF, `scripts/check-inference.py` verifies native CPU/RPC inference,
actual placement, reloads, worker loss/recovery, and termination cleanup. RPC
workers in these tests run on loopback; Android hardware testing is separate.

---

# Android UI pass — 0.2.1+4

- Added a shared branded Material 3 design system with adaptive content widths,
  responsive actions, semantic status pills, notices, and light/dark themes.
- Reworked Home into a connection/compute dashboard; pairing now has explicit
  scan/manual modes, scanner lifecycle protection, validation, and secure token
  entry.
- Chat uses lazy message rendering, accessible speaker roles, intentional
  auto-follow, jump-to-latest, and stop-generation controls. Pool refreshes run
  in parallel and preserve the last snapshot when the laptop is offline.
- Node switching no longer stops a worker when reselecting the active laptop;
  changing to another node or advanced URL safely withdraws phone sharing.
- Flutter build `0.2.1+4` passes analysis and 36 tests, including real 320 px
  layout constraints, large text, semantic labels, and long device/model data.

## Canonical design application — 0.2.1+5

- `app/design.md` is now implemented as the product UI contract: cream canvas,
  ink primary, neutral text ladder, hairline sections, 4px controls, flat depth,
  and bracketed ASCII markers throughout all six screens.
- Bundled JetBrains Mono 400/500/700 at pinned upstream commit `1937130` as the
  licensed Berkeley Mono substitute; the OFL text ships with the app assets.
- Removed decorative gradients, shadows, rounded card islands, raster in-app
  branding, and Material icon glyphs. The chat empty state is the one reserved
  near-black TUI-style surface.
- Android launch/window chrome and web metadata use the same cream/ink tokens.
  Flutter analysis and 38 tests cover exact theme tokens, the 640px action
  breakpoint, 320px large-text layout, semantics, and long pool content.

---

# TUI usability pass — 0.2.1

- Uninstalled catalog models can be planned without registration or download;
  `/cluster/start` still requires an installed, inspected, integrity-checked
  GGUF and repeats planning with current RAM.
- The model picker previews fit, shortfall, laptop-first assignments, estimated
  memory, and selectable 1K–16K context. `I` installs without loading.
- Added F1 help, Ctrl+A activity/progress/diagnostics, Ctrl+X stop/unload,
  bracketed paste, Unicode-safe cursor editing, word deletion, and PageUp/
  PageDown chat scrolling with pause/resume auto-follow.
- Async preview, model-operation, and chat-generation IDs prevent stale results
  from overwriting newer state. Ratatui and PTY tests cover these controls.

---

# localhost.ai Flutter rebrand — 0.2.1+6

- Active spec: `app/DESIGN_FLUTTER.md`. The cream/ASCII visual source is archived
  as `app/design_opencode_reference.md`.
- Pure black/green UI, bundled Outfit and JetBrains Mono, Lucide icons,
  terminal windows, low-alpha featured glow, and the supplied Downloads logo.
  Mark/lockup/icon/splash provenance is recorded in `assets/logo/provenance.json`.
- Riverpod owns shared coordinator UI snapshots; GoRouter provides adaptive
  Chat / Models / Server / Settings navigation and three-page onboarding.
- New UI controls use existing managed/model/cluster endpoints. User constraint:
  no backend/inference changes. Server terminal displays real client-observed
  coordinator/worker events and the existing log path; no new log API is added.
- Tab switches preserve chat drafts and phone sharing. Reduced motion disables
  status pulses and terminal typing. Tests use the requested four viewports,
  1.3x text, authenticated model-control ordering, stale-result guards, and
  snapshot retention. Preview images use mock coordinator data.
- At that release, the backend executable, config locations, pairing URI scheme,
  Android application ID, and signing key retained compatibility. The later
  localhost.ai identity reset intentionally supersedes that compatibility.

## LAN pairing address correction

- A live WARP setup exposed a QR-address bug: the route to `192.0.2.1` selected
  the VPN interface while UDP discovery used Wi-Fi. `core/src/discovery.rs`
  now selects the QR host through the LAN broadcast route without sending a
  packet.
- The rebuilt release coordinator advertises the Wi-Fi address with WARP still
  active. Authenticated pairing and parallel health/model/pool requests were
  verified locally; 68 core unit tests and 3 QR integration tests pass.
- Existing TUI processes must restart to load the new binary. Phones paired to
  the old VPN address must scan the new QR; saved phone addresses are not
  changed by rebuilding the coordinator.

## RPC lifecycle investigation — 0.2.2 / Android +7

- Reported incident: `~/.localhostai/logs/cluster-502128-1.log`, Qwen 2.5 7B Q3,
  4096 context, 25 laptop blocks + 3 phone blocks. Load completed and three
  replies finished before a socket read failed during the fourth request.
  The old log cannot establish a phone kill/crash versus transport loss; the
  "same LAN" hint was misleading. Native source/runtime commits match b12110.
- Fixed a reproduced failure where fatal RPC output left the coordinator
  reporting Running until the child finished abort handling. A 500 ms monitor
  now fails the run and clears readiness promptly.
- Serialized/cached idle probes, excluded them from preflight/launch, and
  rechecked ownership after waits. Stop cancels a load pending validation or
  RPC access. Real startup/reload is tested with concurrent polls/heartbeats.
- Phone reports include session, monotonic sequence, state, exit code, and
  native output. Old sessions/out-of-order requests cannot resurrect or withdraw
  the current worker. Address/session changes invalidate loaded buffers; old
  allocation credit requires the same session. Late phone evidence augments
  the failed run's log.
- APK adds `WorkerKeepAliveService` (connected-device foreground type), CPU and
  Wi-Fi locks, notification stop action, tracked-process cleanup, and private
  retained error output. The native wrapper observes app process death and
  ignores SIGPIPE. A real test caught and avoided PR_SET_PDEATHSIG following
  Dart's spawning thread rather than app lifetime.
- Verified: 101 Rust tests, strict Clippy, 57 Flutter tests, analysis, native
  lifecycle checks, real CPU/two-worker inference, 12-turn RPC with concurrent
  polling, worker loss/recovery, reloads, and signal cleanup. Native RPC tests
  use loopback stand-ins; Android foreground/lock behavior needs device testing.

## User-selected models

- The starter catalog is now optional. TUI **↑ → B** and Android **Models →
  Find and download models** search Hugging Face, open `owner/repository`, or
  accept a direct HTTP(S) GGUF link. Users select their own quantization.
- `core/src/model_hub.rs` resolves repository revisions to commits and groups
  complete split GGUFs. `/models/browse` returns sources; `/managed/download`
  downloads/registers the chosen source. `/managed/install` also resumes or
  verifies saved custom IDs. Pairing applies to the new endpoints.
- Custom manifests/files are stored under `~/.localhostai/models/custom/` and
  survive restart. Partial downloads stay available for retry, but do not enter
  `/models`. SHA-256, GGUF metadata, complete shard sets, and load-time planning
  remain authoritative. Unversioned direct links restart rather than splice
  unrelated partial data; validators use If-Range. HF_TOKEN is coordinator-only.
- Split-file weight estimates include every shard. Additional shards are not
  advertised as independent models. Custom models are downloaded before their
  real layer metadata is used for planning; downloads do not auto-start inference.
- Verification includes 114 Rust tests, strict Clippy, Flutter analysis and 66
  controller/widget checks (including four viewports at 1.3x text), release TUI
  CLI/PTY checks, and an isolated HTTP download/resume/restart test. Live Hugging
  Face search and nine real GGUF variants were checked without downloading their
  weights.

## Reliability and security pass — 0.2.3 / Android +8

- Fresh runtime installation follows validated HTTPS redirects and was checked
  from an isolated empty home through GitHub download, checksum, extraction,
  model-phase transition, and cancellation.
- `/pair` and all controls require the pair token even on loopback. Browser
  origins are rejected, connection/body/inference admission is bounded, and
  response writes have deadlines.
- Model sources are DNS-pinned at every hop; private, loopback, link-local,
  metadata, documentation, benchmark, redirect-loop, and HTTPS-downgrade targets
  are blocked unless a LAN host is explicitly trusted in config.
- Linux cgroup limits and native platform memory probes cap placement. Swap,
  zram, and PSI are reported separately. Replacement credit uses measured
  anonymous RSS, not the planner estimate.
- GGUF validation uses exact b12110 block layouts and records real per-layer and
  coordinator tensor sizes. Placement reserves coordinator/native overhead and
  honors model context metadata. Catalog KV estimates match their Qwen metadata.
- Installation jobs are bounded, pollable, cancellable, and deadline-limited;
  peak repair disk use is checked. Model deletion, runtime reset, log/peer
  retention, and persisted device inclusion controls are exposed.
- Android secrets use encrypted storage. The worker is hidden behind a native
  source-IP gate for the paired coordinator, and sharing stops on severe thermal
  pressure or critically low unplugged battery. Discovery uses the active LAN
  prefix with bounded fallback ports.
- Interrupted assistant text is retained and labelled. Laptop conversation
  history is conservatively bounded by context, persisted history is capped,
  and optional status failures retain healthy snapshots.
- Verification: 125 Rust tests, 71 Flutter tests, strict Clippy/analysis, exact
  b12110 Android worker build, cold runtime redirect/install test, lifecycle
  checks, and real local/two-worker inference with reload, loss/recovery, polling,
  heartbeat, SIGHUP, and SIGKILL cleanup.

## Shared conversations and performance — 0.3.0 / Android +9

- The coordinator owns named conversations under `~/.localhostai/conversations`.
  TUI and Android clients share revision-checked messages, full-text search,
  rename/delete, Markdown/JSON export, interruption checkpoints, and explicit
  phone-history import. Stale clients cannot overwrite newer edits.
- TUI Ctrl+O/Ctrl+N and Android Chat history expose the shared library. Android
  uses the system document picker and retains a credential-scoped offline snapshot.
- `/performance` separates runtime-reported token counts, time to first token,
  HTTP round-trip latency, measured process RSS, planned memory, and active layer
  placement. Missing measurements remain unavailable instead of becoming zero.
- The exclusive laptop/distributed benchmark reloads and warms each mode with the
  same prompt/settings, forces real remote placement, records measured timings/RSS,
  preserves device inclusion choices, and stops the model after every outcome.
- Verification: 130 Rust tests (plus a real-process ignored benchmark), 78 Flutter
  tests across four scaled viewports, strict Clippy/analysis, HTTP/restart/PTY
  conversation checks, native CPU/RPC benchmark checks, Linux release, and signed
  Android ARM64 release build. Physical Android worker validation remains separate.
