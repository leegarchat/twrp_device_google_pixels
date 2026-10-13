#!/usr/bin/env python3
"""extract_touch_modules.py — pull touch .ko set for ONE device out of firmware.

Sources (auto-detected):
  --zip FILE   fastboot factory zip (contains image-<device>-*.zip with
               android-info.txt) OR a payload.bin-style ROM zip
               (META-INF/com/android/metadata + payload.bin).
  --img DIR    directory with pre-extracted *.img (vendor_boot.img,
               vendor_kernel_boot.img, vendor_dlkm.img, system_dlkm.img).

What it does:
  1. Detects the archive type (fastboot vs payload) and the EXACT device
     (not family): bootloader/radio filenames > image zip name >
     android-info board list > OTA metadata pre-device > dirname hint.
     --device overrides everything.
  2. Reads the device's module set from devices/<codename>/pixel.json
     (touch_modules + preload_modules) — the recorded per-device sets.
  3. Extracts every .ko from all module-bearing partitions:
       vendor_boot / vendor_kernel_boot : magiskboot unpack -> ramdisk
         fragments (*.cpio) -> lib/modules (boot-image path)
       vendor_dlkm / system_dlkm        : ext4/erofs raw image -> lib/modules
         via 7z, then debugfs, then sudo loop-mount (raw-data path)
  4. Reads each .ko's vermagic (modinfo, bytes fallback) to learn the exact
     kernel it was built for (6.1.162-android14-...) and copies the device
     set into:
       <out>/<device>/touch/<major.minor>/<patch>/android<XX>/<pagesize>/
         <mod>_android<XX>-<major.minor>.<patch>_<pagesize>.ko
     (--all copies every .ko found, unfiltered.)

Needs on PATH (or --magiskboot/--dumper): magiskboot, payload-dumper-go
(payload zips only), 7z or debugfs for ext4 images.

Examples:
  ./tools/extract_touch_modules.py --zip ~/panther-factory.zip --out /tmp/mods --tmpdir /mnt/big/tmp
  ./tools/extract_touch_modules.py --zip ~/EvolutionX-husky.zip --out /tmp/mods --tmpdir /mnt/big/tmp
  ./tools/extract_touch_modules.py --img out/target/product/shiba --out /tmp/mods
"""

import argparse
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import zipfile

# Partitions that can carry kernel modules, in source-priority order
# (first hit wins when the same module+version exists twice).
WANT_PARTS = ("vendor_dlkm", "vendor_kernel_boot", "vendor_boot", "system_dlkm")
BOOT_PARTS = ("vendor_boot", "vendor_kernel_boot")
FS_PARTS = ("vendor_dlkm", "system_dlkm")

TREE_ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def log(msg, quiet=False):
    if not quiet:
        print(f"[komods] {msg}", flush=True)


def run(cmd, cwd=None, quiet=False):
    if not quiet:
        log("$ " + " ".join(cmd))
    return subprocess.run(cmd, cwd=cwd, capture_output=True, text=True)


def find_tool(name, extra=(), override=None):
    if override and os.path.isfile(override) and os.access(override, os.X_OK):
        return override
    found = shutil.which(name)
    if found:
        return found
    home = os.path.expanduser("~")
    for d in extra + (os.path.join(home, "platform-tools"), "/home/leegarboot/platform-tools"):
        cand = os.path.join(d, name)
        if os.path.isfile(cand) and os.access(cand, os.X_OK):
            return cand
    return None


def norm(name):
    return name.replace("-", "_").lower()


def load_device_set(tree_root, device):
    pj = os.path.join(tree_root, "devices", device, "pixel.json")
    if not os.path.isfile(pj):
        known = sorted(d for d in os.listdir(os.path.join(tree_root, "devices"))
                       if os.path.isdir(os.path.join(tree_root, "devices", d)))
        raise SystemExit(f"unknown device '{device}'. known: {' '.join(known)} "
                         f"(or fix devices/<codename>/pixel.json)")
    data = json.load(open(pj))
    mods = list(data.get("touch_modules", [])) + list(data.get("preload_modules", []))
    if not mods:
        raise SystemExit(f"devices/{device}/pixel.json has no touch_modules")
    return mods


