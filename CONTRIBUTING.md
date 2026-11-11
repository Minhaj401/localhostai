# Contributing to localhost.ai

Useful contributions include reproducible fixes, memory-estimation improvements,
supported-model compatibility reports, native-worker reliability, and accessible
UI improvements.

## Development setup

1. Build the Rust workspace with a recent stable Rust toolchain:
   `cargo build --workspace --locked`.
2. For Android, install Flutter 3.24.5 or a compatible release, Android SDK 36,
   and JDK 17. Run `flutter pub get` inside `app/`.
3. Read [PROJECT_GUIDE.md](PROJECT_GUIDE.md) for protocols, planning, and lifecycle
   invariants, and [app/README.md](app/README.md) for native-worker and signing setup.

## Before a pull request

Run the checks relevant to your change:

```bash
cargo fmt --all -- --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo build --workspace --locked
python3 scripts/check-runtime.py
```

Inside `app/`, run `flutter analyze` and `flutter test`. Inference or RPC changes
should also be checked with `scripts/check-inference.py` and matching native
binaries. Label loopback integration results separately from physical Android
testing.

Keep changes focused. Explain the user-visible problem, implementation, and
verification in your pull request. Do not commit model weights, credentials,
pairing secrets, signing keys, build caches, or generated binaries.

## Reporting a problem

Include the app/coordinator version, OS and device architecture, model and
quantization, context size, reproduction steps, and relevant error output.
Redact pairing tokens and provider keys from logs.

The model installer and phone worker use the same pinned llama.cpp RPC revision.
Keep them compatible when changing either side.
