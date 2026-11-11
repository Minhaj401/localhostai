#!/usr/bin/env python3
"""Derive localhost.ai assets from the supplied Downloads logo, without redrawing it."""
import argparse
import base64
import hashlib
import json
from pathlib import Path

from PIL import Image


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    options = parser.parse_args()
    source = Image.open(options.source).convert("RGB")
    if abs(source.width / source.height - 3.0) > 0.02:
        raise ValueError("Expected the supplied 3:1 localhost.ai lockup")
    def crop(rect):
        return source.crop(tuple(round(value * (source.width / 2000 if i % 2 == 0
                                               else source.height / 667))
                                 for i, value in enumerate(rect)))
    options.output.mkdir(parents=True, exist_ok=True)
    # The source contains the mark at left and the full wordmark/tagline at right.
    lockup = crop((330, 180, 1670, 490))
    lockup.save(options.output / "lockup.png", optimize=True)
    mark = crop((340, 190, 665, 475)).convert("RGBA")
    mark.thumbnail((325, 285), Image.Resampling.LANCZOS)
    # Make only the near-black background transparent; preserve logo foreground pixels.
    pixels = []
    for red, green, blue, alpha in mark.get_flattened_data():
        pixels.append((red, green, blue, 0 if max(red, green, blue) < 28 else alpha))
    mark.putdata(pixels)
    square = Image.new("RGBA", (384, 384), (0, 0, 0, 0))
    square.alpha_composite(mark, ((384 - mark.width) // 2, (384 - mark.height) // 2))
    square.save(options.output / "mark.png", optimize=True)
    encoded = base64.b64encode((options.output / "mark.png").read_bytes()).decode("ascii")
    svg = ('<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" '
           'viewBox="0 0 384 384"><image width="384" height="384" '
           f'xlink:href="data:image/png;base64,{encoded}"/></svg>\n')
    (options.output / "mark.svg").write_text(svg)
    for name, transparent in [("icon.png", False), ("icon_foreground.png", True)]:
        icon = Image.new("RGBA", (1024, 1024), (0, 0, 0, 0 if transparent else 255))
        resized = square.crop(square.getbbox())
        target = 1000 if transparent else 676
        resized.thumbnail((target, target), Image.Resampling.LANCZOS)
        # Adaptive icon XML adds its own 16% inset, so foreground uses full size.
        ratio = target / max(resized.size)
        resized = resized.resize((round(resized.width * ratio), round(resized.height * ratio)),
                                 Image.Resampling.LANCZOS)
        icon.alpha_composite(resized, ((1024 - resized.width) // 2, (1024 - resized.height) // 2))
        icon.save(options.output / name, optimize=True)
    # 4x native splash asset renders as a compact lockup on black.
    splash = Image.new("RGB", (1152, 400), "black")
    resized = lockup.resize((1152, 266), Image.Resampling.LANCZOS)
    splash.paste(resized, (0, 67))
    splash.save(options.output / "splash.png", optimize=True)
    (options.output / "provenance.json").write_text(json.dumps({
        "brand": "localhost.ai",
        "source": options.source.name,
        "source_sha256": hashlib.sha256(options.source.read_bytes()).hexdigest(),
        "source_dimensions": list(source.size),
        "mark_crop": [340, 190, 665, 475],
        "lockup_crop": [330, 180, 1670, 490],
        "svg": "Lossless PNG crop embedded in SVG; supplied logo was not redrawn",
    }, indent=2) + "\n")


if __name__ == "__main__":
    main()
