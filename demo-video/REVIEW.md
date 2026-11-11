# Moving preview review

Status: ready for final preview approval; no delivery render has been made.

## Creative calls

- Built the approved **Make room** storyline: a model exceeds laptop capacity,
  the system places a 25+3 example split, and the second half demonstrates the
  same workflow in recreated localhost.ai UI.
- Did not build **Follow one prompt** or **Open the machine**. The chosen version
  keeps the memory problem as the reason for every demo action.
- Preserved the source app's black/green palette, Outfit/JetBrains Mono type,
  labels, navigation, status semantics, model identity, and supplied mark.
- Narration is Adam (`am_adam`) generated privately on-device with Kokoro-82M,
  at 1.08×. Hosted narration was attempted only after user choice, but the Free
  plan refused it; no hosted audio charge was made.
- No music or sound effects, by explicit user request.

## Measured

- HyperFrames full check passes: no lint errors, runtime errors, layout errors,
  contrast failures, craft warnings, or audio errors.
- The retained warning is `timeline_track_too_dense` for one captions host with
  sixteen narration groups. This is intentional: the product contract keeps one
  caption track rather than scattering captions across scene lanes.
- 16 narration clips: about 81.2 seconds of speech within 120 seconds; longest
  clip 6.165 seconds in a 7.5-second scene.
- Caption timing follows the measured local audio clip starts/durations.
- Review pass includes 32 scene beats, all 15 seams, and 360px phone sheets.
- Cut-proof snapshots verify a readable title at frame zero and every scene cut.

## Looked at

- All storyboard poses at full resolution before animation.
- Early/mid moving states for all sixteen scenes, phone-width sheets, and the
  explanation-to-demo and preview-to-load seams.
- The pictures read in sequence as capacity problem → device roles → safe split
  → resident state → pairing → model selection → sharing → placement → chat.
- The s12 notification cue was advanced after fresh-eyes review so its message
  and Stop sharing control arrive with the narration rather than after it.

## Not checked

- No human listening pass has occurred inside this agent environment. Audio
  files were measured and passed media checks; the user should listen in the
  final preview before approving render.
- No physical handset screen-off test is part of this recreated UI film.

## Weakest shot

- **s10 Scan and connect** carries two recreated surfaces and therefore has the
  densest UI. With more time, I would create a tighter camera handoff from QR to
  connected Server while retaining the source labels. For the current desktop
  destination, the headline, action, status, and endpoint remain readable.

## Warnings kept intentionally

- One dense captions composition, as described above.
- Hard cuts between the consistent black/green worlds. Every cut now lands on a
  fully readable title; the UI itself provides the within-scene causal motion.