def known_devices(tree_root):
    devdir = os.path.join(tree_root, "devices")
    return sorted(d for d in os.listdir(devdir) if os.path.isdir(os.path.join(devdir, d)))


# ---------------- device detection ----------------

def detect_from_android_info(text):
    """android-info.txt -> board list. Returns (single_board_or_None, full_list)."""
    boards = []
    for line in text.splitlines():
        line = line.strip()
        if line.startswith("require board="):
            boards = [b.strip() for b in line.split("=", 1)[1].split("|") if b.strip()]
    if len(boards) == 1:
        return boards[0], boards
    return None, boards


def hint_from_name(path, known):
    base = os.path.basename(path).lower()
    for dev in known:
        if dev.lower() in base:
            return dev
    return None


def detect_device(args, staged, tree_root):
    """staged: dict with optional keys android_info, ota_metadata, imgdir, inner_name, outer_names."""
    known = known_devices(tree_root)
    if args.device:
        if args.device not in known:
            raise SystemExit(f"--device {args.device} not in devices/ (known: {' '.join(known)})")
        return args.device, "override --device"
    if staged.get("android_info"):
        single, boards = detect_from_android_info(staged["android_info"])
        if single and single in known:
            return single, "android-info.txt board"
        # multi-board factory (e.g. cloudripper|ravenclaw|cheetah|panther):
        # disambiguate via image/bootloader filenames, else fail loudly.
        for src in (staged.get("inner_name", ""), " ".join(staged.get("outer_names", []))):
            hit = hint_from_name(src, [b for b in boards if b in known] or known)
            if hit:
                return hit, f"filename hint ({src}) over multi-board {boards}"
        if boards:
            cands = [b for b in boards if b in known] or boards
            raise SystemExit(f"factory covers several devices {boards}; pass --device "
                             f"({' / '.join(cands)})")
    if staged.get("ota_metadata"):
        for line in staged["ota_metadata"].splitlines():
            if line.startswith("pre-device="):
                dev = line.split("=", 1)[1].strip()
                if dev in known:
                    return dev, "OTA metadata pre-device"
                raise SystemExit(f"OTA pre-device '{dev}' not in devices/")
    if staged.get("imgdir"):
        ai = os.path.join(staged["imgdir"], "android-info.txt")
        if os.path.isfile(ai):
            single, _ = detect_from_android_info(open(ai).read())
            if single and single in known:
                return single, "imgdir android-info.txt"
        hit = hint_from_name(os.path.basename(os.path.normpath(staged["imgdir"])), known)
        if hit:
            return hit, "imgdir name hint"
    # last resort: zip filename hint
    if args.zip:
        hit = hint_from_name(args.zip, known)
        if hit:
            return hit, "zip filename hint"
    raise SystemExit("cannot detect device; pass --device CODENAME")


# ---------------- image acquisition ----------------

def ensure_space(path, need_gb, quiet):
    try:
        free_gb = shutil.disk_usage(path).free / 1e9
    except OSError:
        return
    if free_gb < need_gb:
        log(f"WARNING: {free_gb:.1f} GB free in {path}, want ~{need_gb} GB", quiet)


