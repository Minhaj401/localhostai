# Shared conversations and performance — 0.3.0

## Conversations

The paired laptop coordinator owns the conversation library. Android and the
TUI open the same named conversations and can continue them on either device.
Only one reply can run in a conversation at a time. Opening a conversation does
not change the conversation being viewed on another device.

### TUI

- **Ctrl+O** opens saved conversations. Type to search names and message contents;
  use Up/Down and Enter to open a result.
- **Ctrl+N** starts a new conversation without deleting previous chats.
- In the conversation picker, **Ctrl+R** renames the current conversation.
- **Ctrl+E** exports Markdown; **Ctrl+Y** exports JSON. Files go to
  `~/.localhostai/exports/`, with private file permissions.
- **Delete** removes the selected conversation from the shared library.
- **Ctrl+X** stops a reply. Saved partial text is labelled interrupted.

The TUI loads a recent conversation at startup. Unsent drafts are retained when
switching between conversations during the same TUI session.

### Android

Use **Chat → Saved conversations** (history icon) to search, open, rename,
delete, or export. **New chat** keeps previous conversations. Exports use
Android's document picker, without broad storage access.

The last selected conversation is cached for offline reading. Its scope combines
the coordinator address and a hash of the pairing credential. Sending requires
the coordinator; failed requests are not automatically replayed. Unsent drafts
survive conversation switching during the app session.

**Import this phone’s saved chat** explicitly copies the previous phone-local
history into a new shared conversation. External-provider chats are not
automatically uploaded to the coordinator.

### Persistence and concurrency

Files live under `~/.localhostai/conversations/` (`LOCALHOSTAI_HOME` overrides the
parent). Each conversation records its ID, title, revision, timestamps,
model/context/system settings, and messages. Writes use temporary files, fsync,
and atomic replacement. A file lease prevents concurrent generation across
coordinator processes. Abandoned generations recover as interrupted after the
owning process exits.

Stale edits/sends return **409** instead of overwriting newer messages. Clients
refresh before retrying. Request IDs reject replay of an already saved turn.
Streaming text is checkpointed about every 500 ms and at termination; a forced
kill can lose the latest fraction of a second.

Limits: 500 conversations, 128 MiB total, 2 MiB per conversation, and 1,000
messages per conversation. New turns reserve reply space and may reach the
limit before the on-disk maximum. Export/delete chats to free space; history is
not silently evicted. Only a context-bounded recent window is sent to the model.

All shared-chat routes require pairing, even when ordinary API chat is public.

## Live dashboard

- TUI: **Ctrl+G**, PageUp/PageDown to scroll.
- Android: **Server → Performance dashboard** (gauge icon), or the section on Server.

Polling runs approximately once a second. Android pauses dashboard/library
polling in the background; worker heartbeats have their own lifecycle.

| Measurement | Meaning |
| --- | --- |
| Tokens/second | Runtime-reported generation speed; otherwise final upstream usage divided by observed request time. The source is displayed. |
| Output tokens | Runtime timings or upstream usage, never network chunks or character counts. |
| First-token time | Time from starting the upstream request to its first nonempty text delta, including prompt processing. Not reported for nonstreaming responses. |
| HTTP round trip | Client request time including HTTP/server overhead, not raw network or RPC latency. |
| Measured RSS | Engine resident memory or phone-reported worker RSS, including resident mapped pages. This differs from anonymous-memory replacement credit. |
| Planned allocation | The planner's estimate, displayed separately from measured RAM. |
| Assigned layers | Active cluster transformer placement. |

Missing values show **Not reported**. Phone samples arrive about every ten
seconds, display their age, and become unavailable after 30 seconds or worker
shutdown. Dashboard polling never probes an active worker's single-client RPC
socket. Engine RSS is unavailable for external engines whose PID is not owned
by the coordinator.

## Laptop-only versus distributed comparison

Choose an installed GGUF and context, enable a reachable phone worker, then use
**Run comparison** on Android or **B** in the TUI dashboard. **Cancel benchmark**
or **X** stops it.

The comparison exclusively owns inference/model controls. It stops the current
model, loads laptop-only mode, performs an eight-token warm-up, and measures a
request. It repeats in distributed mode, deliberately assigning a tail of
layers to enabled workers. Inclusion preferences are preserved. Both modes use
the same model, prompt, context, seed 42, temperature zero, disabled prompt
caching, and 128-token output limit. Actual counts can differ with early EOS.

Results show speed, first-token latency, peak sampled laptop-engine RSS, load
time, and placement. Reload/warm-up time is excluded from measured chat latency.
Unavailable modes are explicitly skipped with their fit reason. These are
single-run comparisons, not statistical guarantees; desktop load, Wi-Fi,
temperature, and caching affect the result.

The model is stopped after completion, cancellation, or failure. Load it again
to resume chat. A comparison has a 15-minute total deadline and bounded
individual requests. The latest comparison and 24 generation metrics are
session-local; conversations persist across restarts.

## API

All routes require `Authorization: Bearer <pair_token>`.

| Method and route | Payload / response |
| --- | --- |
| `GET /conversations?query=...` | `store_id` and summaries. |
| `POST /conversations` | Optional `title`; optional `messages` explicitly imports history. Returns `conversation`. |
| `GET /conversations/{id}` | Full `conversation`. |
| `POST /conversations/{id}` | `revision`, `title`. |
| `POST /conversations/{id}/delete` | `revision`. |
| `POST /conversations/{id}/cancel` | `request_id` matching the active generation. |
| `GET /conversations/{id}/export?format=markdown` | `filename`, `mime`, text `content`; JSON format also supported. |
| `POST /v1/chat/completions` | Add `conversation_id`, `revision`, 32-hex-character `request_id`, `content`, `ctx`, `system_prompt`, and `stream: true`. The coordinator assembles history and strips private fields before forwarding. |
| `GET /performance` | Generation metrics, timestamped memory samples, layers, latest benchmark. |
| `GET /ping` | Small authenticated response for round-trip measurement. |
| `POST /benchmarks/start` | `model`; optional `ctx`, `prompt`, `max_tokens` (16–512), `remote_layers`. Returns 202 and snapshot. |
| `GET /benchmarks` | Latest benchmark. |
| `POST /benchmarks/cancel` | Empty object. |

These features use the existing LAN transport and add no cloud service.

## Verification

`python3 scripts/check-workspace.py` checks paired access, shared edits, Unicode
search/export, interrupted text, remote cancellation, private permissions,
timing extraction, and restart recovery with an isolated mock runtime. CI runs it.

`scripts/check-inference.py` also checks native laptop/RPC benchmark measurements,
actual placement, and unchanged inclusion choices. It requires enough live RAM
for its model; `--ctx` and `--benchmark-only` support smaller focused checks.

The ignored Rust test `native_benchmark_records_real_timings_placement_and_cleans_up`
uses real native processes with the test node's controlled capacity fixture.
Set `LOCALHOSTAI_BENCHMARK_TEST_MODEL` to a small 128-context GGUF and provide
matching tools, then run `cargo test -p localhostai-core native_benchmark -- --ignored`.
Loopback workers do not replace physical Android testing.
