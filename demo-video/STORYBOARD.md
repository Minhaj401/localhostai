---
format: 1920x1080
duration: 120s
fps: 30
message: "Run a local model on the laptop first, then share the layers that need more room with a reachable phone."
arc: "Memory question → device roles → safe placement → example split → recreated setup → chat result"
audience: "People exploring private local AI"
mode: collaborative
storyline: Make room
review: approved
---

## Frame 1 — Where do the extra layers go? (0–7.5s)
- act: Explain
- status: animated
- duration: 7.5s
- poster: 3.75s
- src: compositions/s01-overflow.html
- scene: A large laptop memory bay holds 25 blocks; 3 blocks remain outside its safe boundary.
- voiceover: A larger local AI model needs more memory than your laptop has available. What now?
- reads: The laptop reaches safe capacity (0–3s); the three remaining blocks (3–7.5s).
- sound: English narration only.
Open on the actual capacity problem. The layer ribbon, not a logo or headline
card, carries the explanation forward. Mark the numbers as an example.

## Frame 2 — One model. Two devices. (7.5–15s)
- act: Explain
- status: animated
- duration: 7.5s
- poster: 3.75s
- src: compositions/s02-devices.html
- scene: The same model ribbon spans a recognizable laptop and phone, joined by a local-network path.
- voiceover: localhost.ai lets your laptop and phone share the work of running one model.
- reads: One model is shared (0–3s); both devices belong to the same system (3–7.5s).
- sound: English narration only.
Give both devices physical screen/bezel cues; keep the model's identity constant.

## Frame 3 — The laptop coordinates (15–22.5s)
- act: Explain
- status: animated
- duration: 7.5s
- poster: 3.75s
- src: compositions/s03-coordinator.html
- scene: A laptop cutaway connects a GGUF file, placement planner, and chat endpoint inside the laptop.
- voiceover: The laptop coordinates everything: storing the model, planning placement, and serving the chat.
- reads: The GGUF lives on the laptop (0–2.5s); the planner places it (2.5–5s); the coordinator serves chat (5–7.5s).
- sound: English narration only.
This is one functional machine, not a row of feature cards.

## Frame 4 — Pair, then share (22.5–30s)
- act: Explain
- status: animated
- duration: 7.5s
- poster: 3.75s
- src: compositions/s04-pair-worker.html
- scene: A local-network link joins the phone to the coordinator, and the phone's CPU/worker area activates.
- voiceover: Pair over your local network. Switch on sharing to start the phone’s bundled worker.
- reads: Pair over LAN (0–3s); sharing explicitly starts the worker (3–7.5s).
- sound: English narration only.
Do not imply that pairing alone shares resources or that the phone is running Ollama.

## Frame 5 — Keep room for the OS (30–37.5s)
- act: Explain
- status: animated
- duration: 7.5s
- poster: 3.75s
- src: compositions/s05-budget.html
- scene: The memory region visibly separates OS reserve, RPC headroom, and the usable model area; a reachable-worker label is attached to the phone.
- voiceover: The planner reserves operating-system headroom, and includes only workers it can reach and use.
- reads: Reserve is excluded (0–3s); only usable capacity is assigned (3–5s); the worker must be reachable (5–7.5s).
- sound: English narration only.
Use labels directly on the regions rather than a separate color legend. No fabricated free-RAM value.

## Frame 6 — Laptop first (37.5–45s)
- act: Explain
- status: animated
- duration: 7.5s
- poster: 3.75s
- src: compositions/s06-laptop-first.html
- scene: The layer strip fills the laptop; a single routed remainder leads into the phone.
- voiceover: It fills the laptop first, then sends only the remaining layers to the phone.
- reads: Fill the laptop first (0–3.5s); send the remainder (3.5–7.5s).
- sound: English narration only.
The direction is sequential, not an equal load-balancing promise.

## Frame 7 — Example: 28 layers (45–52.5s)
- act: Explain
- status: animated
- duration: 7.5s
- poster: 3.75s
- src: compositions/s07-split.html
- scene: Twenty-five layer blocks occupy the laptop and three occupy the phone, with large attached counts.
- voiceover: Here’s an example: twenty-five layers on the laptop, and three more on the phone.
- reads: 25 laptop layers (0–3s); 3 phone layers (3–5s); all 28 placed (5–7.5s).
- sound: English narration only.
Use the exact example from demo-data.json. Make every block countable.

## Frame 8 — Layers stay resident (52.5–60s)
- act: Explain
- status: animated
- duration: 7.5s
- poster: 3.75s
- src: compositions/s08-request.html
- scene: The model layers and context memory remain inside their devices while a small intermediate-data marker crosses the LAN path.
- voiceover: Each device holds its layers and their context memory. Intermediate data travels between them during chat.
- reads: Resident layers and context (0–3s); intermediate data crosses the link (3–7.5s).
- sound: English narration only.
Do not show the entire model moving across Wi-Fi for every token. Transition to
the laptop terminal shape for the demo half.