def acquire_from_zip(zpath, tmpdir, dumper, args):
    """Returns (imgdir_with_.img_files, staged_hints)."""
    staged = {}
    with zipfile.ZipFile(zpath) as zf:
        names = zf.namelist()
    staged["outer_names"] = names
    has_payload = any(n == "payload.bin" or n.endswith("/payload.bin") for n in names)
    inner_imgs = [n for n in names if re.search(r"image-.*\.zip$", n)]
    is_image_zip = any(os.path.basename(n) == "android-info.txt" for n in names) and \
        any(re.search(r"vendor_boot\.img$", n) for n in names)

    imgdir = os.path.join(tmpdir, "imgs")
    os.makedirs(imgdir, exist_ok=True)

    if has_payload and not args.img:
        log("archive type: payload.bin ROM zip")
        with zipfile.ZipFile(zpath) as zf:
            for tiny in ("META-INF/com/android/metadata", "payload_properties.txt"):
                if tiny in names:
                    open(os.path.join(tmpdir, os.path.basename(tiny)), "wb").write(zf.read(tiny))
            md = os.path.join(tmpdir, "metadata")
            if os.path.isfile(md):
                staged["ota_metadata"] = open(md).read()
        ensure_space(tmpdir, 8, args.quiet)
        # OTA diffs carry only CHANGED partitions (gs101 OTAs lack
        # vendor_kernel_boot/system_dlkm entirely). Ask the dumper what the
        # payload has and dump the intersection — a missing partition is a
        # skipped source, never a fatal error.
        have = set()
        r = run([dumper, "-l", zpath], quiet=True)
        # "-l" prints one "Found partitions:" line, comma-separated names
        # with sizes: "abl (2.1 MB), vendor_dlkm (43 MB), ...".
        for m in re.finditer(r"([a-z0-9_]+)\s*\(", r.stdout or "", re.I):
            have.add(m.group(1).lower())
        parts = [p for p in WANT_PARTS if p in have] if have else list(WANT_PARTS)
        skipped = [p for p in WANT_PARTS if p not in parts]
        if skipped:
            log(f"partitions absent from payload, skipped: {', '.join(skipped)}")
        if not parts:
            raise SystemExit("payload has none of: " + ", ".join(WANT_PARTS))
        r = run([dumper, "-p", ",".join(parts), "-o", imgdir,
                 "-c", "16", os.path.join(zpath)], quiet=args.quiet)
        # payload-dumper-go wants (payload, outdir) positionally on some builds;
        # retry that form when the flag form fails.
        if r.returncode != 0:
            log(f"dumper flag form failed, retrying positional: {r.stderr.strip()[:200]}")
            r = run([dumper, "-p", ",".join(parts), zpath, imgdir], quiet=args.quiet)
        if r.returncode != 0:
            raise SystemExit(f"payload-dumper-go failed:\n{r.stderr[-2000:]}")
    elif inner_imgs or is_image_zip:
        log("archive type: fastboot factory zip")
        with zipfile.ZipFile(zpath) as zf:
            if is_image_zip:
                inner = None
                pool, poolname = zf, "(outer zip)"
            else:
                inner = sorted(inner_imgs, key=len)[0]
                staged["inner_name"] = inner
                log(f"extracting inner {inner} ({os.path.getsize(zpath)/1e9:.1f} GB copy)...")
                inner_path = os.path.join(tmpdir, os.path.basename(inner))
                with zf.open(inner) as src, open(inner_path, "wb") as dst:
                    shutil.copyfileobj(src, dst, 1 << 20)
                pool, poolname = zipfile.ZipFile(inner_path), inner
            with pool as pf:
                inames = pf.namelist()
                ai = next((n for n in inames if os.path.basename(n) == "android-info.txt"), None)
                if ai:
                    staged["android_info"] = pf.read(ai).decode(errors="replace")
                for part in WANT_PARTS:
                    hit = next((n for n in inames if re.search(rf"{part}(_[a-z])?\.img$", n)), None)
                    if hit:
                        log(f"extracting {hit}...")
                        with pf.open(hit) as src, open(os.path.join(imgdir, f"{part}.img"), "wb") as dst:
                            shutil.copyfileobj(src, dst, 1 << 20)
                    else:
                        log(f"{part}.img not in factory, skipped")
    else:
        raise SystemExit("zip is neither payload.bin nor fastboot factory "
                         f"(top entries: {names[:8]})")
    return imgdir, staged


# ---------------- .ko harvesting ----------------

def is_boot_image(path):
    try:
        with open(path, "rb") as f:
            magic = f.read(12)
    except OSError:
        return False
    return magic.startswith(b"ANDROID!") or magic.startswith(b"VNDRBOOT")


