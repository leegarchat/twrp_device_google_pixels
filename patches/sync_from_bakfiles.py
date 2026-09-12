#!/usr/bin/env python3
"""Sync patches/files/* from a bakFiles snapshot (made by sync_tree.py -s).

Usage:
  python3 patches/sync_from_bakfiles.py [--snapshot 20260905_085323|latest] [--regen-patches] [--prune]
  python3 patches/sync_from_bakfiles.py --list

Layout produced (stored names; *.bp/*.mk keep a +.bak suffix so the
Soong/make scanners never treat them as build files):
  patches/files/modified/<stored>   modified tree files
  patches/files/original/<stored>   clean HEAD copies
  patches/files/new/<stored>        untracked/new files
  patches/files/patches/<rel>.patch  unified diffs for manual apply
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
from pathlib import Path

PATCHES_DIR = Path(__file__).resolve().parent
FILES_DIR = PATCHES_DIR / "files"
MODIFIED_DIR = FILES_DIR / "modified"
ORIGINAL_DIR = FILES_DIR / "original"
NEW_DIR = FILES_DIR / "new"
UNIFIED_DIR = FILES_DIR / "patches"


def _find_android_root() -> Path:
    env_top = os.environ.get("ANDROID_BUILD_TOP")
    if env_top and Path(env_top).is_dir():
        return Path(env_top).resolve()
    curr = Path(__file__).resolve().parent
    while curr != curr.parent:
        if (curr / ".repo").is_dir() or (curr / "build" / "envsetup.sh").exists():
            return curr
        curr = curr.parent
    return Path(__file__).resolve().parents[4]


DEFAULT_ROOT = _find_android_root()
DEFAULT_BAK = DEFAULT_ROOT / "bakFiles" / "snapshots"


def from_storage_name(stored: str) -> str:
    if stored.endswith(".bak") and stored[:-4].endswith((".bp", ".mk")):
        return stored[:-4]
    return stored


def to_storage_name(rel: str) -> str:
    """Stored file name inside patches/files.

    *.bp / *.mk are kept with a +.bak suffix so the Soong/make scanners
    (PRODUCT_SOONG_NAMESPACES covers the whole device tree and has no
    subdirectory exclude) never see them as build files.
    """
    return rel + ".bak" if rel.endswith((".bp", ".mk")) else rel


def latest_snapshot(snap_root: Path) -> Path | None:
    snaps = sorted([d for d in snap_root.iterdir() if d.is_dir()], key=lambda d: d.name)
    return snaps[-1] if snaps else None


def resolve_snapshot(ref: str, snap_root: Path) -> Path:
    if ref in ("latest", "last"):
        snap = latest_snapshot(snap_root)
        if snap is None:
            raise SystemExit(f"No snapshots in {snap_root}")
        return snap
    cand = Path(ref)
    if cand.is_dir():
        return cand.resolve()
    cand2 = snap_root / ref
    if cand2.is_dir():
        return cand2
    raise SystemExit(f"Snapshot not found: {ref} (looked in {snap_root})")


def copy_tree_flat(src_root: Path, dst_root: Path) -> list[str]:
    """Copy src -> dst keeping stored (.bak) names. Returns stored rel paths."""
    out: list[str] = []
    if not src_root.exists():
        return out
    for f in sorted(p for p in src_root.rglob("*") if p.is_file()):
        rel_stored = f.relative_to(src_root).as_posix()
        target = dst_root / rel_stored
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(f, target)
        out.append(rel_stored)
    return out


def sync_snapshot(snap: Path, regen_patches: bool, prune: bool) -> dict:
    for d in (MODIFIED_DIR, ORIGINAL_DIR, NEW_DIR, UNIFIED_DIR):
        d.mkdir(parents=True, exist_ok=True)

    modified = copy_tree_flat(snap / "modified", MODIFIED_DIR)
    original = copy_tree_flat(snap / "original", ORIGINAL_DIR)

    # new files: new layout (new_files/) + legacy (new_files_temp/)
    # stored with the same .bak rule as modified/original.
    new_files: list[str] = []
    for cand in (snap / "new_files", snap / "new_files_temp"):
        for f in sorted(p for p in cand.rglob("*") if p.is_file()) if cand.exists() else []:
            rel = f.relative_to(cand).as_posix()
            stored = to_storage_name(rel)
            target = NEW_DIR / stored
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(f, target)
            new_files.append(stored)

    # unified patches: copy as-is (names already normalized in snapshot)
    patch_files: list[str] = []
    src_patches = snap / "patches"
    if src_patches.exists():
        for f in sorted(src_patches.rglob("*.patch")):
            rel = f.relative_to(src_patches).as_posix()
            target = UNIFIED_DIR / rel
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(f, target)
            patch_files.append(rel)

    # regenerate missing patches from original/modified pairs.
    # .patch files keep real-rel naming (<real>.patch); the pair itself
    # lives under stored names.
    if regen_patches:
        for stored in modified:
            rel = from_storage_name(stored)
            patch_path = UNIFIED_DIR / f"{rel}.patch"
            if patch_path.exists():
                continue
            orig = ORIGINAL_DIR / stored
            mod = MODIFIED_DIR / stored
            if orig.exists() and mod.exists():
                _make_patch(orig, mod, patch_path, rel)
                patch_files.append(patch_path.relative_to(UNIFIED_DIR).as_posix())

    if prune:
        _prune_extra(MODIFIED_DIR, set(modified), skip_suffixes=())
        _prune_extra(ORIGINAL_DIR, set(original), skip_suffixes=())
        _prune_extra(NEW_DIR, set(new_files), skip_suffixes=())
        want_patches = {f"{from_storage_name(s)}.patch" for s in modified}
        _prune_extra(UNIFIED_DIR, want_patches, skip_suffixes=())

    # provenance
    manifest = snap / "manifest.json"
    if manifest.exists():
        shutil.copy2(manifest, FILES_DIR / "source_snapshot.json")
        try:
            data = json.loads(manifest.read_text(encoding="utf-8"))
            (FILES_DIR / "source_snapshot.txt").write_text(
                f"snapshot={snap.name}\nmode={data.get('mode')}\ncreated={data.get('created')}\n",
                encoding="utf-8",
            )
        except Exception:
            pass

    return {
        "snapshot": snap.name,
        "modified": len(modified),
        "original": len(original),
        "new": len(new_files),
        "patches": len(patch_files),
    }


def _prune_extra(root: Path, keep: set[str], skip_suffixes: tuple = ()) -> int:
    removed = 0
    for f in [p for p in root.rglob("*") if p.is_file()]:
        rel = f.relative_to(root).as_posix()
        if rel in ("source_snapshot.json", "source_snapshot.txt"):
            continue
        if rel not in keep:
            f.unlink()
            removed += 1
    # remove empty dirs
    for d in sorted([p for p in root.rglob("*") if p.is_dir()], reverse=True):
        try:
            next(d.iterdir())
        except StopIteration:
            d.rmdir()
    return removed


def _make_patch(orig: Path, mod: Path, patch_path: Path, rel: str) -> bool:
    res = subprocess.run(
        ["diff", "-u", "--label", f"a/{rel}", "--label", f"b/{rel}", str(orig), str(mod)],
        capture_output=True,
    )
    if res.returncode == 1 and res.stdout:
        patch_path.parent.mkdir(parents=True, exist_ok=True)
        patch_path.write_bytes(res.stdout)
        return True
    return False


def main() -> int:
    parser = argparse.ArgumentParser(description="Sync patches/files from bakFiles snapshot")
    parser.add_argument("--snapshot", default="latest", help="Snapshot name, path, or 'latest'")
    parser.add_argument("--snap-root", type=Path, default=DEFAULT_BAK)
    parser.add_argument("--regen-patches", action="store_true", help="Generate missing .patch files")
    parser.add_argument("--prune", action="store_true", help="Delete files in patches/files not in snapshot")
    parser.add_argument("--list", action="store_true", help="List available snapshots and exit")
    args = parser.parse_args()

    snap_root: Path = args.snap_root
    if args.list:
        if not snap_root.exists():
            print(f"No snap root: {snap_root}")
            return 1
        for d in sorted(snap_root.iterdir()):
            if d.is_dir():
                print(d.name)
        return 0

    snap = resolve_snapshot(args.snapshot, snap_root)
    print(f"Snapshot: {snap}")
    stats = sync_snapshot(snap, regen_patches=args.regen_patches, prune=args.prune)
    print(f"Synced -> {FILES_DIR}: modified={stats['modified']} original={stats['original']} "
          f"new={stats['new']} patches={stats['patches']}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
