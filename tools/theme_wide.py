#!/usr/bin/env python3
"""Generate wide-panel TWRP theme variants from the base portrait theme.

The base theme is authored for 1080x1920. On wider panels the engine
scales X and Y by different factors while fonts/images use min(), so
elements drift apart (double battery icon, squeezed input text). A
variant re-authored for the panel size makes all three scalers agree
(~1.0), which removes the drift by construction.

For each variant (name, W, H) this copies from <twres>/:
  ui.xml, splash.xml, pages/**/*.xml, themes/font.xml,
  resources/vars.xml
into <twres>_<variant>/, with geometry scaled to preserve the current
on-device look (values are final pixels at ~1.0 runtime scale):
  X-geometry x (W/1080), Y-geometry x (H/1920), font sizes x (W/1080).
Images, fonts, languages and the rest stay shared under /twres (the
variant ui.xml keeps absolute /twres/... include paths for everything
except pages + its own font.xml).

Only explicitly classified attributes are scaled; anything else numeric
is passed through with a loud warning (collected into the report, never
a silent guess). Non-numeric values (colors, paths, %var% expressions)
are never touched.

Usage:
  theme_wide.py <twres_dir> <out_root>
Generates twres_1280 (1280x2856), twres_1344 (1344x2992),
twres_1440 (1440x3120) under <out_root> (= ramdisk root, so the dirs
land next to twres/). Exit 2 on I/O errors. Prints a per-variant
report; warnings go to stdout (visible in the build log).
"""

import os
import re
import sys
import xml.etree.ElementTree as ET

BASE_W, BASE_H = 1080, 1920

VARIANTS = [
    # Wide slabs (portrait panels).
    ("1280", 1280, 2856),
    ("1344", 1344, 2992),
    ("1440", 1440, 3120),
    # Tablet (tangorpro, portrait 1600x2560) and fold inner displays
    # (nearly square): native full-bleed instead of the 16:9 letterbox.
    # X ratios here are large (up to 1.92) — proportions stay consistent
    # (runtime scales ~1.0), aesthetics get a hand-tune pass later.
    ("1600", 1600, 2560),
    ("1840", 1840, 2208),
    ("2076", 2076, 2152),
]

# Element attributes holding X geometry (loader: ScaleX).
# NOTE: "h" is Y everywhere it occurs (placement h= and the splash
# <resolution h=> spelling alike): base value x Y-ratio always lands
# on the exact target, so no context-sensitive rule is needed.
X_ATTRS = {"x", "w", "width", "dx", "dw", "padding", "sense", "radius"}
# Element attributes holding Y geometry (loader: ScaleY).
Y_ATTRS = {"y", "h", "height", "dy", "dh"}
# Font size attributes (loader: min-scale at runtime). `spacing` is the
# monospace letter-spacing in pixels (files.xml fixed font).
SIZE_ATTRS = {"size", "spacing"}

# Attributes that are provably not geometry (enums, counts, timings,
# multipliers, versions, paths-as-values). Silent passthrough.
SKIP_ATTRS = {
    "placement", "persist", "version", "minlen", "maxlen", "folders",
    "files", "default", "nav", "showrange", "showcurr", "changeondrag",
    "hasfocus", "disable", "render", "frame", "fps", "double",
    "retainaspect", "hidden", "allow", "scale", "revert_layout",
    "requireReload", "noalt", "multi", "length", "capslock", "min",
    "max", "side", "name", "value", "filename", "resource", "function",
    "style", "color", "bgcolor", "highlight", "text", "action",
    "multiplier", "variable",
}

