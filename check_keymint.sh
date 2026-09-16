#!/bin/bash
# test_check_keymint.sh — analyzer: which KeyMint HAL (rust|cpp) does a
# Google factory image need?
#
# Usage:
#   ./test_check_keymint.sh <factory_rom.zip> [--expect rust|cpp]
#
# Google factory layout is a nested zip: outer <device>-factory-*.zip
# contains image-<device>-*.zip which contains vendor.img. The script
# extracts (outer -> inner zip -> vendor.img), loop-mounts vendor.img
# read-only via sudo, inspects /bin/hw for the keymint service binaries,
# then unmounts and deletes everything it created.
#
# Evidence collected (no device codename knowledge required):
#   1. /bin/hw/android.hardware.security.keymint-service.rust.trusty -> rust
#      /bin/hw/android.hardware.security.keymint-service.trusty (exact) -> cpp
#      (citadel/strongbox binaries are a different chip and are ignored)
#   2. vendor/etc/init/*keymint*.rc service declarations (supporting)
#   3. Rust source marker (keymint_hal_main) inside the binary (supporting)
#   4. build.prop fingerprint/device (context only)
#
# Workdir: first of $KEYMINT_TMPDIR, $PWD, $HOME with enough free space
# (needs inner.zip + vendor.img + margin; /tmp is often a small tmpfs).
# A stale workdir from a killed run is removed at startup (with umount).
set -u

