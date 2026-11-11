# Still-pose authoring contract

This pass creates storyboard stills, not the final animation. The user explicitly
chose review first and build together. Do not generate voice, add music, or render
an MP4 in this pass.

## Scene shape

Each assigned file in `compositions/` is a sub-composition with short id `s01`
through `s16`. Example:

```html
<template>
  <link rel="stylesheet" href="assets/video.css">
  <style>
    #root { position:absolute; inset:0; }
    /* prefix all local selectors/ids with the scene id */
  </style>
  <div id="root" data-composition-id="s01" data-width="1920"
       data-height="1080" data-duration="7.5" data-no-timeline>
    <div class="scene">
      <h1 id="s01-title" class="headline" data-var-text="s01-title">Title</h1>
      <!-- static, fully visible key pose; no animation or runtime DOM generation -->
    </div>
  </div>
</template>
```

- `#root` must be styled inline by id. Everything lives inside `<template>`.
- Use `assets/video.css`; do not change that shared file.
- All actual content is static authored HTML/SVG, with stable prefixed IDs for
  the later animation pass. No GSAP is needed in these intentionally still scenes.
- No per-scene captions: the main composition supplies one shared caption track.
- Reserve y=874–972 for those captions. Use the stage y=258–860 and headline
  near x=192,y=110. Essential content stays within x=192–1728.
- Read `SPEC.md`, your `STORYBOARD.md` entries, and `assets/demo-data.json`.
- Background is black, ink white, secondary text #A1A1AA, accent #5CF26A.
  Fonts and source icon glyphs already live in assets/video.css.
- Make the key object at least 40% of frame width or height. No tiny floating
  device/card compositions. Use actual device silhouettes in explanatory graphics.
- Source UI may be enlarged/cropped to keep key controls readable. Preserve
  source labels, navigation, hierarchy and styling; do not invent a dashboard.
- For the UI, use the exact source Lucide classes in video.css; no emoji.
- Provided logo: `assets/brand/mark.svg`, lockup: `assets/brand/lockup.png`.
- Provided illustrative pairing QR will be `assets/brand/demo-qr.svg`.
- UI sample label: `Recreated UI · sample data`; a loading scene must also make
  time compression explicit. Example graphics use `Illustrative example`.
- The layer counts always total 28, split as 25 laptop + 3 phone. No speed claims.
- Every primary line is at least 28 px. Text belongs on the part it names.
- The parent agent assembles, checks, snapshots and reviews the complete board.
  Do not run CLI rendering/preview/history or edit another agent's scene files.

## Source references

- `../app/DESIGN_FLUTTER.md`
- `../app/lib/features/server/server_page.dart`
- `../app/lib/features/models/models_page.dart`
- `../app/lib/features/chat/chat_page.dart`
- `../app/lib/ui/pair_screen.dart`
- `../app/lib/ui/pool_screen.dart`
- `../tui/src/ui.rs`

Registry source references are in `references/registry/`: device-slot staging,
terminal presentation, and connector geometry. Their generic demo content,
macOS traffic lights, unrelated colors, shadows and idle drift are not used.