def _u32(buf, off):
    import struct
    return struct.unpack_from("<I", buf, off)[0]


def _roundup(n, page):
    return (n + page - 1) // page * page


def harvest_vendor_boot_v4(img, workdir, quiet):
    """vendor_boot v4 (bootimg.h): header | ramdisk section | dtb |
    ramdisk table | bootconfig. Table entries give (size, offset, type)
    per fragment; each blob is lz4 -> cpio. Returns [ko paths]."""
    out = []
    with open(img, "rb") as f:
        data = f.read()
    if data[:8] != b"VNDRBOOT" or _u32(data, 8) != 4:
        return out
    page = _u32(data, 12)
    total = _u32(data, 24)
    dtb_size = _u32(data, 2100)
    table_size = _u32(data, 2112)
    entry_num = _u32(data, 2116)
    entry_size = _u32(data, 2120)
    if page not in (2048, 4096) or not (1 <= entry_num <= 16) or entry_size < 60:
        log(f"{os.path.basename(img)}: implausible v4 table "
            f"(page={page} num={entry_num} esize={entry_size}), skipped", quiet)
        return out
    o = _roundup(2128, page)
    table_off = o + _roundup(total, page) + _roundup(dtb_size, page)
    lz4 = shutil.which("lz4")
    if not lz4:
        raise SystemExit("lz4 CLI not found (needed for vendor_boot ramdisks)")
    for i in range(entry_num):
        e = table_off + i * entry_size
        size, off, typ = _u32(data, e), _u32(data, e + 4), _u32(data, e + 8)
        name = data[e + 12:e + 44].split(b"\0")[0].decode(errors="replace")
        if size == 0 or off + size > total + 4096:
            log(f"fragment {i} ({name or '?'}): bad size/off, skipped", quiet)
            continue
        blob = os.path.join(workdir, f"frag{i}.lz4")
        cpio = os.path.join(workdir, f"frag{i}.cpio")
        with open(blob, "wb") as f:
            f.write(data[o + off:o + off + size])
        r = run([lz4, "-d", "-f", blob, cpio], quiet=True)
        if r.returncode != 0 or not os.path.isfile(cpio):
            log(f"fragment {i} ({name or '?'}): lz4 decode failed, skipped", quiet)
            continue
        with open(cpio, "rb") as f:
            if f.read(6) != b"070701":
                log(f"fragment {i} ({name or '?'}): not a cpio, skipped", quiet)
                continue
        xdir = os.path.join(workdir, f"frag{i}")
        os.makedirs(xdir, exist_ok=True)
        if run(["cpio", "-id", "--quiet", f"--file={cpio}"], cwd=xdir, quiet=True).returncode != 0:
            log(f"fragment {i}: cpio extract failed, skipped", quiet)
            continue
        for r2, _, fs in os.walk(xdir):
            for f in fs:
                if f.endswith(".ko"):
                    out.append(os.path.join(r2, f))
        log(f"fragment {i} ({name or '?'} type={typ}): "
            f"{sum(1 for r2, _, fs in os.walk(xdir) for f in fs if f.endswith('.ko'))} .ko", quiet)
    return out


def harvest_boot_image(img, workdir, magiskboot, quiet):
    """vendor_boot v4: own table parser. Classic ANDROID! images: magiskboot
    unpack -> every *.cpio. Returns [ko paths]."""
    out = []
    try:
        with open(img, "rb") as f:
            magic = f.read(8)
    except OSError:
        return out
    if magic == b"VNDRBOOT":
        return harvest_vendor_boot_v4(img, workdir, quiet)
    if magic != b"ANDROID!":
        log(f"{os.path.basename(img)}: not a boot image, skipped", quiet)
        return out
    udir = os.path.join(workdir, "unpack_" + os.path.basename(img).replace(".img", ""))
    os.makedirs(udir, exist_ok=True)
    r = run([magiskboot, "unpack", img], cwd=udir, quiet=quiet)
    if r.returncode != 0:
        log(f"magiskboot unpack failed for {img}: {(r.stderr or r.stdout)[-300:]}", quiet)
        return out
    for root, _, files in os.walk(udir):
        for fn in files:
            if not fn.endswith(".cpio"):
                continue
            cpio = os.path.join(root, fn)
            xdir = os.path.join(workdir, "cpio_" + fn.replace(".cpio", ""))
            os.makedirs(xdir, exist_ok=True)
            # system cpio first (keeps modes), magiskboot extract as fallback.
            r1 = run(["cpio", "-id", "--quiet", f"--file={cpio}"],
                     cwd=xdir, quiet=True)
            if r1.returncode != 0:
                run([magiskboot, "cpio", cpio, "extract"], cwd=xdir, quiet=True)
            for r2, _, fs in os.walk(xdir):
                for f in fs:
                    if f.endswith(".ko"):
                        out.append(os.path.join(r2, f))
    return out


