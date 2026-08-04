#!/usr/bin/env python3
"""Inspect a SamarOS screenshot without a display.

Prints a coarse luminance map, the dominant colours and the bounding boxes of
bright regions so the compositor can be regression-checked from a terminal.

    usage: inspect_shot.py shot.png [cols]
"""
import sys
from collections import Counter
from PIL import Image

RAMP = " .:-=+*#%@"


def crop_map(path, x, y, w, h, cols=110):
    """Zoom into a region: inspect_shot.py shot.png --crop x y w h"""
    im = Image.open(path).convert("RGB").crop((x, y, x + w, y + h))
    rows = max(1, int(cols * h / w / 2.1))
    small = im.resize((cols, rows), Image.BOX)
    px = small.load()
    print(f"crop {x},{y} {w}x{h}")
    print("+" + "-" * cols + "+")
    for yy in range(rows):
        line = []
        for xx in range(cols):
            r, g, b = px[xx, yy]
            lum = (r * 299 + g * 587 + b * 114) // 1000
            line.append(RAMP[min(len(RAMP) - 1, lum * len(RAMP) // 256)])
        print("|" + "".join(line) + "|")
    print("+" + "-" * cols + "+")


def main():
    path = sys.argv[1]
    if "--crop" in sys.argv:
        i = sys.argv.index("--crop")
        x, y, w, h = (int(v) for v in sys.argv[i + 1:i + 5])
        crop_map(path, x, y, w, h)
        return
    cols = int(sys.argv[2]) if len(sys.argv) > 2 else 96
    im = Image.open(path).convert("RGB")
    w, h = im.size
    rows = max(1, int(cols * h / w / 2.1))

    print(f"{path}: {w}x{h}")

    small = im.resize((cols, rows), Image.BOX)
    px = small.load()
    print("+" + "-" * cols + "+")
    for y in range(rows):
        line = []
        for x in range(cols):
            r, g, b = px[x, y]
            lum = (r * 299 + g * 587 + b * 114) // 1000
            line.append(RAMP[min(len(RAMP) - 1, lum * len(RAMP) // 256)])
        print("|" + "".join(line) + "|")
    print("+" + "-" * cols + "+")

    counts = Counter(im.getdata())
    print("dominant colours:")
    for color, n in counts.most_common(10):
        print("  #%02X%02X%02X  %6.2f%%" % (color[0], color[1], color[2], 100 * n / (w * h)))
    print("distinct colours:", len(counts))


if __name__ == "__main__":
    main()


