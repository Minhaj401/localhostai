# Bundled typography

Outfit and JetBrains Mono are copied from the application's existing licensed
font assets. Lucide comes from the exact `lucide_icons 0.257.0` dependency used
by the app. These local files keep storyboard and video typography reproducible.

Outfit weights: 400, 500, 600, 700. JetBrains Mono: 400, 500, 700.
Lucide icon codes are taken from that package's `lib/lucide_icons.dart`.

GSAP 3.15.0 is frozen separately in `assets/vendor/gsap.min.js`, matching the
version used by the scaffold. It is loaded locally so preview and render do not
depend on a CDN.
