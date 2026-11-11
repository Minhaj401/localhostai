# Make room — director's specification

GOAL: The viewer understands laptop-first model placement, then can follow
the pairing, model, sharing, preview, load, and chat workflow.

OUTPUT: HyperFrames; 1920 × 1080; 30 fps; editable scenes and captions.

TIME: 120 seconds × 30 fps = 3600 frames. Sixteen planned 7.5-second shots.
Shots 01–08 explain the system; shots 09–16 recreate the UI workflow. Narration
will be measured before animation timing is locked; keep the total near 120 s.

LOOK: Existing localhost.ai brand, taken from ../app/DESIGN_FLUTTER.md.
- Ground #000000; elevated #0A0A0B; surfaces #101012 and #16161A.
- Ink #FFFFFF; secondary text #A1A1AA; accent #5CF26A; accent highlight #8BFF95.
- Borders #1F1F24 and #2E2E35. Increase diagram strokes to 3–4 px for video.
- Display: Outfit 500, 76 px, line-height 1.06, tracking -2 px.
- Body: Outfit 400, 34 px, line-height 1.35; essential UI text at least 28 px.
- Technical labels: JetBrains Mono 500, 28 px, line-height 1.35.
- Captions: Outfit 500, 34 px, max two lines, centered between x=192 and 1728,
  in y=874–972. Scene content ends before that band.
- UI source: actual Flutter feature screens and theme. Use enlarged close-ups
  of the relevant controls; do not stretch a phone to landscape or substitute
  a fictional dashboard. Whole-phone context is secondary to readable details.
- No photographic grade, film grain, vignette, decorative glow, or shadows.
  The original supplied mark is reused without recoloring or drawing a substitute.

OBJECTS: A recognizable laptop screen/base; a recognizable Android-style phone
outline; 28 discrete layer blocks; a physically attached OS-reserve region;
GGUF model identity; a routed data marker; the app's actual buttons, switch,
cards, four-tab navigation, terminal, and chat composer. Device graphics are
explanatory drawings; app views are explicitly labelled recreated demo/sample data.

MOTION (production, after storyboard approval):
- Entries: power3.out, 15–21 frames; connector draw: power2.inOut, 24 frames.
- Block placement: power2.inOut, 18 frames per move, 2-frame stagger.
- UI cursor travel: power2.inOut, 18 frames; click: 4-frame scale impulse;
  screen state swaps: 8-frame fade-through; no idle UI drift.
- Camera/crop reframes: power2.inOut, 24–30 frames, then hold completely still.
- One hero move per shot; reveal the next explanatory part on its spoken cue.
- Match shapes across cuts: layer strip → device allocation; connector →
  pairing state; terminal frame → app surface; model name persists through load.

SHOTS:
| ID | Frames | Time | Hero move / still key moment | Viewer reads |
|---|---:|---|---|---|
| s01 | 0–224 | 0–7.5 | Layer blocks hit the laptop's safe capacity | Where do the extra layers go? |
| s02 | 225–449 | 7.5–15 | Phone joins the same model diagram | One model. Two devices. |
| s03 | 450–674 | 15–22.5 | Laptop opens to model, planner, chat endpoint | The laptop coordinates |
| s04 | 675–899 | 22.5–30 | Pair link lands; phone worker switches on | Pair, then share |
| s05 | 900–1124 | 30–37.5 | OS reserve stays separate from usable capacity | Keep room for the OS |
| s06 | 1125–1349 | 37.5–45 | Laptop fills before the remainder moves | Laptop first |
| s07 | 1350–1574 | 45–52.5 | The 25 + 3 split resolves into labelled devices | Example: 28 layers |
| s08 | 1575–1799 | 52.5–60 | A data marker crosses the resident layer groups | Layers and context stay resident |
| s09 | 1800–2024 | 60–67.5 | Recreated terminal opens its pairing view | Start the laptop app; Ctrl+P |
| s10 | 2025–2249 | 67.5–75 | Pairing screen resolves to connected Server | Scan and connect |
| s11 | 2250–2474 | 75–82.5 | Models card selects Qwen 2.5 7B | Choose your model |
| s12 | 2475–2699 | 82.5–90 | Share switch turns on; notification appears | Share this phone |
| s13 | 2700–2924 | 90–97.5 | Compute Pool shows 4K context and 25 + 3 | Preview the split |
| s14 | 2925–3149 | 97.5–105 | Load completes into the Server Running state | Load the model |
| s15 | 3150–3374 | 105–112.5 | Chat composer sends the example question | Ask your question |
| s16 | 3375–3599 | 112.5–120 | Answer streams and holds on the result | One model across your devices |

Each shot's sound is English narration only. No music and no sound effects.
The detailed narration and read windows live in SCRIPT.md and STORYBOARD.md.

BANNED: Empty black scenes with tiny floating objects; logo-only opener;
three-card feature slides; purple/blue palettes; traffic-light terminal dots;
decorative particles, bloom, HUD labels, corner brackets, or a docked logo;
unmotivated floating, repeated slide-up entrances, moving text during reading;
fake benchmark numbers; unlabelled mock UI or implying a live phone recording.

QA: Still proof times 3.75, 11.25, 18.75, 26.25, 33.75, 41.25, 48.75, 56.25,
63.75, 71.25, 78.75, 86.25, 93.75, 101.25, 108.75, 116.25 seconds. Check every
shot at full size and 360 px wide. Verify exactly 28 blocks wherever the full
model is shown, 25+3 allocation, caption clearance, readable primary UI,
original logo assets, and identical source-derived wording across the demo.

## Shared factual model

`assets/demo-data.json` is the number/copy source. Values used as sample UI
data are labelled illustrative. This film makes no speed or latency claim.

## Registry use

Searched the local words catalog for network diagrams, device mockups, and
terminal windows. Installed references are browser-device-stage,
code-terminal-run, and flowchart. Adapt their device-slot, terminal, and
connector vocabulary to the source app and still poses; do not mount their
generic demo content or default branding in the film.