# Variable-name rules for resources/vars.xml numerics (checked in order).
# Anything else numeric is warned + passed through; 0/1 pass silently
# (flags; scaling cannot change them meaningfully).
VAR_Y_RES = [
    re.compile(r"_y\d*$"), re.compile(r"_h\d*$"), re.compile(r"height"),
    re.compile(r"^lb_l\d+$"), re.compile(r"^lb_group_l\d+$"),
    re.compile(r"^bl_h\d+$"), re.compile(r"lineh$"), re.compile(r"recth$"),
    re.compile(r"sliderh$"),
]
VAR_X_RES = [
    re.compile(r"_x\d*$"), re.compile(r"_w\d*$"), re.compile(r"width"),
    re.compile(r"del"), re.compile(r"indent"), re.compile(r"caption"),
    re.compile(r"pill"), re.compile(r"cards"), re.compile(r"col"),
    re.compile(r"txt"), re.compile(r"nav"), re.compile(r"btn"),
    re.compile(r"input"), re.compile(r"content"), re.compile(r"listbox"),
    re.compile(r"about"), re.compile(r"card"), re.compile(r"row"),
    re.compile(r"back_button"), re.compile(r"slideout"), re.compile(r"ch_"),
    re.compile(r"chtxt"), re.compile(r"pattern"), re.compile(r"sliderw"),
    re.compile(r"rectw"), re.compile(r"cap"), re.compile(r"offset"),
    re.compile(r"slideout"), re.compile(r"switch_text"),
    re.compile(r"gl_text"), re.compile(r"ab_menu"), re.compile(r"snackbar"),
    re.compile(r"bg_"), re.compile(r"db_"), re.compile(r"dlg_"),
    re.compile(r"storage"), re.compile(r"console"), re.compile(r"terminal"),
    re.compile(r"dialog"), re.compile(r"tab_"), re.compile(r"main_"),
]
VAR_Y_EXPLICIT = {"dlg_input_line", "input_line_height", "mb_h_hide"}
VAR_X_EXPLICIT = {"btn_float_size", "pattern_size", "screen_width"}
VAR_Y_VARS_EXPLICIT = {"screen_height"}
# Known enum/flag variables: numeric but not geometry. Silent passthrough.
VAR_SKIP_EXPLICIT = {
    "center_clock", "auto_generate", "style_battery", "tw_app_install_status",
}

NUM_RE = re.compile(r"^-?\d+(\.\d+)?$")
ATTR_RE = re.compile(r'([A-Za-z_]+)="([^"]*)"')
VAR_RE = re.compile(r'<variable name="([^"]+)" value="([^"]*)"')


def scale_num(text, ratio):
    v = float(text)
    out = int(round(v * ratio))
    if v > 0 and out < 1:
        out = 1
    if "." in text:
        return str(float(out))
    return str(out)


def classify_attr(name):
    if name in X_ATTRS:
        return "x"
    if name in Y_ATTRS:
        return "y"
    if name in SIZE_ATTRS:
        return "min"
    if name in SKIP_ATTRS:
        return None
    return "?"


def classify_var(name, value):
    if value in ("0", "1"):
        return None
    if name in VAR_SKIP_EXPLICIT:
        return None
    if name in VAR_Y_EXPLICIT or name in VAR_Y_VARS_EXPLICIT:
        return "y"
    if name in VAR_X_EXPLICIT:
        return "x"
    for rx in VAR_Y_RES:
        if rx.search(name):
            return "y"
    for rx in VAR_X_RES:
        if rx.search(name):
            return "x"
    return "?"


def scale_attrs(text, ratios, warnings, where):
    def sub(m):
        name, val = m.group(1), m.group(2)
        if not NUM_RE.match(val):
            return m.group(0)
        axis = classify_attr(name)
        if axis is None:
            return m.group(0)
        if axis == "?":
            warnings.append(f"{where}: unclassified numeric attr {name}={val}, kept")
            return m.group(0)
        ratio = ratios[axis]
        return f'{name}="{scale_num(val, ratio)}"'

    return ATTR_RE.sub(sub, text)


def scale_vars(text, ratios, warnings, where):
    def sub(m):
        name, val = m.group(1), m.group(2)
        if not NUM_RE.match(val):
            return m.group(0)
        axis = classify_var(name, val)
        if axis is None:
            return m.group(0)
        if axis == "?":
            warnings.append(f"{where}: unclassified numeric var {name}={val}, kept")
            return m.group(0)
        return f'<variable name="{name}" value="{scale_num(val, ratios[axis])}"'

    return VAR_RE.sub(sub, text)


