#!/usr/bin/env python3
"""16:9 virtual-canvas calculator for fold inner displays and tablets.

The OrangeFox portrait theme is authored ~16:9; a 16:9 virtual canvas
(DOF_SCREEN_W/H) renders it undistorted, letterboxed on both axes
(DOF_PROGRESSIVE_SCALE=1). Rule: take the real panel height, derive the
integer width closest to 9/16 of it; if that width would exceed the real
panel width, shrink the height just enough to fit. The resulting ratio
must stay within 5 % of 9:16 or the script fails.

Usage:
    display_16x9.py <fb_w> <fb_h> <label>
Prints "<label>: <w>x<h> (h reduced by N / unchanged, deviation D%)".
Exit 2 on tolerance violation.
"""

import sys

TARGET_RATIO = 9 / 16
TOLERANCE = 0.05


def canvas_16x9(fb_w: int, fb_h: int) -> tuple[int, int, int]:
    """Returns (w, h, h_reduced_by) for the largest fitting 16:9 canvas."""
    h = fb_h
    w = round(h * TARGET_RATIO)
    if w > fb_w:
        # Super-narrow panel: shrink height until the width fits.
        h = int(fb_w * 16 / 9)
        w = round(h * TARGET_RATIO)
    if w <= 0 or h <= 0 or w > fb_w or h > fb_h:
        raise ValueError(f"no fitting 16:9 canvas in {fb_w}x{fb_h}")
    deviation = abs((w / h) / TARGET_RATIO - 1)
    if deviation > TOLERANCE:
        raise ValueError(
            f"ratio deviation {deviation * 100:.2f}% exceeds 5 % for {w}x{h}"
        )
    return w, h, fb_h - h


def main(argv: list[str]) -> int:
    if len(argv) != 4:
        print(f"usage: {argv[0]} <fb_w> <fb_h> <label>", file=sys.stderr)
        return 2
    fb_w, fb_h, label = int(argv[1]), int(argv[2]), argv[3]
    try:
        w, h, reduced = canvas_16x9(fb_w, fb_h)
    except ValueError as e:
        print(f"{label}: ERROR: {e}", file=sys.stderr)
        return 2
    note = "unchanged H" if reduced == 0 else f"H reduced by {reduced}"
    dev = abs((w / h) / TARGET_RATIO - 1) * 100
    print(f"{label}: {w}x{h} ({note}, deviation {dev:.3f}%)")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