def harvest_fs_image(img, workdir, quiet):
    """ext4/erofs image -> lib/modules/*.ko. Cascade: 7z, debugfs, sudo mount."""
    out = []
    xdir = os.path.join(workdir, "fs_" + os.path.basename(img).replace(".img", ""))
    # 1) 7z reads ext4/erofs directly, no privileges needed.
    if shutil.which("7z"):
        r = run(["7z", "x", img, f"-o{xdir}", "lib/modules", "-y"], quiet=True)
        if r.returncode == 0:
            for r2, _, fs in os.walk(os.path.join(xdir, "lib", "modules")):
                out.extend(os.path.join(r2, f) for f in fs if f.endswith(".ko"))
            if out:
                return out
    # 2) debugfs (ext4): list + dump each .ko.
    if shutil.which("debugfs"):
        r = run(["debugfs", "-R", "ls -l lib/modules", img], quiet=True)
        kos = re.findall(r"(\S+\.ko)\b", r.stdout or "")
        if kos:
            ddir = os.path.join(xdir, "lib", "modules")
            os.makedirs(ddir, exist_ok=True)
            for ko in dict.fromkeys(kos):
                dst = os.path.join(ddir, os.path.basename(ko))
                run(["debugfs", "-R", f"dump lib/modules/{ko} {dst}", img], quiet=True)
                if os.path.isfile(dst):
                    out.append(dst)
            if out:
                return out
    # 3) privileged loop mount, read-only, non-interactive sudo only.
    # NOTE: files MUST be copied out before umount — paths inside the
    # mountpoint dangle afterwards (erofs images only ever reach this
    # step: 7z/debugfs cannot read erofs, so without the copy every .ko
    # below comes back with unreadable vermagic).
    mnt = os.path.join(workdir, "mnt_" + os.path.basename(img).replace(".img", ""))
    os.makedirs(mnt, exist_ok=True)
    r = run(["sudo", "-n", "mount", "-o", "loop,ro", img, mnt], quiet=True)
    if r.returncode == 0:
        try:
            ddir = os.path.join(xdir, "lib", "modules")
            for cand in ("lib/modules", "lib"):
                base = os.path.join(mnt, cand)
                hit = False
                for r2, _, fs in os.walk(base):
                    for f in fs:
                        if not f.endswith(".ko"):
                            continue
                        rel = os.path.relpath(r2, base)
                        dstdir = os.path.join(ddir, rel)
                        os.makedirs(dstdir, exist_ok=True)
                        try:
                            shutil.copy2(os.path.join(r2, f),
                                         os.path.join(dstdir, f))
                        except OSError:
                            continue
                        out.append(os.path.join(dstdir, f))
                        hit = True
                if hit:
                    break
        finally:
            run(["sudo", "-n", "umount", mnt], quiet=True)
    else:
        log(f"{os.path.basename(img)}: 7z+debugfs found nothing, sudo mount unavailable", quiet)
    return out


VERMAGIC_RE = re.compile(rb"vermagic=([^\x00]+)")
KVER_RE = re.compile(r"(\d+)\.(\d+)\.(\d+).*?android(\d+)")


