# Narration draft — Make room

English narration only, with matching captions. Sixteen short phrases pair with
the sixteen storyboard shots. Record after the user approves the board/script.
Pronunciation: localhost.ai = “local host dot A I”; GGUF = “G G U F”;
Ctrl+P = “Control P”; LAN = “lan”.

## 01 — The memory question (0–7.5s)
A larger local AI model needs more memory than your laptop has available. What now?

## 02 — Two devices (7.5–15s)
localhost.ai lets your laptop and phone share the work of running one model.

## 03 — The coordinator (15–22.5s)
The laptop coordinates everything: storing the model, planning placement, and serving the chat.

## 04 — The phone worker (22.5–30s)
Pair over your local network. Switch on sharing to start the phone’s bundled worker.

## 05 — Safe capacity (30–37.5s)
The planner reserves operating-system headroom, and includes only workers it can reach and use.

## 06 — Laptop first (37.5–45s)
It fills the laptop first, then sends only the remaining layers to the phone.

## 07 — The example split (45–52.5s)
Here’s an example: twenty-five layers on the laptop, and three more on the phone.

## 08 — How a reply runs (52.5–60s)
Each device holds its layers and their context memory. Intermediate data travels between them during chat.

## 09 — Start the laptop (60–67.5s)
Let’s try the workflow. Start the laptop app, then press Control P to show the pairing code.

## 10 — Pair the phone (67.5–75s)
On the phone, open Server, choose Pairing, and scan that code to connect to the coordinator.

## 11 — Choose a model (75–82.5s)
Open Models and choose a GGUF. The library shows its size, quantization, and whether it’s installed.

## 12 — Share the phone (82.5–90s)
In Server, enable Share this phone. The foreground notification gives you a Stop sharing control.

## 13 — Preview placement (90–97.5s)
Preview the split at your chosen context size, and check each device’s estimated layer allocation.

## 14 — Load (97.5–105s)
Load the model. Once it is ready, the Server screen shows Running and the active model.

## 15 — Ask (105–112.5s)
Open Chat and send a question. The coordinator uses the loaded model across its assigned devices.

## 16 — The result (112.5–120s)
The answer streams back. In laptop mode, prompts stay on your LAN: one model across your devices.