def convert_file(src, dst, variant, w, h, ratios, warnings):
    # newline="" keeps source line endings (CRLF files stay CRLF) so
    # diffs against the base theme show only real value changes.
    with open(src, "r", encoding="utf-8", errors="replace", newline="") as f:
        text = f.read()
    rel = os.path.relpath(src, TWRES)
    if rel == "ui.xml":
        text = text.replace("/twres/pages/", f"/twres_{variant}/pages/")
        # Variant font.xml carries scaled sizes; navbar/style/accent
        # stay shared (no geometry inside).
        text = text.replace(
            '<xml name="%fox_theme_path%/font.xml"',
            f'<xml name="/twres_{variant}/themes/font.xml"',
        )
    if os.path.basename(src) == "vars.xml" or src.endswith("splash.xml"):
        # vars.xml numerics classify by variable name; splash.xml mixes
        # the same <variable> form with element attrs below.
        text = scale_vars(text, ratios, warnings, rel)
        if src.endswith("splash.xml"):
            text = scale_attrs(text, ratios, warnings, rel)
    else:
        text = scale_attrs(text, ratios, warnings, rel)
    os.makedirs(os.path.dirname(dst), exist_ok=True)
    with open(dst, "w", encoding="utf-8", newline="") as f:
        f.write(text)


TWRES = ""


def generate(twres_dir, out_root):
    global TWRES
    TWRES = twres_dir
    # Files that make up a variant: layout + splash + own fonts/vars.
    # Images, shared styles, languages stay under /twres (absolute
    # include paths in ui.xml are kept, except pages + font.xml).
    wanted = ["ui.xml", "splash.xml", "themes/font.xml", "resources/vars.xml"]
    pages_dir = os.path.join(twres_dir, "pages")
    for root, _, files in os.walk(pages_dir):
        for fn in files:
            if fn.endswith(".xml"):
                wanted.append(os.path.relpath(os.path.join(root, fn), twres_dir))
    total = 0
    for variant, w, h in VARIANTS:
        ratios = {
            "x": w / BASE_W,
            "y": h / BASE_H,
            "min": min(w / BASE_W, h / BASE_H),
        }
        warnings = []
        count = 0
        for rel in wanted:
            src = os.path.join(twres_dir, rel)
            if not os.path.isfile(src):
                warnings.append(f"missing in base theme: {rel}, skipped")
                continue
            # Absolute resolution for the tag; ratios for the rest.
            tag_ratios = dict(ratios)
            dst = os.path.join(out_root, f"twres_{variant}", rel)
            convert_file(src, dst, variant, w, h, tag_ratios, warnings)
            count += 1
        # Validate: output must stay well-formed XML when the base is.
        try:
            ET.parse(os.path.join(twres_dir, "ui.xml"))
            base_ok = True
        except ET.ParseError:
            base_ok = False
        if base_ok:
            for rel in wanted:
                dst = os.path.join(out_root, f"twres_{variant}", rel)
                if os.path.isfile(dst):
                    try:
                        ET.parse(dst)
                    except ET.ParseError as e:
                        warnings.append(f"OUTPUT MALFORMED: {rel}: {e}")
        print(f"[THEME] twres_{variant} ({w}x{h}): {count} files")
        for wn in warnings:
            print(f"[THEME] WARNING: {wn}")
        total += count
    return total


def main(argv):
    if len(argv) != 3:
        print(f"usage: {argv[0]} <twres_dir> <out_root>", file=sys.stderr)
        return 2
    twres_dir, out_root = argv[1], argv[2]
    if not os.path.isfile(os.path.join(twres_dir, "ui.xml")):
        print(f"ERROR: no ui.xml in {twres_dir}", file=sys.stderr)
        return 2
    n = generate(twres_dir, out_root)
    print(f"[THEME] done: {n} files across {len(VARIANTS)} variants")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