def read_vermagic(path):
    """modinfo first, raw .modinfo bytes fallback. Returns vermagic string or None."""
    if shutil.which("modinfo"):
        r = run(["modinfo", "-F", "vermagic", path], quiet=True)
        if r.returncode == 0 and r.stdout.strip():
            return r.stdout.strip()
    try:
        with open(path, "rb") as f:
            data = f.read(1 << 20)
    except OSError:
        return None
    m = VERMAGIC_RE.search(data)
    return m.group(1).decode(errors="replace").strip() if m else None


def parse_vermagic(vm):
    """'6.1.162-android14-11-...' -> (6, 1, 162, 14). android part optional."""
    m = KVER_RE.search(vm or "")
    if m:
        return tuple(int(g) for g in m.groups())
    m = re.search(r"(\d+)\.(\d+)\.(\d+)", vm or "")
    if m:
        return int(m.group(1)), int(m.group(2)), int(m.group(3)), 0
    return None


def sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()[:16]


# ---------------- main ----------------

def main():
    ap = argparse.ArgumentParser(description="extract per-device touch .ko set from firmware")
    src = ap.add_mutually_exclusive_group(required=True)
    src.add_argument("--zip", help="fastboot factory zip or payload.bin ROM zip")
    src.add_argument("--img", help="dir with pre-extracted *.img files")
    ap.add_argument("--out", required=True, help="output root; layout <out>/<device>/touch/...")
    ap.add_argument("--tmpdir", help="scratch dir (default: auto under /tmp; big zips want disk)")
    ap.add_argument("--device", help="force exact codename, skip auto-detect")
    ap.add_argument("--pagesize", default="4k", choices=("4k", "16k"),
                    help=".ko files carry no pagesize; target device value (default 4k)")
    ap.add_argument("--all", action="store_true", help="copy every .ko found, not just the device set")
    ap.add_argument("--keep-tmp", action="store_true", help="do not delete scratch dir")
    ap.add_argument("--magiskboot", help="path override")
    ap.add_argument("--dumper", help="payload-dumper-go path override")
    ap.add_argument("--quiet", action="store_true")
    args = ap.parse_args()

    magiskboot = find_tool("magiskboot", override=args.magiskboot)
    if not magiskboot:
        raise SystemExit("magiskboot not found (PATH, ~/platform-tools, or --magiskboot)")
    dumper = None
    if args.zip:
        with zipfile.ZipFile(args.zip) as zf:
            need_dump = any(n == "payload.bin" or n.endswith("/payload.bin") for n in zf.namelist())
        if need_dump:
            dumper = find_tool("payload-dumper-go", extra=("payload_dumper",),
                              override=args.dumper)
            if not dumper:
                raise SystemExit("payload-dumper-go not found (PATH, ~/platform-tools, or --dumper)")

    tmpdir = args.tmpdir or tempfile.mkdtemp(prefix="foxkomods_")
    os.makedirs(tmpdir, exist_ok=True)
    log(f"scratch: {tmpdir}")
    workdir = os.path.join(tmpdir, "work")
    alldir = os.path.join(tmpdir, "all_ko")
    os.makedirs(workdir, exist_ok=True)
    os.makedirs(alldir, exist_ok=True)

    try:
        staged = {}
        if args.zip:
            imgdir, staged = acquire_from_zip(args.zip, tmpdir, dumper, args)
        else:
            imgdir = args.img
            staged["imgdir"] = args.img

        device = detect_device(args, staged, TREE_ROOT)[0]
        log(f"device: {device}")
        want = load_device_set(TREE_ROOT, device)
        want_norm = {norm(m): m for m in want}
        log(f"device set: {len(want)} modules ({', '.join(want)})")

        found = {}  # norm_name -> [(priority, src_path)]
        for prio, part in enumerate(WANT_PARTS):
            img = os.path.join(imgdir, f"{part}.img")
            if not os.path.isfile(img):
                continue
            log(f"--- {part}.img ({os.path.getsize(img)/1e6:.0f} MB)")
            if part in BOOT_PARTS or is_boot_image(img):
                kos = harvest_boot_image(img, workdir, magiskboot, args.quiet)
            else:
                kos = harvest_fs_image(img, workdir, args.quiet)
            log(f"{part}: {len(kos)} .ko harvested")
            for ko in kos:
                stem = os.path.basename(ko)[:-3]
                found.setdefault(norm(stem), []).append((prio, ko))

        if not found:
            raise SystemExit("no .ko harvested from any partition")

        out_dev = os.path.join(args.out, device, "touch")
        copied, skipped, missing = 0, [], []
        seen_versions = {}
        for wnorm, worig in sorted(want_norm.items()):
            hits = found.get(wnorm, [])
            if not hits and not args.all:
                # file spelling may keep the dash (rare); try raw too.
                hits = found.get(wnorm.replace("_", "-"), [])
            if not hits:
                if not args.all:
                    missing.append(worig)
                continue
            hits.sort(key=lambda t: t[0])
            src = hits[0][1]
            vm = read_vermagic(src)
            parsed = parse_vermagic(vm)
            if not parsed:
                skipped.append((worig, f"unreadable vermagic ({vm})"))
                continue
            maj, mn, patch, api = parsed
            seen_versions.setdefault(f"{maj}.{mn}.{patch}/android{api}", []).append(worig)
            fname = f"{os.path.basename(src)[:-3]}_android{api}-{maj}.{mn}.{patch}_{args.pagesize}.ko"
            dstdir = os.path.join(out_dev, f"{maj}.{mn}", str(patch), f"android{api}", args.pagesize)
            os.makedirs(dstdir, exist_ok=True)
            dst = os.path.join(dstdir, fname)
            if os.path.isfile(dst):
                if sha256(dst) == sha256(src):
                    log(f"{worig}: identical already in {dstdir}, skipped")
                    copied += 1
                    continue
                log(f"WARNING: {dst} differs from {src} (dup module, keeping first)")
                continue
            shutil.copy2(src, dst)
            log(f"{worig}: {vm} -> {os.path.relpath(dst, args.out)}")
            copied += 1

        extra = 0
        if args.all:
            for wnorm, hits in sorted(found.items()):
                if wnorm in want_norm:
                    continue
                src = sorted(hits, key=lambda t: t[0])[0][1]
                vm = read_vermagic(src)
                parsed = parse_vermagic(vm)
                if not parsed:
                    continue
                maj, mn, patch, api = parsed
                fname = f"{os.path.basename(src)[:-3]}_android{api}-{maj}.{mn}.{patch}_{args.pagesize}.ko"
                dstdir = os.path.join(out_dev, f"{maj}.{mn}", str(patch), f"android{api}", args.pagesize)
                os.makedirs(dstdir, exist_ok=True)
                shutil.copy2(src, os.path.join(dstdir, fname))
                extra += 1

        print("=" * 60)
        print(f"device : {device}")
        print(f"kvers  : {', '.join(sorted(seen_versions)) or '-'}")
        print(f"copied : {copied}/{len(want)} device modules -> {out_dev}")
        if missing:
            print(f"MISSING ({len(missing)}): {' '.join(missing)}")
        if skipped:
            print(f"SKIPPED: {skipped}")
        if args.all:
            print(f"extra non-set modules: {extra}")
        if copied == 0 and extra == 0:
            # Total failure must be loud: callers (fetch_google_firmware
            # surgical/full) delete the source + logs on RC=0, which turns
            # "nothing matched" into "zero modules, empty logs".
            raise SystemExit(
                f"no modules copied for {device} "
                f"(missing {len(missing)}, skipped {len(skipped)}); "
                f"source kept for inspection")
    finally:
        if not args.keep_tmp and not args.tmpdir:
            shutil.rmtree(tmpdir, ignore_errors=True)
        elif not args.keep_tmp and args.tmpdir:
            pass  # user-owned tmpdir: leave for inspection, mention it
        if args.tmpdir and not args.keep_tmp:
            log(f"scratch kept at {tmpdir} (user --tmpdir; delete when done)")


if __name__ == "__main__":
    main()
