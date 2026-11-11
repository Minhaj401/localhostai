#!/usr/bin/env python3
"""Create offline Outfit weights from the pinned Google Fonts variable release."""
import argparse
from pathlib import Path

from fontTools.ttLib import TTFont
from fontTools.varLib.instancer import instantiateVariableFont


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--variable", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    options = parser.parse_args()
    for weight, name in [(400, "Regular"), (500, "Medium"), (600, "SemiBold"), (700, "Bold")]:
        font = TTFont(options.variable)
        instance = instantiateVariableFont(font, {"wght": weight}, inplace=False)
        instance.save(options.output / f"Outfit-{name}.ttf")
        font.close()


if __name__ == "__main__":
    main()
