#!/usr/bin/env python3

"""OrangeFox pixels source patch launcher (files/ based)."""

from __future__ import annotations

import argparse
import json
import os
import sys
from pathlib import Path
from typing import Iterable


sys.dont_write_bytecode = True

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
DEFAULT_CONFIG = PATCHES_DIR / "config.json"


def _patch_id_for(rel: str, prefix: str = "snap") -> str:
    return prefix + "-" + rel.replace("/", "-").replace(".", "-").replace("_", "-")


def from_storage_name(stored: str) -> str:
    """Map a stored file name to the real tree-relative path.

    *.bp / *.mk are stored with a +.bak suffix so the Soong/make
    scanners (PRODUCT_SOONG_NAMESPACES covers the whole device tree,
    and it has no subdirectory exclude) never see them as build files.
    """
    if stored.endswith(".bak") and stored[:-4].endswith((".bp", ".mk")):
        return stored[:-4]
    return stored


def load_patches() -> list:
    from patchlib import NewFilePatch, SnapshotPatch

    patches: list = []

    if MODIFIED_DIR.exists():
        for mod_file in sorted(p for p in MODIFIED_DIR.rglob("*") if p.is_file()):
            stored = mod_file.relative_to(MODIFIED_DIR).as_posix()
            rel = from_storage_name(stored)
            orig_file = ORIGINAL_DIR / stored
            patch_file = UNIFIED_DIR / f"{rel}.patch"
            patches.append(
                SnapshotPatch(
                    patch_id=_patch_id_for(rel),
                    title=f"Snapshot {rel}",
                    target=rel,
                    original_file=orig_file,
                    modified_file=mod_file,
                    patch_file=patch_file if patch_file.exists() else None,
                )
            )

    if NEW_DIR.exists():
        for new_file in sorted(p for p in NEW_DIR.rglob("*") if p.is_file()):
            stored = new_file.relative_to(NEW_DIR).as_posix()
            rel = from_storage_name(stored)
            # Skip files that are also covered as modified (should not happen).
            if (MODIFIED_DIR / stored).exists():
                continue
            patches.append(
                NewFilePatch(
                    patch_id=_patch_id_for(rel, prefix="new"),
                    title=f"New file {rel}",
                    target=rel,
                    source_file=new_file,
                )
            )

    return patches


def load_config(path: Path) -> dict:
    if not path.exists():
        return {"version": 1, "bypass_paths": [], "patches": {}}
    with path.open("r", encoding="utf-8") as config_file:
        data = json.load(config_file)
    data.setdefault("version", 1)
    data.setdefault("bypass_paths", [])
    data.setdefault("patches", {})
    return data


def save_config(path: Path, config: dict) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("w", encoding="utf-8") as config_file:
        json.dump(config, config_file, indent=2, ensure_ascii=True)
        config_file.write("\n")


def configure_bypass(config_path: Path, patch_id: str, enabled: bool) -> None:
    config = load_config(config_path)
    patch_config = config.setdefault("patches", {}).setdefault(patch_id, {})
    patch_config["mode"] = "bypass" if enabled else "apply"
    save_config(config_path, config)
    state = "bypass" if enabled else "apply"
    print(f"Configured {patch_id}: mode={state}")


def print_patch_list(patches: Iterable, config: dict, root: Path) -> None:
    from patchlib import PatchContext

    context = PatchContext(
        root=root,
        config=config,
        apply=False,
        apply_bypassed=False,
        verbose=False,
    )
    for patch in patches:
        bypass = "bypass" if context.should_bypass(patch) else "apply"
        print(f"{patch.id:70} {bypass:7} {patch.title}")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Apply OrangeFox pixels source patches from patches/files.",
    )
    parser.add_argument(
        "--root",
        type=Path,
        default=DEFAULT_ROOT,
        help="Android source tree root. Defaults to the detected workspace root.",
    )
    parser.add_argument(
        "--config",
        type=Path,
        default=DEFAULT_CONFIG,
        help="Patch configurator JSON.",
    )
    parser.add_argument(
        "--check",
        action="store_true",
        help="Dry-run mode. This is also the default when --apply is not passed.",
    )
    parser.add_argument(
        "--apply",
        action="store_true",
        help="Write changes. Without this flag the launcher only reports what would happen.",
    )
    parser.add_argument(
        "--apply-bypassed",
        action="store_true",
        help="Apply patches even when their config mode is bypass.",
    )
    parser.add_argument(
        "--list",
        action="store_true",
        help="List known patches and effective modes.",
    )
    parser.add_argument(
        "--configure-bypass",
        nargs=2,
        metavar=("PATCH_ID", "on|off"),
        help="Set a patch mode in config.json. 'on' means bypass, 'off' means apply.",
    )
    parser.add_argument(
        "--only",
        action="append",
        default=None,
        help="Apply/check only matching target path (repeatable, substring match).",
    )
    parser.add_argument(
        "--verbose",
        action="store_true",
        help="Print extra details for failed patches.",
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    config_path = args.config.resolve()

    if args.configure_bypass:
        patch_id, state = args.configure_bypass
        normalized = state.lower()
        if normalized not in {"on", "off", "true", "false", "1", "0"}:
            print("--configure-bypass state must be on/off", file=sys.stderr)
            return 2
        configure_bypass(config_path, patch_id, normalized in {"on", "true", "1"})
        return 0

    config = load_config(config_path)
    patches = load_patches()
    if args.only:
        patches = [p for p in patches if any(s in p.target for s in args.only)]
    root = args.root.resolve()

    if args.list:
        print_patch_list(patches, config, root)
        print(f"\nTotal: {len(patches)} (modified={sum(1 for p in patches if p.id.startswith('snap-'))}, "
              f"new={sum(1 for p in patches if p.id.startswith('new-'))})")
        print(f"Sources: {MODIFIED_DIR} / {ORIGINAL_DIR} / {NEW_DIR} / {UNIFIED_DIR}")
        return 0

    from patchlib import PatchContext

    context = PatchContext(
        root=root,
        config=config,
        apply=args.apply,
        apply_bypassed=args.apply_bypassed,
        verbose=args.verbose,
    )

    results = [patch.run(context) for patch in patches]
    for result in results:
        print(result.format())

    failed = [result for result in results if result.status == "failed"]
    if failed:
        print("\nManual action required:", file=sys.stderr)
        for result in failed:
            print(f"- {result.id}: {result.manual_hint}", file=sys.stderr)
            if args.verbose and result.details:
                print(result.details, file=sys.stderr)
        return 1

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
