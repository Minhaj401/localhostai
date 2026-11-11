# localhost.ai

**Run AI Locally.** A dark, terminal-native Flutter client for the existing LAN
coordinator and bundled ARM64 llama.cpp RPC worker.

1. Run `privateml --require-pairing` on the laptop, choose/install a model in
   its picker, and press **Ctrl+P**.
2. Open **Server → Pair laptop** and scan the coordinator's QR.
3. Use **Models → Find and download models** to search Hugging Face, enter a
   repository, or paste a GGUF link. Choose a quantization, download it to the
   laptop, then preview and load it from the library. Split GGUFs from Hugging
   Face are downloaded as a complete group.
4. Enable **Share this phone** in Server to contribute spare RAM. A foreground
   notification keeps sharing active and provides **Stop sharing**;
   **Compute pool** shows live device assignments.

The phone worker is off by default. Even after enabling it, the laptop user can
include or exclude the phone from future model loads in the TUI with
**Ctrl+D**, **↑/↓**, and **Space**.

The starter catalog is optional. Custom models persist across coordinator
restarts, and incomplete downloads can be resumed from the library. Both the
laptop TUI and this app see the same downloaded files. Loading still requires a
GGUF supported by the bundled runtime and a fitting memory plan. Private/gated
Hugging Face repositories use `HF_TOKEN` from the coordinator's environment.

The worker reports a fresh RAM sample after startup and stores llama.cpp's
eligible weight cache in Android's private cache directory. Android can evict
that cache; it contains model weights, not prompts or chat history.

Version `0.2.2+7` adds CPU/Wi-Fi locks tied to the foreground sharing service,
cleanup when the app process exits, and sequenced worker-session reports.
Android can still terminate a worker under memory pressure or on a force-stop.
An exited/restarted worker has lost its model buffers: enable sharing again and
reload the model rather than continuing the broken RPC session.

Worker failures retain their exit code and final native output in the Server
terminal and app-private `files/worker-last-failure.json`. The laptop also receives
these details through `/peers` and appends them to the failed run's log when it
is reachable. Successful polling does not clear the phone's failure message.

Chat history, pairing, device identity, and backend settings are stored locally.
Cloud backends are available in **Settings**, including before pairing.
Android backup and device-transfer backup are disabled for this app. Use the
coordinator and phone worker only on a trusted LAN: their traffic is not
encrypted.

The UI follows [`DESIGN_FLUTTER.md`](DESIGN_FLUTTER.md): pure black, one green
accent, hairline dark cards, Outfit UI typography, and JetBrains Mono labels and
terminal content. Both font families and their OFL licenses are bundled for
offline rendering. The supplied Downloads logo provides the mark, lockup,
launcher icons, and black native splash; provenance is in
`assets/logo/provenance.json`.

Navigation has **Chat / Models / Server / Settings**, with a bottom bar below
840px and a rail on larger screens. First-launch onboarding is three pages.
Reduced motion disables typing and status pulses. Chat keeps its draft and
scroll state across tabs, lazily renders messages, and offers stop generation.

The Flutter connection snapshot lives in
`lib/features/connection/connection_controller.dart` through Riverpod; GoRouter
owns navigation. Existing backend services supply authenticated requests, SSE,
checksum/model installation, and worker process lifecycle.

## Build and check

Use Flutter 3.24.5 or a compatible newer release, Android SDK 36, and JDK 17.

```bash
flutter pub get
flutter analyze
flutter test
flutter build apk --release --target-platform android-arm64
```

APK: `build/app/outputs/flutter-apk/app-release.apk`.

Brand assets can be regenerated from the supplied image with
`scripts/prepare-brand-assets.py` at the repository root, then:

```bash
dart run flutter_launcher_icons
dart run flutter_native_splash:create
```

Responsive previews with mock coordinator data can be captured for review:

```bash
LOCALHOST_UI_PREVIEW_DIR=/tmp/opencode/localhost-ai-previews \
  flutter test test/ui_design_test.dart
```

Native window colors are configured in `android/app/src/main/res/values*/styles.xml`
to stay black with light system icons after regeneration.

The compute worker supports ARM64. Rebuild it from the repository root with
`bash scripts/build-phone-worker.sh`. It is pinned to the same llama.cpp
`b12110` revision as the managed laptop runtime; the script accepts
`LLAMA_CPP_REF`, `ANDROID_NDK_ROOT`, and `LLAMA_CPP_DIR` overrides. Its ggml
and C++ dependencies are statically linked.
The `phone-rpc/` CMake wrapper adds only process-lifetime handling around the
pinned upstream entry point. Its Linux signal checks run with
`python3 scripts/check-worker-lifecycle.py`.

For Android lint on this older Gradle/Jetifier toolchain, run
`./gradlew :app:lintRelease -Pandroid.enableJetifier=false` from `app/android`.
The override avoids Jetifier attempting to transform Java 24 classes in the
scanner plugin's test-only Byte Buddy dependency.

Release signing uses the ignored `android/key.properties` when configured,
and otherwise uses Flutter's debug signing configuration.