if [[ $# -lt 1 ]]; then
    echo "Usage: $0 <factory_rom.zip> [--expect rust|cpp]"
    exit 2
fi

exec python3 - "$@" <<'PYEOF'
import hashlib
import os
import re
import shutil
import subprocess
import sys
import tempfile
import zipfile
import xml.etree.ElementTree as ET

RUST_BIN = "android.hardware.security.keymint-service.rust.trusty"
CPP_BIN = "android.hardware.security.keymint-service.trusty"  # exact name only
RUST_MARKER = b"keymint_hal_main"      # system/core/trusty/keymint/src/...
CPP_MARKER = b"TrustyKeymaster"        # system/core/trusty/keymaster/...
WORKDIR_NAME = ".keymint_check_work"


def log(msg):
    print(msg, flush=True)


def sh(cmd, **kw):
    """Run cmd, return CompletedProcess (text). Raises on failure."""
    return subprocess.run(cmd, check=True, text=True,
                          stdout=subprocess.PIPE, stderr=subprocess.PIPE, **kw)


def workdir_for(need_bytes):
    cands = [os.environ.get("KEYMINT_TMPDIR", ""), os.getcwd(),
             os.path.expanduser("~"), "/tmp"]
    for c in cands:
        if c and os.path.isdir(c):
            if shutil.disk_usage(c).free > need_bytes:
                return c
    raise SystemExit("ERROR: no workdir with %.1f GB free" % (need_bytes / 1e9))


def main():
    args = sys.argv[1:]
    expect = None
    if "--expect" in args:
        i = args.index("--expect")
        expect = args[i + 1]
        del args[i:i + 2]
        if expect not in ("rust", "cpp"):
            raise SystemExit("ERROR: --expect needs rust|cpp")
    if len(args) != 1:
        raise SystemExit("Usage: test_check_keymint.sh <factory_rom.zip> [--expect rust|cpp]")
    outer_path = args[0]
    if not os.path.isfile(outer_path):
        raise SystemExit("ERROR: not a file: %s" % outer_path)

    with zipfile.ZipFile(outer_path) as zf:
        outer_names = zf.namelist()
    inner_names = [n for n in outer_names
                   if os.path.basename(n).startswith("image-") and n.endswith(".zip")]
    direct_vendor = [n for n in outer_names if os.path.basename(n) == "vendor.img"]
    if inner_names:
        inner_name = inner_names[0]
        with zipfile.ZipFile(outer_path) as zf:
            inner_size = zf.getinfo(inner_name).file_size
        log("[1/6] outer zip OK, inner: %s (%.1f GB)" % (inner_name, inner_size / 1e9))
        nested = True
    elif direct_vendor:
        inner_name, inner_size, nested = None, 0, False
        log("[1/6] outer zip holds vendor.img directly")
    else:
        raise SystemExit("ERROR: no image-*.zip and no vendor.img in %s" % outer_path)

    base = workdir_for(inner_size + 3 * 1024**3)
    work = os.path.join(base, WORKDIR_NAME)
    if os.path.exists(work):  # stale workdir from a killed run: reclaim it
        log("(!) removing stale workdir from a previous run")
        subprocess.run(["sudo", "umount", os.path.join(work, "mnt")],
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        shutil.rmtree(work, ignore_errors=True)
    os.makedirs(os.path.join(work, "mnt"))
    mnt = os.path.join(work, "mnt")
    mounted = False
    try:
        if nested:
            inner_path = os.path.join(work, "inner.zip")
            log("[2/6] extracting inner zip (%.1f GB, takes a while) ..."
                % (inner_size / 1e9))
            with zipfile.ZipFile(outer_path) as zf:
                with zf.open(inner_name) as src, open(inner_path, "wb") as dst:
                    shutil.copyfileobj(src, dst, length=64 * 1024**2)
            inner_zf_path = inner_path
        else:
            inner_zf_path = outer_path

        with zipfile.ZipFile(inner_zf_path) as zf:
            vendors = [n for n in zf.namelist() if os.path.basename(n) == "vendor.img"]
            if not vendors:
                imgs = sorted(n for n in zf.namelist() if n.endswith(".img"))
                raise SystemExit("ERROR: no vendor.img inside; images present:\n  "
                                 + "\n  ".join(imgs[:30]))
            vendor_name = vendors[0]
            vendor_size = zf.getinfo(vendor_name).file_size
        vendor_path = os.path.join(work, "vendor.img")
        log("[3/6] extracting %s (%.1f GB) ..." % (vendor_name, vendor_size / 1e9))
        with zipfile.ZipFile(inner_zf_path) as zf:
            with zf.open(vendor_name) as src, open(vendor_path, "wb") as dst:
                shutil.copyfileobj(src, dst, length=64 * 1024**2)

        log("[4/6] mounting vendor.img read-only (sudo) ...")
        sh(["sudo", "mount", "-r", "-o", "loop", vendor_path, mnt])
        mounted = True

        log("[5/6] inspecting ...")
        hw = subprocess.run(["sudo", "ls", "-1", mnt + "/bin/hw"],
                            check=True, text=True,
                            stdout=subprocess.PIPE).stdout.split()
        keymint_bins = sorted(n for n in hw if "keymint" in n)
        log("  /bin/hw keymint files: %s" % (", ".join(keymint_bins) or "(none)"))
        rust_present = RUST_BIN in hw
        cpp_present = CPP_BIN in hw  # exact match; rust/citadel names differ

        ev = os.path.join(work, "ev")
        os.makedirs(ev)

        def sget(src, dst_name):
            """sudo-copy a small file out of the mount for normal reading."""
            dst = os.path.join(ev, dst_name)
            sh(["sudo", "cp", mnt + src, dst])
            sh(["sudo", "chown", "%d:%d" % (os.getuid(), os.getgid()), dst])
            return dst

        info = {}
        try:
            with open(sget("/build.prop", "build.prop"), errors="replace") as f:
                props = dict(l.split("=", 1) for l in
                             (x.strip() for x in f if "=" in x and not x.startswith("#")))
            info["fingerprint"] = props.get("ro.vendor.build.fingerprint", "?")
            info["device"] = props.get("ro.product.vendor.device",
                                       props.get("ro.product.device", "?"))
        except subprocess.CalledProcessError:
            pass
        log("  fingerprint: %s" % info.get("fingerprint", "?"))
        log("  device: %s" % info.get("device", "?"))

        try:
            rc_list = subprocess.run(
                ["sudo", "sh", "-c", "ls %s/etc/init/ | grep -i keymint" % mnt],
                check=True, text=True, stdout=subprocess.PIPE).stdout.split()
        except subprocess.CalledProcessError:
            rc_list = []
        services = []
        for rc in rc_list:
            try:
                with open(sget("/etc/init/" + rc, "rc_" + rc), errors="replace") as f:
                    for line in f:
                        m = re.match(r"\s*service\s+(\S+)\s+(\S+)", line)
                        if m:
                            services.append("%s -> %s" % (m.group(1), m.group(2)))
            except subprocess.CalledProcessError:
                pass
        log("  keymint services: %s" % ("; ".join(services) or "(none)"))

        markers = {}
        for tag, name, marker in (("rust", RUST_BIN, RUST_MARKER),
                                  ("cpp", CPP_BIN, CPP_MARKER)):
            if (tag == "rust" and rust_present) or (tag == "cpp" and cpp_present):
                dst = sget("/bin/hw/" + name, name)
                h = hashlib.md5()
                found = False
                with open(dst, "rb") as f:
                    while True:
                        chunk = f.read(8 * 1024**2)
                        if not chunk:
                            break
                        h.update(chunk)
                        if marker in chunk:
                            found = True
                size = os.path.getsize(dst)
                markers[tag] = (size, h.hexdigest(), found)
                log("  %s binary: %d bytes md5=%s source-marker=%s"
                    % (tag, size, h.hexdigest(), "yes" if found else "no"))

        try:
            with open(sget("/etc/vintf/manifest.xml", "manifest.xml"),
                      errors="replace") as f:
                root = ET.fromstring(f.read())
            hals = []
            for hal in root.iter("hal"):
                name = hal.findtext("name", "")
                if "keymint" in name.lower():
                    hals.append("%s v%s %s" % (
                        name, hal.findtext("version", "?"),
                        ",".join(x.text or "" for x in hal.findall("fqname"))))
            log("  vintf keymint HALs: %s" % ("; ".join(hals) or "(none)"))
        except (subprocess.CalledProcessError, ET.ParseError):
            pass

        log("[6/6] verdict:")
        if rust_present and not cpp_present:
            verdict = "rust"
        elif cpp_present and not rust_present:
            verdict = "cpp"
        elif rust_present and cpp_present:
            verdict = "CONFLICT (both binaries present)"
        else:
            verdict = "UNKNOWN (no keymint-service HAL binary found)"
        log("  KEYMINT=%s" % verdict)

        rc = 0
        if expect:
            match = (verdict == expect)
            log("  expected=%s -> %s" % (expect, "MATCH" if match else "MISMATCH"))
            rc = 0 if match else 1
        elif verdict not in ("rust", "cpp"):
            rc = 2
        return rc
    finally:
        if mounted:
            subprocess.run(["sudo", "umount", mnt],
                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        shutil.rmtree(work, ignore_errors=True)
        log("(cleanup: unmounted and removed workdir)")


if __name__ == "__main__":
    sys.exit(main())
PYEOF
