#!/usr/bin/env python3
"""Kernel profile resolver for OrangeFox Pixel builds.

Reads per-family/per-device `kernels` profiles from
families/*/family.json and devices/*/pixel.json and either:

  --fingerprint FAMILY [DEVICE] VER
      Print one JSON object per device: {device, hash, flags} where flags
      is the canonical sorted flag list of the effective profile
      (cmdline + bootconfig_append). Used by build.sh to group devices
      with identical configs into shared images.
  --generate FAMILY [DEVICE] VER OUT.mk
      Write a Make snippet with VENDOR_CMDLINE / BOARD_BOOTCONFIG /
      FOX_KERNEL_VER for the effective profile. Included at the end of
      family.mk (override wins over the hardcoded default).
  --list FAMILY [DEVICE]
      Print space-separated available kernel versions (family ∪ device)
      plus " (default: X)" suffix. Used for error messages and menus.

Priority: devices/<dev>/pixel.json `kernels[VER]` replaces
families/<fam>/family.json `kernels[VER]` wholesale when present.
Unknown version / bad typed envelope -> exit 2 with available list.

Typed cmdline envelope: {"type": "str"|"arr"|"list", "value": ...}
  str  -> one string, split on whitespace
  arr  -> ["k=v", ...] verbatim
  list -> [{"k": "v"}, ...] in order
"""

from __future__ import annotations

import hashlib
import json
import sys
from pathlib import Path

TREE = Path(__file__).resolve().parent.parent.parent  # .../include/prebuilt -> tree root


def fail(msg: str) -> int:
    print(f"gen_kernel_mk: ERROR: {msg}", file=sys.stderr)
    return 2


def load_json(path: Path):
    try:
        return json.loads(path.read_text())
    except Exception as e:
        print(f"gen_kernel_mk: ERROR: bad JSON {path}: {e}", file=sys.stderr)
        sys.exit(2)


def norm_cmdline(node) -> list:
    """Normalize the typed cmdline envelope to a flag list."""
    if not isinstance(node, dict) or "type" not in node or "value" not in node:
        print("gen_kernel_mk: ERROR: cmdline must be {type, value}", file=sys.stderr)
        sys.exit(2)
    t, v = node["type"], node["value"]
    if t == "str":
        if not isinstance(v, str):
            print("gen_kernel_mk: ERROR: str cmdline needs a string", file=sys.stderr)
            sys.exit(2)
        return v.split()
    if t == "arr":
        if not isinstance(v, list) or not all(isinstance(x, str) for x in v):
            print("gen_kernel_mk: ERROR: arr cmdline needs [str]", file=sys.stderr)
            sys.exit(2)
        return list(v)
    if t == "list":
        if not isinstance(v, list) or not all(isinstance(x, dict) for x in v):
            print("gen_kernel_mk: ERROR: list cmdline needs [{k: v}]", file=sys.stderr)
            sys.exit(2)
        out = []
        for d in v:
            out.extend(f"{k}={val}" for k, val in d.items())
        return out
    print(f"gen_kernel_mk: ERROR: unknown cmdline type {t!r}", file=sys.stderr)
    sys.exit(2)


def family_devices(family: str) -> list:
    devs = []
    for pix in sorted((TREE / "devices").glob("*/pixel.json")):
        pj = load_json(pix)
        if pj.get("family") == family:
            devs.append((pix.parent.name, pj))
    return devs


def effective_profile(fam_json: dict, dev_json: dict | None, ver: str):
    """Return (profile_dict, source) with device-over-family priority."""
    if dev_json and ver in (dev_json.get("kernels") or {}):
        return dev_json["kernels"][ver], "device"
    if ver in (fam_json.get("kernels") or {}):
        return fam_json["kernels"][ver], "family"
    return None, None


def canonical(profile: dict) -> list:
    """Canonical sorted flag list: cmdline + bootconfig_append.
    Sorting is for GROUP COMPARISON ONLY (order-insensitive); the
    generated CMDLINE keeps author order (see cmd_generate)."""
    flags = norm_cmdline(profile.get("cmdline", {"type": "arr", "value": []}))
    for b in profile.get("bootconfig_append", []) or []:
        flags.append(f"bootconfig:{b}")
    return sorted(flags)