## Frame 9 — Start the laptop app (60–67.5s)
- act: Demo
- status: animated
- duration: 7.5s
- poster: 3.75s
- src: compositions/s09-terminal.html
- scene: Recreated PrivateLM terminal with the actual privateml command, Ctrl+P, and a clearly marked demo QR.
- voiceover: Let’s try the workflow. Start the laptop app, then press Control P to show the pairing code.
- reads: privateml --require-pairing (0–3.5s); Ctrl+P opens pairing (3.5–7.5s).
- sound: English narration only.
The displayed QR is a non-pairable storyboard placeholder. No private host token is used.

## Frame 10 — Scan and connect (67.5–75s)
- act: Demo
- status: animated
- duration: 7.5s
- poster: 3.75s
- src: compositions/s10-pairing.html
- scene: Recreated pairing controls transition to the Server view with the sample coordinator address and Online / no model state.
- voiceover: On the phone, open Server, choose Pairing, and scan that code to connect to the coordinator.
- reads: Pairing / Scan QR (0–3.5s); the coordinator is online (3.5–7.5s).
- sound: English narration only.
Use source labels and the source app's single-green-status-dot treatment.

## Frame 11 — Choose your model (75–82.5s)
- act: Demo
- status: animated
- duration: 7.5s
- poster: 3.75s
- src: compositions/s11-models.html
- scene: A readable Models close-up selects Qwen 2.5 7B Instruct and shows 3.55 GB, Q3_K_M, and Installed.
- voiceover: Open Models and choose a GGUF. The library shows its size, quantization, and whether it’s installed.
- reads: Qwen 2.5 7B Instruct (0–3s); size and quantization (3–5.5s); Installed (5.5–7.5s).
- sound: English narration only.
The model is already installed for this demonstration. No download-speed claim.

## Frame 12 — Share this phone (82.5–90s)
- act: Demo
- status: animated
- duration: 7.5s
- poster: 3.75s
- src: compositions/s12-sharing.html
- scene: The source Server sharing switch is on; a schematic Android foreground notification includes Stop sharing.
- voiceover: In Server, enable Share this phone. The foreground notification gives you a Stop sharing control.
- reads: Share this phone is on (0–3.5s); notification and Stop sharing (3.5–7.5s).
- sound: English narration only.
The notification is a labelled recreation, not a claim of handset verification.

## Frame 13 — Preview the split (90–97.5s)
- act: Demo
- status: animated
- duration: 7.5s
- poster: 3.75s
- src: compositions/s13-preview.html
- scene: A Compute Pool close-up shows model placement, 4096-token context, and the same 25+3 example split.
- voiceover: Preview the split at your chosen context size, and check each device’s estimated layer allocation.
- reads: Context size: 4096 (0–2.5s); Preview split (2.5–4s); the two allocations (4–7.5s).
- sound: English narration only.
The displayed allocation is estimated placement. Keep the example numbers consistent.

## Frame 14 — Load the model (97.5–105s)
- act: Demo
- status: animated
- duration: 7.5s
- poster: 3.75s
- src: compositions/s14-loading.html
- scene: The Load model action resolves into Server Running with Qwen 2.5 7B Instruct; a time-compressed-demo label is visible.
- voiceover: Load the model. Once it is ready, the Server screen shows Running and the active model.
- reads: Load model (0–2s); Loading (2–4s); Running and the model name (4–7.5s).
- sound: English narration only.
The shortened loading beat is explicitly illustrative, not a benchmark.

## Frame 15 — Ask your question (105–112.5s)
- act: Demo
- status: animated
- duration: 7.5s
- poster: 3.75s
- src: compositions/s15-chat.html
- scene: A faithful Chat composer contains “Explain local AI in one sentence.” beside the green send control.
- voiceover: Open Chat and send a question. The coordinator uses the loaded model across its assigned devices.
- reads: Chat is selected (0–2s); the example question (2–5.5s); send (5.5–7.5s).
- sound: English narration only.
Keep the same model identity. Do not add made-up token-per-second counters.

## Frame 16 — One model across your devices (112.5–120s)
- act: Demo
- status: animated
- duration: 7.5s
- poster: 3.75s
- src: compositions/s16-answer.html
- scene: The example answer is readable in Chat, with the final idea attached to the two-device model strip.
- voiceover: The answer streams back. In laptop mode, prompts stay on your LAN: one model across your devices.
- reads: The answer arrives (0–4.5s); one model across your devices (4.5–7.5s).
- sound: English narration only.
Finish on the result, with a final 2–3 second readable hold rather than a logo-only card.
