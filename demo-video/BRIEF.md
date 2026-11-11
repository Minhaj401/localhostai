---
workflow: hyperframes
flow: companion
storyboard: yes
mode: collaborative
message: "Run a local model on the laptop first, then share the layers that need more room with a reachable phone."
destination: desktop
aspect: 1920x1080
fps: 30
language: en
length: 120s
angle: graphics-led-explanation-then-recreated-ui-demo
narration: yes
captions: yes
music: no
sound_effects: no
---

## Intent

Create a two-minute localhost.ai demo and explainer with the user, reviewing
the storyboard before production. The first roughly 60 seconds explain the
system through graphics; the second roughly 60 seconds recreate the current
UI and demonstrate pairing, model selection, phone sharing, preview/loading,
and chat. Use English narration and matching captions.

User's wording: "1st it should explain how this works using graphics then it
should recreate the ui and show the demo i am epecting this to be 2 minute long".

The user selected landscape 16:9, storyboard review, "Build together",
"Narration only", localhost.ai branding, and existing project assets. The
combined brief and proposed 60/60 sequence were explicitly confirmed.

## Assets

- ../app/DESIGN_FLUTTER.md — brand and UI design source.
- ../app/lib/features/ — current app screens and interaction labels.
- ../app/lib/theme/ — authoritative black/green theme tokens.
- ../app/assets/logo/mark.svg — supplied logo mark, reused without redrawing.
- ../app/assets/logo/lockup.png — supplied localhost.ai wordmark/lockup.
- ../app/assets/logo/provenance.json — logo provenance.
- ../app/assets/fonts/ — bundled Outfit and JetBrains Mono families and licenses.
- ../README.md and ../PROJECT_GUIDE.md — product behavior and technical facts.
- ../core/src/plan.rs and ../core/src/server.rs — laptop-first planning,
  reachable-worker checks, memory reservations, and API workflow.

## Customizations

- Selected storyline: **A — Make room**, explicitly chosen after the three
  visual story options. A model that exceeds the laptop's safe capacity drives
  the graphics, then the same example drives the recreated UI workflow.

- Diagram-led first half: coordinator and phone roles, safe memory budgets,
  laptop-first layer placement, and the inference request path over LAN.
- Faithful HTML UI recreation for the second half, based on the source app.
- Label the recreated UI/sample values as an illustrative demo.
- Show model planning before loading and explicit opt-in phone sharing.
- Narration and captions only; no background music or sound effects.
- Review the scene plan, storyboard, narration, and voice together before the
  production build. Final preview approval precedes delivery rendering.
- The user approved the complete 16-frame storyboard and narration draft.
  Production may proceed; narration provider/voice selection is the next choice.
- Narrator selected: Adam (`am_adam`), generated privately on-device with
  Kokoro-82M after hosted TTS required a paid plan. No hosted narration charge
  was made.

## Notes

- Default audience: people exploring private local AI, without assuming they
  already understand RPC or model-layer placement.
- The message is usable combined memory, not a promise of faster inference.
- Prompts remain on the local network in the demonstrated laptop mode.
  Do not imply encrypted RPC or that optional cloud backends are local.
- An example split may use 28 blocks, 25 on the laptop and 3 on the phone,
  grounded in the reported run and clearly identified as an example.
- Do not include real pairing tokens or claim live performance measurements.
- This is a recreated demo, not a recording of a verified phone session.
- Voice recording is deferred until the script/storyboard review. The media
  doctor reports no HeyGen helper/auth; offer hosted setup or opt-in local
  Kokoro when selecting the narration voice.
- HyperFrames 0.9.26 consolidated the retired general-video companion workflow
  into the main hyperframes film references. This project follows that current
  execution contract while retaining all confirmed choices above.