def cmd_fingerprint(args) -> int:
    if len(args) < 2:
        return fail("fingerprint needs FAMILY [DEVICE] VER")
    family, ver = args[0], args[-1]
    only = args[1] if len(args) == 3 else None
    famfile = TREE / "families" / family / "family.json"
    if not famfile.is_file():
        return fail(f"no family.json for {family}")
    fam = load_json(famfile)
    devs = family_devices(family)
    if only:
        devs = [(d, j) for d, j in devs if d == only]
        if not devs:
            return fail(f"device {only} not in family {family}")
    result = []
    for dev, pj in devs:
        prof, src = effective_profile(fam, pj, ver)
        if prof is None:
            avail = sorted(
                set((fam.get("kernels") or {}).keys())
                | set((pj.get("kernels") or {}).keys())
            )
            return fail(f"no kernels[{ver}] for {dev}; available: {avail or ['(none)']}")
        flags = canonical(prof)
        h = hashlib.sha256(json.dumps(flags).encode()).hexdigest()[:12]
        result.append({"device": dev, "hash": h, "source": src, "flags": flags})
    print(json.dumps(result))
    return 0


def mk_escape(s: str) -> str:
    return s.replace("\\", "\\\\").replace('"', '\\"')


def cmd_list(args) -> int:
    if len(args) not in (1, 2):
        return fail("list needs FAMILY [DEVICE]")
    family = args[0]
    only = args[1] if len(args) == 2 else None
    famfile = TREE / "families" / family / "family.json"
    if not famfile.is_file():
        return fail(f"no family.json for {family}")
    fam = load_json(famfile)
    vers = set((fam.get("kernels") or {}).keys())
    if only:
        pixfile = TREE / "devices" / only / "pixel.json"
        if not pixfile.is_file():
            return fail(f"no pixel.json for {only}")
        vers |= set((load_json(pixfile).get("kernels") or {}).keys())
    else:
        for _, pj in family_devices(family):
            vers |= set((pj.get("kernels") or {}).keys())
    default = fam.get("default_kernel", "")
    line = " ".join(sorted(vers)) if vers else "(none)"
    if default:
        line += f" (default: {default})"
    print(line)
    return 0


def cmd_generate(args) -> int:
    if len(args) != 4:
        return fail("generate needs FAMILY [DEVICE] VER OUT.mk (use - for DEVICE)")
    family, dev, ver, out = args[0], args[1], args[2], args[3]
    famfile = TREE / "families" / family / "family.json"
    if not famfile.is_file():
        return fail(f"no family.json for {family}")
    fam = load_json(famfile)
    pj = None
    if dev != "-":
        pixfile = TREE / "devices" / dev / "pixel.json"
        if not pixfile.is_file():
            return fail(f"no pixel.json for {dev}")
        pj = load_json(pixfile)
    prof, src = effective_profile(fam, pj, ver)
    if prof is None:
        return fail(f"no kernels[{ver}] (family {family}, device {dev})")
    # Author order for the image (byte-stable vs the old hardcoded string);
    # bootconfig_append rides BOARD_BOOTCONFIG, never the cmdline.
    flags = norm_cmdline(prof.get("cmdline", {"type": "arr", "value": []}))
    boot = list(prof.get("bootconfig_append", []) or [])
    lines = [
        "# Generated by gen_kernel_mk.py — DO NOT EDIT.",
        f"# family={family} device={dev} kernel={ver} source={src}",
        f"FOX_KERNEL_VER := {ver}",
        f'VENDOR_CMDLINE := "{" ".join(mk_escape(f) for f in flags)}"',
    ]
    for b in boot:
        lines.append(f"BOARD_BOOTCONFIG += {b}")
    Path(out).write_text("\n".join(lines) + "\n")
    print(f"gen_kernel_mk: wrote {out} ({src} profile, {len(flags)} flags)")
    return 0


def main() -> int:
    if len(sys.argv) < 3 or sys.argv[1] not in ("--fingerprint", "--generate", "--list"):
        print(__doc__.strip().splitlines()[0], file=sys.stderr)
        print("usage: gen_kernel_mk.py --fingerprint FAMILY [DEVICE] VER", file=sys.stderr)
        print("       gen_kernel_mk.py --generate FAMILY [DEVICE] VER OUT.mk", file=sys.stderr)
        print("       gen_kernel_mk.py --list FAMILY [DEVICE]", file=sys.stderr)
        return 2
    if sys.argv[1] == "--fingerprint":
        return cmd_fingerprint(sys.argv[2:])
    if sys.argv[1] == "--list":
        return cmd_list(sys.argv[2:])
    return cmd_generate(sys.argv[2:])


if __name__ == "__main__":
    sys.exit(main())
